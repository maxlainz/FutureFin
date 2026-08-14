//! **Runway** y base de gasto de `GET /v1/summary` (`financial_health`).
//!
//! Los dos primeros tests son la captura de regresión previa al cambio de base de 2.2.0 y deben
//! seguir verdes DESPUÉS de él (sin rentabilidad ni inflación el runway compuesto se reduce
//! exactamente a la división simple):
//! - `runway_months == liquid_assets_total / expense_total_monthly_equivalent` (modo A).
//! - `expense_derived_monthly_equivalent` = cuotas de pasivos activos.
//! - `expense_total_monthly_equivalent` = gasto de presupuesto + cuotas derivadas.
//! - sin gasto (`expense_total == 0`) el campo `runway_months` **no se serializa**.
//!
//! El resto cubre el comportamiento nuevo (todos con el número o la desigualdad PREDICHOS en el
//! comentario de cada test): rentabilidad que alarga, inflación que acorta, caso indefinido, y la
//! base de gasto efectiva en modo B (`transactions_avg`) con sus identidades.

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

/// Activo líquido con rentabilidad esperada (entra en la media ponderada del runway).
async fn liquid_asset_with_return(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    name: &str,
    value: &str,
    annual_return_pct: &str,
) {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": name, "current_value": value, "is_liquid": true,
                    "expected_annual_return_percent": annual_return_pct }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset {name}: {r:?}");
}

/// Pasivo activo (fin de pago dentro de 5 años) con cuota mensual.
async fn active_liability(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    label: &str,
    payment: &str,
    today: NaiveDate,
) -> String {
    let future = NaiveDate::from_ymd_opt(today.year() + 5, 1, 15)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let l = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": cat, "label": label, "principal": "50000",
                    "payment_amount": payment, "payment_frequency": "monthly",
                    "payment_end_date": future }),
            cookie,
        )
        .await;
    assert_eq!(l.status, http::StatusCode::CREATED, "liability {label}: {l:?}");
    l.json()["id"].as_str().unwrap().to_string()
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

/// Movimiento manual (opcionalmente vinculado a un pasivo).
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
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "manual {concept}: {r:?}");
}

async fn set_inflation(app: &TestApp, cookie: &str, pct: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "annual_inflation_assumption_percent": pct }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set inflation: {r:?}");
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
    assert_eq!(
        h["runway_is_indefinite"], false,
        "sin base de gasto NO es «cubierto»: es «no definido»"
    );
}

// ---------------------------------------------------------------------------
// Runway compuesto: rentabilidad, inflación, caso indefinido
// ---------------------------------------------------------------------------

/// Mismo escenario que la baseline (líquidos 12.000, gasto total 1.200 → 10 meses sin retorno) pero
/// con `expected_annual_return_percent = 5` en ambos activos.
///
/// PREDICCIÓN: el saldo remanente crece ~0,407 %/mes (1,05^(1/12)) mientras se drena, así que el
/// runway es **estrictamente > 10** y sigue siendo **finito** (el rendimiento mensual del saldo,
/// ≤ 12.000 × 0,00407 ≈ 49 €, está muy por debajo del gasto de 1.200 €). El extra acumulado ronda
/// los 245 € ≈ 0,2 meses, así que también debe quedar **< 11**.
#[tokio::test]
async fn runway_with_return_exceeds_plain_division() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    liquid_asset_with_return(&app, &owner.cookie, &asset_cat, "Cuenta A", "9000", "5").await;
    liquid_asset_with_return(&app, &owner.cookie, &asset_cat, "Cuenta B", "3000", "5").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;
    let today = server_today(&app, &owner.cookie).await;
    active_liability(&app, &owner.cookie, &liab_cat, "L1", "200", today).await;

    let h = health(&app, &owner.cookie).await;
    assert_eq!(dec(&h["liquid_assets_total"]), d(12_000));
    assert_eq!(dec(&h["expense_total_monthly_equivalent"]), d(1_200));
    let runway = dec(&h["runway_months"]);
    assert!(
        runway > d(10),
        "la rentabilidad debe alargar el runway por encima de la división simple (10), got {runway}"
    );
    assert!(runway < d(11), "el extra por retorno ronda 0,2 meses, got {runway}");
    assert_eq!(h["runway_is_indefinite"], false);
}

/// Baseline exacta (12.000 / 1.200 = 10) pero con inflación de instalación al 3 %.
///
/// PREDICCIÓN: el gasto del mes k es 1.200 × 1,03^((k−1)/12) > 1.200, así que el runway es
/// **estrictamente < 10**. El sobrecoste acumulado ronda 133 € ≈ 0,11 meses → debe quedar **> 9**.
#[tokio::test]
async fn runway_with_inflation_is_shorter_than_without() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    // Sin rentabilidad esperada: el único efecto medido es la inflación del gasto.
    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta A", "12000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;
    let today = server_today(&app, &owner.cookie).await;
    active_liability(&app, &owner.cookie, &liab_cat, "L1", "200", today).await;

    // Antes del PATCH (inflación 0 por defecto) el runway es la división simple: 10 exactos.
    let sin_inflacion = health(&app, &owner.cookie).await;
    assert_eq!(dec(&sin_inflacion["runway_months"]), d(10));

    set_inflation(&app, &owner.cookie, "3").await;

    let h = health(&app, &owner.cookie).await;
    assert_eq!(dec(&h["expense_total_monthly_equivalent"]), d(1_200));
    let runway = dec(&h["runway_months"]);
    assert!(runway < d(10), "la inflación debe acortar el runway, got {runway}");
    assert!(runway > d(9), "el recorte ronda 0,11 meses, got {runway}");
    assert_eq!(h["runway_is_indefinite"], false);
}

/// 1.000.000 € al 7 % anual rinden ~5.650 €/mes, muy por encima de un gasto de 1.000 €/mes sin
/// inflación.
///
/// PREDICCIÓN: el saldo nunca baja → el bucle agota el cap de 100 años → `runway_months` **null** y
/// `runway_is_indefinite == true` (el campo distingue este caso del «sin base de gasto»).
#[tokio::test]
async fn runway_indefinite_when_returns_cover_expense() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    liquid_asset_with_return(&app, &owner.cookie, &asset_cat, "Cartera", "1000000", "7").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let h = health(&app, &owner.cookie).await;
    assert_eq!(dec(&h["liquid_assets_total"]), d(1_000_000));
    assert_eq!(dec(&h["expense_total_monthly_equivalent"]), d(1_000));
    assert!(
        h["runway_months"].is_null(),
        "runway indefinido ⇒ `runway_months` omitido, got {:?}",
        h["runway_months"]
    );
    assert_eq!(h["runway_is_indefinite"], true);
}

// ---------------------------------------------------------------------------
// Modo B: base de gasto efectiva + identidades
// ---------------------------------------------------------------------------

/// Modo B (`transactions_avg`) con datos: la base de gasto del panel pasa a ser el gasto real medio
/// más el servicio de deuda, y el runway se calcula sobre ella.
///
/// Escenario: presupuesto deliberadamente distinto (income 9.000, gasto 8.000). Pasivos activos L1
/// (cuota nominal 500, con movimiento vinculado de 400 → gana el real) y L2 (nominal 300, sin
/// vincular). Único mes real: income 3.000, gasto 1.500 (400 de ellos a L1). Líquidos 16.000 sin
/// rentabilidad, inflación 0.
///
/// PREDICCIÓN (a mano):
/// - resta híbrida = 400 (L1 real) + 300 (L2 nominal) = 700 ⇒ `expense_regular` = 1.500 − 700 = 800
/// - `debt_service` nominal = 500 + 300 = 800 ⇒ `expense_derived` = **800**
/// - `expense_total` = 800 + 800 = **1.600** (en modo A sería 8.000 + 800 = 8.800)
/// - `net` = 3.000 − 800 − 800 = **1.400**
/// - `runway` = 16.000 / 1.600 = **10** exactos (con la base de presupuesto saldrían 16.000/8.800 ≈ 1,82)
/// más las identidades `expense_total = expense_reg + expense_der` y `net = income − expense_total`.
#[tokio::test]
async fn mode_b_runway_uses_effective_expense_base() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta A", "16000").await;
    budget(&app, &owner.cookie, &income_cat, "9000").await;
    budget(&app, &owner.cookie, &expense_cat, "8000").await;

    let today = server_today(&app, &owner.cookie).await;
    let l1 = active_liability(&app, &owner.cookie, &liab_cat, "L1", "500", today).await;
    active_liability(&app, &owner.cookie, &liab_cat, "L2", "300", today).await;

    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Cuota L1", "-400", "expense", Some(&l1)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 12), "Resto", "-1100", "expense", None).await;

    // Antes del cambio de modo, la base es la del presupuesto: 8.000 + 800 = 8.800.
    let modo_a = health(&app, &owner.cookie).await;
    assert_eq!(dec(&modo_a["expense_total_monthly_equivalent"]), d(8_800));

    set_mode_b(&app, &owner.cookie).await;

    let h = health(&app, &owner.cookie).await;
    assert_eq!(h["savings_source"], "transactions_avg");
    assert_eq!(h["savings_source_months_with_data"].as_u64().unwrap(), 1);

    let income = dec(&h["income_monthly_equivalent"]);
    let expense_reg = dec(&h["expense_regular_monthly_equivalent"]);
    let expense_der = dec(&h["expense_derived_monthly_equivalent"]);
    let expense_tot = dec(&h["expense_total_monthly_equivalent"]);
    let net = dec(&h["net_monthly_equivalent"]);

    assert_eq!(income, d(3_000));
    assert_eq!(expense_reg, d(800));
    assert_eq!(expense_der, d(800), "en modo B la línea derivada es el servicio de deuda");
    assert_eq!(expense_tot, d(1_600));
    assert_eq!(net, d(1_400));
    // Identidades restauradas en modo B (rotas antes del cambio de base).
    assert_eq!(
        expense_tot,
        expense_reg + expense_der,
        "expense_total = expense_regular + expense_derived"
    );
    assert_eq!(net, income - expense_tot, "net = income − expense_total");

    // Runway sobre la base efectiva: 16.000 / 1.600 = 10 (sin rentabilidad ni inflación).
    assert_eq!(dec(&h["liquid_assets_total"]), d(16_000));
    assert_eq!(dec(&h["runway_months"]), d(10));
    assert_eq!(
        dec(&h["runway_months"]),
        dec(&h["liquid_assets_total"]) / expense_tot,
        "runway = líquidos / gasto total efectivo"
    );
    assert_eq!(h["runway_is_indefinite"], false);
}

/// Modo B **sin** meses reales en la ventana → fallback silencioso al presupuesto: la respuesta debe
/// ser IDÉNTICA a la de modo A (runway y `expense_total` presupuestarios incluidos).
///
/// PREDICCIÓN: `expense_total` = 1.000 + 200 = **1.200**, `runway` = 12.000 / 1.200 = **10**, y el
/// bloque `financial_health` completo igual antes y después del PATCH (mismo JSON).
#[tokio::test]
async fn mode_b_zero_months_falls_back_to_budget_runway() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    liquid_asset(&app, &owner.cookie, &asset_cat, "Cuenta A", "12000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;
    let today = server_today(&app, &owner.cookie).await;
    active_liability(&app, &owner.cookie, &liab_cat, "L1", "200", today).await;

    // Única transacción en el mes en curso (parcial) → fuera de la ventana → 0 meses reales.
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

    let modo_a = health(&app, &owner.cookie).await;
    assert_eq!(dec(&modo_a["expense_derived_monthly_equivalent"]), d(200));
    assert_eq!(dec(&modo_a["expense_total_monthly_equivalent"]), d(1_200));
    assert_eq!(dec(&modo_a["runway_months"]), d(10));

    set_mode_b(&app, &owner.cookie).await;

    let modo_b = health(&app, &owner.cookie).await;
    assert_eq!(modo_b["savings_source"], "budget", "fallback ⇒ fuente efectiva budget");
    assert_eq!(modo_b["savings_source_months_with_data"].as_u64().unwrap(), 0);
    assert_eq!(
        modo_b, modo_a,
        "sin meses reales el modo B debe devolver exactamente el bloque del modo A"
    );
}
