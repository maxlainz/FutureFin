//! `/v1/summary` sigue el toggle `savings_source` (Fase 5):
//! - Modo A (`budget`): KPIs desde el presupuesto; `savings_source == "budget"`, months 0.
//! - Modo B (`transactions_avg`) con datos: income/expense_regular/net/savings_rate desde el promedio
//!   real 12m CRUDO (reforma 3.4.0: las cuotas viven dentro del gasto; sin resta híbrida ni debt
//!   service re-sumado); campos nuevos correctos.
//! - Modo B sin datos: fallback silencioso al presupuesto (números = modo A, `budget`, months 0).
//! - Scoping household vs mine.

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
    assert!((a - b).abs() < 0.5, "expected ~{b}, got {a}");
}

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    (
        (zero.div_euclid(12)) as i32,
        (zero.rem_euclid(12) + 1) as u32,
    )
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

#[allow(clippy::too_many_arguments)]
async fn manual(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
    linked_liability: Option<&str>,
) {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind });
    if let Some(l) = linked_liability {
        body["linked_liability_id"] = json!(l);
    }
    let r = app
        .post_json_with_cookie("/v1/transactions", body, cookie)
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::CREATED,
        "manual {concept}: {r:?}"
    );
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

async fn set_mode_b(app: &TestApp, cookie: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode B: {r:?}");
}

async fn set_mode_c(app: &TestApp, cookie: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "budget_income_real_expense" } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode C: {r:?}");
}

/// Alta con recurrencia (nómina/gasto recurrente): el origen + su backfill quedan con
/// `recurring_rule_id NOT NULL` → «pseudovacíos» a efectos del promedio.
async fn recurring(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
) {
    let body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind,
                       "recurrence": {} });
    let r = app
        .post_json_with_cookie("/v1/transactions", body, cookie)
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::CREATED,
        "recurring {concept}: {r:?}"
    );
}

/// `financial_health` del summary para una query dada.
async fn health(app: &TestApp, cookie: &str, query: &str) -> Value {
    let resp = app.get_with_cookie(query, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "summary: {resp:?}");
    resp.json()["financial_health"].clone()
}

// ---------------------------------------------------------------------------
// Modo A — presupuesto
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_a_summary_is_budget_based() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    // Transacciones presentes: en modo A deben IGNORARSE por completo.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 10),
        "Sueldo",
        "9999",
        "income",
        None,
    )
    .await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 5000.0);
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 3000.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 2000.0);
    approx(parse_dec(&h["savings_rate"]), 0.4); // 2000/5000
    assert_eq!(h["savings_source"], "budget");
    assert_eq!(h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Modo B con datos — promedio real crudo (reforma 3.4.0: sin resta híbrida)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_summary_uses_raw_avg() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    // Presupuesto MUY distinto para probar que el modo B lo ignora.
    budget(&app, &owner.cookie, &income_cat, "9000").await;
    budget(&app, &owner.cookie, &expense_cat, "8000").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);

    // L1 activa, nominal 500 pero con txn vinculada (avg real 400 → gana el real).
    let l1 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat, "label": "L1", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l1.status, http::StatusCode::CREATED, "{l1:?}");
    let l1_id = l1.json()["id"].as_str().unwrap().to_string();

    // L2 activa, nominal 300, sin txn vinculada → cuota nominal.
    let l2 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat, "label": "L2", "principal": "100000",
                    "payment_amount": "300", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l2.status, http::StatusCode::CREATED, "{l2:?}");

    // Único mes con datos (el último completo): income 3000; expense total 1500 (400 → L1).
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 10),
        "Sueldo",
        "3000",
        "income",
        None,
    )
    .await;
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 11),
        "Cuota L1",
        "-400",
        "expense",
        Some(&l1_id),
    )
    .await;
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 12),
        "Resto",
        "-1100",
        "expense",
        None,
    )
    .await;

    set_mode_b(&app, &owner.cookie).await;

    // Reforma 3.4.0: promedio CRUDO — las cuotas (vinculadas o no) ya viven dentro del gasto real,
    // así que ni se restan ni se re-suman como debt service. expense_reg = expense_avg = 1500,
    // expense_der = 0, expense_tot = 1500; net = 3000 − 1500 = 1500; savings_rate = 0.5.
    // (Antes: resta híbrida 700 + debt service 800 → expense_reg 800, net 1400.)
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 3000.0);
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 1500.0);
    approx(parse_dec(&h["expense_total_monthly_equivalent"]), 1500.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 1500.0);
    approx(parse_dec(&h["savings_rate"]), 0.5);
    assert_eq!(h["savings_source"], "transactions_avg");
    assert_eq!(h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Modo B sin datos — fallback al presupuesto
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_summary_zero_months_falls_back_to_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    // Solo una transacción en el mes en curso (parcial) → fuera de ventana → months_with_data = 0.
    let today = server_today(&app, &owner.cookie).await;
    manual(
        &app,
        &owner.cookie,
        &date_in(today.year(), today.month(), 5),
        "Hoy",
        "9999",
        "income",
        None,
    )
    .await;

    set_mode_b(&app, &owner.cookie).await;

    // Fallback: idéntico a modo A. net = 5000 − 2000 = 3000.
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 5000.0);
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 2000.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 3000.0);
    assert_eq!(
        h["savings_source"], "budget",
        "sin datos → fuente efectiva = budget"
    );
    assert_eq!(h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Scoping household vs mine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_summary_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 10),
        "Owner in",
        "2000",
        "income",
        None,
    )
    .await;
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 11),
        "Owner out",
        "-800",
        "expense",
        None,
    )
    .await;
    manual(
        &app,
        &member.cookie,
        &date_in(y1, m1, 12),
        "Member in",
        "1000",
        "income",
        None,
    )
    .await;
    manual(
        &app,
        &member.cookie,
        &date_in(y1, m1, 13),
        "Member out",
        "-400",
        "expense",
        None,
    )
    .await;

    set_mode_b(&app, &owner.cookie).await;

    // household: income 3000, expense 1200 → net 1800.
    let hh = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&hh["income_monthly_equivalent"]), 3000.0);
    approx(parse_dec(&hh["expense_regular_monthly_equivalent"]), 1200.0);
    approx(parse_dec(&hh["net_monthly_equivalent"]), 1800.0);
    assert_eq!(hh["savings_source"], "transactions_avg");

    // mine (owner): income 2000, expense 800 → net 1200.
    let mine = health(&app, &owner.cookie, "/v1/summary?view=mine").await;
    approx(parse_dec(&mine["income_monthly_equivalent"]), 2000.0);
    approx(
        parse_dec(&mine["expense_regular_monthly_equivalent"]),
        800.0,
    );
    approx(parse_dec(&mine["net_monthly_equivalent"]), 1200.0);
}

// ---------------------------------------------------------------------------
// Bloque 1 — ponderación sin meses pseudovacíos (espejo del summary)
// ---------------------------------------------------------------------------

/// El `/v1/summary` en modo B también excluye los meses solo-recurrentes: 1 mes real (income manual
/// 2000 en M-2) + 1 mes solo-recurrente (nómina 3000 en M-1) → `savings_source_months_with_data = 1`,
/// income = 2000 (no 2500).
#[tokio::test]
async fn mode_b_summary_pseudo_empty_month_excluded() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    // Presupuesto muy distinto para probar que el modo B (con datos) lo ignora.
    budget(&app, &owner.cookie, &income_cat, "8000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y2, m2) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(today.year(), today.month(), -1);

    // Mes real M-2 (income manual 2000) + mes solo-recurrente M-1 (nómina recurrente 3000).
    manual(
        &app,
        &owner.cookie,
        &date_in(y2, m2, 10),
        "Sueldo",
        "2000",
        "income",
        None,
    )
    .await;
    recurring(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 1),
        "Nomina rec",
        "3000",
        "income",
    )
    .await;

    set_mode_b(&app, &owner.cookie).await;
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 2000.0);
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 0.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 2000.0);
    assert_eq!(h["savings_source"], "transactions_avg");
    assert_eq!(
        h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(),
        1,
        "el mes solo-recurrente no cuenta"
    );
}

// ---------------------------------------------------------------------------
// Bloque 2 — modo C: income NO se sobreescribe (presupuesto), gasto real
// ---------------------------------------------------------------------------

/// Modo C (`budget_income_real_expense`): el income del summary es el del PRESUPUESTO (no el de las
/// transacciones) y el gasto es el promedio real CRUDO (reforma 3.4.0 — las cuotas viven dentro).
/// Budget income 5000; real M-1: income 3000 (ignorado), expense 1500 (400 vinculados a L1, da
/// igual) → net = 5000 − 1500 = 3500. (Antes: resta híbrida + debt service → 3400.)
#[tokio::test]
async fn mode_c_income_not_overwritten() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);

    let l1 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat, "label": "L1", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l1.status, http::StatusCode::CREATED, "{l1:?}");
    let l1_id = l1.json()["id"].as_str().unwrap().to_string();

    let l2 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat, "label": "L2", "principal": "100000",
                    "payment_amount": "300", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l2.status, http::StatusCode::CREATED, "{l2:?}");

    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 10),
        "Sueldo",
        "3000",
        "income",
        None,
    )
    .await;
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 11),
        "Cuota L1",
        "-400",
        "expense",
        Some(&l1_id),
    )
    .await;
    manual(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 12),
        "Resto",
        "-1100",
        "expense",
        None,
    )
    .await;

    set_mode_c(&app, &owner.cookie).await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 5000.0); // presupuesto, NO las txns (3000)
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 1500.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 3500.0);
    assert_eq!(h["savings_source"], "budget_income_real_expense");
    assert_eq!(h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// KPI «ahorro real vs esperado» — campos servidos en los TRES modos
// ---------------------------------------------------------------------------

/// Modo B: `net_monthly_equivalent` pasa a base real, pero el «esperado» sigue siendo el neto del
/// PRESUPUESTO (captura pre-override). Misma data que el test de modo A: real 750, esperado 2000.
#[tokio::test]
async fn mode_b_expected_is_budget_net_not_override() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y2, m2) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y2, m2, 10), "S1", "3000", "income", None).await;
    manual(&app, &owner.cookie, &date_in(y2, m2, 11), "G1", "-2000", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "S2", "3000", "income", None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "G2", "-2500", "expense", None).await;

    set_mode_b(&app, &owner.cookie).await;

    // Sin pasivos: net del modo B = 3000 − 2250 = 750 ≠ esperado 2000.
    // `savings_expected_monthly_equivalent` es el ÚNICO campo que sobrevive de la comparativa
    // (3.10.0 retiró `savings_actual_*`): alimenta el delta «vs plan» de la tarjeta de ahorro, y
    // se captura ANTES del override, así que no sigue al modo.
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["net_monthly_equivalent"]), 750.0);
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 2000.0);
    assert_eq!(
        h["savings_income_basis"]["months_with_data"].as_u64().unwrap(),
        2,
        "ambos lados promedian en modo B"
    );
    assert_eq!(
        h["savings_expense_basis"]["months_with_data"].as_u64().unwrap(),
        2
    );
}
