//! Fase 4.6 — Paridad cliente↔servidor sobre el cálculo FIRE.
//!
//! Carga los casos canónicos desde `tests/fixtures/fire-parity.json` y para cada uno:
//!   1. configura una instalación con los `fire_settings` del caso,
//!   2. inserta un asset en estado inicial (mes 0),
//!   3. inserta budget entries que reproducen los `monthly` del caso (income/expense),
//!   4. llama `GET /v1/projection/series`,
//!   5. compara `jubilacion_target_net_worth` con `expected_target_nw` ± 1 €.
//!
//! El mismo JSON lo consume el test del frontend (`apps/web/src/lib/fire.test.ts`). Si alguien
//! cambia los tramos fiscales o la fórmula en un lado y no en el otro, uno de los dos suites
//! falla. La fuente de verdad es el JSON.

mod common;

use common::TestApp;
use serde_json::Value;
use std::fs;

#[derive(Debug)]
struct FireCase {
    name: String,
    fire_settings: Value,
    income_monthly: String,
    income_retirement_monthly: String,
    expense_retirement_monthly: String,
    expected_target_nw: Option<f64>,
}

fn load_cases() -> Vec<FireCase> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fire-parity.json");
    let raw = fs::read_to_string(path).expect("fire-parity.json missing");
    let v: Value = serde_json::from_str(&raw).expect("fire-parity.json malformed");
    let cases = v["cases"].as_array().expect("cases array");
    cases
        .iter()
        .map(|c| FireCase {
            name: c["name"].as_str().unwrap().to_string(),
            fire_settings: c["fire_settings"].clone(),
            income_monthly: c["monthly"]["income"].as_str().unwrap().to_string(),
            income_retirement_monthly: c["monthly"]["income_retirement"]
                .as_str()
                .unwrap()
                .to_string(),
            expense_retirement_monthly: c["monthly"]["expense_retirement"]
                .as_str()
                .unwrap()
                .to_string(),
            expected_target_nw: c["expected_target_nw"].as_f64(),
        })
        .collect()
}

/// Para cada caso del fixture, monta el estado y verifica `jubilacion_target_net_worth`.
#[tokio::test]
async fn server_target_matches_canonical_fixtures() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "fixtures vacíos");

    for case in cases {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner("alice").await;

        // PATCH installation con los FIRE settings del caso.
        let patch = app
            .patch_json_with_cookie(
                "/v1/installation",
                serde_json::json!({ "fire_settings": case.fire_settings }),
                &owner.cookie,
            )
            .await;
        assert_eq!(
            patch.status,
            http::StatusCode::OK,
            "[{}] patch fire_settings failed: {patch:?}",
            case.name
        );

        // Seed: una categoría income, una expense, un asset para tener algo en proyección.
        let asset_cat = app.create_category(&owner, "asset", "Cash").await;
        let income_cat = app.create_category(&owner, "income", "Nómina").await;
        let expense_cat = app.create_category(&owner, "expense", "Alquiler").await;

        let asset = app
            .post_json_with_cookie(
                "/v1/assets",
                serde_json::json!({
                    "category_id": asset_cat,
                    "name": "EUR",
                    "current_value": "10000",
                    "is_liquid": true,
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(asset.status, http::StatusCode::CREATED);

        // income normal + posible income post-jubilación (persists_after_retirement=true).
        let income_normal: f64 = case.income_monthly.parse().unwrap();
        let income_ret: f64 = case.income_retirement_monthly.parse().unwrap();
        if income_normal > income_ret {
            let pre = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": income_cat,
                        "amount": format!("{:.2}", income_normal - income_ret),
                        "persists_after_retirement": false,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(pre.status, http::StatusCode::CREATED, "{pre:?}");
        }
        if income_ret > 0.0 {
            let post = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": income_cat,
                        "amount": format!("{:.2}", income_ret),
                        "persists_after_retirement": true,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(post.status, http::StatusCode::CREATED, "{post:?}");
        }
        // Expense: ends_at_retirement=false → cuenta como expense_retirement (el que usa el servidor).
        let expense_ret: f64 = case.expense_retirement_monthly.parse().unwrap();
        if expense_ret > 0.0 {
            let exp = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": expense_cat,
                        "amount": format!("{:.2}", expense_ret),
                        "ends_at_retirement": false,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(exp.status, http::StatusCode::CREATED, "{exp:?}");
        }

        // GET projection y comparar.
        let series = app
            .get_with_cookie("/v1/projection/series", &owner.cookie)
            .await;
        assert_eq!(
            series.status,
            http::StatusCode::OK,
            "[{}] series failed: {series:?}",
            case.name
        );
        let body = series.json();
        let target = body["jubilacion_target_net_worth"].as_str();

        match (case.expected_target_nw, target) {
            (None, None) => { /* ok */ }
            (None, Some(_)) => panic!(
                "[{}] esperado sin target, servidor devolvió uno",
                case.name
            ),
            (Some(_), None) => panic!(
                "[{}] esperado target, servidor devolvió null",
                case.name
            ),
            (Some(expected), Some(s)) => {
                let actual: f64 = s.parse().expect("decimal");
                let diff = (actual - expected).abs();
                assert!(
                    diff <= 1.0,
                    "[{}] target divergente: esperado ≈ {expected}, servidor {actual} (diff {diff})",
                    case.name
                );
            }
        }
    }
}
