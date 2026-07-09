//! Contrato de cache de proyección condicionado al modo `savings_source`:
//! - Modo A (`budget`, default): las transacciones NO son inputs del engine → **ninguna** mutación
//!   invalida la cache (contrato histórico, espejo de `snapshot_mutations_do_not_touch_...`).
//! - Modo B (`transactions_avg`): las transacciones SÍ son inputs del engine → **cada** mutación que
//!   cambia el conjunto (create, batch, patch, delete, import confirm, delete import, materialize)
//!   invalida; borrar una regla recurrente NO (sus instancias sobreviven).
//! - Flip A↔B vía PATCH /v1/installation invalida (el propio PATCH de installation refresca).

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{Datelike, NaiveDate};
use common::TestApp;
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use serde_json::json;
use uuid::Uuid;

async fn installation_id(app: &TestApp) -> Uuid {
    sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("installation id")
}

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

/// "Hoy" del servidor (anchor de history) para fechas relativas independientes del reloj.
async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let r = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(r.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

fn household_key(iid: Uuid) -> ProjectionCacheKey {
    ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Monthly,
    }
}

async fn present(app: &TestApp, key: &ProjectionCacheKey) -> bool {
    app.state.projection_cache.read().await.contains_key(key)
}

/// Calienta la entrada household (monthly) con un GET y verifica que quedó cacheada.
async fn warm(app: &TestApp, cookie: &str, key: &ProjectionCacheKey) {
    let r = app.get_with_cookie("/v1/projection/series", cookie).await;
    assert_eq!(r.status, http::StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(present(app, key).await, "la cache debería estar caliente tras el GET");
}

/// Espera (polling) a que la invalidación en background (tokio::spawn) tire la entrada.
async fn assert_invalidated(app: &TestApp, key: &ProjectionCacheKey, what: &str) {
    for _ in 0..40 {
        if !present(app, key).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("modo B: la mutación «{what}» debía invalidar la cache de proyección");
}

async fn set_mode(app: &TestApp, cookie: &str, source: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": source } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode {source}: {r:?}");
}

/// Import CSV MyInvestor de una sola fila; devuelve `(file_b64, file_sha256)` listos para confirm.
async fn preview_csv(app: &TestApp, cookie: &str, concept: &str, amount: &str, day: u32) -> (String, String) {
    let csv = format!(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         {day:02}/06/2026;{day:02}/06/2026;{concept};{amount};EUR\n"
    );
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    (b64, sha)
}

// ---------------------------------------------------------------------------
// Modo A (default `budget`): NINGUNA mutación invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_a_mutations_do_not_touch_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    let iid = installation_id(&app).await;
    let key = household_key(iid);
    warm(&app, &owner.cookie, &key).await;

    // --- Batería de mutaciones de transacciones (modo A, default) ---
    // 1. Alta manual.
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let txn_id = created.json()["id"].as_str().unwrap().to_string();

    // 2. Batch.
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2026-06-11", "concept": "Lote", "amount": "-5", "kind": "expense" }
        ] }),
        &owner.cookie,
    )
    .await;

    // 3. Import CSV (preview + confirm).
    let (b64, sha) = preview_csv(&app, &owner.cookie, "IMPORTADA", "-9", 15).await;
    let conf = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                    "decisions": [ { "kind": "expense" } ], "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    let import_id = conf.json()["import_id"].as_str().unwrap().to_string();

    // 4. PATCH + 5. delete_import + 6. DELETE de la manual.
    app.patch_json_with_cookie(
        &format!("/v1/transactions/{txn_id}"),
        json!({ "notes": "editada" }),
        &owner.cookie,
    )
    .await;
    app.delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie).await;

    // 7. Alta con recurrencia + 8. materialize + 9. borrado de la regla.
    let rec = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-15", "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": { "day_of_month": 15 } }),
            &owner.cookie,
        )
        .await;
    let rule_id = rec.json()["recurring_rule_id"].as_str().unwrap().to_string();
    app.post_json_with_cookie("/v1/transactions/recurring/materialize", json!({}), &owner.cookie)
        .await;
    app.delete_with_cookie(&format!("/v1/transactions/recurring/{rule_id}"), &owner.cookie)
        .await;

    // Margen para cualquier tarea de fondo (no debería haber ninguna que invalide en modo A).
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        present(&app, &key).await,
        "modo A: las mutaciones de transacciones NO deben invalidar la cache (no son inputs del engine)"
    );
}

// ---------------------------------------------------------------------------
// Modo B (`transactions_avg`): CADA mutación invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_each_mutation_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    let iid = installation_id(&app).await;
    let key = household_key(iid);

    // 1. create
    warm(&app, &owner.cookie, &key).await;
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-05-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let txn_id = created.json()["id"].as_str().unwrap().to_string();
    assert_invalidated(&app, &key, "create").await;

    // 2. batch
    warm(&app, &owner.cookie, &key).await;
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2026-05-11", "concept": "Lote", "amount": "-5", "kind": "expense" }
        ] }),
        &owner.cookie,
    )
    .await;
    assert_invalidated(&app, &key, "batch").await;

    // 3. patch
    warm(&app, &owner.cookie, &key).await;
    app.patch_json_with_cookie(
        &format!("/v1/transactions/{txn_id}"),
        json!({ "notes": "editada" }),
        &owner.cookie,
    )
    .await;
    assert_invalidated(&app, &key, "patch").await;

    // 4. import confirm
    warm(&app, &owner.cookie, &key).await;
    let (b64, sha) = preview_csv(&app, &owner.cookie, "IMPORTADA", "-9", 15).await;
    let conf = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                    "decisions": [ { "kind": "expense" } ], "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    let import_id = conf.json()["import_id"].as_str().unwrap().to_string();
    assert_invalidated(&app, &key, "import confirm").await;

    // 5. delete import (cascadea sus transacciones)
    warm(&app, &owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    assert_invalidated(&app, &key, "delete import").await;

    // 6. delete
    warm(&app, &owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie).await;
    assert_invalidated(&app, &key, "delete").await;

    // 7. materialize (regla con cursor un par de meses atrás → genera ≥1 instancia). Fechas
    // relativas al "hoy" del servidor para que el assert no dependa del reloj de la máquina.
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let op_date = NaiveDate::from_ymd_opt(oy, om, 15).unwrap();
    let rec = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op_date.format("%Y-%m-%d").to_string(), "concept": "Nomina",
                    "amount": "1500", "kind": "income", "recurrence": { "day_of_month": 15 } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(rec.status, http::StatusCode::CREATED);
    let rule_id = rec.json()["recurring_rule_id"].as_str().unwrap().to_string();
    // El alta con fecha pasada ya backfillea hasta hoy; rebobinamos por SQL (cursor a un mes previo
    // al origen, sin instancias) para que el ENDPOINT materialize vuelva a generar ≥1 y podamos
    // probar que invalida en modo B.
    let rid = Uuid::parse_str(&rule_id).unwrap();
    sqlx::query("DELETE FROM transactions WHERE recurring_rule_id = $1")
        .bind(rid)
        .execute(&app.pool)
        .await
        .expect("clear rule instances");
    let (cy, cm) = shift_month(oy, om, -1);
    let cursor = NaiveDate::from_ymd_opt(cy, cm, 1).unwrap();
    sqlx::query("UPDATE recurring_transaction_rules SET last_materialized_month = $1 WHERE id = $2")
        .bind(cursor)
        .bind(rid)
        .execute(&app.pool)
        .await
        .expect("rewind cursor");
    warm(&app, &owner.cookie, &key).await;
    let mat = app
        .post_json_with_cookie("/v1/transactions/recurring/materialize", json!({}), &owner.cookie)
        .await;
    assert!(mat.json()["materialized"].as_u64().unwrap() >= 1, "materialize debe generar ≥1: {mat:?}");
    assert_invalidated(&app, &key, "materialize").await;

    // 8. borrar la regla recurrente NO invalida (instancias sobreviven, conjunto sin cambios).
    warm(&app, &owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/recurring/{rule_id}"), &owner.cookie)
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        present(&app, &key).await,
        "modo B: borrar una regla recurrente NO cambia el conjunto de transacciones → no debe invalidar"
    );
}

// ---------------------------------------------------------------------------
// Flip A↔B vía PATCH /v1/installation invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flipping_savings_source_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    let iid = installation_id(&app).await;
    let key = household_key(iid);

    // A → B
    warm(&app, &owner.cookie, &key).await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    assert_invalidated(&app, &key, "flip A→B").await;

    // B → A
    warm(&app, &owner.cookie, &key).await;
    set_mode(&app, &owner.cookie, "budget").await;
    assert_invalidated(&app, &key, "flip B→A").await;
}

// ---------------------------------------------------------------------------
// Modo C (`budget_income_real_expense`): también usa transacciones → invalida (paridad con B)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_c_mutation_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    set_mode(&app, &owner.cookie, "budget_income_real_expense").await;

    let iid = installation_id(&app).await;
    let key = household_key(iid);

    warm(&app, &owner.cookie, &key).await;
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-05-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    assert_invalidated(&app, &key, "modo C create").await;
}
