//! **Los reproductores de la segunda revisión adversarial (D20, 2026-09-03)**, convertidos en
//! regresiones.
//!
//! Cada test de este fichero fija UNA de las correcciones del pase, con los números medidos
//! antes y después en el propio doc. Viven juntos —y no repartidos por los módulos— porque lo que
//! los une no es el área del motor sino su procedencia: si alguien deshace una de estas
//! decisiones, aquí es donde se entera de cuál era el hallazgo y cuánto costaba.
//!
//! | test | hallazgo | qué fijaba |
//! |---|---|---|
//! | `an_exact_landing_that_covers_every_later_need_is_not_a_depletion` | #2 | `>=` sobre el aterrizaje exacto |
//! | `the_partial_phase_readings_need_a_partial_phase_that_happened` | #9 | lectura de una fase que no se vivió |
//!
//! **Faltan tres de los hallazgos #3 y #4** (`the_binding_allowance_is_a_cut_on_the_mixed_path_too`,
//! `guardrails_under_a_binding_allowance_classify_on_both_paths`,
//! `rule_is_spend_funds_the_month_surplus_first`): fijaban el techo de una regla de retirada por
//! saldo y la venta forzada de `rule_is_spend`, y se borraron con su sujeto el 2026-10-10
//! (decisión «solo SWR»: las reglas por saldo y el modo de gasto salieron del motor).
//!
//! Y falta otro: `a_bridge_discount_too_negative_is_a_typed_error_not_a_panic` (hallazgo #1, `powd`
//! sin `checked`). **E4 lo borró con su sujeto**: el objetivo puente y su descuento se retiraron
//! del motor (decisión M4 del modelo v2), y con ellos la tabla que desbordaba y el error tipado
//! `BridgeDiscountOverflow`. La disciplina que dejó —`checked_powd_fraction` en `MoneyOps`—
//! sigue en el contrato numérico.

use chrono::NaiveDate;
use futurefin_engine::{
    project_net_worth_series, AllocationKind, AllocationRule, FireNeed, FireTarget, PartialPhase,
    PensionSchedule, PhasePlan, ProjectionInput, RetirementTrigger, SimAsset, TaxBracket,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("literal decimal válido")
}

/// Escala del ahorro española 2025-26, la misma de `common/cases.rs`.
fn es() -> Vec<TaxBracket> {
    [
        (Some("6000"), "19"),
        (Some("50000"), "21"),
        (Some("200000"), "23"),
        (Some("300000"), "27"),
        (None, "30"),
    ]
    .into_iter()
    .map(|(up_to, pct)| TaxBracket {
        up_to: up_to.map(dec),
        pct: dec(pct),
    })
    .collect()
}

fn asset(i: u128, value: &str, basis: Option<&str>, liquid: bool, rate: &str) -> SimAsset {
    SimAsset {
        id: Uuid::from_u128(i),
        value: dec(value),
        purchase_price: basis.map(dec),
        is_liquid: liquid,
        expected_annual_return_percent: Some(dec(rate)),
        start_month: None,
    }
}

fn remainder_to(target_index: usize) -> AllocationRule {
    AllocationRule {
        target_index,
        kind: AllocationKind::Remainder,
        amount: None,
        cap: None,
    }
}

fn base(horizon: u32, income: &str, expense: &str, assets: Vec<SimAsset>) -> ProjectionInput {
    ProjectionInput {
        income_expense_changes: Default::default(),
        ref_date: NaiveDate::from_ymd_opt(2026, 9, 1).expect("fecha válida"),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: es(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        income_regular_monthly: dec(income),
        expense_regular_monthly: dec(expense),
        assets,
        allocation_rules: vec![remainder_to(0)],
        liabilities: Vec::new(),
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        phase_plan: PhasePlan::classic(Decimal::ZERO, dec(expense)),
        fire_target: None,
    }
}

// =================================================================================================
// Hallazgo #2 — el aterrizaje exacto no es un agotamiento
// =================================================================================================

/// **La cartera que se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto
/// posterior no se ha agotado: ha cumplido.**
///
/// El hogar: 2.000 €/mes de gasto y pensión de 2.500 € desde el índice 120, jubilado a la fuerza
/// desde el mes 1. El capital son `120 × 2.000 = 240.000 €` EXACTOS —los meses que faltan hasta
/// la pensión, al 0 %—, así que la venta del mes 120 consume el último euro y desde el 121 la
/// pensión paga sola: 500 €/mes de sobra durante 120 meses ⇒ 60.000 € al final del horizonte, y
/// **ni un euro de necesidad sin cubrir**.
///
/// (Ese capital lo dimensionaba el objetivo PUENTE cuando el caso se escribió; E4 lo retiró. No
/// cambia nada aquí: la jubilación es FORZADA en el mes 1 y ningún objetivo la dispara.)
///
/// Hasta el pase de correcciones el motor publicaba `assets_depleted_month_index = Some(120)`
/// porque el predicado era `venta_bruta >= drenable` y el aterrizaje exacto cae del lado del
/// `>=`. Peor: la misma entrada daba `None` en `Decimal` y `Some(120)` en `f64` —el tipo sobre el
/// que corre cada camino de Monte Carlo—, así que el mismo plan salía «arruinado» en el fan chart
/// y «perfecto» en la línea.
///
/// Un euro menos de capital SÍ es un agotamiento, y el test lo comprueba: el discriminante es la
/// necesidad sin cubrir, no el filo de la comparación.
#[test]
fn an_exact_landing_that_covers_every_later_need_is_not_a_depletion() {
    let build = |capital: &str| {
        let mut input = base(
            240,
            "0",
            "2000",
            vec![asset(1, capital, Some("0"), true, "0")],
        );
        input.taxes_enabled = false;
        input.tax_brackets = Vec::new();
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.pension = Some(PensionSchedule {
            start_index: 120,
            monthly_today: dec("2500"),
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
        });
        input.fire_target = Some(FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: dec("2000"),
                pension_monthly: Decimal::ZERO,
            },
            swr_pct: dec("4"),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::ZERO,
            debt_payments_remaining: Vec::new(),
        });
        project_net_worth_series(&input).expect("simula")
    };

    // (a) El aterrizaje EXACTO: cartera a cero en el mes 120 y nada sin cubrir después.
    let exact = build("240000");
    assert_eq!(
        exact.assets_depleted_month_index, None,
        "vaciar la cartera el mes en que la pensión toma el relevo no es agotarla"
    );
    assert_eq!(exact.uncovered_deficit_total, Decimal::ZERO);
    assert_eq!(exact.liquid_worth[120], Decimal::ZERO, "sí se vacía");
    assert_eq!(
        exact.liquid_worth[240],
        dec("60000"),
        "y desde el 121 la pensión deja 500 €/mes"
    );

    // (a bis) E1: el mismo discriminante gobierna el VEREDICTO del camino. El aterrizaje exacto
    //         no falla —ni un euro de necesidad sin fundar en 240 meses—, así que
    //         `failure_month_index` es `None` igual que el agotamiento.
    assert_eq!(exact.failure_month_index, None);
    assert_eq!(exact.failure_kind, None);

    // (b) Un euro menos SÍ deja necesidad sin cubrir, y entonces el mes se publica.
    let short = build("239999");
    assert_eq!(short.assets_depleted_month_index, Some(120));
    assert_eq!(short.uncovered_deficit_total, Decimal::ONE);
    // Y el fallo del camino cae en el MISMO mes: aquí el mes que vacía la cartera es también el
    // primero cuya venta no se pudo fundar. No siempre coinciden —cuando el vaciado es exacto y
    // el descubierto llega después, el agotamiento va por delante (P23 de la batería: agota en el
    // 16 y falla en el 17)—, y por eso son dos campos y no uno.
    assert_eq!(short.failure_month_index, Some(120));
    assert_eq!(
        short.failure_kind,
        Some(futurefin_engine::PathFailure::PortfolioDepleted)
    );

    // (c) Y un céntimo más ni se vacía ni se agota.
    let spare = build("240000.01");
    assert_eq!(spare.assets_depleted_month_index, None);
    assert_eq!(spare.uncovered_deficit_total, Decimal::ZERO);
}

// =================================================================================================
// Hallazgo #9 — las lecturas de la media jornada necesitan una fase parcial que ocurriera
// =================================================================================================

/// **Las lecturas de la media jornada solo existen si la media jornada se llegó a vivir.**
///
/// El hogar declara media jornada a partir del mes 60, pero con 598.000 € y 6.000 €/mes de
/// superávit cruza su número FIRE (600.000 €) en el mes 2 y se jubila del todo **58 meses antes**.
/// La fase parcial no ocurre —`partial_retirement_month_index` es `None` y `phase_transitions` no
/// la menciona— y sin embargo el objetivo del hueco se publicaba: 270.000 €, calculados de la
/// fase DECLARADA.
///
/// **E4 retiró ese `partial_gap_target`** junto con el resto del objetivo como criterio, así que
/// lo que queda vigilado aquí es su gemelo `partial_phase_capital_growing`, que se gateaba con el
/// mismo criterio y por el que se descubrió la divergencia.
#[test]
fn the_partial_phase_readings_need_a_partial_phase_that_happened() {
    let mut input = base(
        120,
        "8000",
        "2000",
        vec![asset(1, "598000", Some("598000"), true, "0")],
    );
    input.taxes_enabled = false;
    input.tax_brackets = Vec::new();
    input.phase_plan = PhasePlan::classic(Decimal::ZERO, dec("2000"));
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 60,
        income_monthly: dec("1100"),
        expense_basis: futurefin_engine::ExpenseBasis::Retirement,
    });
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: dec("2000"),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: dec("4"),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    let out = project_net_worth_series(&input).expect("simula");

    assert_eq!(out.retirement_month_index, Some(2), "cruza en el mes 2");
    assert_eq!(
        out.partial_retirement_month_index, None,
        "la fase no ocurre"
    );
    assert!(
        !out.partial_phase_capital_growing,
        "y sus lecturas tampoco: nada crece en una fase que nadie vivió"
    );

    // Con la fase parcial de verdad (sin cruce que la adelante), la lectura vuelve a existir.
    let mut lived = input.clone();
    lived.assets = vec![asset(1, "1000", Some("1000"), true, "0")];
    lived.income_regular_monthly = dec("2500");
    let out = project_net_worth_series(&lived).expect("simula");
    assert_eq!(out.partial_retirement_month_index, Some(60));
    assert!(
        out.warnings
            .contains(&futurefin_engine::EngineWarning::PartialPhaseCapitalShrinking),
        "vivida la fase, la lectura vuelve a describir algo: aquí la media jornada come capital \
         (1.100 de ingreso contra 2.000 de gasto) y el aviso lo dice"
    );
    assert!(
        !out.partial_phase_capital_growing,
        "y su complementario también"
    );
}
