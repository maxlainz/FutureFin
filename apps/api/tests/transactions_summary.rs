//! Integración de la comparativa (`GET /v1/transactions/summary`).
//!
//! Números PREDICHOS antes de ejecutar (ver comentarios). El "hoy" se deriva del servidor
//! (`/v1/history/series` anchor) para no depender del reloj de la máquina; el mes seleccionado se
//! sitúa 2 meses en el pasado (siempre completo). Los Decimals viajan como string → se comparan
//! valores parseados con tolerancia.

mod common;

use chrono::{Datelike, NaiveDate};
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

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

fn date_in(year: i32, month: u32, day: u32) -> String {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn manual(app: &TestApp, cookie: &str, date: &str, concept: &str, amount: &str, kind: &str, cat: Option<&str>) {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind });
    if let Some(c) = cat {
        body["category_id"] = json!(c);
    }
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "manual {concept}: {r:?}");
}

fn line<'a>(arr: &'a Value, name: &str) -> &'a Value {
    arr.as_array()
        .unwrap()
        .iter()
        .find(|l| l["category_name"] == name)
        .unwrap_or_else(|| panic!("no category line '{name}' in {arr:?}"))
}

#[tokio::test]
async fn summary_numbers_windows_and_no_double_count() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    let nomina_cat = app.create_category(&owner, "income", "Nómina").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    // Mes seleccionado = 2 meses antes de hoy (completo). Ventana avg_months=3 = sel-1,-2,-3.
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(sy, sm, -1);
    let (y3, m3) = shift_month(sy, sm, -3);

    // Presupuesto: Super 300 (expense), Nómina 2000 (income).
    for (cat, amount) in [(&super_cat, "300"), (&nomina_cat, "2000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({ "category_id": cat, "amount": amount }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "budget: {r:?}");
    }

    // Pasivo con cuota 500/mes (plan vivo) → derived_debt_line.budget = 500 (solo lado budget).
    let future = date_in(today.year() + 4, 1, 15);
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "Hipoteca", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability: {r:?}");

    // --- Transacciones ---
    // Mes seleccionado: Super -100 y -50 (=150), Sin categoría -30, income +2000, savings -200.
    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Super A", "-100", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 12), "Super B", "-50", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 8), "Kiosko", "-30", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 1), "Sueldo", "2000", "income", Some(&nomina_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 15), "Aporte", "-200", "savings", None).await;
    // sel-1: Super -60, income +1000, savings -100.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Super C", "-60", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 3), "Sueldo", "1000", "income", Some(&nomina_cat)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 20), "Aporte", "-100", "savings", None).await;
    // sel-3: Super -90.
    manual(&app, &owner.cookie, &date_in(y3, m3, 10), "Super D", "-90", "expense", Some(&super_cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_months=3");
    let resp = app.get_with_cookie(&url, &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "summary: {resp:?}");
    let b = resp.json();

    assert_eq!(b["year"].as_i64().unwrap(), sy as i64);
    assert_eq!(b["month"].as_u64().unwrap(), sm as u64);
    assert_eq!(b["is_partial"], false, "mes 2 atrás → completo");
    assert_eq!(b["avg_months"].as_u64().unwrap(), 3);

    // Línea Super: actual 150, budget 300, avg (60+0+90)/3=50, deltas -150 / +100.
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["actual"]), 150.0);
    approx(parse_dec(&sup["budget"]), 300.0);
    approx(parse_dec(&sup["avg"]), 50.0);
    approx(parse_dec(&sup["delta_vs_budget"]), -150.0);
    approx(parse_dec(&sup["delta_vs_avg"]), 100.0);

    // Sin categoría: actual 30, budget 0, avg 0.
    let sc = line(&b["expense_categories"], "Sin categoría");
    approx(parse_dec(&sc["actual"]), 30.0);
    approx(parse_dec(&sc["budget"]), 0.0);
    approx(parse_dec(&sc["avg"]), 0.0);

    // Ingreso Nómina: actual 2000, budget 2000, avg 1000/3.
    let nom = line(&b["income_categories"], "Nómina");
    approx(parse_dec(&nom["actual"]), 2000.0);
    approx(parse_dec(&nom["budget"]), 2000.0);
    approx(parse_dec(&nom["avg"]), 1000.0 / 3.0);
    approx(parse_dec(&nom["delta_vs_budget"]), 0.0);

    // Derived debt: 500 (solo lado budget).
    approx(parse_dec(&b["derived_debt_line"]["budget"]), 500.0);

    // Savings block: actual 200, avg 100/3.
    approx(parse_dec(&b["savings"]["actual"]), 200.0);
    approx(parse_dec(&b["savings"]["avg"]), 100.0 / 3.0);

    // Income block agregado.
    approx(parse_dec(&b["income"]["actual"]), 2000.0);
    approx(parse_dec(&b["income"]["avg"]), 1000.0 / 3.0);

    // Totales. expense_actual = 150+30 = 180 (SIN la cuota del pasivo → sin doble conteo).
    let t = &b["totals"];
    approx(parse_dec(&t["expense_actual"]), 180.0);
    approx(parse_dec(&t["expense_avg"]), 50.0);
    approx(parse_dec(&t["expense_budget"]), 800.0); // 300 Super + 500 derived
    approx(parse_dec(&t["income_actual"]), 2000.0);
    approx(parse_dec(&t["income_budget"]), 2000.0);
    approx(parse_dec(&t["savings_actual"]), 200.0);
    approx(parse_dec(&t["net_actual"]), 1820.0); // 2000 - 180
}

#[tokio::test]
async fn summary_current_month_is_partial() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let url = format!(
        "/v1/transactions/summary?year={}&month={}&avg_months=6",
        today.year(),
        today.month()
    );
    let resp = app.get_with_cookie(&url, &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(resp.json()["is_partial"], true, "mes en curso → parcial");
}

#[tokio::test]
async fn summary_defaults_to_last_complete_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let resp = app.get_with_cookie("/v1/transactions/summary", &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let (y, m) = shift_month(today.year(), today.month(), -1);
    assert_eq!(resp.json()["year"].as_i64().unwrap(), y as i64);
    assert_eq!(resp.json()["month"].as_u64().unwrap(), m as u64);
    assert_eq!(resp.json()["is_partial"], false);
}

#[tokio::test]
async fn summary_avg_months_bounds_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bad = app
        .get_with_cookie("/v1/transactions/summary?year=2026&month=6&avg_months=0", &owner.cookie)
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST);
}
