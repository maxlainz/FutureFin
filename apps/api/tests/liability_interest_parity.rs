//! Paridad del interés mensual aproximado (#136-6, 4.11.0):
//! `apps/api/tests/fixtures/liability-interest-parity.json` casa el helper del cliente
//! (`liabilitiesApproxMonthlyInterestSum`, suite Vitest `ledger.repayment-model.test.ts`) con la
//! base compartida del servidor: el predicado `futurefin_engine::liability_interest_accrues`
//! (#121, la ÚNICA definición de «devenga») más la fórmula `principal × TIN/1200`. No hay campo
//! de hogar que consumir — el servidor publica la TASA (`net_return_*`) y el € solo por pasivo —
//! así que la duplicación es aceptada y ESTE fixture es lo que impide que los dos lados se
//! separen en silencio.

use chrono::NaiveDate;
use futurefin_engine::{liability_interest_accrues, RepaymentModel};
use rust_decimal::Decimal;

fn model_of(s: &str) -> RepaymentModel {
    match s {
        "fixed_payments" => RepaymentModel::FixedPayments,
        "french" => RepaymentModel::French,
        "interest_only" => RepaymentModel::InterestOnly,
        "revolving" => RepaymentModel::Revolving,
        other => panic!("modelo desconocido en el fixture: {other}"),
    }
}

#[test]
fn approx_monthly_interest_matches_the_client_fixture() {
    let raw = include_str!("fixtures/liability-interest-parity.json");
    let fixture: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
    let cases = fixture["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());

    // Fecha fija dentro del rango en que el fixture es insensible al día (fines null/2099/2020).
    let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();

    for c in cases {
        let name = c["name"].as_str().unwrap();
        let expected: Decimal =
            c["expected_monthly_interest"].as_str().unwrap().parse().unwrap();

        let mut sum = Decimal::ZERO;
        for row in c["rows"].as_array().unwrap() {
            let model = model_of(row["repayment_model"].as_str().unwrap());
            let principal: Decimal = row["principal"].as_str().unwrap().parse().unwrap();
            let apr: Option<Decimal> =
                row["apr_percent"].as_str().map(|a| a.parse().unwrap());
            let payment: Decimal = row["payment_amount"]
                .as_str()
                .map(|p| p.parse().unwrap())
                .unwrap_or(Decimal::ZERO);
            let end: Option<NaiveDate> = row["payment_end_date"]
                .as_str()
                .map(|d| d.parse().unwrap());

            if liability_interest_accrues(model, apr, payment, end, today) {
                sum += principal * apr.unwrap() / Decimal::from(1200u32);
            }
        }
        assert!(
            (sum - expected).abs() < Decimal::new(1, 2),
            "{name}: Rust {sum}, fixture {expected}"
        );
    }
}
