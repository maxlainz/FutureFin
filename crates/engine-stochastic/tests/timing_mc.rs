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

use cases::{p9_household, projection_cases_all};
use futurefin_engine::{project_net_worth_series, ProjectionInput};
use futurefin_engine_stochastic::{
    project_percentile_bands, run_path, simulate_f64, McConfig, DEFAULT_PATHS,
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


/// (e) **El solve de la FECHA VÁLIDA**, que es el que paga el usuario cuando abre Jubilación.
///
/// Presupuesto declarado en el plan de 5.0.0: **≤ 3,5 s típico y ≤ 10 s en el peor caso**, con la
/// partición «buscar con 500, confirmar con 2.500» que sale de los ~0,2 ms/camino de (c). El peor
/// caso teórico son `15 + 4 + 6 = 25` sorteos de búsqueda más `1 + 12 + 1 = 14` de confirmación.
///
/// Se imprimen tres cosas, en este orden:
///
/// 1. el coste de UN sorteo con cada presupuesto — la primitiva de la que sale todo lo demás;
/// 2. las cotas DERIVADAS (típica y peor caso) para poder ponerlas al lado del número del plan;
/// 3. solves de verdad, con sus sorteos y sus segundos.
///
/// **Mide, no afirma**: no hay `assert` de tiempo.
#[test]
#[ignore = "mide, no afirma: correr con --release --ignored --nocapture"]
fn the_date_solve_costs_what_the_plan_says() {
    use futurefin_engine::InitialRateGate;
    use futurefin_engine_stochastic::{success_at_month, valid_retirement_month};
    use rust_decimal::Decimal;

    let seed = 20_260_906u64;
    let search = McConfig { seed, paths: 500, ..Default::default() };
    let confirm = McConfig { seed, paths: 2_500, ..Default::default() };

    let gate = |mut input: ProjectionInput, swr: Decimal| -> ProjectionInput {
        input.phase_plan.initial_rate = Some(InitialRateGate { swr_pct: swr, bridge: None });
        input
    };

    // ---- 1. La primitiva: un sorteo ---------------------------------------------------------
    let base = gate(p9(), Decimal::new(35, 1));
    let v = vols();
    let t0 = Instant::now();
    black_box(success_at_month(&base, &v, &search, 400).expect("no falla"));
    let one_search = t0.elapsed().as_secs_f64();
    let t0 = Instant::now();
    black_box(success_at_month(&base, &v, &confirm, 400).expect("no falla"));
    let one_confirm = t0.elapsed().as_secs_f64();
    println!(
        "[mc-timing/{}] P9 840 meses · un sorteo de éxito(k): 500 caminos = {:.0} ms · \
         2.500 caminos = {:.0} ms",
        profile(),
        one_search * 1000.0,
        one_confirm * 1000.0
    );

    // ---- 2. Las cotas derivadas -------------------------------------------------------------
    println!(
        "[mc-timing/{}] cotas derivadas · típico (12 búsqueda + 2 confirmación) = {:.2} s · \
         peor caso (25 + 14) = {:.2} s · plan: ≤ 3,5 s / ≤ 10 s",
        profile(),
        12.0 * one_search + 2.0 * one_confirm,
        25.0 * one_search + 14.0 * one_confirm
    );

    // ---- 3. Solves de verdad ----------------------------------------------------------------
    // P9 tal cual **no tiene fecha válida** en el modelo v2, y no es un artefacto del arnés: su
    // gasto de jubilación se indexa al 2,5 % y su pensión es plana, así que a 70 años la
    // necesidad se ha multiplicado por 4,9 y ningún mes del horizonte pasa la puerta de tasa
    // inicial. Es la forma BARATA del solve (bracket entero, cero confirmaciones).
    //
    // La forma cara —A→E completo— se mide sobre el MISMO hogar con la inflación apagada y sin
    // saldo en la cuenta corriente: entonces la necesidad de jubilación es constante (1.400 €/mes
    // netos de pensión) y existe un mes a partir del cual la cartera la sostiene.
    let mut flat = p9_household(Decimal::ZERO);
    flat.annual_inflation_percent = Decimal::ZERO;
    if let Some(t) = flat.fire_target.as_mut() {
        t.annual_inflation_percent = Decimal::ZERO;
    }
    let flat = gate(flat, Decimal::new(35, 1));
    let tamer: Vec<Option<f64>> = vec![None, Some(5.0), Some(16.0), Some(8.0), Some(20.0)];

    for (label, input, vv, threshold) in [
        ("P9 tal cual", &base, &v, 95u32),
        ("P9 sin inflación", &flat, &tamer, 80),
        ("P9 sin inflación", &flat, &tamer, 90),
        ("P9 sin inflación", &flat, &tamer, 95),
    ] {
        let t0 = Instant::now();
        let solve = valid_retirement_month(input, vv, &search, &confirm, threshold, 1)
            .expect("el sorteo no falla");
        let secs = t0.elapsed().as_secs_f64();
        let paths_total = solve.draws_search * search.paths + solve.draws_confirm * confirm.paths;
        println!(
            "[mc-timing/{}] {label} · umbral {threshold} ⇒ mes {:?} \
             (éxito {:.4}, wilson_low {:.4}, barra {:.3} pp, aprox {}, fallos {:?}, \
             predecesor {:?}, best_effort {:?}) · sorteos {}×500 + {}×2.500 = {paths_total} caminos \
             · **{secs:.2} s**",
            profile(),
            solve.month,
            solve.success,
            solve.wilson_low,
            solve.half_width_pp,
            solve.date_is_approximate,
            solve.failures_by_kind,
            solve.predecessor_success.map(|x| (x * 1000.0).round() / 1000.0),
            solve.best_effort.map(|(m, x)| (m, (x * 1000.0).round() / 1000.0)),
            solve.draws_search,
            solve.draws_confirm,
        );
        black_box(solve);
    }
}
