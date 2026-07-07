//! Integración del CRUD de transacciones (`/v1/transactions`, `/months`, `/imports`).
//!
//! Cubre alta manual (individual + batch), savings sin categoría, mismatch de scope, PATCH con
//! campos inmutables en importadas, recompute de huella en manuales, `ON DELETE SET NULL` de los
//! links, remap de categoría (incluye transacciones), meses con datos, y borrado de lotes.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::TestApp;
use serde_json::{json, Value};

async fn create_manual(app: &TestApp, cookie: &str, body: Value) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/transactions", body, cookie).await
}

/// Importa una única transacción de gasto y devuelve su id.
async fn import_one_expense(app: &TestApp, cookie: &str) -> String {
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;TIENDA IMPORTADA;-9;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            cookie,
        )
        .await;
    let pb = p.json();
    let sha = pb["file_sha256"].as_str().unwrap();
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense" } ], "learn_rules": false,
            }),
            cookie,
        )
        .await;
    assert_eq!(c.status, http::StatusCode::OK, "import: {c:?}");
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", cookie)
        .await;
    list.json()[0]["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Alta manual + listado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manual_create_and_list() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    let r = create_manual(
        &app,
        &owner.cookie,
        json!({
            "op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
            "kind": "expense", "category_id": cat, "notes": "efectivo"
        }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create: {r:?}");
    let b = r.json();
    assert_eq!(b["source"], "manual");
    assert!(b["import_id"].is_null(), "manual → import_id null");
    assert_eq!(b["kind"], "expense");
    assert_eq!(b["category_name"], "Ocio");
    assert_eq!(b["amount"].as_str().unwrap().parse::<f64>().unwrap(), -12.5);

    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await;
    assert_eq!(list.json().as_array().unwrap().len(), 1);
    // Otro mes → vacío.
    let empty = app
        .get_with_cookie("/v1/transactions?month=2026-05", &owner.cookie)
        .await;
    assert_eq!(empty.json().as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn batch_create() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [
                { "op_date": "2026-06-01", "concept": "A", "amount": "-5", "kind": "expense" },
                { "op_date": "2026-06-02", "concept": "B", "amount": "1000", "kind": "income" },
                { "op_date": "2026-06-03", "concept": "C", "amount": "-200", "kind": "savings" },
            ] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "batch: {r:?}");
    assert_eq!(r.json().as_array().unwrap().len(), 3);
    assert_eq!(app.count_rows("transactions").await, 3);
}

// ---------------------------------------------------------------------------
// Validaciones de scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn savings_requires_null_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "X").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Aporte", "amount": "-100",
                "kind": "savings", "category_id": cat }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("savings_no_category"));
}

#[tokio::test]
async fn expense_with_income_category_mismatch() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-100",
                "kind": "expense", "category_id": income_cat }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("category_scope_mismatch"));
}

#[tokio::test]
async fn zero_amount_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "0", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("amount_zero"));
}

// ---------------------------------------------------------------------------
// PATCH: inmutabilidad en importadas, mutabilidad + recompute en manuales
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_imported_op_date_is_immutable() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = import_one_expense(&app, &owner.cookie).await;

    // Cambiar kind/categoría/notas → OK.
    let ok = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "kind": "expense", "notes": "revisado" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::OK, "patch kind/notes: {ok:?}");

    // Cambiar op_date en una importada → 400 immutable_field.
    let bad = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "op_date": "2020-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST);
    assert!(bad.json()["message"].as_str().unwrap().contains("immutable_field"));
}

#[tokio::test]
async fn patch_manual_amount_recomputes_and_allows_reuse() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Manual con importe -5.
    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    let id = created.json()["id"].as_str().unwrap().to_string();

    // Cambiar el importe a -7 → recomputa huella.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "amount": "-7" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "patch amount: {patched:?}");
    assert_eq!(patched.json()["amount"].as_str().unwrap().parse::<f64>().unwrap(), -7.0);

    // La huella original (-5) quedó libre → crear otra idéntica no colisiona.
    let again = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    assert_eq!(again.status, http::StatusCode::CREATED, "reuse freed fingerprint: {again:?}");
}

// ---------------------------------------------------------------------------
// Links ON DELETE SET NULL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn linked_asset_set_null_on_asset_delete() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let acat = app.create_category(&owner, "asset", "Cartera").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": acat, "name": "Fondo", "current_value": "1000" }),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();

    let txn = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Aporte", "amount": "-200",
                "kind": "savings", "linked_asset_id": asset_id }),
    )
    .await;
    assert_eq!(txn.status, http::StatusCode::CREATED, "{txn:?}");
    let txn_id = txn.json()["id"].as_str().unwrap().to_string();
    assert_eq!(txn.json()["linked_asset_id"], json!(asset_id));

    // Borrar el asset → el movimiento sobrevive con linked_asset_id NULL.
    let del = app.delete_with_cookie(&format!("/v1/assets/{asset_id}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);

    let after = app
        .get_with_cookie(&format!("/v1/transactions?month=2026-06"), &owner.cookie)
        .await;
    let arr = after.json();
    let t = arr.as_array().unwrap().iter().find(|t| t["id"] == json!(txn_id)).unwrap().clone();
    assert!(t.get("linked_asset_id").is_none(), "linked_asset_id debe ser NULL tras borrar el asset");
    assert_eq!(app.count_rows("transactions").await, 1, "el movimiento sobrevive");
}

// ---------------------------------------------------------------------------
// Remap de categoría incluye transacciones
// ---------------------------------------------------------------------------

#[tokio::test]
async fn category_delete_remaps_transactions() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_a = app.create_category(&owner, "expense", "A").await;
    let cat_b = app.create_category(&owner, "expense", "B").await;

    let txn = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10",
                "kind": "expense", "category_id": cat_a }),
    )
    .await;
    let txn_id = txn.json()["id"].as_str().unwrap().to_string();

    // Borrar A sin remap → 400 (en uso por la transacción, ON DELETE RESTRICT).
    let bad = app.delete_with_cookie(&format!("/v1/categories/{cat_a}"), &owner.cookie).await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST, "categoría en uso: {bad:?}");

    // Borrar A con remap a B → la transacción pasa a B.
    let ok = app
        .delete_with_cookie(&format!("/v1/categories/{cat_a}?remap_to={cat_b}"), &owner.cookie)
        .await;
    assert_eq!(ok.status, http::StatusCode::NO_CONTENT, "remap: {ok:?}");

    let list = app.get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie).await;
    let t = list.json();
    let found = t.as_array().unwrap().iter().find(|t| t["id"] == json!(txn_id)).unwrap().clone();
    assert_eq!(found["category_id"], json!(cat_b), "transacción remapeada a B");
}

// ---------------------------------------------------------------------------
// Meses, delete, cross-user, viewer, batches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn months_endpoint_marks_complete() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2020-01-15", "concept": "Viejo", "amount": "-5", "kind": "expense" },
            { "op_date": "2020-02-15", "concept": "Viejo2", "amount": "-5", "kind": "expense" },
        ] }),
        &owner.cookie,
    )
    .await;
    let m = app.get_with_cookie("/v1/transactions/months", &owner.cookie).await;
    let arr = m.json();
    let months = arr.as_array().unwrap();
    assert_eq!(months.len(), 2);
    // Orden DESC.
    assert_eq!(months[0]["month"], "2020-02");
    assert_eq!(months[1]["month"], "2020-01");
    // Meses de 2020 son completos (no el mes en curso).
    assert_eq!(months[0]["is_complete"], true);
    assert_eq!(months[0]["txn_count"], 1);
}

#[tokio::test]
async fn delete_transaction_and_cross_user_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10", "kind": "expense" }),
    )
    .await;
    let id = created.json()["id"].as_str().unwrap().to_string();

    // Bob no puede tocar el movimiento de Alice → 404.
    let bob_del = app.delete_with_cookie(&format!("/v1/transactions/{id}"), &bob.cookie).await;
    assert_eq!(bob_del.status, http::StatusCode::NOT_FOUND, "cross-user delete");

    let del = app.delete_with_cookie(&format!("/v1/transactions/{id}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("transactions").await, 0);
}

#[tokio::test]
async fn viewer_cannot_create_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vic", "viewer").await;
    let r = create_manual(
        &app,
        &viewer.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_import_cascades_and_requires_confirm() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let _id = import_one_expense(&app, &owner.cookie).await;
    assert_eq!(app.count_rows("transactions").await, 1);
    assert_eq!(app.count_rows("transaction_imports").await, 1);

    let imports = app.get_with_cookie("/v1/transactions/imports", &owner.cookie).await;
    let batch = imports.json();
    let import_id = batch[0]["id"].as_str().unwrap().to_string();
    assert_eq!(batch[0]["txn_count"], 1);

    // Sin confirm → 400.
    let no_confirm = app
        .delete_with_cookie(&format!("/v1/transactions/imports/{import_id}"), &owner.cookie)
        .await;
    assert_eq!(no_confirm.status, http::StatusCode::BAD_REQUEST);
    assert!(no_confirm.json()["message"].as_str().unwrap().contains("confirm_required"));

    // Con confirm → 204 + cascade.
    let ok = app
        .delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    assert_eq!(ok.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("transactions").await, 0, "cascade a transacciones");
    assert_eq!(app.count_rows("transaction_imports").await, 0);
}
