//! Paridad del DEFLACTOR (#136-4b, 4.11.0): `apps/api/tests/fixtures/deflator-parity.json` es la
//! fuente única que casa `deflator_at_month_index` (Decimal, servido por
//! `GET /v1/projection/deflate` y detrás de `net_worth_real`/`milestones_real`) con el helper f64
//! del chart (`deflationFactorAt`, suite Vitest `projection-chart.test.ts`). Si uno de los dos
//! lados cambia sin actualizar el JSON, SU suite falla — eso es el fixture funcionando, no un
//! test flaky. El dominio compartido es k >= 0 entero; los meses históricos (k < 0) y el grid
//! fino fraccionario son TS-only, divergencia aceptada declarada en financial-contracts §4.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::json;

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str().unwrap().parse().unwrap()
}

async fn set_inflation(app: &TestApp, owner: &LoggedInOwner, pct: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "annual_inflation_assumption_percent": pct }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "inflación {pct}: {r:?}");
}

#[tokio::test]
async fn served_deflator_matches_the_chart_helper_fixture() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let raw = include_str!("fixtures/deflator-parity.json");
    let fixture: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
    let cases = fixture["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());

    for c in cases {
        let pct = c["annual_inflation_percent"].as_str().unwrap();
        let k = c["month_index"].as_i64().unwrap();
        let expected: f64 = c["expected_deflator"].as_str().unwrap().parse().unwrap();

        set_inflation(&app, &owner, pct).await;
        let r = app
            .get_with_cookie(
                &format!("/v1/projection/deflate?amount=1000&month_index={k}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "deflate {pct} % k={k}: {r:?}");
        let got = dec(&r.json()["deflator"]);
        assert!(
            (got - expected).abs() < 1e-9,
            "deflactor {pct} % k={k}: servido {got}, fixture {expected}"
        );
    }
}
