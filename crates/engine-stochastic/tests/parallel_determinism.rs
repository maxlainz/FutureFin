//! **La puerta del paralelismo** (E12 de 5.0.0): repartir los caminos entre núcleos no mueve un
//! bit.
//!
//! El sorteo de Monte Carlo se reparte entre hilos desde E12 (`crate::parallel`), y la promesa que
//! acompaña a ese cambio es tan fuerte como incómoda de comprobar: **no «casi igual», no «dentro de
//! tolerancia», sino IDÉNTICO bit a bit** — en las bandas, en las probabilidades, en los conteos,
//! en la tabla de fallo acumulado, en las coberturas, en la fecha válida, en el capital necesario
//! de hoy, en la curva por edad y en la tira anual de éxito.
//!
//! Un test que comparase con `assert!((a - b).abs() < 1e-9)` no probaría nada de eso: dejaría pasar
//! exactamente el fallo que este cambio puede introducir —una suma de `f64` plegada en el orden en
//! que terminan los hilos— que produce diferencias del orden del último bit y que, precisamente por
//! ser diminutas, harían que la probabilidad de éxito de un usuario bailara al refrescar sin que
//! nada fallara. Por eso aquí se compara con `f64::to_bits()`.
//!
//! # Por qué la promesa se puede sostener
//!
//! 1. **Los caminos son independientes**: el RNG de un camino se construye desde `(seed,
//!    path_index)` y de nada más (`mc::path_rng`), y el motor es una función pura sin estado
//!    global. El camino 7 es el camino 7 lo ejecute quien lo ejecute.
//! 2. **El pliegue va en orden de índice**: `mc::for_each_path` entrega los resultados al hilo
//!    llamante con `p = 0, 1, 2, …`, siempre. Las únicas sumas de `f64` que existen (cobertura y
//!    recorte) son INTRA-camino; entre caminos solo hay conteos enteros y ordenaciones con
//!    `total_cmp`, ninguno de los cuales depende del orden de llegada.
//!
//! # La batería
//!
//! Cinco casos de la del motor, elegidos para cubrir los caminos que más pliegan: **P9** (el hogar
//! realista de cinco activos, dos pasivos y cascada con topes), **P13** (`g` denormal — el caso que
//! existe para que la aritmética se vea), **P18** (puente de pensión), **P15**
//! (`percent_of_balance` con techo: la regla por SALDO, que hace que cada camino retire una
//! cantidad distinta y que la cobertura sea una suma no trivial) y **P17** (guardrails con
//! impuestos ES).

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{projection_cases_5_0, projection_cases_all, ProjCase};
use futurefin_engine::ProjectionInput;
use futurefin_engine_stochastic::{
    needed_capital_curve, needed_capital_today, parallel, project_percentile_bands,
    success_by_retirement_month, valid_retirement_month, McConfig, McOutcome, NeededCapital,
    RetirementDateSolve, SuccessAt,
};

// =================================================================================================
// Utilidades
// =================================================================================================

fn all_cases() -> Vec<ProjCase> {
    let mut out = projection_cases_all();
    out.extend(projection_cases_5_0());
    out
}

fn case(name: &str) -> ProjectionInput {
    all_cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} debe existir en la batería del motor"))
        .input
}

/// Los cinco casos de la batería, por nombre. Se declaran aquí y no se derivan de un filtro para
/// que retirar un caso de la batería del motor **falle** en vez de reducir la cobertura en
/// silencio.
const CASES: [&str; 5] = [
    "P9_hogar_realista",
    "P13_cash8k_denormal_g",
    "P18_pension_bridge",
    "P15_percent_of_balance_ceiling",
    "P17_guardrails_taxes_es",
];

/// Volatilidades alineadas con los activos del caso, por posición y con magnitudes realistas
/// (efectivo 0 · RF 5 % · RV 16 % · inmueble 8 % · cripto 70 %, cíclicas). **Determinista**: el
/// mismo caso da siempre el mismo vector, que es lo que permite comparar dos ejecuciones.
fn volatilities_for(input: &ProjectionInput) -> Vec<Option<f64>> {
    const CYCLE: [Option<f64>; 5] = [None, Some(5.0), Some(16.0), Some(8.0), Some(70.0)];
    (0..input.assets.len()).map(|i| CYCLE[i % 5]).collect()
}

/// Umbral de éxito de los solves. **50 %, no el 95 % de producto**, y es deliberado: con 95 y
/// presupuestos pequeños los cinco casos contestan «no hay fecha» por el mismo camino corto, y el
/// test dejaría sin ejercitar la bisección entera —que es donde un pliegue mal ordenado se
/// propagaría de nodo en nodo—. Lo que aquí se compara es que las DOS vías den lo mismo, no que el
/// hogar se jubile.
const THRESHOLD_PCT: u32 = 50;

fn config(paths: u32, threads: usize) -> McConfig {
    McConfig {
        seed: 0x0E12_2026_0907,
        paths,
        percentiles: vec![10, 50, 90],
        threads: Some(threads),
    }
}

// =================================================================================================
// Comparadores BIT A BIT
// =================================================================================================

/// Igualdad **de bits** de dos `f64`. No es `==`: `NaN != NaN` y `0.0 == -0.0`, y las dos
/// diferencias son exactamente las que un comparador laxo dejaría pasar aquí.
#[track_caller]
fn same_bits(what: &str, a: f64, b: f64) {
    assert_eq!(
        a.to_bits(),
        b.to_bits(),
        "{what}: {a:?} ({:#018x}) != {b:?} ({:#018x})",
        a.to_bits(),
        b.to_bits()
    );
}

#[track_caller]
fn same_bits_slice(what: &str, a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len(), "{what}: longitudes distintas");
    for (k, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        same_bits(&format!("{what}[{k}]"), *x, *y);
    }
}

/// **TODAS las salidas de [`McOutcome`]**, campo a campo. Es exhaustivo a propósito: si el tipo
/// gana un campo, el `let McOutcome { .. } = a` de abajo deja de compilar y hay que decidir
/// explícitamente cómo se compara — que es justo lo que un `assert_eq!(a, b)` con `PartialEq`
/// derivado NO obliga a hacer (y que además compararía los `f64` con `==`, no por bits).
#[track_caller]
fn outcomes_are_bit_identical(what: &str, a: &McOutcome, b: &McOutcome) {
    let McOutcome {
        seed,
        paths,
        percentiles,
        horizon_months,
        net_worth,
        liquid_worth,
        success_probability,
        wilson_low,
        half_width_pp,
        failures_by_kind,
        cumulative_failure_by_age,
        months_below_need_p50,
        withdrawal_to_need_ratio_p50,
        any_volatility_declared,
    } = a;

    assert_eq!(*seed, b.seed, "{what}: seed");
    assert_eq!(*paths, b.paths, "{what}: paths");
    assert_eq!(*percentiles, b.percentiles, "{what}: percentiles");
    assert_eq!(*horizon_months, b.horizon_months, "{what}: horizon_months");

    assert_eq!(net_worth.len(), b.net_worth.len(), "{what}: bandas nw");
    for (j, (x, y)) in net_worth.iter().zip(b.net_worth.iter()).enumerate() {
        same_bits_slice(&format!("{what}: net_worth[p{j}]"), x, y);
    }
    assert_eq!(liquid_worth.len(), b.liquid_worth.len(), "{what}: bandas lq");
    for (j, (x, y)) in liquid_worth.iter().zip(b.liquid_worth.iter()).enumerate() {
        same_bits_slice(&format!("{what}: liquid_worth[p{j}]"), x, y);
    }

    same_bits(
        &format!("{what}: success_probability"),
        *success_probability,
        b.success_probability,
    );
    same_bits(&format!("{what}: wilson_low"), *wilson_low, b.wilson_low);
    same_bits(
        &format!("{what}: half_width_pp"),
        *half_width_pp,
        b.half_width_pp,
    );
    assert_eq!(
        *failures_by_kind, b.failures_by_kind,
        "{what}: failures_by_kind"
    );

    assert_eq!(
        cumulative_failure_by_age.len(),
        b.cumulative_failure_by_age.len(),
        "{what}: filas de cumulative_failure_by_age"
    );
    for (i, ((m1, p1), (m2, p2))) in cumulative_failure_by_age
        .iter()
        .zip(b.cumulative_failure_by_age.iter())
        .enumerate()
    {
        assert_eq!(m1, m2, "{what}: cumulative_failure_by_age[{i}].mes");
        same_bits(&format!("{what}: cumulative_failure_by_age[{i}].p"), *p1, *p2);
    }

    assert_eq!(
        *months_below_need_p50, b.months_below_need_p50,
        "{what}: months_below_need_p50"
    );
    match (withdrawal_to_need_ratio_p50, b.withdrawal_to_need_ratio_p50) {
        (None, None) => {}
        (Some(x), Some(y)) => same_bits(&format!("{what}: withdrawal_to_need_ratio_p50"), *x, y),
        (x, y) => panic!("{what}: withdrawal_to_need_ratio_p50 {x:?} != {y:?}"),
    }
    assert_eq!(
        *any_volatility_declared, b.any_volatility_declared,
        "{what}: any_volatility_declared"
    );
}

#[track_caller]
fn success_at_is_bit_identical(what: &str, a: &SuccessAt, b: &SuccessAt) {
    let SuccessAt {
        month,
        paths,
        failures,
        by_kind,
        success,
        wilson_low,
        half_width_pp,
        rule_of_three_upper,
    } = a;
    assert_eq!(*month, b.month, "{what}: month");
    assert_eq!(*paths, b.paths, "{what}: paths");
    assert_eq!(*failures, b.failures, "{what}: failures");
    assert_eq!(*by_kind, b.by_kind, "{what}: by_kind");
    same_bits(&format!("{what}: success"), *success, b.success);
    same_bits(&format!("{what}: wilson_low"), *wilson_low, b.wilson_low);
    same_bits(
        &format!("{what}: half_width_pp"),
        *half_width_pp,
        b.half_width_pp,
    );
    match (rule_of_three_upper, b.rule_of_three_upper) {
        (None, None) => {}
        (Some(x), Some(y)) => same_bits(&format!("{what}: rule_of_three_upper"), *x, y),
        (x, y) => panic!("{what}: rule_of_three_upper {x:?} != {y:?}"),
    }
}

#[track_caller]
fn date_solve_is_bit_identical(what: &str, a: &RetirementDateSolve, b: &RetirementDateSolve) {
    let RetirementDateSolve {
        month,
        success,
        wilson_low,
        half_width_pp,
        rule_of_three_upper,
        predecessor_success,
        date_is_approximate,
        draws_search,
        draws_confirm,
        failures_by_kind,
        best_effort,
    } = a;
    assert_eq!(*month, b.month, "{what}: month");
    same_bits(&format!("{what}: success"), *success, b.success);
    same_bits(&format!("{what}: wilson_low"), *wilson_low, b.wilson_low);
    same_bits(
        &format!("{what}: half_width_pp"),
        *half_width_pp,
        b.half_width_pp,
    );
    for (name, x, y) in [
        (
            "rule_of_three_upper",
            rule_of_three_upper,
            &b.rule_of_three_upper,
        ),
        (
            "predecessor_success",
            predecessor_success,
            &b.predecessor_success,
        ),
    ] {
        match (x, y) {
            (None, None) => {}
            (Some(x), Some(y)) => same_bits(&format!("{what}: {name}"), *x, *y),
            (x, y) => panic!("{what}: {name} {x:?} != {y:?}"),
        }
    }
    match (best_effort, b.best_effort) {
        (None, None) => {}
        (Some((m1, s1)), Some((m2, s2))) => {
            assert_eq!(*m1, m2, "{what}: best_effort.mes");
            same_bits(&format!("{what}: best_effort.éxito"), *s1, s2);
        }
        (x, y) => panic!("{what}: best_effort {x:?} != {y:?}"),
    }
    assert_eq!(
        *date_is_approximate, b.date_is_approximate,
        "{what}: date_is_approximate"
    );
    assert_eq!(*draws_search, b.draws_search, "{what}: draws_search");
    assert_eq!(*draws_confirm, b.draws_confirm, "{what}: draws_confirm");
    assert_eq!(
        *failures_by_kind, b.failures_by_kind,
        "{what}: failures_by_kind"
    );
}

/// `NeededCapital` publica importes en `Decimal` (exactos por construcción) y dos `f64`: la `λ` y
/// el éxito confirmado en ella. Los dos se comparan por bits; el resto, por igualdad.
#[track_caller]
fn needed_capital_is_bit_identical(what: &str, a: &NeededCapital, b: &NeededCapital) {
    let NeededCapital {
        month,
        lambda,
        amount_nominal,
        amount_today,
        absent_reason,
        success_at_lambda,
        capital_is_approximate,
        draws_search,
        draws_confirm,
    } = a;
    assert_eq!(*month, b.month, "{what}: month");
    assert_eq!(*amount_nominal, b.amount_nominal, "{what}: amount_nominal");
    assert_eq!(*amount_today, b.amount_today, "{what}: amount_today");
    match (lambda, b.lambda) {
        (None, None) => {}
        (Some(x), Some(y)) => same_bits(&format!("{what}: lambda"), *x, y),
        (x, y) => panic!("{what}: lambda {x:?} != {y:?}"),
    }
    match (success_at_lambda, b.success_at_lambda) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            success_at_is_bit_identical(&format!("{what}: success_at_lambda"), x, &y)
        }
        (x, y) => panic!("{what}: success_at_lambda {x:?} != {y:?}"),
    }
    assert_eq!(
        *capital_is_approximate, b.capital_is_approximate,
        "{what}: capital_is_approximate"
    );
    assert_eq!(*draws_search, b.draws_search, "{what}: draws_search");
    assert_eq!(*draws_confirm, b.draws_confirm, "{what}: draws_confirm");
    assert_eq!(*absent_reason, b.absent_reason, "{what}: absent_reason");
}

// =================================================================================================
// Las puertas
// =================================================================================================

/// **La puerta principal**: secuencial (1 hilo) contra paralelo (el pool por defecto, acotado a
/// `[1, 8]`), en los cinco casos y en TODAS las salidas.
///
/// Se corre con 128 caminos —bastantes para que el reparto por bloques de `for_each_path` tenga
/// más de un bloque con cualquier número de hilos, y pocos para que la suite siga siendo rápida en
/// `debug`—. El número de bloques importa: es donde el pliegue podría desordenarse.
#[test]
fn parallel_and_sequential_runs_are_bit_identical() {
    let threads = parallel::pool_threads().max(2);
    println!(
        "[E12] comparando 1 hilo contra {threads} (pool compartido = {})",
        parallel::pool_threads()
    );
    for name in CASES {
        let input = case(name);
        let vols = volatilities_for(&input);

        // 1. El sorteo completo con bandas.
        let seq = project_percentile_bands(&input, &vols, &config(128, 1)).expect("no falla");
        let par = project_percentile_bands(&input, &vols, &config(128, threads)).expect("no falla");
        outcomes_are_bit_identical(&format!("{name}/bandas"), &seq, &par);

        // 2. La tira anual de éxito — el mismo `Draws` que usan todos los solves.
        let horizon = input.horizon_months;
        let grid: Vec<u32> = [1u32, horizon / 4, horizon / 2, horizon]
            .into_iter()
            .map(|m| m.max(1))
            .collect();
        let seq_grid = success_by_retirement_month(&input, &vols, &config(96, 1), &grid)
            .expect("no falla");
        let par_grid = success_by_retirement_month(&input, &vols, &config(96, threads), &grid)
            .expect("no falla");
        assert_eq!(seq_grid.len(), par_grid.len(), "{name}: filas de la tira");
        for (i, (x, y)) in seq_grid.iter().zip(par_grid.iter()).enumerate() {
            success_at_is_bit_identical(&format!("{name}/success_by_retirement_month[{i}]"), x, y);
        }

        // 3. La FECHA válida: la bisección entera, con sus dos presupuestos.
        let seq_date = valid_retirement_month(
            &input,
            &vols,
            &config(64, 1),
            &config(128, 1),
            THRESHOLD_PCT,
            1,
        )
        .expect("no falla");
        let par_date = valid_retirement_month(
            &input,
            &vols,
            &config(64, threads),
            &config(128, threads),
            THRESHOLD_PCT,
            1,
        )
        .expect("no falla");
        date_solve_is_bit_identical(&format!("{name}/valid_retirement_month"), &seq_date, &par_date);

        // 4. El capital necesario de HOY: bisección en λ sobre el mismo sorteo.
        let seq_today =
            needed_capital_today(&input, &vols, &config(64, 1), &config(128, 1), THRESHOLD_PCT)
                .expect("no falla");
        let par_today = needed_capital_today(
            &input,
            &vols,
            &config(64, threads),
            &config(128, threads),
            THRESHOLD_PCT,
        )
        .expect("no falla");
        needed_capital_is_bit_identical(
            &format!("{name}/needed_capital_today"),
            &seq_today,
            &par_today,
        );

        // 5. La CURVA por edad, con el `λ` caliente arrastrándose de nodo a nodo — el sitio donde
        //    una diferencia de un bit en un nodo se propagaría a todos los siguientes.
        let curve_grid: Vec<u32> = (1..=4).map(|i| (horizon * i / 4).max(1)).collect();
        let seq_curve = needed_capital_curve(&input, &vols, &config(64, 1), THRESHOLD_PCT, &curve_grid)
            .expect("no falla");
        let par_curve = needed_capital_curve(&input, &vols, &config(64, threads), THRESHOLD_PCT, &curve_grid)
            .expect("no falla");
        assert_eq!(seq_curve.len(), par_curve.len(), "{name}: nodos de la curva");
        for (i, (x, y)) in seq_curve.iter().zip(par_curve.iter()).enumerate() {
            needed_capital_is_bit_identical(&format!("{name}/needed_capital_curve[{i}]"), x, y);
        }

        println!(
            "[E12] {name}: éxito {:.6} · fecha {:?} ({}+{} sorteos) · capital hoy {:?} \
             [{:?}, {}+{} sorteos] · curva {:?} — idénticos con 1 y {threads} hilos",
            seq.success_probability,
            seq_date.month,
            seq_date.draws_search,
            seq_date.draws_confirm,
            seq_today.amount_nominal,
            seq_today.absent_reason,
            seq_today.draws_search,
            seq_today.draws_confirm,
            seq_curve
                .iter()
                .map(|n| n.amount_nominal.is_some())
                .collect::<Vec<_>>()
        );
    }
}

/// **El número de hilos no es una entrada del modelo**: 1, 2, 4 y 8 dan el MISMO resultado.
///
/// Es una prueba distinta de la anterior y no una repetición: allí se compara secuencial contra
/// «lo que la máquina tenga»; aquí se barre el eje entero, incluidos números de hilos que **no**
/// dividen el bloque de reparto (`threads · 16`) ni el número de caminos, que es donde un pliegue
/// mal ordenado se delataría.
#[test]
fn results_do_not_depend_on_the_thread_count() {
    for name in CASES {
        let input = case(name);
        let vols = volatilities_for(&input);
        // 100 caminos: NO es múltiplo de 8·16 ni de 4·16, así que el último bloque va corto y los
        // trozos por hilo quedan desiguales. Ese es el punto.
        let reference = project_percentile_bands(&input, &vols, &config(100, 1)).expect("no falla");
        for threads in [2usize, 4, 8] {
            let out =
                project_percentile_bands(&input, &vols, &config(100, threads)).expect("no falla");
            outcomes_are_bit_identical(&format!("{name}/{threads} hilos"), &reference, &out);
        }
        // Y la tira de éxito, que es la que alimenta los solves.
        let grid = [1u32, input.horizon_months.max(1)];
        let ref_grid =
            success_by_retirement_month(&input, &vols, &config(100, 1), &grid).expect("no falla");
        for threads in [2usize, 4, 8] {
            let got = success_by_retirement_month(&input, &vols, &config(100, threads), &grid)
                .expect("no falla");
            for (i, (x, y)) in ref_grid.iter().zip(got.iter()).enumerate() {
                success_at_is_bit_identical(&format!("{name}/tira/{threads} hilos [{i}]"), x, y);
            }
        }
        println!("[E12] {name}: 1 · 2 · 4 · 8 hilos ⇒ el mismo McOutcome bit a bit");
    }
}

/// El techo del pool está donde dice el contrato, y pedir más no lo sube. Es la mitad de la
/// promesa que `heavy.rs` necesita (la otra —«un solo pool compartido»— la sostiene el
/// `OnceLock` de `parallel.rs`).
#[test]
fn the_thread_ceiling_is_the_one_the_contract_declares() {
    let n = parallel::pool_threads();
    assert!(
        (1..=parallel::MAX_POOL_THREADS).contains(&n),
        "el pool compartido debe caber en [1, {}]: {n}",
        parallel::MAX_POOL_THREADS
    );
    // Un `threads` absurdo se acota en vez de abrir doscientos hilos.
    let input = case("P13_cash8k_denormal_g");
    let vols = volatilities_for(&input);
    let a = project_percentile_bands(&input, &vols, &config(32, 1)).expect("no falla");
    let b = project_percentile_bands(&input, &vols, &config(32, 10_000)).expect("no falla");
    outcomes_are_bit_identical("threads absurdo", &a, &b);
}
