//! `/v1/summary` sigue el toggle `savings_source` (Fase 5):
//! - Modo A (`budget`): KPIs desde el presupuesto; `savings_source == "budget"`, months 0.
//! - Modo B (`transactions_avg`) con datos: income/expense_regular/net/savings_rate desde el promedio
//!   real 12m con resta híbrida de cuotas; campos nuevos correctos.
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
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Modo B con datos — promedio real + resta híbrida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_summary_uses_avg_with_hybrid_subtraction() {
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
            json!({ "category_id": liab_cat, "label": "L1", "principal": "100000",
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
            json!({ "category_id": liab_cat, "label": "L2", "principal": "100000",
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

    // resta híbrida = 400 (L1 real) + 300 (L2 nominal) = 700; expense_eff = 1500 − 700 = 800.
    // debt_service nominal = 500 (L1) + 300 (L2) = 800; el `net` resta las cuotas para casar con el
    // modo A (que las incluye) y con la pendiente del chart: net = 3000 − 800 − 800 = 1400.
    // savings_rate = 1400/3000 ≈ 0.4667.
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 3000.0);
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 800.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 1400.0);
    approx(parse_dec(&h["savings_rate"]), 1400.0 / 3000.0);
    assert_eq!(h["savings_source"], "transactions_avg");
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 1);
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
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 0);
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
        h["savings_source_months_with_data"].as_u64().unwrap(),
        1,
        "el mes solo-recurrente no cuenta"
    );
}

// ---------------------------------------------------------------------------
// Bloque 2 — modo C: income NO se sobreescribe (presupuesto), gasto real
// ---------------------------------------------------------------------------

/// Modo C (`budget_income_real_expense`): el income del summary es el del PRESUPUESTO (no el de las
/// transacciones), el gasto es el real (expense_eff con resta híbrida) y el net = income_presupuesto −
/// expense_eff − debt_service. Budget income 5000; real M-1: income 3000 (ignorado), expense 1500
/// (400 → L1); L1 nominal 500 (avg real 400), L2 nominal 300 → expense_eff = 1500 − 700 = 800,
/// debt_service = 800, net = 5000 − 800 − 800 = 3400.
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
            json!({ "category_id": liab_cat, "label": "L1", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l1.status, http::StatusCode::CREATED, "{l1:?}");
    let l1_id = l1.json()["id"].as_str().unwrap().to_string();

    let l2 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "L2", "principal": "100000",
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
    approx(parse_dec(&h["expense_regular_monthly_equivalent"]), 800.0);
    approx(parse_dec(&h["net_monthly_equivalent"]), 3400.0);
    assert_eq!(h["savings_source"], "budget_income_real_expense");
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// KPI «ahorro real vs esperado» — campos servidos en los TRES modos
// ---------------------------------------------------------------------------

/// Modo A con transacciones: el numerador «real» existe aunque el modo ignore las transacciones
/// para el resto de KPIs. Presupuesto 5000/3000 → esperado 2000 (== net del modo A). Dos meses
/// cerrados: M-2 (+3000/−2000) y M-1 (+3000/−2500) → real = (6000 − 4500) / 2 = 750.
#[tokio::test]
async fn mode_a_real_vs_expected_present_with_transactions() {
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

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 2000.0);
    approx(
        parse_dec(&h["savings_expected_monthly_equivalent"]),
        parse_dec(&h["net_monthly_equivalent"]),
    );
    approx(parse_dec(&h["savings_actual_monthly_avg_12m"]), 750.0);
    assert_eq!(h["savings_actual_months_with_data"].as_u64().unwrap(), 2);
    // El contrato del modo A no se contamina: fuente efectiva budget con months 0.
    assert_eq!(h["savings_source"], "budget");
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 0);
}

/// Sin transacciones: el «real» va AUSENTE del JSON (no "0", que significaría «ahorras 0 €»).
#[tokio::test]
async fn real_vs_expected_absent_without_transactions() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "4000").await;
    budget(&app, &owner.cookie, &expense_cat, "3600").await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 400.0);
    assert!(
        h.get("savings_actual_monthly_avg_12m").is_none(),
        "sin meses con datos el campo debe omitirse: {h:?}"
    );
    assert_eq!(h["savings_actual_months_with_data"].as_u64().unwrap(), 0);
}

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
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["net_monthly_equivalent"]), 750.0);
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 2000.0);
    approx(parse_dec(&h["savings_actual_monthly_avg_12m"]), 750.0);
    assert_eq!(h["savings_actual_months_with_data"].as_u64().unwrap(), 2);
}

/// Modo C: el «real» del KPI es idéntico al de los modos A/B con la misma data — es bruto y no
/// sigue el modo (ni la resta híbrida ni el income de presupuesto lo tocan).
#[tokio::test]
async fn mode_c_actual_is_raw() {
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

    set_mode_c(&app, &owner.cookie).await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["income_monthly_equivalent"]), 5000.0); // income de presupuesto (modo C)
    approx(parse_dec(&h["savings_actual_monthly_avg_12m"]), 750.0); // bruto, idéntico a A/B
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 2000.0);
    assert_eq!(h["savings_actual_months_with_data"].as_u64().unwrap(), 2);
}

/// Ambos lados del KPI siguen el scope: household suma presupuesto y transacciones de todos;
/// `?view=mine` solo los del solicitante.
#[tokio::test]
async fn real_vs_expected_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    budget(&app, &owner.cookie, &income_cat, "3000").await;
    budget(&app, &member.cookie, &income_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "O in", "2000", "income", None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "O out", "-800", "expense", None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 12), "M in", "1000", "income", None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 13), "M out", "-400", "expense", None).await;

    // household: esperado 4000 (3000+1000); real (3000 − 1200) / 1 = 1800.
    let hh = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&hh["savings_expected_monthly_equivalent"]), 4000.0);
    approx(parse_dec(&hh["savings_actual_monthly_avg_12m"]), 1800.0);

    // mine (owner): esperado 3000; real (2000 − 800) / 1 = 1200.
    let mine = health(&app, &owner.cookie, "/v1/summary?view=mine").await;
    approx(parse_dec(&mine["savings_expected_monthly_equivalent"]), 3000.0);
    approx(parse_dec(&mine["savings_actual_monthly_avg_12m"]), 1200.0);
}

/// El «real» hereda el contrato de meses «reales»: un mes solo-recurrente no cuenta (ni numerador
/// ni denominador), un mes con manual sí.
#[tokio::test]
async fn real_vs_expected_pseudo_empty_month_excluded() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    budget(&app, &owner.cookie, &income_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y2, m2) = shift_month(today.year(), today.month(), -2);
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y2, m2, 10), "Sueldo", "2000", "income", None).await;
    recurring(
        &app,
        &owner.cookie,
        &date_in(y1, m1, 1),
        "Nomina rec",
        "3000",
        "income",
    )
    .await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    assert_eq!(
        h["savings_actual_months_with_data"].as_u64().unwrap(),
        1,
        "el mes solo-recurrente no cuenta"
    );
    approx(parse_dec(&h["savings_actual_monthly_avg_12m"]), 2000.0);
}

/// El mes en curso (parcial) queda fuera de la ventana del «real».
#[tokio::test]
async fn real_vs_expected_ignores_current_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    budget(&app, &owner.cookie, &income_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    manual(
        &app,
        &owner.cookie,
        &date_in(today.year(), today.month(), 1),
        "Hoy",
        "9999",
        "income",
        None,
    )
    .await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    approx(parse_dec(&h["savings_expected_monthly_equivalent"]), 1000.0);
    assert!(
        h.get("savings_actual_monthly_avg_12m").is_none(),
        "el mes en curso no alimenta el promedio: {h:?}"
    );
    assert_eq!(h["savings_actual_months_with_data"].as_u64().unwrap(), 0);
}
