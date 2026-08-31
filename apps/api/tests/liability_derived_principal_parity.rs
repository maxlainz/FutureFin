//! Paridad cliente↔servidor del PRINCIPAL DERIVADO del plan de pago (4.2.0).
//!
//! Carga `tests/fixtures/liability-derived-principal-parity.json` y, para cada caso, reproduce lo
//! que hace `derive_principal_from_payment_plan` (`apps/api/src/handlers/liabilities.rs`) una vez
//! contado el número de intervalos:
//!
//! - `fixed_payments` → `cuota × n`, exacto, sin pasar por el engine.
//! - `french` → `futurefin_engine::present_value_of_payments` redondeado a 4 decimales
//!   (`MidpointAwayFromZero`, la escala de money del proyecto) — el mismo redondeo vive en el
//!   handler y no en el engine.
//!
//! El mismo JSON lo consume el frontend (`apps/web/src/lib/ledger.repayment-model.test.ts`), que
//! calcula la **vista previa** del formulario en `number` (f64). Si alguien cambia la fórmula en un
//! lado y no en el otro, uno de los dos suites falla: es exactamente el papel que ya cumple
//! `fire-parity.json` para el cálculo FIRE.
//!
//! Qué NO cubre: el conteo de intervalos (`payment_interval_count` ↔ `paymentIntervalCountUtc`),
//! que depende de «hoy» y del calendario, y que el test de integración
//! `liability_derived_principal.rs` verifica contra el servidor real. Aquí `intervals` viene dado
//! por el fixture, así que este test es puro y no necesita Postgres.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::Value;
use std::fs;
use std::str::FromStr;

#[derive(Debug)]
struct DerivedCase {
    name: String,
    repayment_model: String,
    payment_amount: Decimal,
    intervals: u32,
    apr_percent: Option<Decimal>,
    expected_principal: f64,
}

fn dec(v: &Value, field: &str, case: &str) -> Decimal {
    let s = v[field]
        .as_str()
        .unwrap_or_else(|| panic!("{case}: `{field}` debe ser un string decimal"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{case}: `{field}` = {s:?} no es Decimal: {e}"))
}

fn load() -> (Vec<DerivedCase>, f64) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/liability-derived-principal-parity.json"
    );
    let raw = fs::read_to_string(path).expect("liability-derived-principal-parity.json missing");
    let v: Value = serde_json::from_str(&raw).expect("fixture malformed");
    let tolerance = v["_tolerance_eur"].as_f64().expect("_tolerance_eur");
    let cases = v["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .map(|c| {
            let name = c["name"].as_str().expect("name").to_string();
            DerivedCase {
                repayment_model: c["repayment_model"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{name}: repayment_model"))
                    .to_string(),
                payment_amount: dec(c, "payment_amount", &name),
                intervals: c["intervals"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("{name}: intervals")) as u32,
                apr_percent: c["apr_percent"]
                    .as_str()
                    .map(|s| Decimal::from_str(s).expect("apr_percent decimal")),
                expected_principal: c["expected_principal"]
                    .as_f64()
                    .unwrap_or_else(|| panic!("{name}: expected_principal")),
                name,
            }
        })
        .collect::<Vec<_>>();
    (cases, tolerance)
}

/// Réplica exacta de `derive_principal_from_payment_plan` (el handler no expone la función; es
/// privada, y con razón: recibe fechas y `PaymentFrequency`). Desde 4.7.0 (#144/#121) es una
/// RAMA ÚNICA — valor actual al TIN — porque el modelo ya no decide nada: `fixed_payments` no
/// puede llevar TIN, y `present_value_of_payments` sin TIN devuelve `M·n` exacto (retorno
/// temprano, sin exponencial). El `model` se conserva en la firma solo para vetar los que no
/// derivan. El conteo de intervalos queda para el test de integración.
fn derived_principal(model: &str, payment: Decimal, n: u32, apr: Option<Decimal>) -> Decimal {
    match model {
        "french" | "fixed_payments" => {
            futurefin_engine::present_value_of_payments(payment, Decimal::from(n), apr)
                .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
        }
        other => panic!("modelo sin derivación soportada en el fixture: {other}"),
    }
}

#[test]
fn server_derived_principal_matches_canonical_fixtures() {
    let (cases, tolerance) = load();
    assert!(!cases.is_empty(), "fixture vacío");

    for case in &cases {
        let got = derived_principal(
            &case.repayment_model,
            case.payment_amount,
            case.intervals,
            case.apr_percent,
        );
        let got_f = got.to_f64().expect("principal representable en f64");
        let diff = (got_f - case.expected_principal).abs();
        assert!(
            diff <= tolerance,
            "{}: principal derivado {got_f} vs fixture {} (Δ {diff} > {tolerance})",
            case.name,
            case.expected_principal
        );
    }
}

/// `fixed_payments` no es «PV con TIN 0»: es `Σ cuotas` EXACTO, sin redondeo ni transcendental de
/// por medio. Se comprueba con igualdad de `Decimal` —no con la tolerancia del fixture— porque es
/// el contrato que heredan todos los pasivos anteriores a 4.2.0.
#[test]
fn fixed_payments_cases_are_exact_not_approximate() {
    let (cases, _) = load();
    let mut seen = 0;
    for case in cases.iter().filter(|c| c.repayment_model == "fixed_payments") {
        seen += 1;
        assert_eq!(
            derived_principal("fixed_payments", case.payment_amount, case.intervals, case.apr_percent),
            case.payment_amount * Decimal::from(case.intervals),
            "{}: Σ cuotas debe ser exacto",
            case.name
        );
    }
    // Era `>= 2` hasta 4.7.0: el segundo caso («fixed_payments con TIN informado») describía un
    // estado que #144 hizo irrepresentable y se retiró del fixture (ver su
    // `_why_no_fixed_payments_with_apr_case`).
    assert!(seen >= 1, "el fixture debe conservar el caso fixed_payments");
}

/// El fixture describe SOLO los dos modelos que derivan. `interest_only` y `revolving` los rechaza
/// el handler (`derive_not_supported_for_model`), así que colar uno aquí significaría que el
/// fixture y la validación se han separado.
#[test]
fn fixture_only_carries_models_that_can_derive() {
    let (cases, _) = load();
    for case in &cases {
        assert!(
            matches!(case.repayment_model.as_str(), "fixed_payments" | "french"),
            "{}: modelo {} no deriva principal",
            case.name,
            case.repayment_model
        );
        if case.repayment_model == "french" {
            assert!(
                matches!(case.apr_percent, Some(a) if a > Decimal::ZERO),
                "{}: french exige apr_percent > 0 (apr_required_for_model)",
                case.name
            );
        }
    }
}
