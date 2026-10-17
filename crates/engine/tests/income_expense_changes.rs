//! **Cambios de ingreso y gasto con fecha** (6.0.0 F1b, RFC Planes D-4/D-5/D-6/D-8, §4).
//!
//! Fórmula del mes `k` (rejilla 1-based del bucle, la de `start_month` de F1a):
//!
//! ```text
//! income_k  = base_income  · f_inc(k) · income_scale[k−1]  + income_delta[k−1]
//! expense_k = base_expense · f_exp(k) · expense_scale[k−1] + expense_delta[k−1]
//! f(k) = inflation_factor_at_month_index(inflación, k−1) = (1 + i/100)^((k−1)/12)
//! ```
//!
//! con suelo en 0 y aviso `CashFlowClampedAtZero`. Observable: en un hogar con una cuenta
//! líquida al 0 % y todo el sobrante a ella, sin impuestos, `NW[k] − NW[k−1] = income_k −
//! expense_k` EXACTO (índice `i` = cierre del mes `i`). Cada test declara su predicción ANTES de
//! los asserts (futurefin-research-methodology).

#[path = "common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, projection_cases_5_0, projection_cases_all, rule_remainder};
use futurefin_engine::{
    build_change_vectors, first_month_allocation, inflation_factor_at_month_index,
    project_net_worth_series, ChangeKind, ChangeTarget, ChangeVectors, EngineError, EngineWarning,
    IncomeExpenseChange, IncomeExpenseChanges, ProjectionInput,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn eur(n: i64) -> Decimal {
    Decimal::from(n)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// 100.000 € líquidos al 0 %, todo el sobrante a la cuenta, sin impuestos.
fn household(horizon: u32, income: i64, expense: i64, inflation: Decimal) -> ProjectionInput {
    let mut input = base_input(
        horizon,
        eur(income),
        eur(expense),
        vec![mk_asset(1, eur(100_000), true, None)],
        vec![rule_remainder(0)],
    );
    input.annual_inflation_percent = inflation;
    input
}

/// `NW[k] − NW[k−1]` para `k = 1..=horizonte`; el índice 0 del vector devuelto es el mes 1.
fn monthly_net(input: &ProjectionInput) -> Vec<Decimal> {
    let out = project_net_worth_series(input).unwrap();
    out.net_worth.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Igualdad para cifras con factor NO terminante (`1,025^(6/12)`…): `NW` lleva 100.000 € de
/// saldo y `Decimal` tiene 28 dígitos de mantisa, así que `NW[k] − NW[k−1]` pierde los últimos
/// 1–2 decimales del flujo del mes. La cota (1e−18 €) solo absorbe ese redondeo de la
/// observación; las cifras terminantes (2.050, 1.833,01, 0) se comparan EXACTAS.
fn assert_close(got: Decimal, want: Decimal, what: &str) {
    let tol = Decimal::new(1, 18);
    assert!((got - want).abs() < tol, "{what}: {got} vs {want}");
}

fn change(
    target: ChangeTarget,
    start_month: u32,
    end_month: Option<u32>,
    kind: ChangeKind<Decimal>,
    keeps_purchasing_power: bool,
) -> IncomeExpenseChange<Decimal> {
    IncomeExpenseChange {
        target,
        start_month,
        end_month,
        kind,
        keeps_purchasing_power,
    }
}

fn with_vectors(mut input: ProjectionInput, vectors: ChangeVectors) -> ProjectionInput {
    input.income_expense_changes.vectors = vectors;
    input
}

// ---------------------------------------------------------------------------------------------
// Legacy intacto
// ---------------------------------------------------------------------------------------------

/// Vectores NO vacíos pero neutros (escala 1, delta 0) en toda la batería: la salida es igual en
/// valor a la legacy en cada mes. Predicción: igual en todos los casos de 4.15 y 5.0 (ningún caso
/// tiene ingreso ni gasto base negativos, así que el suelo no actúa).
#[test]
fn neutral_vectors_reproduce_every_battery_case() {
    let mut cases = projection_cases_all();
    cases.extend(projection_cases_5_0());
    for case in cases {
        let legacy = project_net_worth_series(&case.input).unwrap();
        let h = case.input.horizon_months as usize;
        let mut input = case.input.clone();
        input.income_expense_changes.vectors = ChangeVectors {
            income_scale_monthly: vec![Decimal::ONE; h],
            expense_scale_monthly: vec![Decimal::ONE; h],
            income_delta_monthly: vec![Decimal::ZERO; h],
            expense_delta_monthly: vec![Decimal::ZERO; h],
        };
        let out = project_net_worth_series(&input).unwrap();
        assert_eq!(out.net_worth, legacy.net_worth, "{}", case.name);
        assert_eq!(out.liquid_worth, legacy.liquid_worth, "{}", case.name);
        assert_eq!(
            out.retirement_month_index, legacy.retirement_month_index,
            "{}",
            case.name
        );
        assert_eq!(out.warnings, legacy.warnings, "{}", case.name);
        let (a, b) = (
            first_month_allocation(&input).unwrap(),
            first_month_allocation(&case.input).unwrap(),
        );
        assert_eq!(a.per_asset, b.per_asset, "{}", case.name);
        assert_eq!(a.base_cash, b.base_cash, "{}", case.name);
        assert_eq!(a.leftover, b.leftover, "{}", case.name);
    }
}

/// `income_follows_inflation = true` con inflación 0 % es la legacy: `f(k) = 1` exacto en todo el
/// horizonte. Predicción: igual en todos los casos de la batería con inflación 0.
#[test]
fn income_indexing_with_zero_inflation_is_legacy() {
    let mut n = 0;
    for case in projection_cases_all() {
        if !case.input.annual_inflation_percent.is_zero() {
            continue;
        }
        n += 1;
        let legacy = project_net_worth_series(&case.input).unwrap();
        let mut input = case.input.clone();
        input.income_expense_changes.income_follows_inflation = true;
        let out = project_net_worth_series(&input).unwrap();
        assert_eq!(out.net_worth, legacy.net_worth, "{}", case.name);
        assert_eq!(out.liquid_worth, legacy.liquid_worth, "{}", case.name);
    }
    assert!(n > 0, "la batería tiene casos con inflación 0");
}

// ---------------------------------------------------------------------------------------------
// Ingreso indexado
// ---------------------------------------------------------------------------------------------

/// Ingreso 2.000 €, gasto 0, inflación 2,5 %, `income_follows_inflation`.
///
/// Predicción: mes 1 → 2.000 (`f = 1`); mes 13 → `2.000 · 1,025^(12/12) = 2.050` EXACTO (el
/// exponente 12/12 normaliza a entero y `powd` va por potencia exacta); mes 25 → `2.000 ·
/// 1,025² = 2.101,25` exacto; mes 7 → `2.000 · f(7)` con el factor público.
#[test]
fn indexed_income_at_month_13_is_base_times_1_025_exactly() {
    let mut input = household(36, 2_000, 0, dec("2.5"));
    input.income_expense_changes.income_follows_inflation = true;
    let net = monthly_net(&input);
    assert_eq!(net[0], eur(2_000));
    assert_eq!(net[12], dec("2050"));
    assert_eq!(net[24], dec("2101.25"));
    assert_close(
        net[6],
        eur(2_000) * inflation_factor_at_month_index(dec("2.5"), 6),
        "mes 7",
    );
    // Y sin el flag, plano.
    let flat = monthly_net(&household(36, 2_000, 0, dec("2.5")));
    assert!(flat.iter().all(|x| *x == eur(2_000)));
}

/// `expense_follows_inflation = false`: el gasto queda plano. Ingreso 3.000, gasto 1.000, 5 %.
/// Predicción: `NW[k] − NW[k−1] = 2.000` todos los meses.
#[test]
fn expense_not_following_inflation_stays_flat() {
    let mut input = household(36, 3_000, 1_000, eur(5));
    input.income_expense_changes.expense_follows_inflation = false;
    assert!(monthly_net(&input).iter().all(|x| *x == eur(2_000)));
}

// ---------------------------------------------------------------------------------------------
// Constructor de vectores
// ---------------------------------------------------------------------------------------------

/// Gasto −900 €/mes de hoy, indexado, desde el mes 25, inflación 3 %.
///
/// Predicción (cálculo manual): `delta[24] = −900 · 1,03^(24/12) = −900 · 1,0609 = −954,81`
/// exacto; `delta[23] = 0`; `delta[25] = −900 · 1,03^(25/12)`. En el bucle, ingreso 3.000 y gasto
/// 2.000: mes 25 → `3.000 − (2.000 · 1,0609 − 954,81) = 3.000 − 1.166,99 = 1.833,01`.
#[test]
fn indexed_monthly_expense_cut_from_month_25() {
    let infl = eur(3);
    let v = build_change_vectors(
        &[change(
            ChangeTarget::Expense,
            25,
            None,
            ChangeKind::DeltaMonthly {
                amount_today: eur(-900),
            },
            true,
        )],
        60,
        infl,
    );
    assert!(v.income_scale_monthly.is_empty());
    assert!(v.expense_scale_monthly.is_empty());
    assert!(v.income_delta_monthly.is_empty());
    assert_eq!(v.expense_delta_monthly.len(), 60);
    assert!(v.expense_delta_monthly[..24].iter().all(|x| x.is_zero()));
    assert_eq!(v.expense_delta_monthly[24], dec("-954.81"));
    assert_eq!(
        v.expense_delta_monthly[25],
        eur(-900) * inflation_factor_at_month_index(infl, 25)
    );
    assert_eq!(
        v.expense_delta_monthly[59],
        eur(-900) * inflation_factor_at_month_index(infl, 59)
    );

    let net = monthly_net(&with_vectors(household(60, 3_000, 2_000, infl), v));
    assert_eq!(net[24], dec("1833.01"));
    assert_close(
        net[23],
        eur(3_000) - eur(2_000) * inflation_factor_at_month_index(infl, 23),
        "mes 24",
    );
}

/// Ingreso −100 % desde el mes 10 y +1.400 € de hoy indexados desde el mes 20, inflación 2 %,
/// ingreso 2.000, gasto 0.
///
/// Predicción: meses 1–9 → 2.000; meses 10–19 → 0 EXACTO; mes 20 → `1.400 · 1,02^(19/12)`;
/// mes 21 → `1.400 · 1,02^(20/12)`.
#[test]
fn income_drops_to_zero_then_indexed_pension_starts() {
    let infl = eur(2);
    let v = build_change_vectors(
        &[
            change(
                ChangeTarget::Income,
                10,
                None,
                ChangeKind::Percent { pct: eur(-100) },
                true,
            ),
            change(
                ChangeTarget::Income,
                20,
                None,
                ChangeKind::DeltaMonthly {
                    amount_today: eur(1_400),
                },
                true,
            ),
        ],
        36,
        infl,
    );
    let net = monthly_net(&with_vectors(household(36, 2_000, 0, infl), v));
    for (i, x) in net.iter().enumerate().take(9) {
        assert_eq!(*x, eur(2_000), "mes {}", i + 1);
    }
    for (i, x) in net.iter().enumerate().take(19).skip(9) {
        assert_eq!(*x, Decimal::ZERO, "mes {}", i + 1);
    }
    assert_close(
        net[19],
        eur(1_400) * inflation_factor_at_month_index(infl, 19),
        "mes 20",
    );
    assert_close(
        net[20],
        eur(1_400) * inflation_factor_at_month_index(infl, 20),
        "mes 21",
    );
}

/// Puntual de 75.000 € de hoy en el mes 18, indexado, inflación 2,5 %; ingreso 1.000, gasto 0.
///
/// Predicción: solo `delta[17] ≠ 0`, igual a `75.000 · 1,025^(17/12)`; el bucle suma ese importe
/// al mes 18 y nada más.
#[test]
fn indexed_one_off_lands_in_its_single_month() {
    let infl = dec("2.5");
    let v = build_change_vectors(
        &[change(
            ChangeTarget::Income,
            18,
            Some(30), // ignorado en un puntual
            ChangeKind::OneOff {
                amount_today: eur(75_000),
            },
            true,
        )],
        36,
        infl,
    );
    let one_off = eur(75_000) * inflation_factor_at_month_index(infl, 17);
    for (i, x) in v.income_delta_monthly.iter().enumerate() {
        if i == 17 {
            assert_eq!(*x, one_off);
        } else {
            assert!(x.is_zero(), "índice {i}");
        }
    }
    let net = monthly_net(&with_vectors(household(36, 1_000, 0, infl), v));
    assert_close(net[17], eur(1_000) + one_off, "mes 18");
    assert_eq!(net[16], eur(1_000));
    assert_eq!(net[18], eur(1_000));
}

/// Porcentajes solapados se componen multiplicando; un importe NO indexado es nominal y plano;
/// `end_month` es inclusivo; inicio 0 = 1; inicio tras el horizonte = sin efecto.
///
/// Predicción: escala 0,9 en 5–9, `0,9 · 1,2 = 1,08` en 10–20, 1,2 en 21–30, 1 fuera; delta
/// +500 exacto en 6–12 y 0 fuera; el gasto: −50 en el mes 1 por el inicio 0.
#[test]
fn builder_composes_percents_and_keeps_nominal_amounts_flat() {
    let v = build_change_vectors(
        &[
            change(
                ChangeTarget::Income,
                5,
                Some(20),
                ChangeKind::Percent { pct: eur(-10) },
                true,
            ),
            change(
                ChangeTarget::Income,
                10,
                Some(30),
                ChangeKind::Percent { pct: eur(20) },
                false,
            ),
            change(
                ChangeTarget::Income,
                6,
                Some(12),
                ChangeKind::DeltaMonthly {
                    amount_today: eur(500),
                },
                false,
            ),
            change(
                ChangeTarget::Expense,
                0,
                Some(1),
                ChangeKind::DeltaMonthly {
                    amount_today: eur(-50),
                },
                true,
            ),
            change(
                ChangeTarget::Expense,
                37,
                None,
                ChangeKind::OneOff {
                    amount_today: eur(1_000_000),
                },
                true,
            ),
        ],
        36,
        eur(3),
    );
    for k in 1..=36usize {
        let s = v.income_scale_monthly[k - 1];
        let want = match k {
            5..=9 => dec("0.9"),
            10..=20 => dec("1.08"),
            21..=30 => dec("1.2"),
            _ => Decimal::ONE,
        };
        assert_eq!(s, want, "escala del mes {k}");
        let dl = v.income_delta_monthly[k - 1];
        let want = if (6..=12).contains(&k) {
            eur(500)
        } else {
            Decimal::ZERO
        };
        assert_eq!(dl, want, "delta del mes {k}");
    }
    assert!(v.expense_scale_monthly.is_empty());
    assert_eq!(v.expense_delta_monthly[0], eur(-50));
    assert!(v.expense_delta_monthly[1..].iter().all(|x| x.is_zero()));
}

// ---------------------------------------------------------------------------------------------
// Suelo, validación y primer mes
// ---------------------------------------------------------------------------------------------

/// Gasto 2.000, delta fijo −5.000 desde el mes 3; ingreso 1.000, inflación 0.
///
/// Predicción: meses 1–2 → −1.000 (déficit vendido); desde el 3 el gasto quedaría en −3.000 y se
/// publica 0: +1.000. Un único aviso `CashFlowClampedAtZero`; la legacy no avisa.
#[test]
fn negative_expense_is_floored_at_zero_with_one_warning() {
    let v = build_change_vectors(
        &[change(
            ChangeTarget::Expense,
            3,
            None,
            ChangeKind::DeltaMonthly {
                amount_today: eur(-5_000),
            },
            false,
        )],
        24,
        Decimal::ZERO,
    );
    let input = with_vectors(household(24, 1_000, 2_000, Decimal::ZERO), v);
    let net = monthly_net(&input);
    assert_eq!(net[0], eur(-1_000));
    assert_eq!(net[1], eur(-1_000));
    assert!(net[2..].iter().all(|x| *x == eur(1_000)));
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.warnings, vec![EngineWarning::CashFlowClampedAtZero]);
    assert_eq!(
        EngineWarning::CashFlowClampedAtZero.code(),
        "cash_flow_clamped_at_zero"
    );
    let legacy = project_net_worth_series(&household(24, 1_000, 2_000, Decimal::ZERO)).unwrap();
    assert!(legacy.warnings.is_empty());
}

/// Un vector que no está vacío ni mide el horizonte es un input inválido, en la simulación y en
/// la lectura del primer mes.
#[test]
fn badly_sized_vector_is_rejected() {
    let mut input = household(24, 1_000, 0, Decimal::ZERO);
    input.income_expense_changes.vectors.expense_delta_monthly = vec![Decimal::ZERO; 23];
    assert!(matches!(
        project_net_worth_series(&input),
        Err(EngineError::InvalidIncomeExpenseChanges)
    ));
    assert!(matches!(
        first_month_allocation(&input),
        Err(EngineError::InvalidIncomeExpenseChanges)
    ));
}

/// El primer mes ve los cambios del mes 1. Ingreso 2.000 a la mitad (−50 %), gasto 500.
/// Predicción: `base_cash = 2.000 · 0,5 − 500 = 500` y la cascada manda 500 a la cuenta; el
/// bucle publica la misma cifra en el mes 1.
#[test]
fn first_month_allocation_sees_month_one_changes() {
    let v = build_change_vectors(
        &[change(
            ChangeTarget::Income,
            1,
            None,
            ChangeKind::Percent { pct: eur(-50) },
            true,
        )],
        12,
        Decimal::ZERO,
    );
    let input = with_vectors(household(12, 2_000, 500, Decimal::ZERO), v);
    let a = first_month_allocation(&input).unwrap();
    assert_eq!(a.base_cash, eur(500));
    assert_eq!(a.per_asset, vec![eur(500)]);
    assert_eq!(monthly_net(&input)[0], eur(500));
}

/// El `Default` de los cambios es el comportamiento de 5.0.
#[test]
fn default_is_legacy_shape() {
    let d = IncomeExpenseChanges::default();
    assert!(!d.income_follows_inflation);
    assert!(d.expense_follows_inflation);
    assert_eq!(d.vectors, ChangeVectors::default());
}
