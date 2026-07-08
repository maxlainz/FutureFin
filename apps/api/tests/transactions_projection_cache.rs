//! Regresión del contrato no-cache del módulo `transactions`: las transacciones NO son inputs
//! del motor de proyección, así que ninguna de sus mutaciones (import, alta manual, PATCH,
//! DELETE, borrado de lote) debe invalidar la cache de proyección. Espejo de
//! `snapshot_mutations_do_not_touch_projection_cache` en `history_snapshots.rs`.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
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

#[tokio::test]
async fn transaction_mutations_do_not_touch_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    // Calentar la cache household (monthly) con un GET.
    let warm = app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    assert_eq!(warm.status, http::StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let iid = installation_id(&app).await;
    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Monthly,
    };
    {
        let cache = app.state.projection_cache.read().await;
        assert!(cache.contains_key(&key), "household caliente antes de las mutaciones");
    }

    // --- Batería de mutaciones de transacciones ---
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

    // 2. Import CSV.
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;IMPORTADA;-9;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    app.post_json_with_cookie(
        "/v1/transactions/import/confirm",
        json!({ "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense" } ], "learn_rules": true }),
        &owner.cookie,
    )
    .await;

    // 3. PATCH + 4. DELETE de la manual.
    app.patch_json_with_cookie(
        &format!("/v1/transactions/{txn_id}"),
        json!({ "notes": "editada" }),
        &owner.cookie,
    )
    .await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie).await;

    // 5. Alta con recurrencia (crea regla + instancia de origen enlazada).
    let rec = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-15", "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": { "day_of_month": 15 } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(rec.status, http::StatusCode::CREATED);
    let rule_id = rec.json()["recurring_rule_id"].as_str().unwrap().to_string();

    // 6. Materialize + 7. borrado de la regla.
    app.post_json_with_cookie(
        "/v1/transactions/recurring/materialize",
        json!({}),
        &owner.cookie,
    )
    .await;
    app.delete_with_cookie(&format!("/v1/transactions/recurring/{rule_id}"), &owner.cookie)
        .await;

    // Margen para cualquier tarea de fondo (no debería haber ninguna que invalide).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let cache = app.state.projection_cache.read().await;
    assert!(
        cache.contains_key(&key),
        "las mutaciones de transacciones NO deben invalidar la cache de proyección (no son inputs del engine)"
    );
}
