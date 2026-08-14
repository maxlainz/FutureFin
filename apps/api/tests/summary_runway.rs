//! Captura de regresión del **runway** y de la base de gasto de `GET /v1/summary`
//! (`financial_health`) ANTES del cambio de base previsto para 2.2.0.
//!
//! Fija, con números predichos a mano y comparados en `Decimal` exacto:
//! - `runway_months == liquid_assets_total / expense_total_monthly_equivalent` (modo A, sin
//!   rentabilidad ni inflación: división simple).
//! - `expense_derived_monthly_equivalent` = cuotas de pasivos activos.
//! - `expense_total_monthly_equivalent` = gasto de presupuesto + cuotas derivadas.
//! - sin gasto (`expense_total == 0`) el campo `runway_months` **no se serializa**.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

/// El API serializa importes como string decimal (`"1200.00"`), así que hay que parsear a
/// `Decimal` antes de comparar: `assert_eq!(v, "1200")` sobre el string fallaría.
fn dec(v: &Value) -> Decimal {
    Decimal::from_str(
        v.as_str()
            .unwrap_or_else(|| panic!("expected decimal string, got {v:?}")),
    )
    .expect("parse decimal string")
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn liquid_asset(app: &TestApp, cookie: &str, cat: &str, name: &str, value: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": name, "current_value": value, "is_liquid": true }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset {name}: {r:?}");
}

async fn budget(app: &TestApp, cookie: &str, cat: &str, amount: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({ "category_id": cat, "amount": amount }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "budget: {r:?}");
}

async fn health(app: &TestApp, cookie: &str) -> Value {
    let resp = app.get_with_cookie("/v1/summary", cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "summary: {resp:?}");
    resp.json()["financial_health"].clone()
}

/// Baseline (modo A, sin rentabilidad esperada ni inflación en juego):
/// - líquidos: 9.000 + 3.000 = **12.000**
/// - gasto de presupuesto: **1.000/mes**
/// - cuota derivada de un pasivo activo (mensual, 200): **200/mes**
/// - `expense_total` = 1.000 + 200 = **1.200**
/// - `runway_months` = 12.000 / 1.200 = **10**
#[tokio::test]
async fn runway_pre_change_baseline_liquid_over_expense() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta A", "9000").await;
    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta B", "3000").await;
    // Activo NO líquido: no debe entrar en `liquid_assets_total` ni en el runway.
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "Piso", "current_value": "250000",
                    "is_liquid": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = NaiveDate::from_ymd_opt(today.year() + 5, 1, 15)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let l = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "L1", "principal": "50000",
                    "payment_amount": "200", "payment_frequency": "monthly",
                    "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l.status, http::StatusCode::CREATED, "{l:?}");

    let h = health(&app, &owner.cookie).await;
    assert_eq!(dec(&h["liquid_assets_total"]), d(12_000));
    assert_eq!(dec(&h["expense_regular_monthly_equivalent"]), d(1_000));
    assert_eq!(dec(&h["expense_derived_monthly_equivalent"]), d(200));
    assert_eq!(dec(&h["expense_total_monthly_equivalent"]), d(1_200));
    // 12.000 / 1.200 = 10 meses exactos.
    assert_eq!(dec(&h["runway_months"]), d(10));
    assert_eq!(
        dec(&h["runway_months"]),
        dec(&h["liquid_assets_total"]) / dec(&h["expense_total_monthly_equivalent"]),
        "runway = líquidos / gasto total"
    );
}

/// Sin gasto (ni presupuesto ni cuotas) el runway no existe: `runway_months` se omite del JSON
/// (`skip_serializing_if = Option::is_none`).
#[tokio::test]
async fn runway_zero_expense_is_null() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta A", "5000").await;
    budget(&app, &owner.cookie, &income_cat, "3000").await;

    let h = health(&app, &owner.cookie).await;
    assert_eq!(dec(&h["liquid_assets_total"]), d(5_000));
    assert_eq!(dec(&h["expense_total_monthly_equivalent"]), Decimal::ZERO);
    assert!(
        h["runway_months"].is_null(),
        "sin gasto no hay runway, got {:?}",
        h["runway_months"]
    );
}
