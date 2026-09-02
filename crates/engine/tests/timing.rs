//! **Arnés de tiempos del motor** — la línea base contra la que se mide el refactor 5.0.0.
//!
//! Los tres tests van `#[ignore]` a propósito: miden, no afirman. Un test que falla porque una
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

use cases::projection_cases_all;
use futurefin_engine::{project_net_worth_series, ProjectionInput};
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
