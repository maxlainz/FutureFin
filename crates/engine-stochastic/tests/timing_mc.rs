//! **Arnés de tiempos de Monte Carlo** (WP6) — el gemelo de `crates/engine/tests/timing.rs`, con
//! las mismas reglas de la casa:
//!
//! - los tests van `#[ignore]` porque **miden, no afirman**: un test que falla porque la máquina
//!   va lenta enseña a ignorar el CI, y aquí no hay umbral defendible;
//! - **se corren en RELEASE o no significan nada**: el bucle es aritmética encadenada y en `debug`
//!   el factor es de un orden de magnitud largo.
//!
//! ```text
//! cargo test -p futurefin-engine-stochastic --release --test timing_mc -- --ignored --nocapture
//! ```
//!
//! El caso medido es **P9** (840 meses, 5 activos, 2 pasivos, cascada de 3 reglas con tope,
//! planning flows, objetivo FIRE con pensión, impuestos ES por tramos e inflación): el mismo con
//! el que WP0 midió los ~12,6 ms de una proyección `Decimal`, para que los dos números se puedan
//! poner uno al lado del otro sin traducir nada.

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::projection_cases_all;
use futurefin_engine::{project_net_worth_series, ProjectionInput};
use futurefin_engine_stochastic::{
    project_percentile_bands, run_path, simulate_f64, CashBufferSpec, McConfig, DEFAULT_PATHS,
};
use std::hint::black_box;
use std::time::Instant;

fn p9() -> ProjectionInput {
    projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 debe existir en la batería")
        .input
}

/// Volatilidades realistas de P9: cuenta 0 · bonos 5 % · RV 16 % · vivienda 8 % · cripto 70 %.
fn vols() -> Vec<Option<f64>> {
    vec![None, Some(5.0), Some(16.0), Some(8.0), Some(70.0)]
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// (a) **La referencia**: una proyección `Decimal` y una `f64` del mismo caso. Es el cociente que
/// justifica el crate entero — si `f64` no fuera varias veces más barato, Monte Carlo se habría
/// hecho en `Decimal` y no habría hecho falta ni newtype ni trait.
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn one_projection_decimal_vs_f64() {
    let input = p9();
    let n = 100u32;
    black_box(project_net_worth_series(&input).expect("no falla"));
    let t0 = Instant::now();
    for _ in 0..n {
        black_box(project_net_worth_series(black_box(&input)).expect("no falla"));
    }
    let dec = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);

    black_box(simulate_f64(&input).expect("no falla"));
    let t0 = Instant::now();
    for _ in 0..n {
        black_box(simulate_f64(black_box(&input)).expect("no falla"));
    }
    let flo = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);

    println!(
        "[mc-timing/{}] P9 840 meses · Decimal = {dec:.3} ms/proyección · f64 = {flo:.3} ms/proyección \
         ⇒ {:.1}× más barato",
        profile(),
        dec / flo
    );
}

/// (b) **El coste de UN camino**, con el sorteo dentro: `run_path` reconstruye la maquinaria en
/// cada llamada (conversión de la entrada + buffer de 840×5), así que este número es la cota
/// SUPERIOR del coste por camino. El de dentro de una ejecución completa —donde todo eso se
/// reutiliza— es el de (c).
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn one_monte_carlo_path() {
    let input = p9();
    let v = vols();
    let config = McConfig::default();
    let n = 100u32;
    black_box(run_path(&input, &v, &config, 0).expect("no falla"));
    let t0 = Instant::now();
    for p in 0..n {
        black_box(run_path(black_box(&input), &v, &config, p).expect("no falla"));
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    println!(
        "[mc-timing/{}] P9 · `run_path` (maquinaria reconstruida cada vez): {ms:.3} ms/camino",
        profile()
    );
}

/// (c) **La ejecución que el endpoint va a servir**: 500 caminos de P9 con bandas p10/p50/p90.
///
/// Se imprimen el total, el coste amortizado por camino y —lo que de verdad interesa para el
/// presupuesto de WP6b— **cuánto de ese total es agregación** (ordenar 841 vectores de 500) y no
/// simulación. La memoria de las muestras se calcula, no se estima:
/// `2 · caminos · (horizonte+1) · 8 bytes`.
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn five_hundred_paths_of_p9() {
    let input = p9();
    let v = vols();
    for paths in [100u32, DEFAULT_PATHS, 1_000, 2_000] {
        let config = McConfig {
            seed: 20_260_903,
            paths,
            ..Default::default()
        };
        let t0 = Instant::now();
        let out = project_percentile_bands(&input, &v, &config).expect("no falla");
        let total = t0.elapsed().as_secs_f64() * 1000.0;
        let samples_mb =
            2.0 * f64::from(paths) * f64::from(input.horizon_months + 1) * 8.0 / 1_048_576.0;
        println!(
            "[mc-timing/{}] P9 840 meses · {paths:>4} caminos: {total:>9.1} ms total \
             ({:.3} ms/camino) · muestras {samples_mb:.1} MB · éxito {:.3}",
            profile(),
            total / f64::from(paths),
            out.success_probability,
        );
        black_box(out);
    }
}

/// (d) **Cuánto cuesta el sorteo frente a la simulación.** Box–Muller son dos `next_u64`, un
/// `ln`, un `sqrt` y un `cos` por MES; la simulación es un bucle de 840 meses con cascada,
/// fiscalidad y drenaje. Si el sorteo fuera una fracción apreciable del total habría que
/// replantearse guardar el segundo normal de Box–Muller — este test es la medición que respalda
/// haberlo descartado.
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn the_cost_of_the_draw_against_the_cost_of_the_simulation() {
    let input = p9();
    let v = vols();
    let config = McConfig::default();
    let paths = 200u32;

    // Con volatilidad: se sortea y se aplica.
    let t0 = Instant::now();
    for p in 0..paths {
        black_box(run_path(&input, &v, &config, p).expect("no falla"));
    }
    let with = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(paths);

    // Sin volatilidad declarada: el sorteo SIGUE ocurriendo (el flujo no depende de los datos) y
    // lo que se ahorra es solo la exponencial por activo.
    let zero: Vec<Option<f64>> = vec![None; input.assets.len()];
    let t0 = Instant::now();
    for p in 0..paths {
        black_box(run_path(&input, &zero, &config, p).expect("no falla"));
    }
    let without = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(paths);

    println!(
        "[mc-timing/{}] P9 · camino con σ>0 = {with:.3} ms · con σ=0 = {without:.3} ms \
         ⇒ las 840×5 exponenciales cuestan {:.3} ms ({:.1} %)",
        profile(),
        with - without,
        (with - without) / with * 100.0
    );
}

/// Un jubilado de dos activos (cuenta al 0 % + RV) que pasa TODO el horizonte retirando: el caso
/// donde el relleno del colchón se ejecuta de verdad, mes sí y mes también. P9, en cambio, apenas
/// llega a jubilarse en la mayoría de los caminos, así que su «colchón activo» casi no lo ejerce.
fn retired_lab(horizon: u32) -> ProjectionInput {
    use futurefin_engine::{PhasePlan, SimAsset};
    use rust_decimal::Decimal;
    let monthly =
        Decimal::from(1_000_000) * Decimal::from(4) / Decimal::from(100) / Decimal::from(12);
    ProjectionInput {
        ref_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        income_regular_monthly: Decimal::ZERO,
        expense_regular_monthly: monthly,
        assets: vec![
            SimAsset {
                id: uuid::Uuid::from_u128(1),
                value: Decimal::from(80_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::ZERO),
            },
            SimAsset {
                id: uuid::Uuid::from_u128(2),
                value: Decimal::from(920_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::try_from(6.5).unwrap()),
            },
        ],
        allocation_rules: Vec::new(),
        liabilities: Vec::new(),
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        phase_plan: PhasePlan::forced_at(1, Decimal::ZERO, monthly, Decimal::ZERO),
        fire_target: None,
    }
}

/// (e) **Lo que cuesta el colchón (P4)**: la misma ejecución de 500 caminos con y sin
/// `cash_buffer`, en dos casos.
///
/// El relleno añade, en los meses jubilados con shock positivo, un `gross_up` y un drenaje sobre
/// el orden restringido. En P9 la mayoría de los caminos no llegan a jubilarse, así que el
/// colchón se instala pero casi no se ejerce: ese número mide el coste de tenerlo declarado. El
/// jubilado de dos activos lo ejerce ~la mitad de los 420 meses y es la cota ALTA.
///
/// Se imprime también el éxito de cada uno: el colchón no es gratis en probabilidad, y el arnés
/// de tiempos no es el sitio para esconderlo.
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn the_cost_of_the_cash_buffer() {
    let paths = DEFAULT_PATHS;
    let mut line = String::new();
    for (label, input, v) in [
        ("P9 (apenas se jubila)", p9(), vols()),
        (
            "jubilado 2 activos    ",
            retired_lab(420),
            vec![None, Some(17.0)],
        ),
    ] {
        for months in [None, Some(24u32)] {
            let config = McConfig {
                seed: 20_260_903,
                paths,
                cash_buffer: months.map(CashBufferSpec::Months),
                ..Default::default()
            };
            let t0 = Instant::now();
            let out = project_percentile_bands(&input, &v, &config).expect("no falla");
            let total = t0.elapsed().as_secs_f64() * 1000.0;
            line.push_str(&format!(
                "\n[mc-timing/{}] {label} · {paths} caminos · colchón {:>8}: {total:>8.1} ms \
             ({:.3} ms/camino) · activo {} · rellenos p50 {:?} · éxito {:.3}",
                profile(),
                match months {
                    None => "no".to_string(),
                    Some(n) => format!("{n} meses"),
                },
                total / f64::from(paths),
                out.buffer_active,
                out.buffer_refills_p50,
                out.success_probability,
            ));
            black_box(out);
        }
    }
    println!("{line}");
}
