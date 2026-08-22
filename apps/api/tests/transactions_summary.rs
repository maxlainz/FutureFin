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
    let vivienda_cat = app.create_category(&owner, "expense", "Vivienda").await;
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

    // Pasivo con cuota 500/mes (plan vivo), atribuida a la categoría de gasto «Vivienda» (3.4.0):
    // el lado budget de Vivienda gana 500 sin tocar el resto de líneas.
    let future = date_in(today.year() + 4, 1, 15);
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": vivienda_cat,
                    "label": "Hipoteca", "principal": "100000",
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
    // Promedio PONDERADO: denominador = months_with_data (meses del tramo con datos), no window_months.
    assert_eq!(b["avg_window"], "3");
    assert_eq!(b["window_months"].as_u64().unwrap(), 3, "tramo [sel-3, sel)");
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 2, "sólo sel-1 y sel-3 tienen datos");

    // Línea Super: actual 150, budget 300, avg (60+90)/2=75, deltas -150 / +75.
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["actual"]), 150.0);
    approx(parse_dec(&sup["budget"]), 300.0);
    approx(parse_dec(&sup["avg"]), 75.0);
    approx(parse_dec(&sup["delta_vs_budget"]), -150.0);
    approx(parse_dec(&sup["delta_vs_avg"]), 75.0);

    // Sin categoría: actual 30, budget 0, avg 0.
    let sc = line(&b["expense_categories"], "Sin categoría");
    approx(parse_dec(&sc["actual"]), 30.0);
    approx(parse_dec(&sc["budget"]), 0.0);
    approx(parse_dec(&sc["avg"]), 0.0);

    // Vivienda: la cuota atribuida materializa la fila aunque no tenga movimientos ni partidas
    // (budget = 500 del plan; actual/avg 0 — aún sin recibos vinculados a esa categoría).
    let viv = line(&b["expense_categories"], "Vivienda");
    approx(parse_dec(&viv["actual"]), 0.0);
    approx(parse_dec(&viv["budget"]), 500.0);
    approx(parse_dec(&viv["avg"]), 0.0);
    approx(parse_dec(&viv["delta_vs_budget"]), -500.0);

    // Ingreso Nómina: actual 2000, budget 2000, avg 1000/2=500 (denominador ponderado 2).
    let nom = line(&b["income_categories"], "Nómina");
    approx(parse_dec(&nom["actual"]), 2000.0);
    approx(parse_dec(&nom["budget"]), 2000.0);
    approx(parse_dec(&nom["avg"]), 500.0);
    approx(parse_dec(&nom["delta_vs_budget"]), 0.0);

    // Sin línea derivada SINTÉTICA de cuotas: la key sigue fuera del JSON (la cuota entra
    // atribuida a su categoría de gasto, no como fila aparte sin pareja).
    assert!(b.get("derived_debt_line").is_none(), "derived_debt_line eliminada");

    // Savings block: actual 200, avg 100/2=50.
    approx(parse_dec(&b["savings"]["actual"]), 200.0);
    approx(parse_dec(&b["savings"]["avg"]), 50.0);

    // Income block agregado.
    approx(parse_dec(&b["income"]["actual"]), 2000.0);
    approx(parse_dec(&b["income"]["avg"]), 500.0);

    // Totales. expense_actual = 150+30 = 180 (SIN la cuota del pasivo → sin doble conteo).
    let t = &b["totals"];
    approx(parse_dec(&t["expense_actual"]), 180.0);
    approx(parse_dec(&t["expense_avg"]), 75.0);
    approx(parse_dec(&t["expense_budget"]), 800.0); // 300 Super + 500 cuota atribuida a Vivienda
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

/// Crea un presupuesto para una categoría.
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

/// El denominador del promedio es `months_with_data` (meses del tramo con ≥1 transacción), NO
/// `window_months`: un tramo de 6 meses con datos sólo en 2 divide entre 2.
#[tokio::test]
async fn summary_avg_window_weighted_denominator() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(sy, sm, -1);
    let (y3, m3) = shift_month(sy, sm, -3);

    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Super sel", "-100", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 5), "Super -1", "-40", "expense", Some(&super_cat)).await;
    // sel-2 vacío a propósito.
    manual(&app, &owner.cookie, &date_in(y3, m3, 5), "Super -3", "-80", "expense", Some(&super_cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_window=6");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();

    assert_eq!(b["avg_window"], "6");
    assert_eq!(b["window_months"].as_u64().unwrap(), 6);
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 2, "sólo sel-1 y sel-3 con datos");
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["actual"]), 100.0);
    approx(parse_dec(&sup["avg"]), 60.0); // (40 + 80) / 2
}

/// YTD = enero..mes seleccionado (exclusive). El caso enero deja el tramo vacío → avg 0.
#[tokio::test]
async fn summary_avg_window_ytd() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    budget(&app, &owner.cookie, &super_cat, "200").await;

    let today = server_today(&app, &owner.cookie).await;
    let year = today.year() - 1; // año natural completamente en el pasado.

    manual(&app, &owner.cookie, &date_in(year, 2, 10), "Feb", "-30", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(year, 4, 10), "Abr", "-50", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(year, 6, 10), "Jun", "-120", "expense", Some(&super_cat)).await;

    // Mes seleccionado junio: YTD = ene..may (window_months 5), datos en feb y abr → denom 2.
    let url = format!("/v1/transactions/summary?year={year}&month=6&avg_window=ytd");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["avg_window"], "ytd");
    assert_eq!(b["window_months"].as_u64().unwrap(), 5);
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 2);
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["actual"]), 120.0);
    approx(parse_dec(&sup["avg"]), 40.0); // (30 + 50) / 2

    // Mes seleccionado enero: tramo vacío → months_with_data 0, window_months 0, avg 0.
    let url = format!("/v1/transactions/summary?year={year}&month=1&avg_window=ytd");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["window_months"].as_u64().unwrap(), 0);
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 0);
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["avg"]), 0.0);
}

/// ALL = desde el mes del MIN(op_date) hasta el seleccionado (exclusive). Sin historial → vacío.
#[tokio::test]
async fn summary_avg_window_all() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let (y2, m2) = shift_month(sy, sm, -2);
    let (y5, m5) = shift_month(sy, sm, -5);

    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Super sel", "-100", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(y2, m2, 5), "Super -2", "-40", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(y5, m5, 5), "Super -5", "-60", "expense", Some(&super_cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_window=all");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["avg_window"], "all");
    assert_eq!(b["window_months"].as_u64().unwrap(), 5, "MIN(op_date) en sel-5 → 5 meses");
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 2, "sel-2 y sel-5 con datos");
    let sup = line(&b["expense_categories"], "Super");
    approx(parse_dec(&sup["avg"]), 50.0); // (40 + 60) / 2

    // Sin historial (app nueva) → tramo vacío.
    let app2 = TestApp::spawn().await;
    let owner2 = app2.register_and_login_owner("bob").await;
    let today2 = server_today(&app2, &owner2.cookie).await;
    let (sy2, sm2) = shift_month(today2.year(), today2.month(), -1);
    let url = format!("/v1/transactions/summary?year={sy2}&month={sm2}&avg_window=all");
    let b = app2.get_with_cookie(&url, &owner2.cookie).await.json();
    assert_eq!(b["avg_window"], "all");
    assert_eq!(b["window_months"].as_u64().unwrap(), 0, "sin transacciones → tramo vacío");
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn summary_avg_window_invalid_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bad = app
        .get_with_cookie("/v1/transactions/summary?year=2026&month=6&avg_window=nope", &owner.cookie)
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST);
    assert!(bad.json()["message"].as_str().unwrap().contains("avg_window must be one of"));
}

/// Sin fila derivada SINTÉTICA (la key `derived_debt_line` de la v1.6-1.8 sigue eliminada): la
/// cuota entra atribuida a su categoría de gasto (3.4.0), sumándose al budget de ESA fila —
/// aquí a «Luz», que además tiene partida manual de 120 → budget 620 (el caso «partida propia +
/// cuota en la misma categoría» queda visible en la fila, no silencioso en un total sin pareja).
#[tokio::test]
async fn summary_no_synthetic_derived_line_cuota_lives_in_its_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let luz_cat = app.create_category(&owner, "expense", "Luz").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    budget(&app, &owner.cookie, &luz_cat, "120").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 4, 1, 15);
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": luz_cat,
                    "label": "Hipoteca", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability: {r:?}");

    let b = app.get_with_cookie("/v1/transactions/summary", &owner.cookie).await.json();
    assert!(b.get("derived_debt_line").is_none(), "derived_debt_line eliminada (v1.8.0)");
    let luz = line(&b["expense_categories"], "Luz");
    approx(parse_dec(&luz["budget"]), 620.0); // 120 partida manual + 500 cuota atribuida
    approx(parse_dec(&b["totals"]["expense_budget"]), 620.0);
}

/// Emparejamiento completo (el caso hipoteca real): recibo importado en la categoría X + pasivo
/// atribuido a X → la fila se iguala y el Δ pasa a ser informativo (revisión de tipo, etc.).
/// PREDICCIÓN: Real 512 (recibo), Budget 500 (plan), Δ +12; totales igual de equilibrados.
#[tokio::test]
async fn summary_budget_pairs_categorized_cuota() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let hipoteca_cat = app.create_category(&owner, "expense", "Hipoteca").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 4, 1, 15);
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": hipoteca_cat,
                    "label": "Piso", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability: {r:?}");

    // Recibo real del último mes completo, categorizado en Hipoteca (512 ≠ 500: revisión de tipo).
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 2), "RECIBO PRESTAMO", "-512", "expense", Some(&hipoteca_cat)).await;

    let b = app.get_with_cookie("/v1/transactions/summary", &owner.cookie).await.json();
    let hip = line(&b["expense_categories"], "Hipoteca");
    approx(parse_dec(&hip["actual"]), 512.0);
    approx(parse_dec(&hip["budget"]), 500.0);
    approx(parse_dec(&hip["delta_vs_budget"]), 12.0);
    approx(parse_dec(&b["totals"]["expense_actual"]), 512.0);
    approx(parse_dec(&b["totals"]["expense_budget"]), 500.0);
}

/// La atribución es month-aware: un plan que terminó ANTES del mes seleccionado no inyecta budget
/// ese mes; en un mes en que aún vivía, sí (fin de plan >= primer día del mes seleccionado).
#[tokio::test]
async fn summary_cuota_respects_month_activity() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let hipoteca_cat = app.create_category(&owner, "expense", "Hipoteca").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    // Plan terminado el día 15 del mes -2: activo para el mes -2, inactivo para el mes -1.
    let (y2, m2) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    let end = date_in(y2, m2, 15);
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": hipoteca_cat,
                    "label": "Coche", "principal": "1000",
                    "payment_amount": "200", "payment_frequency": "monthly", "payment_end_date": end }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability: {r:?}");
    // Un movimiento por mes para que ambos meses existan en el selector.
    manual(&app, &owner.cookie, &date_in(y2, m2, 3), "Gasto A", "-10", "expense", Some(&hipoteca_cat)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 3), "Gasto B", "-10", "expense", Some(&hipoteca_cat)).await;

    let b2 = app
        .get_with_cookie(&format!("/v1/transactions/summary?year={y2}&month={m2}"), &owner.cookie)
        .await
        .json();
    approx(parse_dec(&line(&b2["expense_categories"], "Hipoteca")["budget"]), 200.0);

    let b1 = app
        .get_with_cookie(&format!("/v1/transactions/summary?year={y1}&month={m1}"), &owner.cookie)
        .await
        .json();
    approx(parse_dec(&line(&b1["expense_categories"], "Hipoteca")["budget"]), 0.0);
}

/// Pasivo legacy sin categoría de gasto (NULL, anterior a 3.4.0 — la API ya no permite crearlo,
/// se inserta por SQL directo): la comparativa queda EXACTAMENTE como antes (status quo).
#[tokio::test]
async fn summary_null_expense_category_is_status_quo() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let luz_cat = app.create_category(&owner, "expense", "Luz").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    budget(&app, &owner.cookie, &luz_cat, "120").await;

    // INSERT directo (bypass de la obligatoriedad del create, como una fila pre-migración).
    let liab_cat_id = uuid::Uuid::parse_str(&liab_cat).unwrap();
    let iid: uuid::Uuid =
        sqlx::query_scalar("SELECT installation_id FROM categories WHERE id = $1")
            .bind(liab_cat_id)
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    sqlx::query(
        r#"INSERT INTO liabilities (installation_id, category_id, label, principal,
               payment_amount, payment_frequency, principal_derived_from_plan)
           VALUES ($1, $2, 'Legacy', 50000, 400, 'monthly', false)"#,
    )
    .bind(iid)
    .bind(liab_cat_id)
    .execute(&app.state.pool)
    .await
    .unwrap();

    let b = app.get_with_cookie("/v1/transactions/summary", &owner.cookie).await.json();
    approx(parse_dec(&b["totals"]["expense_budget"]), 120.0); // sin atribución → solo Luz
}

// ---------------------------------------------------------------------------
// Conciliación de transferencias (3.5.0): las conciliadas no son gasto ni ingreso
// ---------------------------------------------------------------------------

/// Un par conciliado (−500/+500 a 2 días) desaparece de los totales del mes; al desconciliarlo
/// vuelve. Predicho: gasto 800 → 1300, ingreso 2000 → 2500.
#[tokio::test]
async fn reconciled_excluded_from_month_totals() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    manual(&app, &owner.cookie, &date_in(sy, sm, 1), "Sueldo", "2000", "income", None).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Alquiler", "-800", "expense", None).await;
    // Par de transferencia: salida −500 (día 10) + entrada +500 (día 12) → auto-conciliado.
    manual(&app, &owner.cookie, &date_in(sy, sm, 10), "Traspaso salida", "-500", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 12), "Traspaso entrada", "500", "income", None).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_window=3");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    approx(parse_dec(&b["totals"]["expense_actual"]), 800.0); // el −500 conciliado NO cuenta
    approx(parse_dec(&b["totals"]["income_actual"]), 2000.0); // el +500 conciliado NO cuenta
    approx(parse_dec(&b["totals"]["net_actual"]), 1200.0);

    // Desconciliar → ambas patas vuelven a los totales.
    let list = app
        .get_with_cookie(&format!("/v1/transactions?month={sy}-{sm:02}"), &owner.cookie)
        .await
        .json();
    let out_leg = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["concept"] == "Traspaso salida")
        .unwrap()
        .clone();
    assert!(out_leg["transfer_counterpart_id"].is_string(), "precondición: conciliada");
    let u = app
        .delete_with_cookie(
            &format!("/v1/transactions/{}/reconcile", out_leg["id"].as_str().unwrap()),
            &owner.cookie,
        )
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");

    let b2 = app.get_with_cookie(&url, &owner.cookie).await.json();
    approx(parse_dec(&b2["totals"]["expense_actual"]), 1300.0);
    approx(parse_dec(&b2["totals"]["income_actual"]), 2500.0);
    approx(parse_dec(&b2["totals"]["net_actual"]), 1200.0); // el neto no cambia: el par suma cero
}

/// Un mes cuyo único contenido es un par conciliado NO cuenta en `months_with_data` (misma lógica
/// que los meses pseudovacíos): denominador 1 (solo sel−2), avg de Super = 200/1 = 200.
#[tokio::test]
async fn reconciled_only_month_not_counted_in_months_with_data() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(sy, sm, -1);
    let (y2, m2) = shift_month(sy, sm, -2);

    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Super mes sel", "-100", "expense", Some(&super_cat)).await;
    // sel−1: SOLO un par conciliado.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Salida", "-300", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Entrada", "300", "income", None).await;
    // sel−2: gasto real.
    manual(&app, &owner.cookie, &date_in(y2, m2, 8), "Super real", "-200", "expense", Some(&super_cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_window=3");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["months_with_data"].as_u64(), Some(1), "el mes solo-conciliadas no cuenta: {b:?}");
    approx(parse_dec(&line(&b["expense_categories"], "Super")["avg"]), 200.0);
}

/// La serie mensual por categoría excluye las conciliadas: Super real −100 + pata −40 conciliada
/// (con categoría) → el punto del mes vale 100, no 140.
#[tokio::test]
async fn reconciled_excluded_from_category_series() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    let today = server_today(&app, &owner.cookie).await;
    let d1 = date_in(today.year(), today.month(), 1);

    manual(&app, &owner.cookie, &d1, "Super real", "-100", "expense", Some(&super_cat)).await;
    // Par conciliado cuya pata de salida lleva categoría Super (la categoría se conserva pero no cuenta).
    manual(&app, &owner.cookie, &d1, "Traspaso salida", "-40", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &d1, "Traspaso entrada", "40", "income", None).await;

    let resp = app
        .get_with_cookie("/v1/transactions/category-series?kind=expense&window_months=1", &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "series: {resp:?}");
    let b = resp.json();
    let series = b["series"].as_array().unwrap();
    let super_entry = series
        .iter()
        .find(|s| s["category_name"] == "Super")
        .unwrap_or_else(|| panic!("no Super series: {b:?}"));
    let months = super_entry["months"].as_array().unwrap();
    approx(parse_dec(&months.last().unwrap()["total"]), 100.0);
}

/// Crea una transacción RECURRENTE (queda con `recurring_rule_id` no nulo).
async fn recurring(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
    cat: Option<&str>,
) {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind,
                           "recurrence": {} });
    if let Some(c) = cat {
        body["category_id"] = json!(c);
    }
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "recurring {concept}: {r:?}");
}

/// El auditoría MCP, reproducido: un mes cuyo único contenido son instancias recurrentes NO promedia.
///
/// Queda fuera del numerador Y del denominador, que es la única combinación coherente: excluirlo
/// solo del denominador dejaría su importe arriba y dispararía las categorías presentes en él
/// (el alquiler recurrente saldría a 1,5× su cuota real en vez de a su cuota).
#[tokio::test]
async fn recurring_only_month_excluded_from_avg_numerator_and_denominator() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let comer_cat = app.create_category(&owner, "expense", "Comer Fuera").await;
    let alq_cat = app.create_category(&owner, "expense", "Alquiler").await;

    let today = server_today(&app, &owner.cookie).await;
    // Mes seleccionado = hoy − 1 (completo). Ventana avg_months=3 = sel-1, sel-2, sel-3.
    let (sy, sm) = shift_month(today.year(), today.month(), -1);
    let (y1, m1) = shift_month(sy, sm, -1);
    let (y2, m2) = shift_month(sy, sm, -2);
    let (y3, m3) = shift_month(sy, sm, -3);

    // sel-1 y sel-2: meses REALES (gasto manual en Comer Fuera).
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Bar", "-200", "expense", Some(&comer_cat)).await;
    manual(&app, &owner.cookie, &date_in(y2, m2, 10), "Bar", "-220", "expense", Some(&comer_cat)).await;
    // UNA regla de alquiler con origen en sel-3, que materializa 860 en sel-3, sel-2 y sel-1.
    // sel-3 queda como mes SOLO-recurrente: es el que hasta ahora hundía todas las medias.
    recurring(&app, &owner.cookie, &date_in(y3, m3, 1), "Alquiler", "-860", "expense", Some(&alq_cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_months=3");
    let resp = app.get_with_cookie(&url, &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "summary: {resp:?}");
    let b = resp.json();

    // Las dos cifras se publican y son DISTINTAS: 3 meses tienen algo, solo 2 promedian.
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 3, "los 3 meses tienen movimientos");
    assert_eq!(b["avg_months"].as_u64().unwrap(), 2, "solo sel-1 y sel-2 son reales");

    // Comer Fuera: (200+220)/2 = 210. Con el denominador viejo habría salido 420/3 = 140.
    let comer = line(&b["expense_categories"], "Comer Fuera");
    approx(parse_dec(&comer["avg"]), 210.0);

    // Alquiler: 1720/2 = 860 (su cuota real). Si el mes solo-recurrente siguiera en el numerador
    // pero no en el denominador, saldría 2580/2 = 1290.
    let alq = line(&b["expense_categories"], "Alquiler");
    approx(parse_dec(&alq["avg"]), 860.0);

    // Aditividad: Σ de las líneas == el total. Es lo que se perdería con un denominador por categoría.
    approx(parse_dec(&b["totals"]["expense_avg"]), 1070.0);

    // Base del promedio: sel-2 → sel-1, contiguos.
    let basis = &b["avg_basis"];
    assert_eq!(basis["months"].as_u64().unwrap(), 2);
    assert_eq!(basis["first_month"], format!("{y2:04}-{m2:02}"));
    assert_eq!(basis["last_month"], format!("{y1:04}-{m1:02}"));
    assert_eq!(basis["has_gaps"], false);
    assert!(b.get("avg_unavailable_reason").is_none(), "sí hay promedio");
}

/// Un mes real cuenta ENTERO, recurrentes incluidos: lo que decide es si el mes tiene algún
/// movimiento real, no de qué tipo es cada importe.
#[tokio::test]
async fn real_month_counts_its_recurring_amounts_too() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Super").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -1);
    let (y1, m1) = shift_month(sy, sm, -1);

    // Un único mes en ventana, real, con un movimiento manual y otro recurrente en la MISMA categoría.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Compra", "-100", "expense", Some(&cat)).await;
    recurring(&app, &owner.cookie, &date_in(y1, m1, 1), "Cesta", "-40", "expense", Some(&cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_months=3");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();

    assert_eq!(b["avg_months"].as_u64().unwrap(), 1);
    approx(parse_dec(&line(&b["expense_categories"], "Super")["avg"]), 140.0);
}

/// Meses reales no contiguos → `has_gaps`, para que la UI no etiquete «abr–jun» una media de abr y jun.
#[tokio::test]
async fn avg_basis_reports_gaps_between_real_months() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Super").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -1);
    let (y1, m1) = shift_month(sy, sm, -1);
    let (y3, m3) = shift_month(sy, sm, -3);

    // sel-1 y sel-3 reales; sel-2 completamente vacío.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "A", "-100", "expense", Some(&cat)).await;
    manual(&app, &owner.cookie, &date_in(y3, m3, 10), "B", "-200", "expense", Some(&cat)).await;

    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_months=3");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();

    assert_eq!(b["avg_months"].as_u64().unwrap(), 2);
    assert_eq!(b["avg_basis"]["has_gaps"], true, "sel-1 y sel-3 no son consecutivos");
    approx(parse_dec(&line(&b["expense_categories"], "Super")["avg"]), 150.0);
}

/// Sin meses reales no hay promedio, y la respuesta dice POR QUÉ: «solo recurrentes» y «ventana
/// vacía» piden acciones distintas (bajar la ventana vs importar histórico).
#[tokio::test]
async fn window_without_real_months_reports_no_avg_and_why() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Alquiler").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -1);
    let (y1, m1) = shift_month(sy, sm, -1);

    // Ventana completamente vacía.
    let url = format!("/v1/transactions/summary?year={sy}&month={sm}&avg_months=3");
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 0);
    assert_eq!(b["avg_months"].as_u64().unwrap(), 0);
    assert!(b.get("avg_basis").is_none(), "sin promedio no hay base");
    assert_eq!(b["avg_unavailable_reason"], "empty_window");

    // Ahora la ventana tiene movimientos, pero TODOS recurrentes.
    recurring(&app, &owner.cookie, &date_in(y1, m1, 1), "Alquiler", "-860", "expense", Some(&cat)).await;
    let b = app.get_with_cookie(&url, &owner.cookie).await.json();
    assert_eq!(b["months_with_data"].as_u64().unwrap(), 1, "hay movimientos…");
    assert_eq!(b["avg_months"].as_u64().unwrap(), 0, "…pero ninguno real");
    assert!(b.get("avg_basis").is_none());
    assert_eq!(b["avg_unavailable_reason"], "only_recurring_months");
    // Medias a 0, nunca a un número inventado.
    approx(parse_dec(&b["totals"]["expense_avg"]), 0.0);
}
