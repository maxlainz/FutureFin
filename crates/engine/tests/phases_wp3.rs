//! **Fases, pensión con fecha y solves** (WP3 de 5.0.0, §B.1/§B.3/§B.7 del plan de la issue #207).
//!
//! Todo lo que se afirma aquí está **predicho a mano en el comentario que lo precede**, con la
//! aritmética a la vista. Es la disciplina de `futurefin-research-methodology`: un test que
//! compara el motor consigo mismo solo pinea lo que el motor hace hoy; un test con el número
//! escrito antes de ejecutarlo comprueba que hace lo que se pidió.
//!
//! Por eso casi todos los casos van con rentabilidad 0 %, inflación 0 % y sin impuestos: no
//! porque sea realista, sino porque así **cada euro de la serie es una suma que cabe en una línea**
//! y una discrepancia señala el mes exacto. Los caminos con fiscalidad, inflación y `powd` los
//! cubren los pines dorados (`golden_pins.rs`), que son otra herramienta para otro trabajo.

#[path = "common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, projection_cases_all, rule_remainder};
use futurefin_engine::{
    coast_fire_month_index, max_extra_monthly_expense_keeping_date, project_net_worth_series,
    required_contribution_monthly, retirement_delay_months, EngineWarning, ExpenseBasis, FireNeed,
    FireTarget, IncomePause, PartialPhase, PensionSchedule, Phase, ProjectionInput,
    RetirementTrigger, TargetBasis,
};
use rust_decimal::Decimal;

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

/// Un objetivo plano: sin impuestos, sin inflación y sin deuda, `annual_net_today/SWR` y punto.
fn flat_target(annual_net_today: i64, swr_pct: i64) -> FireTarget {
    FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: d(annual_net_today),
        },
        swr_pct: d(swr_pct),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    }
}

/// Un hogar de laboratorio: un único activo líquido al 0 %, una regla `remainder` y nada más.
fn lab(horizon: u32, income: i64, expense: i64, asset_value: i64) -> ProjectionInput {
    base_input(
        horizon,
        d(income),
        d(expense),
        vec![mk_asset(1, d(asset_value), true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    )
}

// =============================================================================================
// A · Pensión con fecha como INGRESO
// =============================================================================================

/// **La pensión entra en caja en un mes ACUMULANDO**, no solo jubilado (§B.1 paso 3).
///
/// Predicho: horizonte 3, ingreso 1.000 = gasto 1.000 (caja recurrente 0), pensión PLANA de 500
/// desde el índice 1. El mes `k` mira el índice `k−1`, así que:
///
/// | mes | índice | pensión | caja | activo al cierre |
/// |---|---|---|---|---|
/// | 1 | 0 | 0 | 0 | 0 |
/// | 2 | 1 | 500 | +500 | **500** |
/// | 3 | 2 | 500 | +500 | **1.000** |
///
/// Y `pension_start_month_index = 2`: el mes del BUCLE, 1-based, `start_index + 1`.
#[test]
fn a_dated_pension_is_income_while_still_accumulating() {
    let mut input = lab(3, 1_000, 1_000, 0);
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 1,
        monthly_today: d(500),
        indexed: false,
        fraction_while_partial: Decimal::ZERO,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(out.liquid_worth, vec![d(0), d(0), d(500), d(1_000)]);
    assert_eq!(out.pension_start_month_index, Some(2));
    assert_eq!(out.retirement_month_index, None, "sin objetivo no hay cruce");
    assert_eq!(out.phase_transitions, vec![(Phase::Accumulating, 0)]);
}

/// Una pensión INDEXADA se infla con el MISMO factor que el gasto del bucle (`f(k−1)`), y una
/// pensión cuyo `start_index` cae fuera del horizonte no tiene mes: `None`, no un mes inventado.
#[test]
fn an_indexed_pension_uses_the_loops_inflation_factor() {
    // Inflación 100 % anual ⇒ `f(12) = 2` exacto (`powd` enruta el año entero por `checked_powu`).
    let mut input = lab(13, 0, 0, 0);
    input.annual_inflation_percent = d(100);
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 12,
        monthly_today: d(1_000),
        indexed: true,
        fraction_while_partial: Decimal::ZERO,
    });
    let out = project_net_worth_series(&input).unwrap();
    // Solo el mes 13 (índice 12) cobra, y cobra 1.000·f(12) = 2.000.
    assert_eq!(out.liquid_worth[12], Decimal::ZERO);
    assert_eq!(out.liquid_worth[13], d(2_000));
    assert_eq!(out.pension_start_month_index, Some(13));

    let mut short = input.clone();
    short.horizon_months = 6;
    short.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 6];
    assert_eq!(
        project_net_worth_series(&short).unwrap().pension_start_month_index,
        None,
        "la pensión existe en el plan pero esta simulación no llega a verla"
    );
}

// =============================================================================================
// B · Objetivo consciente del plan dentro del BUCLE
// =============================================================================================

/// **El puente cambia el mes de jubilación**, y cambia porque el objetivo es más pequeño.
///
/// Predicho: gasto de jubilación 2.000 €/mes, SWR 4 %, sin impuestos ni inflación, pensión plana
/// de 1.200 desde el índice 24.
/// - Perpetuidad: `T(i) = 600.000` mientras `i < 24`.
/// - Puente sin descuento: `T(0) = 24·2.000 + 240.000 = 288.000`.
///
/// Con 300.000 € líquidos de partida, el puente cruza en el mes 1 (300.000 ≥ 288.000) y la
/// perpetuidad no cruza nunca dentro de un horizonte de 12 meses sin ahorro.
#[test]
fn the_bridge_basis_moves_the_crossing_because_the_target_is_smaller() {
    let mut input = lab(12, 2_000, 2_000, 300_000);
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: d(2_000),
            pension_monthly: Decimal::ZERO,
        },
        ..flat_target(0, 4)
    });
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 24,
        monthly_today: d(1_200),
        indexed: false,
        fraction_while_partial: Decimal::ZERO,
    });

    let perpetuity = project_net_worth_series(&input).unwrap();
    assert_eq!(
        perpetuity.retirement_month_index, None,
        "600.000 € de objetivo con 300.000 € de cartera: no se cruza"
    );

    input.phase_plan.target_basis = TargetBasis::BridgeToPension;
    let bridge = project_net_worth_series(&input).unwrap();
    assert_eq!(
        bridge.retirement_month_index,
        Some(1),
        "el puente son 288.000 €, y hay 300.000"
    );
    assert_eq!(bridge.liquid_crossing_month_index, Some(1));
    // 12·need_full_m(0)/L(0) = 24.000/300.000 = 8 % anual. Muy por encima del SWR — y legítimo:
    // el puente dura dos años, no para siempre (D7).
    assert_eq!(bridge.bridge_effective_withdrawal_pct, Some(d(8)));
    assert_eq!(
        bridge.pension_coverage_ratio,
        Some(Decimal::new(6, 1)),
        "1.200/2.000 del gasto"
    );
}

/// Sin pensión con fecha ni base puente, las dos lecturas nuevas son `None` — **jamás un 0**.
#[test]
fn without_a_pension_the_bridge_readings_are_absent_not_zero() {
    let mut input = lab(12, 3_000, 2_000, 1_000_000);
    input.fire_target = Some(flat_target(24_000, 4));
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.bridge_effective_withdrawal_pct, None);
    assert_eq!(out.pension_coverage_ratio, None);
    assert_eq!(out.partial_gap_target, None);
    assert!(!out.partial_phase_capital_growing);
}

// =============================================================================================
// C · Fase parcial
// =============================================================================================

/// **El mes exacto en que la media jornada conmuta ingreso Y gasto** (§B.1, D10).
///
/// Predicho: horizonte 6, ingreso regular 3.000, gasto regular 2.000, activo 0, sin objetivo.
/// Media jornada desde el mes 4 con ingreso 1.100 y `expense_basis = Retirement`, con el gasto de
/// jubilación en 1.000.
///
/// | mes | fase | ingreso | gasto | caja | activo |
/// |---|---|---|---|---|---|
/// | 1-3 | acumula | 3.000 | 2.000 | +1.000 | 1.000 / 2.000 / **3.000** |
/// | 4-6 | parcial | 1.100 | 1.000 | +100 | 3.100 / 3.200 / **3.300** |
///
/// El capital CRECE en la fase, así que `partial_phase_capital_growing` es `true` y no hay aviso.
#[test]
fn the_partial_phase_switches_income_and_expense_on_its_month() {
    let mut input = lab(6, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(1_000);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 4,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(1_000), d(2_000), d(3_000), d(3_100), d(3_200), d(3_300)]
    );
    assert_eq!(out.partial_retirement_month_index, Some(4));
    assert_eq!(
        out.phase_transitions,
        vec![(Phase::Accumulating, 0), (Phase::Partial, 4)]
    );
    assert!(out.partial_phase_capital_growing);
    assert!(out.warnings.is_empty());
    // Ningún mes vendió nada: la fase parcial va en superávit.
    assert!(out.withdrawal.iter().all(|w| w.is_zero()));
}

/// La misma fase con `expense_basis = Regular` se queda con el gasto de siempre y **come capital**:
/// 1.100 − 2.000 = −900 €/mes vendidos de la cartera, sin techo (la regla de retirada gobierna la
/// jubilación, no la media jornada).
///
/// Predicho: activo 3.000 al entrar en el mes 4 ⇒ 2.100 / 1.200 / **300**. Aviso
/// `PartialPhaseCapitalShrinking` y `partial_phase_capital_growing = false`.
#[test]
fn a_partial_phase_that_eats_capital_warns_and_sells_without_a_ceiling() {
    let mut input = lab(6, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(1_000);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 4,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Regular,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(1_000), d(2_000), d(3_000), d(2_100), d(1_200), d(300)]
    );
    assert_eq!(out.withdrawal[4], d(900));
    assert!(!out.partial_phase_capital_growing);
    assert_eq!(
        out.warnings,
        vec![EngineWarning::PartialPhaseCapitalShrinking]
    );
    assert!(
        out.withdrawal_shortfall.iter().all(|s| s.is_zero()),
        "sin techo no hay recorte: el hogar gasta lo declarado"
    );
}

/// La media jornada cobra la FRACCIÓN declarada de la pensión (D8), y el hueco que queda es el
/// `partial_gap_target`.
///
/// Predicho: gasto parcial 2.000, ingreso 1.100, pensión 1.200 al 50 % ⇒ 600.
/// `gap_m = 2.000 − 1.100 − 600 = 300` ⇒ `300·12/0,04` = **90.000 €**.
#[test]
fn the_partial_phase_collects_its_share_of_the_pension() {
    let mut input = lab(4, 3_000, 2_000, 0);
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: d(2_000),
            pension_monthly: Decimal::ZERO,
        },
        ..flat_target(0, 4)
    });
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 3,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 0,
        monthly_today: d(1_200),
        indexed: false,
        fraction_while_partial: Decimal::new(5, 1),
    });
    let out = project_net_worth_series(&input).unwrap();

    // Meses 1-2 (acumulando): 3.000 + 1.200 − 2.000 = +2.200. Meses 3-4 (parcial):
    // 1.100 + 600 − 2.000 = −300, vendidos de la cartera.
    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(2_200), d(4_400), d(4_100), d(3_800)]
    );
    assert_eq!(out.partial_gap_target, Some(d(90_000)));
}

// =============================================================================================
// D · `crossing_is_reading_only` (D17)
// =============================================================================================

/// **El cruce deja de jubilar y se queda en lectura.**
///
/// Predicho: 1.000.000 € líquidos, objetivo 600.000 (24.000/0,04) ⇒ se cruza en el mes 1.
/// - Con la bandera: NO se jubila; ingreso 3.000 − gasto 2.000 = +1.000 €/mes ⇒
///   `liquid(12) = 1.012.000`, `retirement_month_index = None`, cruce anotado en el mes 1.
/// - Sin la bandera: jubilado en el mes 1, ingreso de jubilación 0 − gasto 2.000 ⇒ 12 ventas de
///   2.000 ⇒ `liquid(12) = 976.000`.
#[test]
fn a_reading_only_crossing_does_not_retire_anyone() {
    let mut input = lab(12, 3_000, 2_000, 1_000_000);
    input.fire_target = Some(flat_target(24_000, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(120);
    input.phase_plan.crossing_is_reading_only = true;

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, None);
    assert_eq!(
        out.liquid_crossing_month_index,
        Some(1),
        "el cruce SÍ se anota: es la lectura que el chart necesita"
    );
    assert_eq!(out.liquid_worth[12], d(1_012_000));
    assert_eq!(out.phase_transitions, vec![(Phase::Accumulating, 0)]);

    let mut retiring = input.clone();
    retiring.phase_plan.crossing_is_reading_only = false;
    let out2 = project_net_worth_series(&retiring).unwrap();
    assert_eq!(out2.retirement_month_index, Some(1));
    assert_eq!(out2.liquid_worth[12], d(976_000));
}

/// Con la bandera puesta y una edad alcanzable, quien jubila es la EDAD — y si el capital no
/// llega, se emite el aviso rojo de D17.
#[test]
fn retiring_by_age_below_the_target_warns() {
    let mut input = lab(12, 3_000, 2_000, 100_000);
    input.fire_target = Some(flat_target(24_000, 4)); // objetivo 600.000
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(6);
    input.phase_plan.crossing_is_reading_only = true;

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, Some(6));
    assert_eq!(out.liquid_crossing_month_index, None, "nunca se cruza");
    assert_eq!(out.warnings, vec![EngineWarning::RetireAtAgeUnderfunded]);
}

// =============================================================================================
// E · Techo de aportación y caja disponible
// =============================================================================================

/// **Identidad contable del mes con techo**: `sobrante = invertido + disponible`, y el disponible
/// NO es patrimonio.
///
/// Predicho: ingreso 5.000, gasto 3.000 ⇒ sobrante 2.000; techo 1.200 ⇒ 1.200 invertidos y 800
/// disponibles cada uno de los 10 meses. `liquid(10) = 12.000`, `disposable_cash_total = 8.000`,
/// y el patrimonio NO incluye esos 8.000.
#[test]
fn a_contribution_cap_splits_the_surplus_and_the_rest_leaves_the_balance() {
    let mut input = lab(10, 5_000, 3_000, 0);
    input.phase_plan.contribution_cap_monthly = Some(d(1_200));
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(out.liquid_worth[10], d(12_000));
    assert_eq!(out.net_worth[10], d(12_000), "el disponible no es patrimonio");
    assert_eq!(out.disposable_cash_total, d(8_000));
    for k in 1..=10usize {
        assert_eq!(out.disposable_cash[k], d(800), "mes {k}");
        let invested = out.liquid_worth[k] - out.liquid_worth[k - 1];
        assert_eq!(
            invested + out.disposable_cash[k],
            d(2_000),
            "identidad del mes {k}: sobrante = invertido + disponible"
        );
    }
    assert_eq!(out.disposable_cash[0], Decimal::ZERO);

    // Sin techo, la serie es cero mes a mes y el total también.
    let plain = project_net_worth_series(&lab(10, 5_000, 3_000, 0)).unwrap();
    assert!(plain.disposable_cash.iter().all(|v| v.is_zero()));
    assert_eq!(plain.disposable_cash_total, Decimal::ZERO);
    assert_eq!(plain.liquid_worth[10], d(20_000));
}

/// El corte de coast es un techo de 0 desde su mes, y manda sobre el techo constante.
#[test]
fn stopping_contributions_beats_the_constant_cap() {
    let mut input = lab(10, 5_000, 3_000, 0);
    input.phase_plan.contribution_cap_monthly = Some(d(1_200));
    input.phase_plan.contributions_stop_month = Some(6);
    let out = project_net_worth_series(&input).unwrap();
    // Meses 1-5 aportan 1.200; del 6 en adelante, nada.
    assert_eq!(out.liquid_worth[5], d(6_000));
    assert_eq!(out.liquid_worth[10], d(6_000));
    assert_eq!(out.disposable_cash[6], d(2_000));
}

// =============================================================================================
// F · Solves (§B.7)
// =============================================================================================

/// El caso de laboratorio de los dos primeros solves: cartera vacía al 0 %, sobrante 2.000 €/mes,
/// objetivo 100.000 € plano, jubilación por EDAD en el mes 101 (el cruce es lectura).
fn solve_lab() -> ProjectionInput {
    let mut input = lab(120, 5_000, 3_000, 0);
    input.fire_target = Some(flat_target(4_000, 4)); // 4.000/0,04 = 100.000
    input.phase_plan.expense_retirement_monthly = d(3_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(101);
    input.phase_plan.crossing_is_reading_only = true;
    input
}

/// **Aportación necesaria, exacta**: con 0 % de rentabilidad, el criterio en el índice 100 son
/// 100 aportaciones. `100·c ≥ 100.000 ⇒ c = 1.000 €/mes`, y la bisección aterriza en el valor
/// EXACTO porque 1.000 es el punto medio de `[0, 2.000]`.
///
/// La serie devuelta es la de esa ejecución: `required_capital_path[100] = 100.000` clavado.
#[test]
fn required_contribution_is_exactly_a_thousand() {
    let input = solve_lab();
    let solved = required_contribution_monthly(&input, 101).unwrap().unwrap();

    assert_eq!(solved.contribution, d(1_000));
    assert!(!solved.underfunded);
    assert!(solved.warnings.is_empty());
    assert_eq!(solved.required_capital_path[100], d(100_000));
    assert_eq!(solved.required_capital_path[0], Decimal::ZERO);
    assert!(
        solved.iterations <= 24,
        "presupuesto de bisección: {} iteraciones",
        solved.iterations
    );
}

/// **Infra-financiado**: con el objetivo diez veces mayor, ni aportando los 2.000 € enteros se
/// llega (100·2.000 = 200.000 < 1.000.000). Se devuelve el sobrante entero, la bandera roja y el
/// aviso de D17.
#[test]
fn an_unreachable_target_reports_the_whole_headroom_as_underfunded() {
    let mut input = solve_lab();
    input.fire_target = Some(flat_target(40_000, 4)); // 1.000.000
    let solved = required_contribution_monthly(&input, 101).unwrap().unwrap();

    assert_eq!(solved.contribution, d(2_000), "todo el sobrante del mes 1");
    assert!(solved.underfunded);
    assert_eq!(solved.warnings, vec![EngineWarning::RetireAtAgeUnderfunded]);
    assert_eq!(solved.required_capital_path[100], d(200_000));
}

/// Un objetivo que ya está cubierto sin aportar nada devuelve **0 aportaciones y 0 iteraciones**.
#[test]
fn an_already_funded_plan_needs_no_contribution() {
    let mut input = solve_lab();
    input.assets[0].value = d(500_000);
    let solved = required_contribution_monthly(&input, 101).unwrap().unwrap();
    assert_eq!(solved.contribution, Decimal::ZERO);
    assert_eq!(solved.iterations, 0);
    assert!(!solved.underfunded);
}

/// Sin objetivo evaluable no hay pregunta: `Ok(None)`, que **no es «no necesitas aportar»**.
#[test]
fn no_target_means_no_solve_at_all() {
    let mut input = solve_lab();
    input.fire_target = None;
    assert!(required_contribution_monthly(&input, 101).unwrap().is_none());
    assert!(coast_fire_month_index(&input, 101).unwrap().is_none());
    // Y un mes fuera de la serie tampoco tiene respuesta.
    let full = solve_lab();
    assert!(required_contribution_monthly(&full, 0).unwrap().is_none());
    assert!(required_contribution_monthly(&full, 200).unwrap().is_none());
}

/// **Mes de coast, exacto**: con 0 % de rentabilidad nada crece, así que parar en el mes `k` deja
/// `(k−1)·2.000` en la cartera. `(k−1)·2.000 ≥ 100.000 ⇒ k ≥ 51`.
///
/// El número coast es el líquido con el que se ENTRA en ese mes: `coast_path[50] = 100.000 €`.
#[test]
fn the_coast_month_is_the_fifty_first() {
    let input = solve_lab();
    let coast = coast_fire_month_index(&input, 101).unwrap().unwrap();

    assert_eq!(coast.coast_month_index, Some(51));
    assert_eq!(coast.coast_number, Some(d(100_000)));
    assert_eq!(coast.coast_path[50], d(100_000));
    assert_eq!(
        coast.coast_path[100],
        d(100_000),
        "desde el mes 51 no se aporta y nada crece: la serie se queda plana"
    );
    assert!(coast.warnings.is_empty());
    assert!(coast.iterations <= 24);
}

/// Coast inalcanzable: aviso propio y la MEJOR serie que el plan da (la de aportar siempre).
#[test]
fn an_unreachable_coast_says_so() {
    let mut input = solve_lab();
    input.fire_target = Some(flat_target(40_000, 4)); // 1.000.000
    let coast = coast_fire_month_index(&input, 101).unwrap().unwrap();
    assert_eq!(coast.coast_month_index, None);
    assert_eq!(coast.coast_number, None);
    assert_eq!(coast.warnings, vec![EngineWarning::CoastNotReachable]);
    assert_eq!(coast.coast_path[100], d(200_000));
}

/// Un hogar que ya puede dejar de aportar HOY: el coast es el mes 1 y el número coast es el
/// patrimonio de partida.
#[test]
fn a_household_that_can_coast_today_coasts_from_month_one() {
    let mut input = solve_lab();
    input.assets[0].value = d(150_000);
    let coast = coast_fire_month_index(&input, 101).unwrap().unwrap();
    assert_eq!(coast.coast_month_index, Some(1));
    assert_eq!(coast.coast_number, Some(d(150_000)));
    assert_eq!(coast.iterations, 0);
}

/// **Cuánto más puedo gastar sin mover la fecha** (P8.b).
///
/// Predicho: cartera 5.000, sobrante 1.000 €/mes, objetivo 10.000 (400/0,04). Base: `liquid(k) =
/// 5.000 + 1.000k`, cruza 10.000 en `k = 5`, así que se jubila en el **mes 6**; el techo de
/// tolerancia es el mes 7, o sea `liquid(6) ≥ 10.000`:
///
/// ```text
/// 5.000 + 6·(1.000 − e) ≥ 10.000  ⇔  e ≤ 1.000/6 = 166,666… €/mes
/// ```
///
/// La bisección sobre `[0, 1.000]` con 24 halvings resuelve a menos de `1.000/2²⁴ ≈ 6·10⁻⁵`.
#[test]
fn the_extra_expense_that_keeps_the_date_is_one_sixth_of_the_headroom() {
    let mut input = lab(24, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let baseline = project_net_worth_series(&input).unwrap();
    assert_eq!(baseline.retirement_month_index, Some(6), "la fecha base");

    let extra = max_extra_monthly_expense_keeping_date(&input)
        .unwrap()
        .unwrap();
    let expected = d(1_000) / d(6);
    assert!(
        (extra - expected).abs() < Decimal::new(1, 3),
        "166,666… €/mes predicho, obtenido {extra}"
    );

    // Sin fecha base no hay fecha que conservar: `None`, no un 0.
    let mut no_date = input.clone();
    no_date.fire_target = None;
    assert_eq!(max_extra_monthly_expense_keeping_date(&no_date).unwrap(), None);
}

/// **Cuánto retrasa una pausa de ingresos** (P8.c).
///
/// Predicho sobre el mismo hogar (cartera 5.000, +1.000 €/mes, objetivo 10.000, base = mes 6):
/// una pausa de 2 meses a fracción 0 desde el mes 2 convierte dos `+1.000` en dos `−2.000`, un
/// vuelco de 6.000 € = **6 meses** a 1.000 €/mes.
///
/// | mes | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
/// |---|---|---|---|---|---|---|---|---|---|---|---|
/// | líquido | 6.000 | 4.000 | 2.000 | 3.000 | 4.000 | 5.000 | 6.000 | 7.000 | 8.000 | 9.000 | 10.000 |
///
/// El cruce se decide contra el cierre anterior, así que se jubila en el **mes 12**: 12 − 6 = 6.
#[test]
fn an_income_pause_delays_retirement_by_six_months() {
    let mut input = lab(24, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let delay = retirement_delay_months(
        &input,
        IncomePause {
            from_month: 2,
            months: 2,
            income_fraction: Decimal::ZERO,
        },
    )
    .unwrap();

    assert_eq!(delay.baseline_month_index, Some(6));
    assert_eq!(delay.paused_month_index, Some(12));
    assert_eq!(delay.delay_months, Some(6));
}

/// Una pausa que empuja la jubilación FUERA del horizonte no devuelve un retraso enorme: devuelve
/// `None`, porque «no se jubila» no es un número de meses.
#[test]
fn a_pause_that_pushes_retirement_past_the_horizon_has_no_delay_number() {
    let mut input = lab(8, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let delay = retirement_delay_months(
        &input,
        IncomePause {
            from_month: 2,
            months: 3,
            income_fraction: Decimal::ZERO,
        },
    )
    .unwrap();
    assert_eq!(delay.baseline_month_index, Some(6));
    assert_eq!(delay.paused_month_index, None);
    assert_eq!(delay.delay_months, None);
}

// =============================================================================================
// G · Invariante de §C — el mes del ingreso ES el mes de la jubilación
// =============================================================================================

/// **Invariante de comportamiento (§C, hallazgo B4)**: el mes en que el ingreso conmuta al de
/// jubilación es exactamente `retirement_month_index`, y el mes en que conmuta al de media
/// jornada es exactamente `partial_retirement_month_index`. Se comprueba sobre la SERIE, no sobre
/// el enum: si algún día las fases y los importes se separan, esto lo caza.
#[test]
fn the_phase_readings_agree_with_the_cash_flow_they_describe() {
    let mut input = lab(24, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(2_500);
    input.phase_plan.income_retirement_monthly = d(500);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 5,
        income_monthly: d(2_400),
        expense_basis: ExpenseBasis::Regular,
    });
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(10);
    let out = project_net_worth_series(&input).unwrap();

    // Δ líquido del mes k (sin rentabilidad ni ventas parciales) = caja del mes.
    let delta = |k: usize| out.liquid_worth[k] - out.liquid_worth[k - 1];
    // Acumulando: 3.000 − 2.000 = +1.000.
    assert_eq!(delta(4), d(1_000));
    // Parcial (gasto REGULAR): 2.400 − 2.000 = +400, desde el mes 5.
    assert_eq!(delta(5), d(400));
    assert_eq!(delta(9), d(400));
    // Jubilado: 500 − 2.500 = −2.000, desde el mes 10.
    assert_eq!(delta(10), d(-2_000));

    assert_eq!(out.partial_retirement_month_index, Some(5));
    assert_eq!(out.retirement_month_index, Some(10));
    assert_eq!(
        out.phase_transitions,
        vec![
            (Phase::Accumulating, 0),
            (Phase::Partial, 5),
            (Phase::Retired, 10)
        ]
    );
}

/// Una media jornada declarada DESPUÉS de la jubilación **no ocurre** (las fases son monótonas), y
/// entonces no se publica su mes: el chart no puede pintar una fase que la simulación no vivió.
#[test]
fn a_partial_phase_after_retirement_never_happens() {
    let mut input = lab(12, 3_000, 2_000, 0);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(3);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 8,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.partial_retirement_month_index, None);
    assert!(!out.partial_phase_capital_growing);
    assert_eq!(
        out.phase_transitions,
        vec![(Phase::Accumulating, 0), (Phase::Retired, 3)]
    );
}

/// **La cota de búsqueda del solve NO es el sobrante del mes 1**, y este test es la regresión de
/// esa decisión (tomada en WP3 con la medición de P9 delante, ver `search_ceiling` en
/// `crates/engine/src/solve.rs`).
///
/// P9 es el hogar realista de la batería: su neto recurrente del mes 1 son **500 €/mes**, pero su
/// caja mensual crece muy por encima cuando los pasivos se extinguen y los «Próximos» entran. Con
/// 500 € como cota, la aportación máxima explorable dejaría `líquido(599)` en 91.444 € frente a
/// los 725.197 € de la cascada real: cualquier objetivo entre esas dos cifras se declararía
/// **infra-financiado siendo alcanzable** — un rojo falso de D17.
///
/// Aquí se fija un objetivo dentro de esa horquilla (SWR forzado al 40 % ⇒ `T(599) ≈ 268.666 €`)
/// y se exige que el solve encuentre una `c` de verdad, no que se rinda.
#[test]
fn the_solve_ceiling_is_the_max_monthly_surplus_not_the_first_months_headroom() {
    let mut input = projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 en la batería")
        .input;
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(600);
    input.fire_target.as_mut().unwrap().swr_pct = d(40);

    let solved = required_contribution_monthly(&input, 600).unwrap().unwrap();

    assert!(
        solved.search_ceiling > d(500),
        "la cota tiene que superar el neto recurrente del mes 1 (500 €): {}",
        solved.search_ceiling
    );
    assert!(
        !solved.underfunded,
        "P9 SÍ alcanza este objetivo; declararlo infra-financiado sería el rojo falso de D17"
    );
    assert!(
        solved.contribution > d(500),
        "y la aportación necesaria está por encima de los 500 €/mes del mes 1: {}",
        solved.contribution
    );
    assert!(solved.iterations > 0, "esta vez la bisección trabaja de verdad");
    assert!(solved.iterations <= 24);
}
