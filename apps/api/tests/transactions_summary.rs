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
