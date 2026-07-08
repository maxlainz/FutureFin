//! Toggle `savings_source` (presupuesto vs promedio real 12m) — Fases 1-3.
//!
//! Números PREDICHOS antes de ejecutar (ver comentarios por test). El "hoy" se deriva del servidor
//! (`/v1/history/series` anchor) para no depender del reloj de la máquina. Todas las lecturas de
//! proyección usan `?months=240` para saltarse la cache (el hot-path solo cachea cuando `months`
//! está ausente) y así ver siempre el efecto del modo/settings vigentes sin condiciones de carrera.

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

#[allow(clippy::too_many_arguments)]
async fn manual(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
    cat: Option<&str>,
    linked_liability: Option<&str>,
) {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind });
    if let Some(c) = cat {
        body["category_id"] = json!(c);
    }
    if let Some(l) = linked_liability {
        body["linked_liability_id"] = json!(l);
    }
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "manual {concept}: {r:?}");
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

/// PATCH mode B (mínimo: solo `savings_source`; el resto de `FireSettings` cae al default de struct).
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

async fn projection_delta(app: &TestApp, cookie: &str, query: &str) -> f64 {
    let resp = app.get_with_cookie(query, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "projection: {resp:?}");
    parse_dec(&resp.json()["monthly_delta_assumption"])
}

/// `contribution_nominal_monthly` del primer (único) activo de GET /v1/assets.
async fn asset_contribution(app: &TestApp, cookie: &str) -> f64 {
    let resp = app.get_with_cookie("/v1/assets", cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "assets: {resp:?}");
    let body = resp.json();
    let first = body.as_array().and_then(|a| a.first()).expect("un activo");
    parse_dec(&first["contribution_nominal_monthly"])
}

// ---------------------------------------------------------------------------
// Fase 1 — serde / PATCH
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fire_settings_defaults_to_budget_when_absent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let resp = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(
        resp.json()["installation"]["fire_settings"]["savings_source"], "budget",
        "instalación nueva → savings_source ausente en JSON → budget"
    );
}

#[tokio::test]
async fn patch_savings_source_transactions_avg_roundtrips() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    assert_eq!(
        patched.json()["installation"]["fire_settings"]["savings_source"],
        "transactions_avg"
    );

    // Round-trip: GET fresco lo persiste.
    let got = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(got.json()["installation"]["fire_settings"]["savings_source"], "transactions_avg");
}

#[tokio::test]
async fn patch_savings_source_unknown_returns_422() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "nope" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        resp.status,
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "valor desconocido debe ser 422, recibido {}",
        resp.status
    );
    let body_text = String::from_utf8_lossy(&resp.body);
    assert!(
        body_text.contains("unknown variant") && body_text.contains("nope"),
        "cuerpo debe nombrar la variante desconocida: {body_text}"
    );
}

// ---------------------------------------------------------------------------
// Fase 2/3 — helper de promedio vía la proyección (monthly_delta_assumption)
// ---------------------------------------------------------------------------

/// Ponderación (hueco no diluye), exclusión de `savings`, ventana [−12m, mes actual): el mes
/// parcial y el mes −13 quedan fuera. Y el modo B se impone sobre el presupuesto.
#[tokio::test]
async fn mode_b_weighted_avg_excludes_savings_and_partial_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    // Presupuesto bien distinto → modo A = 5000 − 3000 = 2000.
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1); // último mes completo (en ventana)
    let (y6, m6) = shift_month(today.year(), today.month(), -6);
    let (y13, m13) = shift_month(today.year(), today.month(), -13); // fuera de ventana (antes)

    // Datos solo en −1 y −6 → months_with_data = 2 (los huecos intermedios no diluyen).
    manual(&app, &owner.cookie, &date_in(y1, m1, 15), "Sueldo", "2400", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 16), "Compra", "-900", "expense", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 17), "Aporte", "-500", "savings", None, None).await;
    manual(&app, &owner.cookie, &date_in(y6, m6, 15), "Sueldo", "1200", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y6, m6, 16), "Compra", "-300", "expense", None, None).await;
    // Ruido excluido: mes actual (parcial) y mes −13 (antes de la ventana).
    manual(&app, &owner.cookie, &date_in(today.year(), today.month(), 5), "Hoy", "9999", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y13, m13, 15), "Viejo", "8888", "income", None, None).await;

    // Modo A (default) = presupuesto.
    let delta_a = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_a, 2000.0);

    // Modo B: income_avg = (2400+1200)/2 = 1800; expense_avg = (900+300)/2 = 600 → delta 1200.
    set_mode_b(&app, &owner.cookie).await;
    let delta_b = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_b, 1200.0);

    assert!((delta_a - delta_b).abs() > 100.0, "modo B debe cambiar la pendiente frente a A");
}

/// `months_with_data == 0` → fallback silencioso al presupuesto (modo A efectivo).
#[tokio::test]
async fn mode_b_zero_months_falls_back_to_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    // Única transacción en el mes en curso (parcial) → fuera de ventana → months_with_data = 0.
    let today = server_today(&app, &owner.cookie).await;
    manual(&app, &owner.cookie, &date_in(today.year(), today.month(), 5), "Hoy", "9999", "income", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 3000.0); // presupuesto 5000 − 2000, NO tocado por la txn del mes parcial
}

/// Resta híbrida de cuotas: liability con txns vinculadas usa su avg real; sin vincular usa la
/// cuota nominal; terminada (payment_end pasado) NO se resta. Clamp implícito no aplica aquí.
#[tokio::test]
async fn mode_b_hybrid_liability_subtraction() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);
    let past = date_in(today.year() - 1, 6, 15);

    // L1 activa, cuota nominal 500 (pero tiene txns vinculadas → se usa el avg real 450).
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

    // L2 activa, sin txns vinculadas → se usa la cuota nominal 300.
    let l2 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "L2", "principal": "100000",
                    "payment_amount": "300", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l2.status, http::StatusCode::CREATED, "{l2:?}");

    // L3 terminada (payment_end pasado) → NO se resta aunque tenga cuota 700.
    let l3 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "L3", "principal": "100000",
                    "payment_amount": "700", "payment_frequency": "monthly", "payment_end_date": past }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l3.status, http::StatusCode::CREATED, "{l3:?}");

    // Transacciones en el último mes completo: income 6000; expense total 2000 (de los cuales 450
    // vinculados a L1) → income_avg 6000, expense_avg 2000, per_liability[L1] = 450.
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "6000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Cuota L1", "-450", "expense", None, Some(&l1_id)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 12), "Resto", "-1550", "expense", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    // resta = 450 (L1 real) + 300 (L2 nominal) = 750; expense_eff = 2000 − 750 = 1250.
    // delta = 6000 − 1250 = 4750. (Si L1 usara nominal 500 → 4800; si restara L3 → 4050.)
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 4750.0);
}

/// `expense_eff` se clampa a ≥ 0: aunque las cuotas nominales superen el gasto real medido.
#[tokio::test]
async fn mode_b_clamps_expense_eff_non_negative() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);
    // Cuota nominal 1000, sin gasto real medido.
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "label": "L", "principal": "100000",
                    "payment_amount": "1000", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // Solo income → expense_avg = 0; months_with_data = 1.
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "2000", "income", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    // expense_eff = max(0, 0 − 1000) = 0 → delta = income_avg − 0 = 2000.
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 2000.0);
}

/// Target FIRE modo B (annual_expense) usa `expense_eff` como base: target = (expense_eff×12)/SWR.
#[tokio::test]
async fn mode_b_target_annual_expense_uses_expense_eff() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Solo transacciones: expense_avg = 1000, income_avg = 3000; sin liabilities → expense_eff = 1000.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    // FIRE: annual_expense, SWR 4%, sin impuestos → target = (1000×12)/0.04 = 300000.
    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "fire_number_mode": "annual_expense",
                "swr_pct": "4",
                "taxes_enabled": false,
                "tax_brackets": [],
                "savings_source": "transactions_avg"
            } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
    let target = parse_dec(&body["jubilacion_target_net_worth"]);
    approx(target, 300_000.0);
}

/// Scoping household vs mine: cada vista promedia solo sus propias transacciones.
#[tokio::test]
async fn mode_b_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);

    // Ambos en el mismo mes → household months_with_data = 1.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Owner in", "2000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Owner out", "-800", "expense", None, None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 12), "Member in", "1000", "income", None, None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 13), "Member out", "-400", "expense", None, None).await;

    set_mode_b(&app, &owner.cookie).await;

    // household: income 3000, expense 1200 → delta 1800.
    let hh = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(hh, 1800.0);
    // mine (owner): income 2000, expense 800 → delta 1200.
    let mine = projection_delta(&app, &owner.cookie, "/v1/projection/series?view=mine&months=240").await;
    approx(mine, 1200.0);
}

/// Step-up al terminar un préstamo: en modo B las liabilities siguen llegando al engine, así que
/// una que termina antes libera caja hacia el activo (que crece) y deja MÁS patrimonio al final
/// que otra idéntica que sigue activa todo el horizonte.
#[tokio::test]
async fn mode_b_liability_end_date_step_up_present() {
    async fn terminal_nw(username: &str, end_year_offset: i32) -> f64 {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner(username).await;
        let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
        let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

        let asset = app
            .post_json_with_cookie(
                "/v1/assets",
                json!({ "category_id": asset_cat, "name": "MSCI", "current_value": "100000",
                        "is_liquid": true, "expected_annual_return_percent": "10" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
        let asset_id = asset.json()["id"].as_str().unwrap().to_string();

        let rule = app
            .post_json_with_cookie(
                "/v1/allocation-rules",
                json!({ "target_asset_id": asset_id, "kind": "remainder" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(rule.status, http::StatusCode::CREATED, "{rule:?}");

        let today = server_today(&app, &owner.cookie).await;
        let end = date_in(today.year() + end_year_offset, today.month(), 28);
        let liab = app
            .post_json_with_cookie(
                "/v1/liabilities",
                json!({ "category_id": liab_cat, "label": "L", "principal": "300000",
                        "payment_amount": "1000", "payment_frequency": "monthly", "payment_end_date": end }),
                &owner.cookie,
            )
            .await;
        assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");

        let (y1, m1) = shift_month(today.year(), today.month(), -1);
        manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "2000", "income", None, None).await;

        set_mode_b(&app, &owner.cookie).await;
        let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
        let points = body["points"].as_array().unwrap();
        points.last().unwrap()["net_worth"].as_f64().unwrap()
    }

    // EARLY: préstamo termina ~1 año → libera 1000/mes al activo desde el mes ~12.
    let early = terminal_nw("alice", 1).await;
    // LATE: préstamo termina ~26 años (más allá del horizonte de 240 meses) → activo todo el tiempo.
    let late = terminal_nw("bob", 26).await;

    assert!(
        early > late,
        "modo B: la liability que termina antes debe dejar MÁS patrimonio al final (step-up); early={early}, late={late}"
    );
}

/// Fix 3 (regresión): GET /v1/assets debe seguir el modo `savings_source`. El reparto del primer mes
/// usa el ahorro mensual efectivo, así que `contribution_nominal_monthly` cambia entre modo A
/// (presupuesto) y modo B (promedio real). Antes el endpoint pasaba `fire_settings = None` al builder
/// → las aportaciones por activo salían SIEMPRE en modo presupuesto.
#[tokio::test]
async fn assets_contribution_follows_savings_source_mode() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    // Presupuesto (modo A): surplus 5000 − 3000 = 2000.
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    // Un único activo con regla remainder → recibe TODO el ahorro mensual del primer mes.
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "MSCI", "current_value": "100000",
                    "is_liquid": true, "expected_annual_return_percent": "5" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let rule = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": asset_id, "kind": "remainder" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(rule.status, http::StatusCode::CREATED, "{rule:?}");

    // Transacciones en el último mes completo (modo B): income 4000, expense 1000 → surplus 3000.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "4000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    // Modo A (default): contribución = surplus presupuesto = 2000.
    let contrib_a = asset_contribution(&app, &owner.cookie).await;
    approx(contrib_a, 2000.0);

    // Modo B: contribución = surplus promedio real = 3000. DEBE diferir de A (regresión del bug).
    set_mode_b(&app, &owner.cookie).await;
    let contrib_b = asset_contribution(&app, &owner.cookie).await;
    approx(contrib_b, 3000.0);

    assert!(
        (contrib_a - contrib_b).abs() > 100.0,
        "GET /v1/assets debe seguir el modo: A={contrib_a}, B={contrib_b}"
    );
}
