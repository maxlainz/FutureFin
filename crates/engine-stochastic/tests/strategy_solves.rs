//! **Los tres solves de estrategia** (WP E8 de 5.0.0): aportación mínima, primer mes de coast y
//! primer mes de media jornada.
//!
//! Mismo arnés y misma disciplina que `tests/solve_mc.rs`: hogares **pequeños y derivados a mano**,
//! con la aritmética de cada predicción escrita al lado del test ANTES de correrlo
//! (`futurefin-research-methodology`, «predice el número antes de ejecutar»). Lo que se prueba aquí
//! es el SOLVER, y un solver se prueba con una función objetivo cuya forma se conoce de memoria.
//!
//! **Volatilidad cero a propósito.** Casi todos los hogares llevan `vec![None]`: sin σ el sorteo
//! degenera —todos los caminos viven el mismo mercado— y `éxito(k)` vale 0 o 1. Eso es
//! exactamente lo que hace falta para pinear un solver: la incertidumbre estadística ya la miden
//! los tests de `solve_mc.rs`; aquí lo que se pinea es **qué escenario elige la bisección**, y con
//! ruido encima el test mediría la semilla.
//!
//! Con cero fallos de `n` caminos, `wilson_low = n/(n + 1,96²)`: con 100 caminos son 0,9629 y con
//! 200, 0,9811 — los dos por encima del umbral del 95 % que usa todo el fichero, y por eso los
//! presupuestos de búsqueda/confirmación son 100/200 (la casa pide ≤ 300 en los tests).

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, rule_remainder};
use futurefin_engine::{
    project_net_worth_series, run_stopping_at, ExpenseBasis, InitialRateGate, PartialPhase,
    ProjectionInput,
};
use futurefin_engine_stochastic::{
    coast_stop_month, contributing_extra, earliest_partial_start, minimum_extra_contribution,
    partial_starting_at, retiring_at, stopping_at, success_at_month, McConfig,
    StrategySolveWarning, KIND_INITIAL_RATE_EXCEEDED, KIND_PORTFOLIO_DEPLETED,
};
use rust_decimal::Decimal;

/// Semilla fija: los tests miden un sorteo concreto, no «un sorteo».
const SEED: u64 = 20_260_906;
/// El umbral con el que se juzga todo el fichero (el default del perfil).
const THRESHOLD: u32 = 95;

fn search() -> McConfig {
    McConfig {
        seed: SEED,
        paths: 100,
        ..Default::default()
    }
}

fn confirm() -> McConfig {
    McConfig {
        seed: SEED,
        paths: 200,
        ..Default::default()
    }
}

/// Sin volatilidad declarada: el sorteo sigue ocurriendo, pero todos los caminos son el camino
/// determinista. Un solo activo.
fn no_vol() -> Vec<Option<f64>> {
    vec![None]
}

fn eur(v: i64) -> Decimal {
    Decimal::from(v)
}

// =================================================================================================
// Los hogares del arnés
// =================================================================================================

/// **El hogar de la PUERTA DE TASA INICIAL**, con todo lo que decide escrito en enteros redondos:
/// un activo líquido al **0 %**, sin inflación, sin impuestos y sin pasivos. Con eso,
///
/// ```text
///   L(k) = activo + k · sobrante          (sobrante = ingreso − gasto, más lo que se inyecte)
/// ```
///
/// y la puerta F2 del mes `R` es una desigualdad de una línea:
/// `12 · gasto_jubilación ≤ swr/100 · L(R−1)`. Todo lo demás (F1) queda holgado por construcción
/// en cada test que lo use.
fn flat_household(
    horizon: u32,
    income: i64,
    expense: i64,
    asset: i64,
    retirement_expense: i64,
    swr_pct: Option<i64>,
) -> ProjectionInput {
    let mut input = base_input(
        horizon,
        eur(income),
        eur(expense),
        vec![mk_asset(1, eur(asset), true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    );
    input.phase_plan.income_retirement_monthly = Decimal::ZERO;
    input.phase_plan.expense_retirement_monthly = eur(retirement_expense);
    input.phase_plan.initial_rate = swr_pct.map(|p| InitialRateGate {
        swr_pct: eur(p),
        bridge: None,
    });
    input
}

/// El hogar de la media jornada: el de arriba más una fase parcial declarada. `start_month` es un
/// marcador — el solve lo reescribe en cada candidato.
fn partial_household(
    horizon: u32,
    income: i64,
    expense: i64,
    asset: i64,
    retirement_expense: i64,
    partial_income: i64,
    swr_pct: Option<i64>,
) -> ProjectionInput {
    let mut input = flat_household(
        horizon,
        income,
        expense,
        asset,
        retirement_expense,
        swr_pct,
    );
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 1,
        income_monthly: eur(partial_income),
        // El gasto de la fase es el de JUBILACIÓN (default D10).
        expense_basis: ExpenseBasis::Retirement,
    });
    input
}

// =================================================================================================
// 1 · Aportación mínima
// =================================================================================================

/// **La aportación publicada está VERIFICADA al umbral, y una decena menos NO cumple.**
///
/// Hogar: 240 meses, 3.000 € de ingreso contra 2.500 € de gasto (sobrante **500 €/mes**),
/// 100.000 € líquidos al 0 %, gasto de jubilación 2.000 €/mes, puerta al **4 %**, jubilación
/// forzada en el mes **61**.
///
/// La aritmética, antes de correr nada:
///
/// ```text
///   necesidad anual en R = 12 · 2.000            = 24.000 €
///   tope de tasa inicial = 4 % · L(60)           ⇒ hace falta L(60) ≥ 600.000 €
///   L(60) = 100.000 + 60 · (500 + c)             ⇒ c ≥ (500.000/60) − 500 = 7.833,33 €/mes
/// ```
///
/// El doblaje arranca en `max(100, sobrante) = 500` y necesita **cuatro** doblajes
/// (500 → 1.000 → 2.000 → 4.000 → **8.000**, la primera que cumple), así que el techo de búsqueda
/// publicado son 8.000 €. Doce halvings sobre `[4.000, 8.000]` dejan `hi` a menos de 1 € del
/// 7.833,33 y el redondeo hacia arriba a decenas lo publica como **7.840 €/mes**:
///
/// ```text
///   c = 7.840  ⇒ L(60) = 100.000 + 60 · 8.340 = 600.400  ⇒ 4 % = 24.016 ≥ 24.000  ✓
///   c = 7.830  ⇒ L(60) = 100.000 + 60 · 8.330 = 599.800  ⇒ 4 % = 23.992 < 24.000  ✗
/// ```
///
/// Esa segunda línea es el test de verdad: lo publicado es **mínimo a la decena de euro**, no un
/// número grande que casualmente funciona.
#[test]
fn the_minimum_extra_contribution_is_verified_at_the_threshold() {
    let input = flat_household(240, 3_000, 2_500, 100_000, 2_000, Some(4));
    let vols = no_vol();
    let (s, c) = (search(), confirm());

    let solve = minimum_extra_contribution(&input, &vols, &s, &c, THRESHOLD, 61)
        .expect("el sorteo no falla");

    assert!(!solve.underfunded, "el hogar sí llega aportando más");
    assert_eq!(solve.month, 61);
    assert_eq!(solve.search_ceiling, eur(8_000), "cuatro doblajes desde 500");
    assert_eq!(
        solve.extra_monthly,
        Some(eur(7_840)),
        "7.833,33 redondeado hacia arriba a decenas"
    );

    // **Verificado**: la medición publicada es la de la CONFIRMACIÓN y cumple el umbral.
    let stats = solve.success_at_solution.expect("hay solución, hay medición");
    assert_eq!(stats.paths, c.paths, "la publicada es la de confirmación");
    assert!(
        stats.meets(THRESHOLD),
        "éxito {} / wilson {} no cumple el umbral",
        stats.success,
        stats.wilson_low
    );

    // **Mínimo a la decena**: una decena menos falla, medido con el mismo presupuesto.
    let one_step_below = eur(7_830);
    let worse = success_at_month(
        &contributing_extra(&input, one_step_below, 61),
        &vols,
        &c,
        61,
    )
    .expect("el sorteo no falla");
    assert!(
        !worse.meets(THRESHOLD),
        "7.830 €/mes no debería cumplir (éxito {})",
        worse.success
    );
    assert_eq!(
        worse.by_kind[KIND_INITIAL_RATE_EXCEEDED], worse.paths,
        "y falla por la PUERTA DE TASA INICIAL, que es lo que este hogar mide"
    );

    // Sin nada extra tampoco: el solve no estaba resolviendo un problema que no existía.
    let bare = success_at_month(&input, &vols, &c, 61).expect("el sorteo no falla");
    assert!(!bare.meets(THRESHOLD));

    println!(
        "[E8] aportación mínima: {:?} €/mes · techo {} · {} sorteos de búsqueda + {} de confirmación",
        solve.extra_monthly, solve.search_ceiling, solve.draws_search, solve.draws_confirm
    );
}

/// **Un plan que ya cumple pide CERO, y lo pide en una sola sonda.**
///
/// `Some(0)` es una respuesta —«no te falta nada»— y no una ausencia; `None` está reservado al
/// infrafinanciado. El pin del presupuesto (1 sorteo de búsqueda) es la otra mitad: si alguien
/// quitara la sonda de `c = 0`, el solve seguiría devolviendo un número correcto pero pagaría
/// diecisiete sorteos por él.
#[test]
fn an_already_funded_plan_needs_no_extra_contribution() {
    // 2.000.000 € al 0 % contra una necesidad anual de 24.000 €: el 4 % son 82.400 €.
    let input = flat_household(240, 3_000, 2_000, 2_000_000, 2_000, Some(4));
    let solve = minimum_extra_contribution(&input, &no_vol(), &search(), &confirm(), THRESHOLD, 61)
        .expect("el sorteo no falla");

    assert_eq!(solve.extra_monthly, Some(Decimal::ZERO));
    assert!(!solve.underfunded);
    assert_eq!(solve.draws_search, 1, "solo la sonda de c = 0");
    assert_eq!(solve.draws_confirm, 1, "y su confirmación");
    assert_eq!(
        solve.search_ceiling,
        eur(1_000),
        "el techo es la escala de arranque: max(100, sobrante del mes 1)"
    );
    assert!(solve
        .success_at_solution
        .expect("hay medición")
        .meets(THRESHOLD));
    assert_eq!(solve.warning(), None);
}

/// **Infrafinanciado NO es cero.** Un hogar al que ni el techo de búsqueda le sirve devuelve
/// `extra_monthly: None` —nunca un `Some(0)`, que diría lo contrario— con `underfunded` y el techo
/// que se llegó a sondear.
///
/// Hogar: 120 meses, 2.600 € contra 2.500 € (sobrante **100 €/mes**), 100.000 € al 0 %, gasto de
/// jubilación 2.000 €, puerta al 4 %, y jubilación forzada en el mes **2**: solo hay UN mes en el
/// que aportar.
///
/// ```text
///   hace falta  L(1) ≥ 24.000 / 0,04                     = 600.000 €
///   techo de búsqueda = max(100, 100) · 2¹²              = 409.600 €/mes
///   L(1) con el techo = 100.000 + 100 + 409.600          = 509.700 €  <  600.000  ✗
/// ```
///
/// El presupuesto también está pineado: 1 sonda de `c = 0` + 1 del techo inicial + 12 doblajes = 14.
#[test]
fn a_household_that_cannot_reach_r_even_saving_everything_is_underfunded_not_zero() {
    let input = flat_household(120, 2_600, 2_500, 100_000, 2_000, Some(4));
    let solve = minimum_extra_contribution(&input, &no_vol(), &search(), &confirm(), THRESHOLD, 2)
        .expect("el sorteo no falla");

    assert!(solve.underfunded);
    assert_eq!(solve.extra_monthly, None, "jamás un cero aquí");
    assert_eq!(solve.search_ceiling, eur(409_600), "100 · 2¹²");
    assert_eq!(solve.draws_search, 14, "1 + 1 + 12 doblajes");
    assert_eq!(solve.draws_confirm, 0, "no hay nada que confirmar");
    assert_eq!(
        solve.warning(),
        Some(StrategySolveWarning::RetireAtAgeUnderfunded)
    );
    let stats = solve
        .success_at_solution
        .expect("se publica la medición del TECHO: lo más lejos que se llegó");
    assert_eq!(stats.paths, 100, "medido con el presupuesto de BÚSQUEDA");
    assert_eq!(
        stats.by_kind[KIND_INITIAL_RATE_EXCEEDED], stats.paths,
        "el motivo es la tasa inicial, y eso es lo que la UI tiene que poder decir"
    );
}

/// **La rejilla de la inyección, escrita como test**: `planning_monthly_cash_adjustment` es
/// **0-based** (el índice `i` es el mes `i+1` del bucle), así que aportar «hasta jubilarse en `r`»
/// son los índices `0..=r−2` — los meses del bucle `1..=r−1`. El mes `r`, el primero jubilado, ya
/// no aporta.
#[test]
fn the_extra_contribution_covers_the_working_months_and_not_one_more() {
    let input = flat_household(12, 3_000, 2_500, 10_000, 2_000, None);
    let with = contributing_extra(&input, eur(250), 5);
    let adj = &with.planning_monthly_cash_adjustment;

    assert_eq!(adj.len(), 12, "la longitud no cambia");
    for (i, v) in adj.iter().enumerate() {
        let expected = if i <= 3 { eur(250) } else { Decimal::ZERO };
        assert_eq!(*v, expected, "índice {i} (mes {} del bucle)", i + 1);
    }

    // `r` más allá del horizonte cubre lo que existe y no panica.
    let all = contributing_extra(&input, eur(7), 999);
    assert!(all
        .planning_monthly_cash_adjustment
        .iter()
        .all(|v| *v == eur(7)));
    // `r = 1` (jubilarse el primer mes) no aporta nada: no hay mes trabajado.
    let none = contributing_extra(&input, eur(7), 1);
    assert!(none
        .planning_monthly_cash_adjustment
        .iter()
        .all(|v| *v == Decimal::ZERO));
}

// =================================================================================================
// 2 · Coast
// =================================================================================================

/// El hogar de coast: 240 meses, 3.000 € contra 2.000 € (sobrante **1.000 €/mes**), un activo al
/// 0 %, gasto de jubilación 1.500 €/mes, puerta al 4 % y jubilación en el mes **61**.
///
/// ```text
///   necesidad anual  = 12 · 1.500 = 18.000 €   ⇒  hace falta L(60) ≥ 450.000 €
///   con corte en C:  L(60) = activo + (C − 1) · 1.000     (se aporta en 1..=C−1)
/// ```
fn coast_household(asset: i64) -> ProjectionInput {
    flat_household(240, 3_000, 2_000, asset, 1_500, Some(4))
}

/// **El PRIMER `C`, no el último** (corrección C8): la pregunta es «¿desde cuándo puedo dejar de
/// ahorrar?».
///
/// Con **399.500 €** de activo: `399.500 + (C−1)·1.000 ≥ 450.000 ⟺ C ≥ 51,5 ⟺ C ≥ 52`.
///
/// ```text
///   C = 51 ⇒ L(60) = 449.500  ⇒ 4 % = 17.980 < 18.000  ✗
///   C = 52 ⇒ L(60) = 450.500  ⇒ 4 % = 18.020 ≥ 18.000  ✓
/// ```
///
/// El test no se cree la bisección: reejecuta el sorteo en `C−1` y en `C` con el presupuesto de
/// confirmación y comprueba el escalón a mano.
#[test]
fn the_coast_month_is_the_earliest_you_can_stop_not_the_latest() {
    let input = coast_household(399_500);
    let vols = no_vol();
    let (s, c) = (search(), confirm());

    let solve =
        coast_stop_month(&input, &vols, &s, &c, THRESHOLD, 61).expect("el sorteo no falla");

    let stop = solve.stop_month.expect("este hogar sí puede parar");
    assert!(solve.warnings.is_empty());
    assert_eq!(solve.retirement_month, 61);
    assert_eq!(stop, 52, "el primer mes en que se puede parar");
    assert!(stop > 1, "si fuera 1 el escalón de abajo no probaría nada");

    // El escalón, medido a mano con el presupuesto grande.
    let before = success_at_month(&stopping_at(&input, stop - 1), &vols, &c, 61).expect("ok");
    let at = success_at_month(&stopping_at(&input, stop), &vols, &c, 61).expect("ok");
    assert!(
        !before.meets(THRESHOLD),
        "parar en {} debería fallar (éxito {})",
        stop - 1,
        before.success
    );
    assert!(
        at.meets(THRESHOLD),
        "parar en {stop} debería cumplir (éxito {})",
        at.success
    );
    assert_eq!(
        before.by_kind[KIND_INITIAL_RATE_EXCEEDED], before.paths,
        "y lo que falla es la puerta de tasa inicial"
    );

    println!(
        "[E8] coast: primer C = {stop} · liberado {:?} €/mes · {} sorteos de búsqueda + {} de confirmación",
        solve.freed_saving_monthly, solve.draws_search, solve.draws_confirm
    );
}

/// **No poder parar es una respuesta con nombre.** Con 300.000 € el hogar llega como mucho a
/// `L(60) = 360.000` (el 4 % son 14.400 € contra una necesidad de 18.000 €), así que ni aportando
/// durante toda la acumulación cumple: `stop_month: None` —jamás un mes— y el aviso
/// `coast_not_reachable`.
///
/// Y se resuelve en **un solo sorteo**: la sonda alta es el mejor plan de coast que existe, y si
/// falla no hay nada más que mirar.
#[test]
fn a_plan_that_can_never_stop_contributing_says_coast_not_reachable() {
    let input = coast_household(300_000);
    let solve = coast_stop_month(&input, &no_vol(), &search(), &confirm(), THRESHOLD, 61)
        .expect("el sorteo no falla");

    assert_eq!(solve.stop_month, None, "ni un cero ni un 61: no hay mes");
    assert_eq!(solve.freed_saving_monthly, None, "no hay ahorro que liberar");
    assert_eq!(
        solve.warnings,
        vec![StrategySolveWarning::CoastNotReachable]
    );
    assert_eq!(solve.draws_search, 1, "solo la sonda alta");
    assert_eq!(solve.draws_confirm, 0);
    assert_eq!(
        solve.warnings.first().map(|w| w.code()),
        Some("coast_not_reachable")
    );
    let stats = solve.success_at_solution.expect("se publica por qué no hay");
    assert_eq!(stats.by_kind[KIND_INITIAL_RATE_EXCEEDED], stats.paths);
}

/// **«Puedes dejar de aportar ya» se dice con `Some(1)`.** Con 500.000 € el hogar ya cumple sin
/// aportar un euro más (`4 % · 500.000 = 20.000 ≥ 18.000`), y el ahorro liberado es el sobrante
/// entero del mes 1: **1.000 €/mes**.
///
/// Presupuesto: dos sondas (alta y baja) y una confirmación. La bisección no llega a correr.
#[test]
fn a_household_that_can_coast_today_stops_at_month_one() {
    let input = coast_household(500_000);
    let solve = coast_stop_month(&input, &no_vol(), &search(), &confirm(), THRESHOLD, 61)
        .expect("el sorteo no falla");

    assert_eq!(solve.stop_month, Some(1));
    assert_eq!(solve.freed_saving_monthly, Some(eur(1_000)));
    assert!(solve.warnings.is_empty());
    assert_eq!(solve.draws_search, 2, "sonda alta + sonda baja");
    assert_eq!(solve.draws_confirm, 1);
}

/// **Supuesto S4 — el ahorro liberado es caja DISPONIBLE y no se reinvierte.**
///
/// Sobre el hogar de `the_coast_month_is_the_earliest…` con su corte en el mes 52, y todo medido
/// en el camino DETERMINISTA (`Decimal`, sin un `f64` de por medio):
///
/// ```text
///   sin corte:  net_worth[60] = 399.500 + 60 · 1.000 = 459.500 €
///   con corte:  net_worth[60] = 399.500 + 51 · 1.000 = 450.500 €   (se aporta en 1..=51)
///   disposable_cash[51] = 0            (el mes 51 todavía aporta)
///   disposable_cash[52] = 1.000        (el primero sin aportar: el sobrante ENTERO)
///   disposable_cash_total = 9 · 1.000 = 9.000   (meses 52..60; del 61 en adelante hay déficit)
/// ```
///
/// Y la identidad que cierra el supuesto: **los 9.000 € liberados son exactamente los que le
/// faltan a la cartera**. Si se reinvirtieran —o si compusieran por su cuenta— esa resta no
/// cuadraría.
///
/// El corte es INCLUSIVO (`k ≥ C` ⇒ techo 0, `phases.rs:321-324`), así que el primer mes sin
/// aportación es `C` y no `C+1`: es el mes que `freed_saving_monthly` lee.
#[test]
fn the_freed_saving_of_coast_is_disposable_and_is_not_reinvested() {
    let input = coast_household(399_500);
    let retiring = retiring_at(&input, 61);

    let no_stop = project_net_worth_series(&retiring).expect("el motor no falla");
    let with_stop = run_stopping_at(&retiring, 52).expect("el motor no falla");

    assert_eq!(no_stop.net_worth[60], eur(459_500));
    assert_eq!(with_stop.net_worth[60], eur(450_500));

    assert_eq!(
        with_stop.disposable_cash[51],
        Decimal::ZERO,
        "el mes 51 todavía aporta"
    );
    assert_eq!(
        with_stop.disposable_cash[52],
        eur(1_000),
        "el mes 52 es el primero sin aportar y libera el sobrante entero"
    );
    assert_eq!(with_stop.disposable_cash_total, eur(9_000));
    assert_eq!(
        no_stop.net_worth[60] - with_stop.net_worth[60],
        with_stop.disposable_cash_total,
        "lo liberado es EXACTAMENTE lo que le falta a la cartera: no se reinvierte ni compone"
    );

    // Y es lo que el solve publica.
    let solve = coast_stop_month(&input, &no_vol(), &search(), &confirm(), THRESHOLD, 61)
        .expect("el sorteo no falla");
    assert_eq!(solve.stop_month, Some(52));
    assert_eq!(solve.freed_saving_monthly, Some(eur(1_000)));
}

/// El escenario que el camino ESTOCÁSTICO bisecciona y la plantilla determinista del motor son la
/// misma mutación. Sin este control, las dos podrían divergir en silencio al primer campo nuevo.
#[test]
fn the_stochastic_stop_scenario_is_the_same_mutation_as_the_engine_template() {
    let input = coast_household(399_500);
    let scenario = stopping_at(&input, 52);
    assert_eq!(scenario.phase_plan.contributions_stop_month, Some(52));

    let via_scenario = project_net_worth_series(&scenario).expect("ok");
    let via_template = run_stopping_at(&input, 52).expect("ok");
    assert_eq!(via_scenario.net_worth, via_template.net_worth);
    assert_eq!(
        via_scenario.disposable_cash,
        via_template.disposable_cash
    );
}

// =================================================================================================
// 3 · Media jornada
// =================================================================================================

/// El hogar de la media jornada: 240 meses, 3.000 € contra 2.000 € (sobrante **1.000 €/mes**),
/// **101.750 €** al 0 %, gasto de jubilación **2.500 €/mes** e ingreso de media jornada
/// **500 €/mes** ⇒ la fase drena **2.000 €/mes**.
///
/// Criterio de la fase (`AtMonth(H+1)`: nunca se jubila del todo, así que solo puede fallar F1):
///
/// ```text
///   L(S−1) = 101.750 + (S−1)·1.000        drenaje de la fase = 2.000 · (241 − S)
///   fase OK ⟺ 101.750 + 1.000(S−1) ≥ 2.000(241 − S) ⟺ 3.000·S ≥ 381.250 ⟺ S ≥ 128
///     S = 127 ⇒ 227.750 contra 228.000  ✗ (le faltan 250 €)
///     S = 128 ⇒ 228.750 contra 226.000  ✓ (le sobran 2.750 €)
/// ```
fn barista_household(swr_pct: Option<i64>) -> ProjectionInput {
    partial_household(240, 3_000, 2_000, 101_750, 2_500, 500, swr_pct)
}

/// **UNA capa anidada, no un producto.**
///
/// Es la propiedad que hace viable este solve: el coste es `bisección_de_S + UNA fecha`, no
/// `bisección_de_S × fecha`. Con los presupuestos del módulo, resolver la fecha dentro del
/// criterio de cada candidato costaría ~14 × ~39 ≈ 550 sorteos.
///
/// Sobre `barista_household(None)` —sin puerta de tasa inicial, para que la jubilación total sí
/// exista— la cuenta derivada a mano:
///
/// ```text
///   fase:   sonda S=1 (falla) + sonda S=240 (cumple) + 8 de bisección  = 10 de búsqueda
///           confirmación de S* = 128                                   =  1 de confirmación
///   fecha (k_min = 129, sin puerta):
///           2.000·(k−128) + 2.500·(241−k) ≤ 228.750  ⟺  k ≥ 235,5  ⟺  k ≥ 236
///           bracket [129, 189, 240] = 3 · anuales 201/213/225/237 = 4 · bisección 231/234/235/236 = 4
///                                                                   = 11 de búsqueda
///           confirmación de 236 + auditoría de 235                   =  2 de confirmación
///   ----------------------------------------------------------------------------------
///   TOTAL 24 sorteos  (y exactamente UNA fecha)
/// ```
#[test]
fn the_partial_phase_solve_is_one_nested_layer_not_a_product() {
    let input = barista_household(None);
    let vols = no_vol();
    let (s, c) = (search(), confirm());

    let solve = earliest_partial_start(&input, &vols, &s, &c, THRESHOLD).expect("el sorteo no falla");

    let start = solve.start_month.expect("esta fase sí puede empezar");
    assert_eq!(start, 128);
    assert!(solve.warnings.is_empty(), "{:?}", solve.warnings);

    // El escalón de la fase, medido a mano con el presupuesto grande.
    let never = input.horizon_months + 1;
    let before =
        success_at_month(&partial_starting_at(&input, start - 1), &vols, &c, never).expect("ok");
    let at = success_at_month(&partial_starting_at(&input, start), &vols, &c, never).expect("ok");
    assert!(!before.meets(THRESHOLD), "éxito en {} = {}", start - 1, before.success);
    assert!(at.meets(THRESHOLD), "éxito en {start} = {}", at.success);
    assert_eq!(
        before.by_kind[KIND_PORTFOLIO_DEPLETED], before.paths,
        "durante la media jornada solo puede fallar F1 (supuesto S1)"
    );

    // **UNA** fecha, y el presupuesto entero.
    let date = solve.full_retirement.expect("hay fase, hay fecha");
    assert_eq!(date.month, Some(236));
    let total = solve.draws_search + solve.draws_confirm + date.draws_search + date.draws_confirm;
    assert!(
        total <= 40,
        "{total} sorteos: esto es un producto, no una capa anidada"
    );
    assert_eq!(
        (solve.draws_search, solve.draws_confirm),
        (10, 1),
        "presupuesto propio de la fase"
    );
    assert_eq!(
        (date.draws_search, date.draws_confirm),
        (11, 2),
        "presupuesto de la ÚNICA fecha anidada"
    );

    println!(
        "[E8] media jornada: S* = {start} · jubilación total {:?} · {total} sorteos \
         ({} + {} propios, {} + {} de la fecha)",
        date.month, solve.draws_search, solve.draws_confirm, date.draws_search, date.draws_confirm
    );
}

/// **Empezar y no jubilarse nunca es un aviso propio**, distinto de no poder empezar.
///
/// El mismo hogar con la puerta al 4 %: la fase arranca igual en el mes 128 (F2 no se evalúa
/// mientras no haya jubilación total), pero la jubilación total pediría
/// `L(k−1) ≥ 12·2.500 / 0,04 = 750.000 €` y la cartera nunca pasa de 228.750 €. La fecha vuelve
/// `month: None` y de ahí sale `partial_never_fully_retires`.
#[test]
fn starting_the_partial_phase_and_never_fully_retiring_is_its_own_warning() {
    let input = barista_household(Some(4));
    let solve = earliest_partial_start(&input, &no_vol(), &search(), &confirm(), THRESHOLD)
        .expect("el sorteo no falla");

    assert_eq!(solve.start_month, Some(128), "la fase sí puede empezar");
    assert_eq!(
        solve.warnings,
        vec![StrategySolveWarning::PartialNeverFullyRetires]
    );
    assert_eq!(
        solve.warnings.first().map(|w| w.code()),
        Some("partial_never_fully_retires")
    );

    let date = solve.full_retirement.expect("la fecha se intentó");
    assert_eq!(date.month, None, "ni un cero ni el horizonte: no hay fecha");
    assert!(
        date.best_effort.is_some(),
        "sin fecha hay que poder enseñar lo más cerca que se llegó"
    );
}

/// **No poder empezar es otro aviso, y se decide en dos sondas.**
///
/// Hogar sin sobrante (2.000 € contra 2.000 €), **1.000 €** de cartera y una media jornada sin
/// ingreso contra un gasto de 2.500 €/mes: la fase pide 2.500 € el primer mes y la cartera tiene
/// 1.000 €, empiece cuando empiece. `start_month: None` y `partial_never_starts`.
#[test]
fn a_phase_that_fails_even_at_the_horizon_says_partial_never_starts() {
    let input = partial_household(240, 2_000, 2_000, 1_000, 2_500, 0, None);
    let solve = earliest_partial_start(&input, &no_vol(), &search(), &confirm(), THRESHOLD)
        .expect("el sorteo no falla");

    assert_eq!(solve.start_month, None);
    assert_eq!(solve.full_retirement, None, "sin fase no se paga una fecha");
    assert_eq!(
        solve.warnings,
        vec![StrategySolveWarning::PartialNeverStarts]
    );
    assert_eq!(
        solve.warnings.first().map(|w| w.code()),
        Some("partial_never_starts")
    );
    assert_eq!(solve.draws_search, 2, "sonda baja + sonda alta");
    assert_eq!(solve.draws_confirm, 0);
    let stats = solve.phase_success.expect("se publica por qué no arranca");
    assert_eq!(stats.by_kind[KIND_PORTFOLIO_DEPLETED], stats.paths);
}

/// **Una fase que no existe no es una fase que fracasa.** Sin `PhasePlan::partial` no hay pregunta
/// que responder: `start_month: None`, **sin aviso** y sin sorteos — pero el `McConfig` se valida
/// igual, que una configuración inválida es un error aunque no haya nada que sortear.
#[test]
fn an_absent_partial_phase_is_not_a_phase_that_fails() {
    let input = coast_household(500_000);
    let solve = earliest_partial_start(&input, &no_vol(), &search(), &confirm(), THRESHOLD)
        .expect("el sorteo no falla");

    assert_eq!(solve.start_month, None);
    assert_eq!(solve.full_retirement, None);
    assert!(solve.warnings.is_empty(), "no hay nada que avisar");
    assert_eq!((solve.draws_search, solve.draws_confirm), (0, 0));

    // Y la validación NO se salta.
    let bad = McConfig {
        seed: SEED,
        paths: 0,
        ..Default::default()
    };
    assert!(
        earliest_partial_start(&input, &no_vol(), &bad, &confirm(), THRESHOLD).is_err(),
        "un McConfig inválido es un error aunque no haya fase"
    );
}
