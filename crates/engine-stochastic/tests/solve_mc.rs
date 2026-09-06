//! **Los solves estocásticos** (WP E6 de 5.0.0): `éxito(k)`, la tira anual y la FECHA VÁLIDA.
//!
//! Los hogares de este arnés son pequeños y están DERIVADOS A MANO — la aritmética de cada
//! predicción está escrita al lado de su test, antes de correrlo, siguiendo la disciplina de
//! `futurefin-research-methodology` («predice el número antes de ejecutar»). Los casos grandes de
//! la batería (`crates/engine/tests/common/cases.rs`, reutilizada por `#[path]`) se usan para los
//! constructores, no como escenarios: aquí lo que se prueba es el SOLVER, y un solver se prueba
//! con una función objetivo cuya forma se conoce.
//!
//! **Presupuesto**: ningún test de este fichero pasa de 300 caminos. La medición de tiempos vive
//! en `tests/timing_mc.rs`, `#[ignore]` y en release.

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, rule_remainder};
use futurefin_engine::{
    FireNeed, FireTarget, InitialRateGate, PathFailure, PensionSchedule, RetirementTrigger,
};
use futurefin_engine_stochastic::{
    retiring_at, run_path, success_at_month, success_by_retirement_month, valid_retirement_month,
    McConfig, SuccessAt, KIND_INITIAL_RATE_EXCEEDED,
};
use rust_decimal::Decimal;

/// Semilla fija: los tests miden un sorteo concreto, no «un sorteo».
const SEED: u64 = 20_260_906;

fn cfg(paths: u32) -> McConfig {
    McConfig {
        seed: SEED,
        paths,
        ..Default::default()
    }
}

// =================================================================================================
// Los dos hogares del arnés
// =================================================================================================

/// **El hogar HOLGADO**: 2.000.000 € líquidos al 3 %, 4.000 € de ingreso contra 2.000 € de gasto,
/// sin inflación, 240 meses.
///
/// Jubilándose en CUALQUIER mes cumple: la necesidad anual de jubilación son `12 · 2.000 =
/// 24.000 €` y el tope de tasa inicial al 4 % sobre 2.000.000 € son 80.000 €. Y drenar 2.000 €/mes
/// durante 240 meses son 480.000 € sobre una cartera que crece: F1 no se acerca.
///
/// Lleva un **objetivo FIRE ridículo a propósito** (25.000 €): el cruce ocurriría en el mes 1, así
/// que si `crossing_is_reading_only` no estuviera puesto, `retirement_month_index` sería 1 y no
/// `k`. Es lo que convierte `success_at_month_retires_every_path_in_k` en una prueba de verdad.
fn comfortable_household() -> futurefin_engine::ProjectionInput {
    let mut input = base_input(
        240,
        Decimal::from(4_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(2_000_000),
            true,
            Some(Decimal::from(3)),
        )],
        vec![rule_remainder(0)],
    );
    input.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    input.phase_plan.income_retirement_monthly = Decimal::ZERO;
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: Decimal::from(4),
        bridge: None,
    });
    input.fire_target = Some(FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: Decimal::from(1_000),
        },
        swr_pct: Decimal::from(4),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    input
}

/// **El hogar de la curva NO MONÓTONA**: 500.000 € al 0 %, ingreso 3.000 € PLANO contra un gasto
/// de 3.000 € INDEXADO al 3 %, gasto de jubilación 2.000 € (también indexado), 240 meses, puerta
/// de tasa inicial al 4 %.
///
/// La forma de `éxito(k)`, derivada a mano antes de correr nada:
///
/// - El ingreso es plano y el gasto se indexa (#139), así que el hogar entra en déficit desde el
///   mes 2 y la cartera **baja** mientras trabaja. A la vez la necesidad anual de jubilación sube
///   al 3 %. Las dos cosas empujan `12·ordinaria(k) / (4 % · L(k−1))` hacia arriba: **el éxito
///   DECRECE con `k`**.
/// - `inheritance = true` mete un «Próximo» de **+500.000 € en el mes 100** (índice 99). Ese mes
///   —y solo ese— el denominador salta y la puerta se abre.
///
/// Resultado: falla, falla, falla… **abre en el mes 101** y vuelve a cerrarse más adelante. Es
/// decir, no monótona en los dos sentidos, con el bracket de 60 meses teniendo que encontrar la
/// ventana. Números (`f(i) = 1,03^(i/12)`, `L` sin volatilidad):
///
/// | mes `k` | `12·ordinaria(k)` | `4 % · L(k−1)` | ¿cumple? |
/// |---|---|---|---|
/// | 73 | 28.657 € | ≈ 19.200 € | no |
/// | 100 | 30.630 € | ≈ 18.400 € | no |
/// | **101** | 30.703 € | ≈ 38.400 € (herencia) | **sí** |
/// | 121 | 32.256 € | ≈ 37.700 € | sí |
/// | 181 | 37.391 € | ≈ 34.600 € | no |
/// | 240 | 43.155 € | ≈ 29.700 € | no |
///
/// La volatilidad es del **2 %** anual: suficiente para que el sorteo importe (la σ acumulada a
/// 100 meses es ~5,8 %) y estrecha frente a los márgenes de la tabla, que son del 20 % o más en
/// los meses que deciden. Sin eso el test mediría el ruido, no el solver.
fn tilting_household(inheritance: bool) -> futurefin_engine::ProjectionInput {
    let mut input = base_input(
        240,
        Decimal::from(3_000),
        Decimal::from(3_000),
        vec![mk_asset(1, Decimal::from(500_000), true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    );
    input.annual_inflation_percent = Decimal::from(3);
    input.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    input.phase_plan.income_retirement_monthly = Decimal::ZERO;
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: Decimal::from(4),
        bridge: None,
    });
    if inheritance {
        // Índice 99 = mes 100 del bucle.
        input.planning_monthly_cash_adjustment[99] = Decimal::from(500_000);
    }
    input
}

fn low_vol() -> Vec<Option<f64>> {
    vec![Some(2.0)]
}

fn some_vol() -> Vec<Option<f64>> {
    vec![Some(8.0)]
}

// =================================================================================================
// éxito(k)
// =================================================================================================

/// **Jubilarse en `k` quiere decir jubilarse en `k`, en TODOS los caminos.**
///
/// El hogar lleva un objetivo FIRE que se cruza en el mes 1: sin `crossing_is_reading_only` el
/// motor jubilaría ahí (la unión `cruce || k ≥ forzado` de 4.15.0) y `éxito(k)` mediría otra cosa.
///
/// De paso ata las dos vías: el recuento de [`success_at_month`] —que sostiene un `PathEngine` y
/// reescribe el trigger en sitio— tiene que coincidir con el de correr `run_path` sobre
/// [`retiring_at`], que reconstruye el motor entero.
#[test]
fn success_at_month_retires_every_path_in_k() {
    let input = comfortable_household();
    let vols = some_vol();
    let k = 137u32;
    let config = cfg(120);
    let scenario = retiring_at(&input, k);

    let mut failures = 0u32;
    for p in 0..config.paths {
        let out = run_path(&scenario, &vols, &config, p).expect("el motor no falla");
        assert_eq!(
            out.retirement_month_index,
            Some(k),
            "camino {p}: se jubila en {:?} y no en {k}",
            out.retirement_month_index
        );
        // El cruce SIGUE anotándose — es lectura, no trigger.
        assert!(
            out.liquid_crossing_month_index.is_some(),
            "el cruce debe seguir publicándose como lectura"
        );
        if out.failure_month_index.is_some() {
            failures += 1;
        }
    }

    let s = success_at_month(&input, &vols, &config, k).expect("el sorteo no falla");
    assert_eq!(s.month, k);
    assert_eq!(s.paths, config.paths);
    assert_eq!(
        s.failures, failures,
        "el recuento del solve y el de `run_path` sobre `retiring_at` tienen que ser el mismo"
    );
    assert_eq!(s.failures, 0, "el hogar holgado no falla en ningún camino");
    assert_eq!(s.by_kind, [0, 0, 0]);
}

/// **La barra de error es Wilson, y con cero fallos NO es cero.**
///
/// Con `p̂ = 1` el intervalo de Wilson se SIMPLIFICA —los dos términos `z²/2n` se cancelan— y
/// queda una forma cerrada: `low = n/(n + z²)`. Con `n = 120` y `z² = 3,8416`:
///
/// ```text
///   low            = 120 / 123,8416          = 0,96897973
///   half_width_pp  = 100 · 3,8416 / 123,8416 = 3,10203 pp
///   regla de tres  = 3/120                   = 0,025
/// ```
///
/// La aproximación normal daría `1,96·√(1·0/120) = 0` — «100 % seguro con 120 caminos», que es
/// exactamente la cifra que este proyecto no publica.
#[test]
fn the_error_bar_is_wilson_and_is_not_zero_with_zero_failures() {
    let s = success_at_month(
        &comfortable_household(),
        &some_vol(),
        &cfg(120),
        137,
    )
    .expect("el sorteo no falla");
    assert_eq!(s.failures, 0);
    assert_eq!(s.success, 1.0);
    println!(
        "[wilson] 0/{} caminos ⇒ éxito {:.6}, wilson_low {:.6}, barra {:.4} pp, regla de tres {:?}",
        s.paths, s.success, s.wilson_low, s.half_width_pp, s.rule_of_three_upper
    );
    let n = f64::from(s.paths);
    let z2 = 1.96f64 * 1.96;
    assert!(
        (s.wilson_low - n / (n + z2)).abs() < 1e-12,
        "con 0 fallos, Wilson ES n/(n+z²): {} vs {}",
        s.wilson_low,
        n / (n + z2)
    );
    assert!(
        (s.wilson_low - 0.968_979_73).abs() < 1e-7,
        "wilson_low = {}",
        s.wilson_low
    );
    assert!(
        (s.half_width_pp - 3.102_03).abs() < 1e-4,
        "half_width_pp = {}",
        s.half_width_pp
    );
    assert!(s.half_width_pp > 0.0, "una barra de error de cero es mentira");
    assert_eq!(s.rule_of_three_upper, Some(0.025));

    // **El umbral acota por abajo el TAMAÑO de la muestra.** De `n/(n+z²) ≥ u` sale
    // `n ≥ z²·u/(1−u)`: 73 caminos para poder decir 95, 381 para poder decir 99. Con menos, el
    // umbral es INALCANZABLE aunque no falle ni un camino — y eso no es un fallo del solver, es lo
    // que significa medir con pocas muestras.
    assert!(
        !SuccessAt::new(1, 60, 0, [0; 3]).meets(95),
        "60 caminos no dan para el 95 %"
    );
    assert!(SuccessAt::new(1, 73, 0, [0; 3]).meets(95));
    assert!(!SuccessAt::new(1, 380, 0, [0; 3]).meets(99));
    assert!(SuccessAt::new(1, 381, 0, [0; 3]).meets(99));
    // Los presupuestos del plan (500 buscando, 2.500 confirmando) cubren el rango 80–100 entero.
    assert!(SuccessAt::new(1, 500, 0, [0; 3]).meets(99));
    assert!(SuccessAt::new(1, 500, 0, [0; 3]).meets(100));
}

/// **La regla del umbral**: Wilson por debajo de 100, cero fallos en 100.
///
/// Se comprueba sobre las dos caras que el sorteo produce de verdad: un hogar que no falla nunca
/// (cumple hasta el 100) y uno que falla SIEMPRE (no cumple ni el 80). El caso intermedio —el que
/// separa el estimador puntual de la cota de Wilson— se ejercita sin sortear en la unidad
/// `the_threshold_rule_is_wilson_below_a_hundred_and_zero_failures_at_a_hundred`.
#[test]
fn the_threshold_rule_is_wilson_below_100_and_zero_failures_at_100() {
    let clean = success_at_month(&comfortable_household(), &some_vol(), &cfg(120), 137)
        .expect("el sorteo no falla");
    assert_eq!(clean.failures, 0);
    assert!(clean.meets(100));
    assert!(clean.meets(95));
    assert!(clean.meets(80));

    // El hogar inclinado SIN herencia, jubilándose en el mes 1: la necesidad anual son
    // `12 · 2.000 = 24.000 €` contra un tope de `4 % · 500.000 = 20.000 €`. Falla en todos los
    // caminos, y ahí no hay volatilidad que valga: `L(0)` es el saldo tecleado.
    let doomed = success_at_month(&tilting_household(false), &low_vol(), &cfg(120), 1)
        .expect("el sorteo no falla");
    assert_eq!(doomed.failures, doomed.paths);
    assert_eq!(doomed.success, 0.0);
    assert_eq!(
        doomed.by_kind[KIND_INITIAL_RATE_EXCEEDED], doomed.paths,
        "todos los fallos son de tasa inicial (F2), no de cartera agotada"
    );
    assert!(!doomed.meets(100));
    assert!(!doomed.meets(80));
    // Y con cero éxitos la cota de Wilson tampoco es exactamente 0 por arriba, pero sí lo es por
    // abajo: es el lado que decide.
    assert_eq!(doomed.wilson_low, 0.0);
    assert_eq!(doomed.rule_of_three_upper, None);
}

/// **La tira anual se evalúa en la rejilla que pasa el llamante**, en su orden y con sus
/// repeticiones: este crate no sabe de fechas de nacimiento y no se inventa un muestreo.
#[test]
fn the_yearly_strip_is_evaluated_on_the_grid_the_caller_passes() {
    let input = comfortable_household();
    let vols = some_vol();
    let config = cfg(60);
    // Desordenada y con un mes repetido a propósito.
    let grid = [24u32, 12, 12, 240];
    let strip = success_by_retirement_month(&input, &vols, &config, &grid)
        .expect("el sorteo no falla");

    assert_eq!(strip.len(), grid.len());
    assert_eq!(
        strip.iter().map(|s| s.month).collect::<Vec<_>>(),
        grid.to_vec(),
        "la tira sale en el MISMO orden que la rejilla"
    );
    assert_eq!(strip[1], strip[2], "el mismo mes da la misma medición");
    // Cada entrada es exactamente lo que `success_at_month` mide por su cuenta: sostener el motor
    // entre evaluaciones no cambia ni un contador.
    for s in &strip {
        let alone = success_at_month(&input, &vols, &config, s.month).expect("el sorteo no falla");
        assert_eq!(*s, alone, "mes {}", s.month);
    }

    // Rejilla vacía: vector vacío, pero la configuración se valida igual.
    let empty = success_by_retirement_month(&input, &vols, &config, &[]).expect("valida");
    assert!(empty.is_empty());
    let bad = success_by_retirement_month(&input, &vols, &cfg(0), &[]);
    assert!(bad.is_err(), "un McConfig inválido es un error aunque no haya nada que sortear");
}

/// **Números aleatorios COMUNES**: la muestra de búsqueda es un PREFIJO bit a bit de la de
/// confirmación.
///
/// `path_rng(seed, p)` no depende ni de `paths` ni del mes de jubilación, así que ampliar la
/// muestra no reescribe la que había. Es lo que hace que la fase (D) del solve pueda desmentir a
/// la búsqueda sin estar comparando dos mercados distintos.
///
/// El test compara **las series enteras**, no solo el veredicto: si el flujo se moviera, dos
/// `net_worth` de 241 meses no coincidirían en el último bit.
#[test]
fn common_random_numbers_make_the_confirmation_a_superset() {
    let input = tilting_household(true);
    let vols = low_vol();
    let small = cfg(120);
    let big = cfg(300);
    let k = 101u32;
    let scenario = retiring_at(&input, k);

    let mut failures_small = 0u32;
    for p in 0..small.paths {
        let a = run_path(&scenario, &vols, &small, p).expect("no falla");
        let b = run_path(&scenario, &vols, &big, p).expect("no falla");
        assert_eq!(a.net_worth, b.net_worth, "camino {p}: net_worth");
        assert_eq!(a.liquid_worth, b.liquid_worth, "camino {p}: liquid_worth");
        assert_eq!(a.failure_month_index, b.failure_month_index, "camino {p}");
        assert_eq!(a.failure_kind, b.failure_kind, "camino {p}");
        if a.failure_month_index.is_some() {
            failures_small += 1;
        }
    }

    // Y el mes NO entra en el sorteo: el camino 0 vive el mismo mercado jubilándose en 101 o en 7.
    let other = retiring_at(&input, 7);
    let at_101 = run_path(&scenario, &vols, &small, 0).expect("no falla");
    let at_7 = run_path(&other, &vols, &small, 0).expect("no falla");
    assert_ne!(
        at_101.retirement_month_index, at_7.retirement_month_index,
        "los dos escenarios sí se jubilan en meses distintos"
    );
    assert_eq!(
        at_101.net_worth[0], at_7.net_worth[0],
        "el saldo inicial es el mismo"
    );

    // El recuento del solve con 120 caminos es el de esos mismos 120 caminos.
    let s = success_at_month(&input, &vols, &small, k).expect("no falla");
    assert_eq!(s.failures, failures_small);
    // Y los 120 primeros de los 300 son estos: el de 300 no puede tener MENOS fallos entre ellos.
    let big_s = success_at_month(&input, &vols, &big, k).expect("no falla");
    assert!(big_s.failures >= s.failures);
}

// =================================================================================================
// La fecha válida
// =================================================================================================

/// **La fecha devuelta está VERIFICADA, no supuesta**: se vuelve a evaluar con el presupuesto de
/// confirmación y cumple. Es el invariante de la bisección de la casa, trasladado a una función
/// objetivo muestral.
#[test]
fn the_valid_date_is_verified_not_assumed() {
    let input = tilting_household(true);
    let vols = low_vol();
    let search = cfg(120);
    let confirm = cfg(300);
    let solve = valid_retirement_month(&input, &vols, &search, &confirm, 90, 1).expect("no falla");

    println!(
        "[fecha] mes {:?} · éxito {:.4} · wilson_low {:.4} · barra {:.3} pp · \
         predecesor {:?} · aprox {} · sorteos {}+{}",
        solve.month,
        solve.success,
        solve.wilson_low,
        solve.half_width_pp,
        solve.predecessor_success,
        solve.date_is_approximate,
        solve.draws_search,
        solve.draws_confirm
    );

    let month = solve.month.expect("hay fecha");
    assert!(!solve.date_is_approximate);
    // Re-evaluación INDEPENDIENTE con el presupuesto de confirmación.
    let again = success_at_month(&input, &vols, &confirm, month).expect("no falla");
    assert!(
        again.meets(90),
        "el mes devuelto no cumple al reevaluarlo: éxito {}, wilson_low {}",
        again.success,
        again.wilson_low
    );
    assert_eq!(again.success, solve.success);
    assert_eq!(again.wilson_low, solve.wilson_low);
    // El presupuesto declarado se respeta.
    assert!(solve.draws_search <= 25, "{} sorteos", solve.draws_search);
    assert!(solve.draws_confirm <= 14, "{} sorteos", solve.draws_confirm);
}

/// **Una curva de éxito NO monótona sigue devolviendo un mes verificado.**
///
/// El hogar es el de la tabla de [`tilting_household`]: el éxito baja con `k` (ingreso plano
/// contra gasto indexado), salta en el mes 101 con la herencia y vuelve a caer. Es exactamente el
/// caso en el que «biseca y confía en la monotonía» devolvería cualquier cosa.
///
/// Predicción escrita antes de correr: la rejilla del bracket es `[1, 61, 121, 181, 240]`; 121 es
/// el primero que cumple; el refinado anual `[73, 85, 97, 109]` abre en 109; y la bisección
/// mensual sobre `(97, 109]` cierra en **101** — el primer mes cuyo `L(k−1)` ya lleva la herencia
/// del mes 100.
#[test]
fn a_non_monotone_success_curve_still_returns_a_verified_month() {
    let input = tilting_household(true);
    let vols = low_vol();
    let config = cfg(120);

    // 1) La curva NO es monótona, y se mide: la tira baja en algún tramo.
    let grid = [1u32, 61, 121, 181, 240];
    let strip = success_by_retirement_month(&input, &vols, &config, &grid).expect("no falla");
    for s in &strip {
        println!(
            "[tira] mes {:>3} · éxito {:.4} · wilson_low {:.4} · fallos {:>3} {:?}",
            s.month, s.success, s.wilson_low, s.failures, s.by_kind
        );
    }
    assert!(
        strip.windows(2).any(|w| w[1].success < w[0].success),
        "la curva de este hogar TIENE que bajar en algún tramo; si no, el test no prueba nada"
    );

    // 2) Y aun así la fecha devuelta cumple al reevaluarla.
    let confirm = cfg(300);
    let solve = valid_retirement_month(&input, &vols, &config, &confirm, 90, 1).expect("no falla");
    let month = solve.month.expect("hay fecha");
    println!(
        "[fecha/no-monótona] mes {month} · éxito {:.4} · predecesor {:?} · sorteos {}+{}",
        solve.success, solve.predecessor_success, solve.draws_search, solve.draws_confirm
    );
    assert_eq!(month, 101, "la ventana se abre con la herencia del mes 100");
    assert!(success_at_month(&input, &vols, &confirm, month)
        .expect("no falla")
        .meets(90));
    // El predecesor se audita y está MUY por debajo: es el mes sin herencia.
    let pred = solve.predecessor_success.expect("hay predecesor");
    assert!(pred < 0.5, "éxito del mes 100 = {pred}");
}

/// **El suelo del puente lo pone el LLAMANTE** — y es el único sitio donde `bridge_max_years`
/// acota la FECHA (el tope de tasa inicial que el puente levanta es cosa del motor, C2).
///
/// El hogar cumple en cualquier mes, así que sin suelo la fecha es el primer mes del bracket. Con
/// `k_min = P − 12·años_máximos` la fecha no puede empezar antes, y sale exactamente el suelo.
#[test]
fn the_bridge_floor_keeps_the_date_from_starting_before_p_minus_the_max_years() {
    let mut input = comfortable_household();
    // Pensión con fecha: entra en caja en el mes `start_index + 1` = 181.
    let pension_month = 181u32;
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: pension_month - 1,
        monthly_today: Decimal::from(1_200),
        indexed: true,
        fraction_while_partial: Decimal::ZERO,
    });
    let vols = some_vol();
    // 120 caminos y no 60: con 0 fallos, `wilson_low = n/(n+z²)`, y 60 caminos topan en 0,9398 —
    // el umbral del 95 % es INALCANZABLE con esa muestra por limpia que salga (hacen falta 73).
    // Es la cota que documenta `SuccessAt::meets`, y aquí muerde de verdad.
    let search = cfg(120);
    let confirm = cfg(300);

    let free = valid_retirement_month(&input, &vols, &search, &confirm, 95, 1).expect("no falla");
    assert_eq!(
        free.month,
        Some(1),
        "sin suelo, el primer mes del bracket ya cumple"
    );

    // Puente de 10 años ⇒ la fecha no puede ser anterior a `P − 120`.
    let bridge_max_years = 10u32;
    let k_min = pension_month.saturating_sub(12 * bridge_max_years).max(1);
    assert_eq!(k_min, 61);
    let floored =
        valid_retirement_month(&input, &vols, &search, &confirm, 95, k_min).expect("no falla");
    println!(
        "[suelo] sin suelo {:?} · con k_min={k_min} ⇒ {:?} (predecesor {:?})",
        free.month, floored.month, floored.predecessor_success
    );
    assert_eq!(floored.month, Some(k_min));
    assert!(floored.month > free.month, "el suelo tiene que morder");
    assert_eq!(
        floored.predecessor_success, None,
        "en el suelo no hay predecesor que auditar: `None`, nunca un 0"
    );

    // Un suelo por encima del horizonte no es una fecha temprana: es que no hay fecha.
    let beyond =
        valid_retirement_month(&input, &vols, &search, &confirm, 95, 10_000).expect("no falla");
    assert_eq!(beyond.month, None);
    assert_eq!(beyond.draws_search, 0, "no se sortea nada");
}

/// **Sin fecha en el horizonte se devuelve `None`, jamás un 0** — un 0 se leería como «ya
/// puedes» — y se publica la mejor observación para que la UI pueda decir cuánto falta.
#[test]
fn no_valid_date_in_the_horizon_is_none_not_zero_and_publishes_best_effort() {
    let input = tilting_household(false);
    let vols = low_vol();
    let search = cfg(120);
    let confirm = cfg(300);
    let solve = valid_retirement_month(&input, &vols, &search, &confirm, 90, 1).expect("no falla");

    println!(
        "[sin fecha] month {:?} · best_effort {:?} · fallos por motivo {:?} · sorteos {}+{}",
        solve.month,
        solve.best_effort,
        solve.failures_by_kind,
        solve.draws_search,
        solve.draws_confirm
    );
    assert_eq!(solve.month, None);
    assert_ne!(solve.month, Some(0), "un cero sería la respuesta contraria");
    assert!(!solve.date_is_approximate, "no hay fecha que aproximar");
    assert_eq!(solve.predecessor_success, None);
    assert_eq!(solve.draws_confirm, 0, "sin candidato no se confirma nada");
    // El bracket se recorre entero: 5 puntos con `k_min = 1` y `H = 240`.
    assert_eq!(solve.draws_search, 5);

    let (month, success) = solve.best_effort.expect("hay observación");
    assert!(month >= 1 && month <= input.horizon_months);
    assert!((0.0..=1.0).contains(&success));
    // Y dice POR QUÉ no hay fecha: la tasa inicial, no la cartera agotada.
    assert!(
        solve.failures_by_kind[KIND_INITIAL_RATE_EXCEEDED] > 0,
        "{:?}",
        solve.failures_by_kind
    );
}

/// **Al 100 % la regla es «cero fallos», y la cota de la regla de tres se publica.**
///
/// Con 0 fallos de `N`, el riesgo real está por debajo de `3/N` con confianza ≈ 95 %: es la única
/// afirmación defendible que se puede hacer sobre un suceso que no se ha observado nunca, y por
/// eso viaja al lado del «100 %» en vez de dejar que se lea como una certeza.
#[test]
fn at_a_hundred_the_rule_is_zero_failures_and_the_rule_of_three_is_published() {
    let input = comfortable_household();
    let vols = some_vol();
    let search = cfg(60);
    let confirm = cfg(300);
    let solve = valid_retirement_month(&input, &vols, &search, &confirm, 100, 1).expect("no falla");

    let month = solve.month.expect("hay fecha");
    println!(
        "[100 %] mes {month} · éxito {:.6} · regla de tres {:?} · barra {:.4} pp",
        solve.success, solve.rule_of_three_upper, solve.half_width_pp
    );
    assert_eq!(solve.success, 1.0);
    assert_eq!(solve.failures_by_kind, [0, 0, 0]);
    assert_eq!(solve.rule_of_three_upper, Some(3.0 / 300.0));
    assert!(
        solve.half_width_pp > 0.0,
        "el 100 % se publica CON su barra de error"
    );

    // Y un solo fallo tumba el 100 % por mucho que el estimador puntual redondee a uno.
    let almost = SuccessAt::new(month, 2_500, 1, [1, 0, 0]);
    assert!(almost.success > 0.999);
    assert!(!almost.meets(100));
    assert!(almost.meets(99), "wilson_low = {}", almost.wilson_low);
    assert_eq!(almost.rule_of_three_upper, None);
}

/// El escenario que evalúa el solve es EXACTAMENTE `AtMonth(k)` + cruce como lectura, y ninguna
/// otra mutación: si alguien añadiera una tercera, este test lo dice.
#[test]
fn retiring_at_touches_the_trigger_and_the_crossing_and_nothing_else() {
    let input = comfortable_household();
    let scenario = retiring_at(&input, 77);
    assert_eq!(
        scenario.phase_plan.retirement_trigger,
        RetirementTrigger::AtMonth(77)
    );
    assert!(scenario.phase_plan.crossing_is_reading_only);
    assert!(
        scenario.fire_target.is_some(),
        "el objetivo sigue viajando: es lectura, no se tira"
    );
    // El resto del plan, intacto: deshechas las dos mutaciones, el `PhasePlan` vuelve a ser el
    // de la entrada campo a campo (`ProjectionInput` no es `PartialEq`; `PhasePlan` sí, y es
    // donde viven las dos únicas mutaciones).
    let mut rebuilt = scenario.phase_plan.clone();
    rebuilt.retirement_trigger = input.phase_plan.retirement_trigger;
    rebuilt.crossing_is_reading_only = input.phase_plan.crossing_is_reading_only;
    assert_eq!(rebuilt, input.phase_plan);
    assert_eq!(scenario.horizon_months, input.horizon_months);
    assert_eq!(scenario.assets.len(), input.assets.len());
    assert_eq!(
        scenario.planning_monthly_cash_adjustment,
        input.planning_monthly_cash_adjustment
    );
    // Y el motivo de fallo se sigue clasificando por el motor, no por este crate.
    assert_eq!(PathFailure::InitialRateExceeded.code(), "initial_rate_exceeded");
}

