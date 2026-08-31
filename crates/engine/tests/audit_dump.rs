//! Arnés de auditoría del modelo financiero: vuelca en CSV (stdout) las series del engine para
//! una batería de casos límite, de modo que un oráculo EXTERNO (reimplementación independiente)
//! pueda compararlas mes a mes sin pasar por la API ni por una base de datos.
//!
//! No afirma nada por sí mismo — las afirmaciones viven en los tests de regresión normales.
//! Uso: `cargo test -p futurefin-engine --test audit_dump -- --nocapture > dump.csv`
//!
//! Formato de las líneas:
//!   `LIABM,<caso>,<mes>,<opening>,<interes>,<principal_amortizado>,<cuota>,<closing>`
//!   `LIABS,<caso>,<payoff_mes|->,<ausencia|->,<interes_total>,<principal_final>,<cuotas_total>`
//!   `PROJ,<caso>,<mes>,<net_worth>,<contributed_capital>`

use chrono::NaiveDate;
use futurefin_engine::{
    liability_amortization_schedule, project_net_worth_series, AllocationRule, FireTarget,
    LiabilityPayoffAbsence, ProjectionInput, ProjectionLiabilityInput, RepaymentModel, SimAsset,
};
use rust_decimal::Decimal;
use uuid::Uuid;

fn d(mantissa: i64, scale: u32) -> Decimal {
    Decimal::new(mantissa, scale)
}

fn ref_date() -> NaiveDate {
    // Fecha fija: el dump debe ser byte-estable entre ejecuciones.
    NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
}

fn mk_asset(id: u128, value: Decimal, liquid: bool, rate: Option<Decimal>) -> SimAsset {
    SimAsset {
        id: Uuid::from_u128(id),
        value,
        purchase_price: None,
        is_liquid: liquid,
        expected_annual_return_percent: rate,
    }
}

fn mk_liab(
    principal: Decimal,
    payment: Decimal,
    apr: Option<Decimal>,
    model: RepaymentModel,
    end_after_months: Option<u32>,
) -> ProjectionLiabilityInput {
    let payment_end = end_after_months.map(|n| {
        // Último día del mes n (1-based) contando desde ref_date.
        let first = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        first
            .checked_add_months(chrono::Months::new(n))
            .unwrap()
            .pred_opt()
            .unwrap()
    });
    ProjectionLiabilityInput {
        principal,
        monthly_payment: payment,
        payment_end,
        repayment_model: model,
        apr_percent: apr,
        min_payment_pct: None,
        min_payment_eur: None,
        extra_principal_monthly: Decimal::ZERO,
        extra_principal_lump_sums: vec![],
        early_repayment_fee_pct: None,
        early_repayment_effect: Default::default(),
    }
}

fn base_input(
    horizon: u32,
    income: Decimal,
    expense: Decimal,
    assets: Vec<SimAsset>,
    rules: Vec<AllocationRule>,
) -> ProjectionInput {
    ProjectionInput {
        ref_date: ref_date(),
        horizon_months: horizon,
        income_regular_monthly: income,
        expense_regular_monthly: expense,
        assets,
        allocation_rules: rules,
        liabilities: vec![],
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        retirement_start_month: None,
        income_retirement_monthly: Decimal::ZERO,
        expense_retirement_monthly: expense,
        retirement_monthly_withdrawal: Decimal::ZERO,
        fire_target: None,
    }
}

fn dump_schedule(case: &str, liab: &ProjectionLiabilityInput, horizon: u32) {
    let s = liability_amortization_schedule(liab, ref_date(), horizon);
    for m in &s.months {
        println!(
            "LIABM,{case},{},{},{},{},{},{}",
            m.month_index,
            m.opening_principal,
            m.interest_accrued,
            m.principal_repaid,
            m.payment,
            m.closing_principal
        );
    }
    let absence = match s.payoff_absent {
        None => "-".to_string(),
        Some(LiabilityPayoffAbsence::NoPaymentPlan) => "no_payment_plan".to_string(),
        Some(LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff) => {
            "plan_ends_before_payoff".to_string()
        }
        Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal) => {
            "payment_does_not_reduce_principal".to_string()
        }
        Some(LiabilityPayoffAbsence::NotWithinHorizon) => "not_within_horizon".to_string(),
    };
    println!(
        "LIABS,{case},{},{absence},{},{},{}",
        s.payoff_month_index
            .map(|k| k.to_string())
            .unwrap_or_else(|| "-".to_string()),
        s.total_interest,
        s.final_principal,
        s.total_payments
    );
}

fn dump_projection(case: &str, input: &ProjectionInput) {
    let out = project_net_worth_series(input).expect("la simulación del caso no debe fallar");
    for (k, nw) in out.net_worth.iter().enumerate() {
        println!("PROJ,{case},{k},{nw},{}", out.contributed_capital[k]);
    }
}

/// Batería de calendarios de amortización (casos L*).
#[test]
fn audit_dump_liability_schedules() {
    // L1: préstamo que VENCE con saldo vivo — 50.000 € al TIN 6 %, cuota 500 €, plan de 60 meses.
    dump_schedule(
        "L1_venc_saldo_vivo",
        &mk_liab(
            Decimal::from(50_000),
            Decimal::from(500),
            Some(Decimal::from(6)),
            RepaymentModel::French,
            Some(60),
        ),
        840,
    );
    // L2: cuota por debajo del interés — 20.000 € al TIN 24 %, cuota 300 € (interés mes 1 = 400 €).
    dump_schedule(
        "L2_cuota_bajo_interes",
        &mk_liab(
            Decimal::from(20_000),
            Decimal::from(300),
            Some(Decimal::from(24)),
            RepaymentModel::French,
            None,
        ),
        120,
    );
    // L3: revolving — 5.000 € al 21 % (tal cual lo teclearía el usuario desde una TAE cotizada),
    // cuota fija 150 €.
    dump_schedule(
        "L3_revolving",
        &mk_liab(
            Decimal::from(5_000),
            Decimal::from(150),
            Some(Decimal::from(21)),
            RepaymentModel::Revolving,
            None,
        ),
        840,
    );
    // L4: pasivo SIN TIN con modelo francés — degeneración deliberada a fixed_payments.
    dump_schedule(
        "L4_sin_tin",
        &mk_liab(
            Decimal::from(100_000),
            Decimal::from(500),
            None,
            RepaymentModel::French,
            None,
        ),
        840,
    );
    // L5: caso patrón contra cuadro externo — 100.000 € al TIN 3 %, cuota de anualidad para
    // n = 278 meses: M = P·i/(1−(1+i)^−n) con i = 0,0025 → 499,51 € (redondeada a céntimo).
    dump_schedule(
        "L5_bde_100k_3pct",
        &mk_liab(
            Decimal::from(100_000),
            d(49_951, 2),
            Some(Decimal::from(3)),
            RepaymentModel::French,
            None,
        ),
        840,
    );
    // L6: interest_only con cuota declarada ≠ interés real — 100.000 € al TIN 4 % (interés real
    // 333,33 €/mes) pero cuota declarada 200 €.
    dump_schedule(
        "L6_interest_only_200",
        &mk_liab(
            Decimal::from(100_000),
            Decimal::from(200),
            Some(Decimal::from(4)),
            RepaymentModel::InterestOnly,
            None,
        ),
        120,
    );
}

/// Batería de proyecciones (casos P*).
#[test]
fn audit_dump_projection_series() {
    // P1: déficit crónico — ingreso 1.000, gasto 2.500, 30.000 € líquidos al 0 %.
    // La realidad: agotamiento en el mes 20 y desde ahí el NW cae 1.500 €/mes (deuda implícita).
    dump_projection(
        "P1_deficit_cronico",
        &base_input(
            60,
            Decimal::from(1_000),
            Decimal::from(2_500),
            vec![mk_asset(1, Decimal::from(30_000), true, None)],
            vec![],
        ),
    );

    // P2: FIRE alcanzado en el mes 0 — NW inicial 900.000 ≥ target 800.000.
    let mut p2 = base_input(
        24,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(900_000), true, None)],
        vec![],
    );
    p2.income_retirement_monthly = Decimal::ZERO;
    p2.expense_retirement_monthly = Decimal::from(2_000);
    p2.fire_target = Some(FireTarget {
        base_amount: Decimal::from(800_000),
        annual_inflation_percent: d(25, 1),
        debt_payments_remaining: Vec::new(),
    });
    dump_projection("P2_fire_mes0", &p2);

    // P3: superávit post-jubilación — pensión 2.500 vs gasto 2.000 tras cruzar un target bajo;
    // el sobrante de 500 €/mes se acumula al 0 % durante décadas.
    let mut p3 = base_input(
        480,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(190_000), true, Some(Decimal::from(3)))],
        vec![],
    );
    p3.income_retirement_monthly = Decimal::from(2_500);
    p3.expense_retirement_monthly = Decimal::from(2_000);
    p3.fire_target = Some(FireTarget {
        base_amount: Decimal::from(200_000),
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    dump_projection("P3_superavit_jubilacion", &p3);

    // P4: retornos en el borde −100 % / −101 % — factor 0, jamás negativo.
    dump_projection(
        "P4_ret_menos100",
        &base_input(
            6,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![
                mk_asset(1, Decimal::from(10_000), true, Some(Decimal::from(-100))),
                mk_asset(2, Decimal::from(10_000), true, Some(Decimal::from(-101))),
            ],
            vec![],
        ),
    );

    // P5: caso patrón de unidades — ingreso 3.000 / gasto 2.000 nominales planos 30 años,
    // activo 10.000 al 7 %. Para confrontar contra un oráculo con flujos indexados al IPC.
    dump_projection(
        "P5_flat_nominal_30y",
        &base_input(
            360,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![mk_asset(1, Decimal::from(10_000), true, Some(Decimal::from(7)))],
            vec![],
        ),
    );

    // P6: el préstamo L1 dentro de una proyección a 30 años — tras vencer el plan en el mes 60,
    // el saldo vivo queda CONGELADO restando al patrimonio hasta el final.
    let mut p6 = base_input(
        360,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(10_000), true, None)],
        vec![],
    );
    p6.liabilities = vec![mk_liab(
        Decimal::from(50_000),
        Decimal::from(500),
        Some(Decimal::from(6)),
        RepaymentModel::French,
        Some(60),
    )];
    dump_projection("P6_venc_saldo_vivo_proj", &p6);
}
