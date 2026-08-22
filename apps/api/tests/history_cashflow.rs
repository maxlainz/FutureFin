//! Integración del cash-flow histórico (`GET /v1/history/cashflow`) — WP B2.
//!
//! Cubre:
//!   1. Agregados mensuales exactos (Decimal-string) de los tres kinds, household y mine.
//!   2. La curva fina moldeada por cash-flow pasa EXACTA por las fechas de snapshot (P1/P2) y se
//!      desvía en el interior (anclaje tier-2). Vínculo vía batch con `account_asset_id`.
//!   3. REGRESIÓN: `GET /v1/history/series` devuelve EXACTAMENTE lo mismo con y sin transacciones.
//!   4. `resolution=daily` con ventana grande → 400 `daily_window_too_large`; weekly funciona;
//!      `fine` ausente cuando no hay vínculos a assets.
//!
//! Los valores de `fine` viajan como f64 → epsilon relativo 1e-6 (cruzaron Decimal→f64). Cada test
//! predice sus números desde el `anchor_date_ymd` que devuelve el propio endpoint (hoy civil del
//! servidor), nunca desde el reloj local.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{Datelike, Duration, NaiveDate};
use common::{LoggedInOwner, TestApp};
use futurefin_engine::add_months_signed;
use serde_json::{json, Value};

/// Epsilon relativo 1e-6 (los valores cruzaron Decimal→f64).
fn assert_close(got: f64, expected: f64, ctx: &str) {
    let tol = 1e-6_f64 * expected.abs().max(1.0);
    assert!(
        (got - expected).abs() <= tol,
        "{ctx}: got {got}, expected {expected} (tol {tol})"
    );
}

fn ymd(v: &Value) -> NaiveDate {
    NaiveDate::parse_from_str(v.as_str().expect("ymd string"), "%Y-%m-%d").expect("parse ymd")
}

/// GET /v1/history/cashflow → (JSON, anchor_date, anchor_month_first).
async fn get_cashflow(app: &TestApp, cookie: &str, qs: &str) -> (Value, NaiveDate, NaiveDate) {
    let resp = app
        .get_with_cookie(&format!("/v1/history/cashflow{qs}"), cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "cashflow GET: {resp:?}");
    let body = resp.json();
    let anchor_date = ymd(&body["anchor_date_ymd"]);
    let anchor_mf = ymd(&body["anchor_month_first_ymd"]);
    (body, anchor_date, anchor_mf)
}

/// Hoy civil del servidor (el endpoint lo devuelve siempre, aun sin datos).
async fn server_anchor(app: &TestApp, cookie: &str) -> (NaiveDate, NaiveDate) {
    let (_, today, anchor) = get_cashflow(app, cookie, "?window_months=6").await;
    (today, anchor)
}

async fn manual_txn(
    app: &TestApp,
    user: &LoggedInOwner,
    op_date: NaiveDate,
    amount: &str,
    kind: &str,
    extra: Value,
) -> Value {
    let mut body = json!({
        "op_date": op_date.format("%Y-%m-%d").to_string(),
        "concept": "Seed",
        "amount": amount,
        "kind": kind,
    });
    if let Value::Object(m) = extra {
        for (k, v) in m {
            body[k] = v;
        }
    }
    let resp = app
        .post_json_with_cookie("/v1/transactions", body, &user.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "manual txn: {resp:?}");
    resp.json()
}

async fn backfill(
    app: &TestApp,
    user: &LoggedInOwner,
    kind: &str,
    date: NaiveDate,
    items: Value,
) {
    let resp = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            json!({
                "kind": kind,
                "snapshot_date": date.format("%Y-%m-%d").to_string(),
                "items": items,
            }),
            &user.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "backfill: {resp:?}");
}

/// Importa un CSV MyInvestor de una sola fila (fecha `op_date`, importe `amount`) colgado de un
/// batch con `account_asset_id`. Devuelve tras confirmar.
async fn import_one_row_myinvestor(
    app: &TestApp,
    user: &LoggedInOwner,
    op_date: NaiveDate,
    amount: &str,
    account_asset_id: &str,
) {
    let csv = format!(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n{d};{d};CARGO CUENTA;{a};EUR\n",
        d = op_date.format("%d/%m/%Y"),
        a = amount,
    );
    let b64 = B64.encode(csv);
    let preview = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64, "account_asset_id": account_asset_id }),
            &user.cookie,
        )
        .await;
    assert_eq!(preview.status, http::StatusCode::OK, "preview: {preview:?}");
    let pv = preview.json();
    let sha = pv["file_sha256"].as_str().unwrap().to_string();
    let n = pv["rows"].as_array().unwrap().len();
    let decisions: Vec<Value> = (0..n).map(|_| json!({ "kind": "expense" })).collect();
    let confirm = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": decisions, "learn_rules": false, "account_asset_id": account_asset_id,
            }),
            &user.cookie,
        )
        .await;
    assert_eq!(confirm.status, http::StatusCode::OK, "confirm: {confirm:?}");
}

/// Elemento `months[]` con `month_index == k`.
fn month_at(body: &Value, k: i32) -> &Value {
    body["months"]
        .as_array()
        .expect("months array")
        .iter()
        .find(|m| m["month_index"].as_i64().unwrap() == k as i64)
        .unwrap_or_else(|| panic!("no month with index {k} in {body}"))
}

// ---------------------------------------------------------------------------
// 1. Agregados mensuales exactos (Decimal-string) — household y mine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn monthly_aggregates_exact_household_and_mine() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let exp_cat = app.create_category(&owner, "expense", "Compras").await;
    let inc_cat = app.create_category(&owner, "income", "Nomina").await;
    let (_, anchor) = server_anchor(&app, &owner.cookie).await;

    // Mes -1 (completo). Día 15 para evitar fronteras.
    let d = add_months_signed(anchor, -1).with_day(15).expect("day 15");

    // Alice: gasto -100, ingreso +2000, ahorro -300.
    manual_txn(&app, &owner, d, "-100", "expense", json!({ "category_id": exp_cat })).await;
    manual_txn(&app, &owner, d, "2000", "income", json!({ "category_id": inc_cat })).await;
    manual_txn(&app, &owner, d, "-300", "savings", json!({})).await;
    // Bob: gasto -50 en el mismo mes.
    manual_txn(&app, &bob, d, "-50", "expense", json!({ "category_id": exp_cat })).await;

    // Household (predicción mes -1): expense -100 + -50 = -150.00; income 2000.00; savings -300.00;
    // net = -150 + 2000 - 300 = 1550.00 (invariante: net = suma de los tres, sobre los strings).
    let (hh, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=6").await;
    assert_eq!(hh["view"], "household");
    // 7 meses contiguos -6..=0.
    let months = hh["months"].as_array().unwrap();
    assert_eq!(months.len(), 7);
    let ks: Vec<i64> = months.iter().map(|m| m["month_index"].as_i64().unwrap()).collect();
    assert_eq!(ks, vec![-6, -5, -4, -3, -2, -1, 0]);
    // date_ymd del mes -1 = primero-de-mes.
    let m_hh = month_at(&hh, -1);
    assert_eq!(ymd(&m_hh["date_ymd"]), add_months_signed(anchor, -1));
    assert_eq!(m_hh["expense"], "-150.00");
    assert_eq!(m_hh["income"], "2000.00");
    assert_eq!(m_hh["savings"], "-300.00");
    // cash_delta          = -150 + 2000 - 300 = 1550.00  (invariante: suma de los tres strings)
    // income_minus_expense = 2000 - 150       = 1850.00  (sin ahorro; == totals.net_actual del mes)
    // Sólo este mes y el de alice DISCRIMINAN entre las dos fórmulas: los otros dos casos no tienen
    // ahorro, así que ambas coinciden y no cazarían una fórmula intercambiada.
    assert_eq!(m_hh["cash_delta"], "1550.00");
    assert_eq!(m_hh["income_minus_expense"], "1850.00");
    // Sin vínculos a assets → fine ausente (omitido).
    assert!(hh.get("fine").is_none(), "fine debe estar ausente sin vínculos: {hh}");

    // Un mes vacío (mes -3): todo a cero, los dos netos 0.00.
    let empty = month_at(&hh, -3);
    assert_eq!(empty["expense"], "0.00");
    assert_eq!(empty["income"], "0.00");
    assert_eq!(empty["savings"], "0.00");
    assert_eq!(empty["cash_delta"], "0.00");
    assert_eq!(empty["income_minus_expense"], "0.00");

    // Mine (alice): solo lo suyo → expense -100.00.
    let (mine, _, _) = get_cashflow(&app, &owner.cookie, "?view=mine&window_months=6").await;
    assert_eq!(mine["view"], "mine");
    let m_mine = month_at(&mine, -1);
    assert_eq!(m_mine["expense"], "-100.00");
    assert_eq!(m_mine["income"], "2000.00");
    assert_eq!(m_mine["savings"], "-300.00");
    // cash_delta = -100 + 2000 - 300 = 1600.00 · income_minus_expense = 2000 - 100 = 1900.00
    assert_eq!(m_mine["cash_delta"], "1600.00");
    assert_eq!(m_mine["income_minus_expense"], "1900.00");

    // Mine (bob): expense -50.00; sin income/savings → los dos netos = -50.00.
    let (mine_b, _, _) = get_cashflow(&app, &bob.cookie, "?view=mine&window_months=6").await;
    let m_b = month_at(&mine_b, -1);
    assert_eq!(m_b["expense"], "-50.00");
    assert_eq!(m_b["income"], "0.00");
    assert_eq!(m_b["savings"], "0.00");
    assert_eq!(m_b["cash_delta"], "-50.00");
    assert_eq!(m_b["income_minus_expense"], "-50.00");
}

// ---------------------------------------------------------------------------
// 2. La curva fina pasa EXACTA por los snapshots y se moldea con el cash-flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fine_series_passes_through_snapshots_and_is_shaped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cuenta").await;
    let (today, _anchor) = server_anchor(&app, &owner.cookie).await;

    // Asset vivo a 1000. Snapshots planos 1000 en d1 (−70 días) y d2 (−14 días): ambos son puntos
    // de la rejilla weekly (múltiplos de 7 hacia atrás desde hoy).
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "Cuenta N26", "current_value": "1000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let asset_id = created.json()["id"].as_str().unwrap().to_string();

    let d1 = today - Duration::days(70);
    let d2 = today - Duration::days(14);
    let charge_date = today - Duration::days(42); // en (d1, d2], punto de rejilla (−6 semanas).
    backfill(&app, &owner, "asset", d1,
        json!([{ "item_id": asset_id, "label": "Cuenta N26", "value": "1000" }])).await;
    backfill(&app, &owner, "asset", d2,
        json!([{ "item_id": asset_id, "label": "Cuenta N26", "value": "1000" }])).await;

    // Cargo de -500 en charge_date, colgado de un batch con account_asset_id = asset → delta -500.
    import_one_row_myinvestor(&app, &owner, charge_date, "-500", &asset_id).await;

    // window=6 weekly (default). fine presente (hay vínculo + snapshots).
    let (body, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=6").await;
    let fine = body.get("fine").expect("fine presente con vínculos + snapshots");
    assert_eq!(fine["resolution"], "weekly");

    let grid = fine["grid"].as_array().unwrap();
    let series = fine["asset_series"].as_array().unwrap();
    assert_eq!(series.len(), 1, "una única serie de asset");
    assert_eq!(series[0]["asset_id"].as_str().unwrap(), asset_id);
    assert_eq!(series[0]["asset_name"], "Cuenta N26");
    let values = series[0]["values"].as_array().unwrap();
    let net_worth = fine["net_worth"].as_array().unwrap();
    assert_eq!(values.len(), grid.len());
    assert_eq!(net_worth.len(), grid.len());

    // Índice de rejilla por fecha.
    let idx_of = |date: NaiveDate| -> usize {
        grid.iter()
            .position(|p| ymd(&p["date_ymd"]) == date)
            .unwrap_or_else(|| panic!("fecha {date} no está en la rejilla"))
    };
    let val = |date: NaiveDate| -> f64 { values[idx_of(date)].as_f64().unwrap() };

    // P1/P2: exacto en las fechas de snapshot.
    assert_close(val(d1), 1000.0, "fine en d1 = Va");
    assert_close(val(d2), 1000.0, "fine en d2 = Vb");
    // El último punto es hoy y empalma con el valor vivo (1000).
    assert_close(val(today), 1000.0, "fine en hoy = live");

    // Anclaje tier-2 (Va=Vb=1000, C_total=-500, residual = f·500):
    //   antes del cargo, en −49 días (f = (70−49)/56 = 0.375): 1000 + 500·0.375 = 1187.5.
    //   en el cargo, −42 días (f = (70−42)/56 = 0.5, C(t)=-500): 1000 − 500 + 500·0.5 = 750.
    let before = today - Duration::days(49);
    assert_close(val(before), 1187.5, "interior antes del cargo (moldeado por el residuo)");
    assert_close(val(charge_date), 750.0, "interior en el cargo (salto por el delta)");
    // El salto discrimina frente a la interpolación lineal plana (que daría 1000 en todo punto).
    assert!(
        (val(before) - val(charge_date)).abs() > 400.0,
        "debe haber un salto claro por el cash-flow"
    );

    // net_worth == valor del asset (sin pasivos).
    assert_close(net_worth[idx_of(charge_date)].as_f64().unwrap(), 750.0, "net_worth = asset");

    // La serie mensual ve el gasto: mes del cargo tiene expense = -500.00.
    let charge_mi = (charge_date.year() - today.year()) * 12
        + (charge_date.month() as i32 - today.month() as i32);
    let m = month_at(&body, charge_mi);
    assert_eq!(m["expense"], "-500.00");
}

// ---------------------------------------------------------------------------
// 3. REGRESIÓN: /v1/history/series intacto con transacciones presentes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn history_series_identical_with_and_without_transactions() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let exp_cat = app.create_category(&owner, "expense", "Compras").await;
    let (today, anchor) = server_anchor(&app, &owner.cookie).await;

    // Asset vivo + dos snapshots (mismo item_id) + un pasivo backfilleado, para una serie rica.
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "Cuenta", "current_value": "1500" }),
            &owner.cookie,
        )
        .await;
    let asset_id = created.json()["id"].as_str().unwrap().to_string();
    let d1 = add_months_signed(anchor, -3);
    let d2 = add_months_signed(anchor, -1);
    backfill(&app, &owner, "asset", d1,
        json!([{ "item_id": asset_id, "label": "Cuenta", "value": "1000" }])).await;
    backfill(&app, &owner, "asset", d2,
        json!([{ "item_id": asset_id, "label": "Cuenta", "value": "1300" }])).await;
    backfill(&app, &owner, "liability", d1,
        json!([{ "label": "Hipoteca", "value": "50000",
                 "apr_percent": "3", "payment_amount": "600", "payment_frequency": "monthly" }])).await;

    // Serie ANTES de cualquier transacción.
    let before = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    assert_eq!(before.status, http::StatusCode::OK);
    let before_json = before.json();
    let before_mine = app.get_with_cookie("/v1/history/series?view=mine", &owner.cookie).await.json();

    // Sembrar transacciones de todos los sabores, incluidas las que vinculan al asset (batch con
    // account_asset_id + savings con linked_asset_id) — precisamente las que moldearían la curva
    // fina. La serie de snapshots (tier-1) NO debe verse afectada.
    let mid = today - Duration::days(30);
    manual_txn(&app, &owner, mid, "-80", "expense", json!({ "category_id": exp_cat })).await;
    manual_txn(&app, &owner, mid, "1200", "income", json!({})).await;
    manual_txn(&app, &owner, mid, "-200", "savings", json!({ "linked_asset_id": asset_id })).await;
    import_one_row_myinvestor(&app, &owner, mid, "-333", &asset_id).await;

    // Serie DESPUÉS: idéntica byte a byte (household y mine).
    let after_json = app.get_with_cookie("/v1/history/series", &owner.cookie).await.json();
    let after_mine = app.get_with_cookie("/v1/history/series?view=mine", &owner.cookie).await.json();
    assert_eq!(
        before_json, after_json,
        "GET /v1/history/series (household) cambió al introducir transacciones"
    );
    assert_eq!(
        before_mine, after_mine,
        "GET /v1/history/series (mine) cambió al introducir transacciones"
    );

    // Y el cashflow SÍ ve la curva fina moldeada (sanity de que las transacciones existen).
    let (cf, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=6").await;
    assert!(cf.get("fine").is_some(), "cashflow debe tener fine con los vínculos sembrados");
}

// ---------------------------------------------------------------------------
// 4. resolution: daily acotado, weekly por defecto, 400 con ventana grande
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolution_daily_bounds_and_weekly_default() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // daily con ventana grande (default 24) → 400 daily_window_too_large.
    let big = app
        .get_with_cookie("/v1/history/cashflow?resolution=daily", &owner.cookie)
        .await;
    assert_eq!(big.status, http::StatusCode::BAD_REQUEST, "{big:?}");
    assert!(
        big.json()["message"].as_str().unwrap().contains("daily_window_too_large"),
        "código estable esperado: {}",
        big.json()
    );
    // daily con window_months=7 (>6) también 400.
    let big7 = app
        .get_with_cookie("/v1/history/cashflow?resolution=daily&window_months=7", &owner.cookie)
        .await;
    assert_eq!(big7.status, http::StatusCode::BAD_REQUEST, "{big7:?}");

    // daily con window_months=6 → 200, resolución reportada daily.
    let ok = app
        .get_with_cookie("/v1/history/cashflow?resolution=daily&window_months=6", &owner.cookie)
        .await;
    assert_eq!(ok.status, http::StatusCode::OK, "{ok:?}");
    // Sin datos → fine ausente aunque la resolución sea válida.
    assert!(ok.json().get("fine").is_none());

    // weekly por defecto → 200.
    let (wk, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=12").await;
    assert_eq!(wk["view"], "household");
    assert_eq!(wk["months"].as_array().unwrap().len(), 13); // -12..=0

    // window_months fuera de rango se clampa (200, sin crash): 500 → clamp 120.
    let (clamped, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=500").await;
    assert_eq!(clamped["months"].as_array().unwrap().len(), 121); // -120..=0
}

// ---------------------------------------------------------------------------
// 5. daily produce rejilla diaria (paso 1 día) cuando hay fine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daily_resolution_produces_daily_grid() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cuenta").await;
    let (today, _anchor) = server_anchor(&app, &owner.cookie).await;

    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "Cuenta", "current_value": "1000" }),
            &owner.cookie,
        )
        .await;
    let asset_id = created.json()["id"].as_str().unwrap().to_string();
    let d1 = today - Duration::days(40);
    backfill(&app, &owner, "asset", d1,
        json!([{ "item_id": asset_id, "label": "Cuenta", "value": "1000" }])).await;
    import_one_row_myinvestor(&app, &owner, today - Duration::days(20), "-100", &asset_id).await;

    let resp = app
        .get_with_cookie("/v1/history/cashflow?resolution=daily&window_months=3", &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let body = resp.json();
    let fine = body.get("fine").expect("fine presente");
    assert_eq!(fine["resolution"], "daily");
    let grid = fine["grid"].as_array().unwrap();
    // Último punto = hoy; el penúltimo, un día antes (paso diario).
    let last = ymd(&grid[grid.len() - 1]["date_ymd"]);
    let prev = ymd(&grid[grid.len() - 2]["date_ymd"]);
    assert_eq!(last, today, "último punto de la rejilla = hoy");
    assert_eq!((last - prev).num_days(), 1, "paso diario");
}

// ---------------------------------------------------------------------------
// 6. Conciliación (3.5.0): months[] excluye conciliadas; la curva fina NO
// ---------------------------------------------------------------------------

/// EL TEST DE LA ASIMETRÍA. Un traspaso conciliado (pata −500 importada con `account_asset_id`,
/// contrapartida manual +500 a 2 días) debe:
///   - desaparecer de `months[]` (no es gasto ni ingreso), y
///   - seguir moldeando la curva fina EXACTAMENTE igual que en
///     `fine_series_passes_through_snapshots_and_is_shaped` (el saldo de la cuenta bajó de
///     verdad): mismos números predichos 1187.5 (antes del cargo) y 750 (en el cargo).
/// Si alguien «arregla» la curva fina excluyendo conciliadas, este test cae — a propósito.
#[tokio::test]
async fn reconciled_excluded_from_months_but_not_from_fine_curve() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cuenta").await;
    let (today, _anchor) = server_anchor(&app, &owner.cookie).await;

    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "Cuenta N26", "current_value": "1000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let asset_id = created.json()["id"].as_str().unwrap().to_string();

    let d1 = today - Duration::days(70);
    let d2 = today - Duration::days(14);
    let charge_date = today - Duration::days(42); // punto de rejilla weekly (−6 semanas).
    let counterpart_date = today - Duration::days(40); // Δ = 2 días → auto-conciliable.
    backfill(&app, &owner, "asset", d1,
        json!([{ "item_id": asset_id, "label": "Cuenta N26", "value": "1000" }])).await;
    backfill(&app, &owner, "asset", d2,
        json!([{ "item_id": asset_id, "label": "Cuenta N26", "value": "1000" }])).await;

    // Contrapartida manual +500 primero; el confirm del import cruza toda la BD y las concilia.
    let in_leg = manual_txn(&app, &owner, counterpart_date, "500", "income", json!({})).await;
    import_one_row_myinvestor(&app, &owner, charge_date, "-500", &asset_id).await;

    // Precondición: el par quedó conciliado (la pata manual apunta a la importada).
    let month_qs = counterpart_date.format("%Y-%m").to_string();
    let list = app
        .get_with_cookie(&format!("/v1/transactions?month={month_qs}"), &owner.cookie)
        .await
        .json();
    let in_leg_now = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == in_leg["id"])
        .expect("pata manual en el listado")
        .clone();
    assert!(
        in_leg_now["transfer_counterpart_id"].is_string(),
        "precondición: par conciliado tras el import: {in_leg_now:?}"
    );

    let (body, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=6").await;

    // months[]: ni el −500 (expense) ni el +500 (income) cuentan.
    let mi_of = |d: NaiveDate| -> i32 {
        (d.year() - today.year()) * 12 + (d.month() as i32 - today.month() as i32)
    };
    assert_eq!(month_at(&body, mi_of(charge_date))["expense"], "0.00", "pata de salida excluida");
    assert_eq!(month_at(&body, mi_of(counterpart_date))["income"], "0.00", "pata de entrada excluida");

    // Curva fina: MISMOS números que el test sin conciliar — el saldo real de la cuenta manda.
    let fine = body.get("fine").expect("fine presente (vínculo + snapshots)");
    let grid = fine["grid"].as_array().unwrap();
    let values = fine["asset_series"].as_array().unwrap()[0]["values"].as_array().unwrap();
    let idx_of = |date: NaiveDate| -> usize {
        grid.iter()
            .position(|p| ymd(&p["date_ymd"]) == date)
            .unwrap_or_else(|| panic!("fecha {date} no está en la rejilla"))
    };
    let val = |date: NaiveDate| -> f64 { values[idx_of(date)].as_f64().unwrap() };
    assert_close(val(d1), 1000.0, "fine en d1 = Va");
    assert_close(val(d2), 1000.0, "fine en d2 = Vb");
    let before = today - Duration::days(49);
    assert_close(val(before), 1187.5, "interior antes del cargo (la conciliada sigue moldeando)");
    assert_close(val(charge_date), 750.0, "interior en el cargo (salto por el delta conciliado)");
}

// ---------------------------------------------------------------------------
// 5. REGRESIÓN (issue #8 §6) — los dos netos, y su relación con la comparativa
// ---------------------------------------------------------------------------

/// `cash_delta` incluye el ahorro; `income_minus_expense` no, y **coincide al céntimo** con
/// `totals.net_actual` de la comparativa mensual.
///
/// Hasta 4.0.0 el campo se llamaba `net` y `get_transactions_summary.net_actual` también, con
/// fórmulas distintas: dos cosas diferentes con el mismo nombre en el mismo catálogo. Un mes con
/// una aportación fuerte a inversión salía en negativo y se leía como una pérdida cuando había sido
/// un mes excelente. Este test siembra exactamente ese caso.
#[tokio::test]
async fn the_two_nets_differ_by_savings_and_one_matches_the_monthly_summary() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let (_, anchor) = server_anchor(&app, &owner.cookie).await;

    // Mes anterior (completo, para que la comparativa lo tome por defecto): un mes excelente que
    // se leía como una pérdida — ingresa 2.853, gasta 2.218 y mueve 3.711 a inversión.
    let m = add_months_signed(anchor, -1);
    let day = |d: u32| m.with_day(d).expect("día válido");
    manual_txn(&app, &owner, day(5), "2853.44", "income", json!({"category_id": cat_inc})).await;
    manual_txn(&app, &owner, day(10), "-2217.73", "expense", json!({"category_id": cat_exp})).await;
    manual_txn(&app, &owner, day(15), "-3710.97", "savings", json!({})).await;

    let (cf, _, _) = get_cashflow(&app, &owner.cookie, "?window_months=6").await;
    let month = month_at(&cf, -1);

    // El número que se leía como «perdí 3.075 €».
    assert_eq!(month["cash_delta"], "-3075.26", "{month}");
    // Y el que dice de verdad cómo fue el mes.
    assert_eq!(month["income_minus_expense"], "635.71", "{month}");

    // La relación cruzada: el neto sin ahorro es EL MISMO número que publica la comparativa, allí
    // con magnitudes ≥ 0. Es lo que ata las dos tools y lo que hace que no puedan volver a
    // divergir sin que un test lo cace.
    let summary = app
        .get_with_cookie(
            &format!(
                "/v1/transactions/summary?year={}&month={}",
                m.format("%Y"),
                m.format("%m").to_string().trim_start_matches('0'),
            ),
            &owner.cookie,
        )
        .await;
    assert_eq!(summary.status, http::StatusCode::OK, "{summary:?}");
    let totals = &summary.json()["totals"];
    assert_close(
        totals["net_actual"].as_str().expect("net_actual").parse::<f64>().unwrap(),
        month["income_minus_expense"].as_str().unwrap().parse::<f64>().unwrap(),
        "net_actual ↔ income_minus_expense",
    );
    // Y NO coincide con la variación de caja: la diferencia son los traspasos a ahorro.
    assert_ne!(
        totals["net_actual"].as_str().unwrap().parse::<f64>().unwrap(),
        month["cash_delta"].as_str().unwrap().parse::<f64>().unwrap(),
    );
}
