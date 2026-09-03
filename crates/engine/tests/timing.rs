//! **Arnés de tiempos del motor** — la línea base contra la que se mide el refactor 5.0.0.
//!
//! Los tests van `#[ignore]` (recuéntalos: `grep -c "#\[ignore\]" crates/engine/tests/timing.rs`) a propósito: miden, no afirman. Un test que falla porque una
//! máquina va lenta enseña a ignorar el CI, y aquí no hay ningún umbral defendible (el número
//! depende del portátil, de la carga y del perfil de compilación).
//!
//! ```text
//! cargo test -p futurefin-engine --release --test timing -- --ignored --nocapture
//! cargo test -p futurefin-engine           --test timing -- --ignored --nocapture   # debug
//! ```
//!
//! **Corre siempre en RELEASE para decidir nada.** El motor es aritmética `Decimal` encadenada:
//! en `debug` cada operación pasa por los `checked_*` sin optimizar y el factor contra `release`
//! es de un orden de magnitud largo. Un número de `debug` solo sirve para compararlo con otro
//! número de `debug`.
//!
//! El caso medido es **P9** (`tests/common/cases.rs`), el más caro de la batería y el único
//! representativo de un hogar real: 840 meses, 5 activos (cuatro con `powd` mensual), 2 pasivos,
//! cascada de 3 reglas con tope, planning flows, objetivo FIRE con pensión, impuestos ES por
//! tramos e inflación — es decir, todos los bucles calientes a la vez.
//!
//! Sin crate de benchmark a propósito: `Instant` + `black_box` basta para el orden de magnitud
//! que interesa (¿una proyección son microsegundos o decenas de milisegundos?), y añadir
//! `criterion` metería una dependencia de desarrollo enorme para afinar un número que ninguna
//! decisión de este WP necesita al 1 %.

#[path = "common/cases.rs"]
mod cases;

use cases::{projection_cases_5_0, projection_cases_all};
use futurefin_engine::{
    project_net_worth_series, required_contribution_monthly, PensionSchedule, ProjectionInput,
    SimInput, TargetBasis,
};
use rust_decimal::Decimal;
use std::hint::black_box;
use std::time::Instant;

fn p9() -> ProjectionInput {
    projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 debe existir en la batería")
        .input
}

/// Corre `n` proyecciones sobre el MISMO input (ya construido: se mide el motor, no el montaje
/// del caso) e imprime el total y la media.
fn measure(label: &str, n: u32, input: &ProjectionInput) {
    // Una pasada en frío fuera del reloj: la primera toca páginas y calienta el asignador.
    let warm = project_net_worth_series(input).expect("P9 no debe fallar");
    assert_eq!(warm.net_worth.len(), input.horizon_months as usize + 1);

    let t0 = Instant::now();
    for _ in 0..n {
        let out = project_net_worth_series(black_box(input)).expect("P9 no debe fallar");
        black_box(out);
    }
    let elapsed = t0.elapsed();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "[timing/{profile}] {label}: {n} proyección(es) de {} meses en {:.3} ms  ({:.3} ms/proyección)",
        input.horizon_months,
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1000.0 / f64::from(n),
    );
}

/// (a) Una proyección de P9 a 840 meses: el coste de UN `GET /v1/projection/series` con cache
/// fría, que es el techo que el usuario percibe.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn one_p9_projection_at_840_months() {
    measure("una proyección", 1, &p9());
}

/// (b) 24 proyecciones consecutivas: el coste de una **bisección** — resolver «¿qué aportación
/// mensual hace falta para jubilarse en el mes X?» son ~24 evaluaciones del motor (2⁻²⁴ del
/// intervalo de búsqueda). Es la unidad de trabajo que cualquier solver futuro va a pagar.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn twenty_four_p9_projections_the_cost_of_a_bisection() {
    measure("bisección (24 iteraciones)", 24, &p9());
}

/// (c) 100 proyecciones: base para extrapolar (Monte Carlo, barridos de sensibilidad, warm-up
/// de cache de varias vistas). Con 100 muestras el ruido de una sola pasada deja de mandar.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn one_hundred_p9_projections_for_extrapolation() {
    measure("100 proyecciones", 100, &p9());
}

fn case_5_0(name: &'static str) -> ProjectionInput {
    projection_cases_5_0()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} debe existir en la batería de 5.0.0"))
        .input
}

/// (d) **El coste del objetivo PUENTE** (WP3, §B.3). P18 tabula `P = 240` gross-ups y 241
/// potencias UNA vez y luego consulta en `O(1)` por mes. Comparado con (a) —P9, que no tiene
/// puente— dice cuánto cuesta esa tabla de verdad.
///
/// La alternativa que NO se implementó es la suma directa `O(P−i)` por evaluación: `O(P²)` con un
/// `gross_up` y una potencia por término, es decir ~29.000 gross-ups solo para P18 (y ~350.000 con
/// una pensión a 70 años). Este test es la medición que respalda haber precomputado.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn one_p18_bridge_projection() {
    measure("proyección con puente (P = 240)", 1, &case_5_0("P18_pension_bridge"));
    measure("100 × puente (P = 240)", 100, &case_5_0("P18_pension_bridge"));
}

/// (e) **El puente más caro representable**: `P = 840`, el horizonte completo. Es la cota superior
/// del coste de la tabla — más allá de `MAX_BRIDGE_MONTHS` el motor degrada a perpetuidad.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn one_bridge_projection_with_the_pension_at_the_horizon() {
    let mut input = case_5_0("P18_pension_bridge");
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 840,
        ..input.phase_plan.pension.expect("P18 tiene pensión")
    });
    input.phase_plan.target_basis = TargetBasis::BridgeToPension;
    measure("puente con P = 840", 100, &input);
}

/// (f) **El coste de un SOLVE** (§B.7), medido de verdad y no extrapolado: la bisección de
/// `required_contribution_monthly` sobre P9 —el caso caro— con jubilación por edad.
///
/// Es la unidad que el handler pagará una vez por proyección y guardará en la cache (M4). El
/// presupuesto es `MAX_SOLVE_ITERATIONS + 3` proyecciones (24 de bisección más las dos sondas de
/// los extremos y el `first_month_allocation` del sobrante), así que (b) es su cota inferior.
///
/// **P9 con el SWR forzado al 20 %**, y hay que decirlo: con su SWR real P9 no alcanza su
/// objetivo en NINGÚN mes del horizonte (líquido 725 k€ contra 3,07 M€ en el mes 600), así que el
/// solve cortocircuita en la sonda del extremo alto y devuelve el sobrante sin biseccionar — 2
/// proyecciones, no 26. Subir el SWR baja el listón sin tocar nada más (misma fiscalidad, misma
/// cascada, mismos pasivos, mismo horizonte): es la forma honesta de medir las 24 iteraciones
/// sobre el caso caro en vez de sobre un laboratorio de juguete.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn one_required_contribution_solve_on_p9() {
    let mut input = p9();
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.retirement_trigger = futurefin_engine::RetirementTrigger::AtMonth(600);
    if let Some(ft) = input.fire_target.as_mut() {
        ft.swr_pct = Decimal::from(20u32);
    }

    let warm = required_contribution_monthly(&input, 600).expect("el solve no falla");
    let iterations = warm.as_ref().map(|r| r.iterations).unwrap_or(0);
    let contribution = warm
        .as_ref()
        .map(|r| r.contribution)
        .unwrap_or(Decimal::ZERO);

    let t0 = Instant::now();
    let out = required_contribution_monthly(black_box(&input), 600).expect("el solve no falla");
    let elapsed = t0.elapsed();
    black_box(out);
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "[timing/{profile}] solve required_contribution (P9, 840 meses, {iterations} iteraciones): \
         {:.3} ms  ⇒ c = {contribution}",
        elapsed.as_secs_f64() * 1000.0,
    );
}

/// (g) **El peaje del núcleo genérico** (WP5.5): lo único que `project_net_worth_series` hace de
/// más desde que el bucle es genérico es convertir su entrada al tipo del núcleo — una copia
/// campo a campo, sin una sola operación aritmética. Se mide aparte porque «no se nota» es una
/// afirmación, y las afirmaciones de rendimiento de esta casa llevan número.
///
/// P9 es el caso caro: 840 ajustes de planning, 841 términos de deuda, 5 activos, 2 pasivos y 3
/// reglas. Compárese con (a): si la conversión costara una fracción apreciable de los ~12 ms de
/// una proyección, el envoltorio no sería gratis y habría que replantear la frontera.
#[test]
#[ignore = "mide, no afirma: correr con --ignored --nocapture"]
fn the_cost_of_converting_the_input_to_the_generic_core() {
    let input = p9();
    let n = 1_000u32;
    let warm = SimInput::<Decimal>::from(&input);
    black_box(&warm);
    let t0 = Instant::now();
    for _ in 0..n {
        let sim = SimInput::<Decimal>::from(black_box(&input));
        black_box(&sim);
    }
    let elapsed = t0.elapsed();
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "[timing/{profile}] conversión ProjectionInput → SimInput (P9, 840 meses): \
         {n} conversiones en {:.3} ms  ({:.4} ms/conversión)",
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1000.0 / f64::from(n),
    );
}
