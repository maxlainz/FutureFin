//! Líneas derivadas de pasivos en `GET /v1/budget` — cobertura dedicada (hueco detectado en la
//! investigación de la reforma 3.4.0; hasta entonces solo había cobertura indirecta).
//!
//! Contrato del predicado «pasivo activo», unificado en 3.4.0 con el resto del sistema:
//! `payment_amount IS NOT NULL AND payment_frequency IS NOT NULL AND
//!  (payment_end_date IS NULL OR payment_end_date >= today)`.
//! Antes esta query era el único outlier (exigía fecha fin NOT NULL y `>` estricto): un pasivo sin
//! fecha fin no generaba línea derivada aunque el engine sí cobrara su cuota en modo A.

mod common;

use chrono::{Duration, NaiveDate};
use common::TestApp;
use serde_json::{json, Value};

fn parse_dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("expected decimal string, got {v:?}"))
        .parse::<f64>()
        .expect("parse decimal string")
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 0.01, "expected ~{b}, got {a}");
}

/// «Hoy» del SERVIDOR (ancla de /v1/history/series, en el calendar_tz de la instalación) — no el
/// del reloj UTC de la máquina, para que el test del borde `>=` no flaquee cerca de medianoche.
async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn create_liability(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    exp_cat: &str,
    label: &str,
    body_extra: Value,
) {
    let mut body = json!({ "category_id": cat, "expense_category_id": exp_cat, "label": label,
                           "principal": "10000" });
    for (k, v) in body_extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = app.post_json_with_cookie("/v1/liabilities", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability {label}: {r:?}");
}

async fn budget_snapshot(app: &TestApp, cookie: &str, query: &str) -> Value {
    let resp = app.get_with_cookie(query, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "budget: {resp:?}");
    resp.json()
}

/// Fecha fin NULL = plan indefinido → SÍ genera línea derivada (regresión de la reforma 3.4.0;
/// antes quedaba excluida y el modo A no contaba la cuota «una vez» de forma consistente).
#[tokio::test]
async fn derived_line_includes_null_end_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Sin fecha fin",
        json!({ "payment_amount": "500", "payment_frequency": "monthly" }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let derived = body["derived_from_liabilities"].as_array().unwrap();
    assert_eq!(derived.len(), 1, "el pasivo sin fecha fin debe derivar línea: {body:?}");
    approx(parse_dec(&derived[0]["monthly_equivalent"]), 500.0);
    assert_eq!(
        derived[0]["expense_category_id"].as_str().unwrap(),
        exp_cat,
        "la línea derivada expone la categoría de gasto de la cuota (3.4.0)"
    );
    approx(parse_dec(&body["totals"]["expense_derived_monthly_equivalent"]), 500.0);
    approx(parse_dec(&body["totals"]["expense_total_monthly_equivalent"]), 500.0);
}

/// Pasivo vencido (fecha fin pasada) → sin línea derivada; borde `>=`: el día EXACTO de fin aún
/// cuenta (mismo criterio que /v1/liabilities y /v1/summary).
#[tokio::test]
async fn derived_line_excludes_expired_but_includes_end_today() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = server_today(&app, &owner.cookie).await;
    let past = (today - Duration::days(5)).format("%Y-%m-%d").to_string();
    let today_s = today.format("%Y-%m-%d").to_string();

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Vencido",
        json!({ "payment_amount": "700", "payment_frequency": "monthly", "payment_end_date": past }),
    )
    .await;
    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Termina hoy",
        json!({ "payment_amount": "200", "payment_frequency": "monthly", "payment_end_date": today_s }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let derived = body["derived_from_liabilities"].as_array().unwrap();
    let labels: Vec<&str> = derived.iter().map(|d| d["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["Termina hoy"], "solo el que termina hoy deriva línea: {body:?}");
    approx(parse_dec(&body["totals"]["expense_derived_monthly_equivalent"]), 200.0);
}

/// Sin plan de pago (payment_amount/frequency NULL) → sin línea derivada aunque el pasivo exista.
#[tokio::test]
async fn derived_line_requires_payment_plan() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat, "Solo principal", json!({})).await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    assert!(
        body["derived_from_liabilities"].as_array().unwrap().is_empty(),
        "sin plan de pago no hay línea derivada: {body:?}"
    );
    approx(parse_dec(&body["totals"]["expense_derived_monthly_equivalent"]), 0.0);
}

/// Cuota semanal → equivalente mensual ×52/12 (70 €/semana ≈ 303,33 €/mes).
#[tokio::test]
async fn derived_line_weekly_monthly_equivalent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Semanal",
        json!({ "payment_amount": "70", "payment_frequency": "weekly" }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let derived = body["derived_from_liabilities"].as_array().unwrap();
    approx(parse_dec(&derived[0]["monthly_equivalent"]), 70.0 * 52.0 / 12.0);
}

/// Scoping: `?view=mine` deriva solo los pasivos del solicitante; household, los de todos.
#[tokio::test]
async fn derived_line_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Del owner",
        json!({ "payment_amount": "500", "payment_frequency": "monthly" }),
    )
    .await;
    create_liability(&app, &member.cookie, &cat, &exp_cat,
        "Del member",
        json!({ "payment_amount": "300", "payment_frequency": "monthly" }),
    )
    .await;

    let hh = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    approx(parse_dec(&hh["totals"]["expense_derived_monthly_equivalent"]), 800.0);

    let mine = budget_snapshot(&app, &owner.cookie, "/v1/budget?view=mine").await;
    approx(parse_dec(&mine["totals"]["expense_derived_monthly_equivalent"]), 500.0);
    let labels: Vec<&str> = mine["derived_from_liabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["Del owner"]);
}
