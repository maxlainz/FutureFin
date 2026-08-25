//! Integration tests for `.ffbackup` schema_version 4 (history snapshots).
//!
//! Covers the round-trip of history snapshots through export → import, the
//! `ledger_index`/`item_key` re-link mechanism, backward compatibility with v3
//! files, out-of-bounds rejection with rollback, the projection-cache
//! invalidation fix on import, preview counts, and the viewer 403 guard.
//!
//! Requires a running Postgres at `TEST_DATABASE_URL` (see `common/mod.rs`).

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{Datelike, NaiveDate};
use common::{ResponseParts, TestApp};
use futurefin_api::handlers::backup_user::crypto::{encrypt_payload, frame_file};
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use uuid::Uuid;

const PW: &str = "correct horse battery staple";

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

async fn create_asset(app: &TestApp, cookie: &str, cat: &str, name: &str, value: &str) -> Uuid {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({ "category_id": cat, "name": name, "current_value": value }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create asset: {r:?}");
    Uuid::parse_str(r.json()["id"].as_str().expect("asset id")).expect("asset uuid")
}

async fn create_liability(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    exp_cat: &str,
    label: &str,
    principal: &str,
) -> Uuid {
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat,
                "expense_category_id": exp_cat,
                "label": label,
                "principal": principal,
                "apr_percent": "3.5",
                "payment_amount": "500",
                "payment_frequency": "monthly",
            }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create liability: {r:?}");
    Uuid::parse_str(r.json()["id"].as_str().expect("liability id")).expect("liability uuid")
}

/// Calls the real export endpoint and returns the framed `.ffbackup` bytes, base64-encoded.
async fn export_ffbackup_b64(app: &TestApp, cookie: &str) -> String {
    let r = app
        .post_json_with_cookie("/v1/backup/user-export", serde_json::json!({ "password": PW }), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "export: status {:?}", r.status);
    B64.encode(&r.body)
}

async fn import_apply(app: &TestApp, cookie: &str, file_b64: &str) -> ResponseParts {
    app.post_json_with_cookie(
        "/v1/backup/user-import",
        serde_json::json!({ "file_b64": file_b64, "password": PW, "confirm_replace": true }),
        cookie,
    )
    .await
}

async fn import_preview(app: &TestApp, cookie: &str, file_b64: &str) -> ResponseParts {
    app.post_json_with_cookie(
        "/v1/backup/user-import/preview",
        serde_json::json!({ "file_b64": file_b64, "password": PW }),
        cookie,
    )
    .await
}

/// Encrypts an arbitrary plaintext payload at a given schema_version and returns the framed
/// `.ffbackup` bytes base64-encoded — used to hand-craft v3 files and malformed v4 files.
fn craft_ffbackup_b64(schema_version: u32, payload: &serde_json::Value, user_id: Uuid) -> String {
    let plaintext = serde_json::to_vec(payload).expect("serialize payload");
    let enc = encrypt_payload(
        &plaintext,
        PW,
        "1.5.0-test",
        schema_version,
        &user_id.to_string(),
        "alice",
        "2026-07-06T00:00:00Z",
    )
    .expect("encrypt");
    let framed = frame_file(&enc.manifest, &enc.ciphertext).expect("frame");
    B64.encode(&framed)
}

fn installation_snapshot_json() -> serde_json::Value {
    serde_json::json!({
        "base_currency": "EUR",
        "calendar_tz": "UTC",
        "show_age_mode": "dates",
        "fire_settings": {
            "fire_number_mode": "annual_expense",
            "fire_number_manual_amount": null,
            "fire_number_expense_adjustment_pct": null,
            "swr_pct": "3.5",
            "taxes_enabled": true,
            "tax_brackets": [ { "up_to": null, "pct": "19" } ]
        }
    })
}

fn past_ymd(days: i64) -> String {
    (chrono::Utc::now().date_naive() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

async fn installation_id(app: &TestApp) -> Uuid {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("installation id")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// End-to-end: seed a ledger + snapshots, save the interpolated history series, export,
/// dirty the live state, import (full replace), and confirm the series is byte-identical.
///
/// The comparison is on `points` (the net-worth series) — the load-bearing invariant. The
/// per-asset `asset_id`s legitimately change (import mints fresh ledger UUIDs and the re-link
/// rewrites `source_item_id` to them), so `asset_series[].asset_id` is intentionally excluded.
#[tokio::test]
async fn backup_v4_roundtrip_series_identical() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    let liab_cat = app.create_category(&owner, "liability", "Loan").await;
    // Categoría de gasto usada SOLO por expense_category_id: cubre el caso que sin la cláusula
    // nueva de fetch_categories_used haría fallar el import (resolve_category hard-fail).
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let ua = create_asset(&app, &owner.cookie, &asset_cat, "A", "10000").await;
    let ub = create_asset(&app, &owner.cookie, &asset_cat, "B", "5000").await;
    let ul = create_liability(&app, &owner.cookie, &liab_cat, &liab_exp_cat, "L", "20000").await;

    // Backfill a past date for both kinds (item_id = live ledger id → links to today).
    let past = past_ymd(200);
    let r = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": past,
                "items": [
                    { "item_id": ua.to_string(), "label": "A", "value": "8000" },
                    { "item_id": ub.to_string(), "label": "B", "value": "4000" }
                ]
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "backfill asset: {r:?}");
    let r = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "liability",
                "snapshot_date": past,
                "items": [
                    { "item_id": ul.to_string(), "label": "L", "value": "22000",
                      "apr_percent": "3.5", "payment_amount": "500", "payment_frequency": "monthly" }
                ]
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "backfill liability: {r:?}");

    // Capture today (both kinds).
    let r = app
        .post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "capture: {r:?}");

    let series_before = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    assert_eq!(series_before.status, http::StatusCode::OK, "series before: {series_before:?}");
    let points_before = series_before.json()["points"].clone();
    let markers_before = series_before.json()["markers"].as_array().unwrap().len();

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;

    // Dirty the live state: delete an asset and add a stray snapshot the backup does not know.
    let del = app.delete_with_cookie(&format!("/v1/assets/{ub}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "delete asset: {del:?}");
    let stray = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": past_ymd(100),
                "items": [ { "label": "stray", "value": "1" } ]
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(stray.status, http::StatusCode::CREATED, "stray snapshot: {stray:?}");

    // Import replaces everything from the backup.
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "import apply: {applied:?}");
    let counts = &applied.json()["imported"];
    assert_eq!(counts["assets"].as_u64(), Some(2), "assets count");
    assert_eq!(counts["liabilities"].as_u64(), Some(1), "liabilities count");
    assert_eq!(counts["snapshots"].as_u64(), Some(4), "snapshots count");
    assert_eq!(counts["snapshot_items"].as_u64(), Some(6), "snapshot_items count");

    let series_after = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    assert_eq!(series_after.status, http::StatusCode::OK, "series after: {series_after:?}");
    assert_eq!(
        series_after.json()["points"],
        points_before,
        "history net-worth series must be identical after a backup round-trip"
    );
    assert_eq!(
        series_after.json()["markers"].as_array().unwrap().len(),
        markers_before,
        "marker count must survive the round-trip"
    );

    // 3.4.0: la categoría de gasto de la cuota sobrevive el round-trip (`expense_category_ref`
    // por (scope, name)); además "Cuotas" solo la usa `expense_category_id`, así que este assert
    // cubre la cláusula nueva de `fetch_categories_used` — sin ella el import habría fallado.
    let liabs_after = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    let l = liabs_after.as_array().unwrap().first().expect("liability after import").clone();
    let exp_cat_after = l["expense_category_id"].as_str().expect("expense_category_id viaja");
    let cats = app.get_with_cookie("/v1/categories?scope=expense", &owner.cookie).await.json();
    let cuotas = cats
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Cuotas")
        .expect("categoría Cuotas reimportada");
    assert_eq!(exp_cat_after, cuotas["id"].as_str().unwrap(), "re-vinculada por (scope, name)");
}

/// After import, every asset-snapshot item that carried a `ledger_index` re-links to a FRESH,
/// live asset UUID (never a stale pre-export id).
#[tokio::test]
async fn backup_v4_items_relink_to_fresh_asset_uuids() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let ua = create_asset(&app, &owner.cookie, &cat, "A", "10000").await;
    let ub = create_asset(&app, &owner.cookie, &cat, "B", "5000").await;

    // Capture → both items reference live assets, so both get a ledger_index at export.
    let r = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({ "kinds": ["asset"] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "capture: {r:?}");

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "import: {applied:?}");

    // No asset-snapshot item points at a source_item_id that is not a live asset of the user.
    let dangling: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM history_snapshot_items i
           JOIN history_snapshots s ON s.id = i.snapshot_id
           WHERE s.kind = 'asset' AND s.owner_user_id = $1
             AND i.source_item_id NOT IN (SELECT id FROM assets WHERE owner_user_id = $1)"#,
    )
    .bind(owner.user_id)
    .fetch_one(&app.pool)
    .await
    .expect("count dangling");
    assert_eq!(dangling, 0, "every re-linked item must point at a live asset");

    // And the re-link genuinely minted new UUIDs (old ids are gone).
    let old_ids_present: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM history_snapshot_items WHERE source_item_id = ANY($1)"#,
    )
    .bind(vec![ua, ub])
    .fetch_one(&app.pool)
    .await
    .expect("count old ids");
    assert_eq!(old_ids_present, 0, "pre-export asset UUIDs must not survive the re-link");
}

/// An item whose source row was deleted before export gets `ledger_index = None`, so on import
/// its `item_key` is preserved verbatim (no re-link).
#[tokio::test]
async fn backup_v4_null_ledger_index_imports_null_relink() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let ua = create_asset(&app, &owner.cookie, &cat, "A", "10000").await;

    // Capture (item references asset A), then delete A: at export A is gone → ledger_index None.
    let r = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({ "kinds": ["asset"] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "capture: {r:?}");
    let del = app.delete_with_cookie(&format!("/v1/assets/{ua}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "delete asset: {del:?}");

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "import: {applied:?}");

    // The single snapshot item still carries the original (deleted) asset UUID verbatim.
    let preserved: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM history_snapshot_items WHERE source_item_id = $1"#,
    )
    .bind(ua)
    .fetch_one(&app.pool)
    .await
    .expect("count preserved");
    assert_eq!(preserved, 1, "deleted-row item_key must be preserved verbatim");
}

/// A v3 `.ffbackup` (no snapshots) still imports; snapshots count is 0.
#[tokio::test]
async fn backup_v3_file_still_imports() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": "1990-01-01" },
        "categories_used": [{ "scope": "asset", "name": "Cash", "sort_index": 0 }],
        "assets": [{
            "category_ref": { "scope": "asset", "name": "Cash" },
            "name": "Cuenta",
            "current_value": "1234.00",
            "is_liquid": true,
            "sort_index": 0
        }],
        "allocation_rules": [],
        "liabilities": [],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json()
    });
    let b64 = craft_ffbackup_b64(3, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::OK, "v3 import: {applied:?}");
    assert_eq!(applied.json()["imported"]["assets"].as_u64(), Some(1));
    assert_eq!(applied.json()["imported"]["snapshots"].as_u64(), Some(0));
    assert_eq!(applied.json()["imported"]["snapshot_items"].as_u64(), Some(0));

    let snap_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_snapshots")
        .fetch_one(&app.pool)
        .await
        .expect("count snapshots");
    assert_eq!(snap_rows, 0, "v3 import must create no snapshots");
}

/// A v4 file with an out-of-bounds `ledger_index` is rejected 400 and the transaction rolls
/// back, leaving the user's pre-import rows intact.
#[tokio::test]
async fn backup_import_out_of_bounds_item_index_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    create_asset(&app, &owner.cookie, &cat, "Original", "7777").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": null },
        "categories_used": [{ "scope": "asset", "name": "Cash", "sort_index": 0 }],
        "assets": [{
            "category_ref": { "scope": "asset", "name": "Cash" },
            "name": "FromBackup",
            "current_value": "100.00",
            "is_liquid": true,
            "sort_index": 0
        }],
        "allocation_rules": [],
        "liabilities": [],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json(),
        "snapshots": [{
            "kind": "asset",
            "snapshot_date": "2025-01-01",
            "source": "backfill",
            "items": [ { "ledger_index": 99, "item_key": Uuid::new_v4().to_string(), "label": "X", "value": "100" } ]
        }]
    });
    let b64 = craft_ffbackup_b64(4, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::BAD_REQUEST, "expected 400: {applied:?}");

    // Rollback: original data intact, backup data not committed.
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM assets ORDER BY name")
        .fetch_all(&app.pool)
        .await
        .expect("asset names");
    assert_eq!(names, vec!["Original".to_string()], "pre-import asset must survive rollback");
    let snap_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_snapshots")
        .fetch_one(&app.pool)
        .await
        .expect("count snapshots");
    assert_eq!(snap_rows, 0, "no snapshot may be committed on a rolled-back import");
}

/// A v4 file with an invalid snapshot `kind` ("assets") is rejected 400 (not a 500 from the
/// `history_snapshots` CHECK constraint) and the transaction rolls back, leaving pre-import rows
/// intact.
#[tokio::test]
async fn backup_import_invalid_snapshot_kind_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    create_asset(&app, &owner.cookie, &cat, "Original", "7777").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": null },
        "categories_used": [{ "scope": "asset", "name": "Cash", "sort_index": 0 }],
        "assets": [{
            "category_ref": { "scope": "asset", "name": "Cash" },
            "name": "FromBackup",
            "current_value": "100.00",
            "is_liquid": true,
            "sort_index": 0
        }],
        "allocation_rules": [],
        "liabilities": [],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json(),
        "snapshots": [{
            "kind": "assets",
            "snapshot_date": "2025-01-01",
            "source": "backfill",
            "items": [ { "ledger_index": null, "item_key": Uuid::new_v4().to_string(), "label": "X", "value": "100" } ]
        }]
    });
    let b64 = craft_ffbackup_b64(4, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(
        applied.status,
        http::StatusCode::BAD_REQUEST,
        "invalid snapshot kind must be 400 not 500: {applied:?}"
    );
    assert!(
        applied.json()["message"].as_str().unwrap().contains("snapshot_kind_invalid"),
        "message must name the offending field: {:?}",
        applied.json()
    );

    // Rollback: original data intact, backup data not committed.
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM assets ORDER BY name")
        .fetch_all(&app.pool)
        .await
        .expect("asset names");
    assert_eq!(names, vec!["Original".to_string()], "pre-import asset must survive rollback");
    let snap_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_snapshots")
        .fetch_one(&app.pool)
        .await
        .expect("count snapshots");
    assert_eq!(snap_rows, 0, "no snapshot may be committed on a rolled-back import");
}

/// Importing a backup invalidates the household projection cache (previously a stale-cache bug).
#[tokio::test]
async fn backup_import_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    create_asset(&app, &owner.cookie, &cat, "A", "10000").await;

    let iid = installation_id(&app).await;
    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Household,
        owner_user_id: Some(owner.user_id),
        density: Density::Monthly,
    };

    // Warm the household cache.
    let r = app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK);
    {
        let cache = app.state.projection_cache.read().await;
        assert!(cache.contains_key(&key), "household projection should be cached after a GET");
    }

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "import: {applied:?}");

    // Since 3.8.0 the invalidation is awaited inside the handler: by the time the import
    // responded, the cache state is already final.
    {
        let cache = app.state.projection_cache.read().await;
        assert!(
            !cache.contains_key(&key),
            "import must invalidate the projection cache (stale-cache bug fix)"
        );
    }
}

/// Preview surfaces snapshot header and item counts.
#[tokio::test]
async fn preview_reports_snapshot_counts() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    create_asset(&app, &owner.cookie, &cat, "A", "10000").await;

    let r = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({ "kinds": ["asset"] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "capture: {r:?}");

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;
    let preview = import_preview(&app, &owner.cookie, &backup).await;
    assert_eq!(preview.status, http::StatusCode::OK, "preview: {preview:?}");
    assert_eq!(preview.json()["counts"]["snapshots"].as_u64(), Some(1), "preview snapshots");
    assert_eq!(preview.json()["counts"]["snapshot_items"].as_u64(), Some(1), "preview snapshot_items");
}

/// End-to-end v5: seed a CSV import (batch + transaction + learned rule) plus a manual
/// savings transaction linked to an asset, export, dirty, import (full replace), and confirm the
/// counts, the transactions/rules survival, and the re-link of category/asset/import refs to the
/// fresh ledger UUIDs.
#[tokio::test]
async fn backup_v5_transactions_round_trip() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    let cash = create_asset(&app, &owner.cookie, &asset_cat, "Cuenta", "5000").await;

    // Import one CSV expense row (categorized as Super, batch linked to the Cash account).
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;SUPERMERCADO ALMENDRO;-9;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            serde_json::json!({ "source": "myinvestor", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    let conf = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            serde_json::json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense", "category_id": super_cat } ],
                "learn_rules": true, "account_asset_id": cash.to_string(),
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(conf.status, http::StatusCode::OK, "import: {conf:?}");
    assert_eq!(conf.json()["rules_learned"].as_u64(), Some(1));

    // A manual savings transaction linked to the Cash asset.
    let man = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-06-20", "concept": "Aporte", "amount": "-100",
                                "kind": "savings", "linked_asset_id": cash.to_string() }),
            &owner.cookie,
        )
        .await;
    assert_eq!(man.status, http::StatusCode::CREATED, "manual: {man:?}");

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;

    // Dirty: add a stray transaction the backup does not know.
    app.post_json_with_cookie(
        "/v1/transactions",
        serde_json::json!({ "op_date": "2026-07-01", "concept": "Stray", "amount": "-1", "kind": "expense" }),
        &owner.cookie,
    )
    .await;

    // Preview reports the v5 counts.
    let preview = import_preview(&app, &owner.cookie, &backup).await;
    assert_eq!(preview.json()["counts"]["transaction_imports"].as_u64(), Some(1));
    assert_eq!(preview.json()["counts"]["transactions"].as_u64(), Some(2));
    assert_eq!(preview.json()["counts"]["categorization_rules"].as_u64(), Some(1));

    // Apply replaces everything from the backup.
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "apply: {applied:?}");
    let counts = &applied.json()["imported"];
    assert_eq!(counts["transaction_imports"].as_u64(), Some(1), "imports count");
    assert_eq!(counts["transactions"].as_u64(), Some(2), "transactions count");
    assert_eq!(counts["categorization_rules"].as_u64(), Some(1), "rules count");

    // Exactly the 2 backed-up transactions survive (the stray is gone).
    assert_eq!(app.count_rows("transactions").await, 2, "solo las 2 del backup");
    assert_eq!(app.count_rows("transaction_imports").await, 1);
    assert_eq!(app.count_rows("categorization_rules").await, 1);

    // The learned rule survives and points at the live Super category.
    let rules = app.get_with_cookie("/v1/transactions/rules", &owner.cookie).await;
    let rb = rules.json();
    assert_eq!(rb.as_array().unwrap().len(), 1);
    let super_now: String = sqlx::query_scalar(
        "SELECT id::text FROM categories WHERE installation_id = (SELECT id FROM installation LIMIT 1) AND name = 'Super'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("super cat");
    assert_eq!(rb[0]["assign_category_id"].as_str().unwrap(), super_now, "rule re-linked to live category");

    // No dangling links: every non-null linked_asset_id / category_id / import_id points at a
    // live row of the user (the re-link minted fresh UUIDs).
    let dangling: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM transactions t
           WHERE t.owner_user_id = $1
             AND (
               (t.linked_asset_id IS NOT NULL AND t.linked_asset_id NOT IN (SELECT id FROM assets))
               OR (t.category_id IS NOT NULL AND t.category_id NOT IN (SELECT id FROM categories))
               OR (t.import_id IS NOT NULL AND t.import_id NOT IN (SELECT id FROM transaction_imports))
             )"#,
    )
    .bind(owner.user_id)
    .fetch_one(&app.pool)
    .await
    .expect("count dangling");
    assert_eq!(dangling, 0, "sin refs colgantes tras el re-link");
}

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

/// End-to-end: create a recurring rule + materialize it, export, dirty, import (full replace),
/// and confirm the rule survives with the SAME anchor (`origin_month`, v9), its instances are
/// re-linked to the fresh rule UUID, and a post-import convergence does not duplicate anything.
#[tokio::test]
async fn backup_recurring_rules_round_trip() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let nomina_cat = app.create_category(&owner, "income", "Nómina").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let origin = NaiveDate::from_ymd_opt(oy, om, 1).unwrap().format("%Y-%m-%d").to_string();

    // 3.9.0: solo los meses ACTIVOS materializan → sembramos un movimiento real en M-1 antes.
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);
    let iid = app.installation_id().await;
    sqlx::query(
        "INSERT INTO transactions (installation_id, owner_user_id, source, op_date, concept, \
         amount, currency, kind, fingerprint, fingerprint_ordinal) \
         VALUES ($1, $2, 'manual', $3, 'Activador', -1, 'EUR', 'expense', 'fp-act', 0)",
    )
    .bind(iid)
    .bind(owner.user_id)
    .bind(NaiveDate::from_ymd_opt(m1y, m1m, 15).unwrap())
    .execute(&app.pool)
    .await
    .expect("activate M-1");

    // Rule (origin 2 months ago) + materialize (fills the ACTIVE closed month M-1; the current
    // month is never materialized since 3.2.0).
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": origin, "concept": "Nomina", "amount": "1500",
                                "kind": "income", "category_id": nomina_cat,
                                "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create rule: {r:?}");
    let mat = app
        .post_json_with_cookie("/v1/transactions/recurring/materialize", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(app.count_rows("transactions").await, 3, "activador + origen + 1 materializada (M-1)");

    let anchor_before = app.get_with_cookie("/v1/transactions/recurring", &owner.cookie).await.json()
        [0]["origin_month"]
        .as_str()
        .unwrap()
        .to_string();

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;

    // Dirty: add a stray manual transaction the backup does not know.
    app.post_json_with_cookie(
        "/v1/transactions",
        serde_json::json!({ "op_date": origin, "concept": "Stray", "amount": "-1", "kind": "expense" }),
        &owner.cookie,
    )
    .await;

    // Preview + apply report the v6 recurring-rule count.
    let preview = import_preview(&app, &owner.cookie, &backup).await;
    assert_eq!(preview.json()["counts"]["recurring_transaction_rules"].as_u64(), Some(1));
    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "apply: {applied:?}");
    assert_eq!(applied.json()["imported"]["recurring_transaction_rules"].as_u64(), Some(1));
    assert_eq!(applied.json()["imported"]["transactions"].as_u64(), Some(3));

    // Exactly the 3 backed-up transactions survive (the stray is gone).
    assert_eq!(app.count_rows("transactions").await, 3, "solo las 3 del backup");
    assert_eq!(app.count_rows("recurring_transaction_rules").await, 1);

    // The rule survives with the same cursor and its category re-linked.
    let rules = app.get_with_cookie("/v1/transactions/recurring", &owner.cookie).await.json();
    assert_eq!(rules.as_array().unwrap().len(), 1);
    assert_eq!(rules[0]["origin_month"].as_str().unwrap(), anchor_before, "ancla conservada");
    assert_eq!(rules[0]["category_name"], "Nómina", "categoría re-enlazada");

    // Every instance is re-linked to the fresh rule UUID (origin + 1 materialized = 2; the
    // activator carries no rule).
    let relinked: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM transactions
           WHERE recurring_rule_id IS NOT NULL
             AND recurring_rule_id IN (SELECT id FROM recurring_transaction_rules)"#,
    )
    .fetch_one(&app.pool)
    .await
    .expect("count relinked");
    assert_eq!(relinked, 2, "instancias re-enlazadas a la regla nueva");

    // Una convergencia post-import no duplica: el restore ya dejó el ledger en punto fijo
    // (el propio import corre converge post-commit).
    let mat2 = app
        .post_json_with_cookie("/v1/transactions/recurring/materialize", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(mat2.json()["materialized"].as_u64().unwrap(), 0, "sin duplicados tras import");
    assert_eq!(mat2.json()["pruned"].as_u64().unwrap(), 0, "nada que podar: M-1 sigue activo");
    assert_eq!(app.count_rows("transactions").await, 3);
}

/// A viewer cannot import (403) — enforced before any file parsing.
#[tokio::test]
async fn viewer_cannot_import_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vic", "viewer").await;

    let applied = import_apply(&app, &viewer.cookie, "AAAA").await;
    assert_eq!(applied.status, http::StatusCode::FORBIDDEN, "viewer import must be 403: {applied:?}");

    let preview = import_preview(&app, &viewer.cookie, "AAAA").await;
    assert_eq!(preview.status, http::StatusCode::FORBIDDEN, "viewer preview must be 403: {preview:?}");
}

// ---------------------------------------------------------------------------
// v8: conciliación de transferencias en el backup
// ---------------------------------------------------------------------------

/// Roundtrip v8 completo: un par conciliado y un par RECHAZADO (desconciliado a mano) sobreviven
/// a export → import. La aserción clave es la anti-resurrección: tras el restore, un pase
/// explícito devuelve `pairs_created: 0` — el rechazo viajó dentro del backup.
#[tokio::test]
async fn v8_transfer_pairing_and_rejections_round_trip() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Par conciliado A↔B (auto en el alta de la segunda pata).
    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-06-10", "concept": "Salida", "amount": "-100", "kind": "expense" }),
            &owner.cookie,
        )
        .await
        .json();
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-06-11", "concept": "Entrada", "amount": "100", "kind": "income" }),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(b["transfer_counterpart_id"], a["id"], "precondición: A↔B conciliadas");

    // Par C↔D conciliado y luego RECHAZADO a mano.
    let c = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-06-15", "concept": "Gasto real", "amount": "-50", "kind": "expense" }),
            &owner.cookie,
        )
        .await
        .json();
    let d = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-06-16", "concept": "Reembolso", "amount": "50", "kind": "income" }),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(d["transfer_counterpart_id"], c["id"], "precondición: C↔D conciliadas");
    let u = app
        .delete_with_cookie(
            &format!("/v1/transactions/{}/reconcile", c["id"].as_str().unwrap()),
            &owner.cookie,
        )
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");

    // Export → import (wipe + restore con UUIDs frescos).
    let b64 = export_ffbackup_b64(&app, &owner.cookie).await;
    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::OK, "apply: {applied:?}");
    let counts = applied.json();
    assert_eq!(counts["imported"]["transactions"].as_u64(), Some(4));
    assert_eq!(counts["imported"]["transfer_match_rejections"].as_u64(), Some(1));

    // A↔B siguen conciliadas (source preservado); C y D siguen sueltas.
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await
        .json();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let by_concept = |name: &str| {
        rows.iter()
            .find(|t| t["concept"] == name)
            .unwrap_or_else(|| panic!("no row '{name}'"))
    };
    let salida = by_concept("Salida");
    let entrada = by_concept("Entrada");
    assert_eq!(salida["transfer_counterpart_id"], entrada["id"], "A↔B restauradas simétricas");
    assert_eq!(salida["transfer_reconciled_source"], "auto", "source preservado");
    assert!(by_concept("Gasto real")["transfer_counterpart_id"].is_null());
    assert!(by_concept("Reembolso")["transfer_counterpart_id"].is_null());

    // ANTI-RESURRECCIÓN: el pase explícito no re-empareja C↔D (rechazo restaurado del backup).
    let pass = app
        .post_json_with_cookie("/v1/transactions/reconcile", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(
        pass.json()["pairs_created"].as_u64(),
        Some(0),
        "los rechazos del backup deben impedir el re-emparejado: {pass:?}"
    );
}

/// Un backup v7 (sin claves de conciliación) importa limpio, y el pase post-import re-concilia
/// RETROACTIVAMENTE los pares opuestos que trae — la vía retro para ficheros antiguos.
#[tokio::test]
async fn v7_backup_imports_and_reconciles_retroactively() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": null },
        "categories_used": [],
        "assets": [],
        "allocation_rules": [],
        "liabilities": [],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json(),
        "snapshots": [],
        "transaction_imports": [],
        "transactions": [
            {
                "import_index": null, "source": "manual", "op_date": "2026-06-10",
                "concept": "Salida traspaso", "amount": "-80.0000", "currency": "EUR",
                "kind": "expense", "fingerprint_ordinal": 0
            },
            {
                "import_index": null, "source": "manual", "op_date": "2026-06-11",
                "concept": "Entrada traspaso", "amount": "80.0000", "currency": "EUR",
                "kind": "income", "fingerprint_ordinal": 0
            }
        ],
        "categorization_rules": [],
        "recurring_transaction_rules": []
    });
    let b64 = craft_ffbackup_b64(7, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::OK, "v7 import: {applied:?}");
    assert_eq!(applied.json()["imported"]["transactions"].as_u64(), Some(2));
    assert_eq!(
        applied.json()["imported"]["transfer_match_rejections"].as_u64(),
        Some(0),
        "v7 no trae rechazos"
    );

    // El pase post-commit del apply ya debió conciliar el par (−80/+80 a 1 día). Polling corto:
    // el pase corre tras el commit pero dentro del mismo handler → ya visible aquí.
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await
        .json();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for t in rows {
        assert!(
            t["transfer_counterpart_id"].is_string(),
            "el pase retro post-import debe conciliar el par v7: {t:?}"
        );
        assert_eq!(t["transfer_reconciled_source"], "auto");
    }
}

/// REGRESIÓN — el manifiesto de un `.ffbackup` es entrada **no autenticada** y sus parámetros
/// de Argon2id tienen techo.
///
/// El AAD del AES-GCM solo cubre `schema_version`, `user_id_original` y `exported_at`; los
/// `kdf.*` viajan en claro y fuera de él, así que quien fabrica el fichero los elige. En
/// argon2 0.5 `MAX_M_COST` es `u32::MAX` y `Params::new` no lo comprueba, mientras que
/// `hash_password_into` reserva 1 KiB por unidad de `m_cost`: un fichero de 200 bytes con
/// `m_cost: 8000000` pedía 8 GB y se llevaba por delante el contenedor entero —PostgreSQL
/// embebido incluido— desde el endpoint de **preview**, que ni siquiera escribe.
///
/// El fichero se manipula sin recifrar nada: se parsea el marco, se toca el JSON del
/// manifiesto y se rearma. Se rechaza ANTES de derivar la clave, así que el test es
/// instantáneo — si algún día deja de serlo, es que la guardia desapareció.
#[tokio::test]
async fn import_rejects_a_manifest_with_out_of_range_kdf_parameters() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    create_asset(&app, &owner.cookie, &cat, "A", "10000").await;
    let good = export_ffbackup_b64(&app, &owner.cookie).await;

    // Desarmar el marco: [magic 4][format_version 1][manifest_len u32 LE][manifest][ciphertext]
    let bytes = B64.decode(good.as_bytes()).expect("base64 del export");
    let manifest_len = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
    let head = &bytes[..9];
    let manifest_json = &bytes[9..9 + manifest_len];
    let ciphertext = &bytes[9 + manifest_len..];
    let mut manifest: serde_json::Value =
        serde_json::from_slice(manifest_json).expect("manifiesto json");

    // El original tiene que ser sano, o el test no probaría nada.
    assert_eq!(manifest["kdf"]["m_cost"], 19_456, "manifiesto exportado: {manifest}");

    for (campo, valor) in [
        ("m_cost", serde_json::json!(8_000_000u32)), // ~8 GB de reserva
        ("t_cost", serde_json::json!(1_000_000u32)), // horas de CPU
        ("p_cost", serde_json::json!(64u32)),
        ("out_len", serde_json::json!(1024u32)),
    ] {
        manifest["kdf"][campo] = valor.clone();
        let tampered = serde_json::to_vec(&manifest).expect("serializar manifiesto");
        let mut frame = head[..5].to_vec();
        frame.extend_from_slice(&(tampered.len() as u32).to_le_bytes());
        frame.extend_from_slice(&tampered);
        frame.extend_from_slice(ciphertext);

        let r = import_preview(&app, &owner.cookie, &B64.encode(&frame)).await;
        assert_eq!(
            r.status,
            http::StatusCode::BAD_REQUEST,
            "kdf.{campo} = {valor} debe rechazarse, no intentar derivar la clave: {r:?}"
        );
        assert_eq!(
            r.json()["code"], "backup_crypto_params_unsupported",
            "kdf.{campo}: código de error inesperado: {}",
            r.json()
        );
        // Restaurar el campo para probar el siguiente de forma aislada.
        manifest = serde_json::from_slice(manifest_json).expect("manifiesto json");
    }

    // Y el fichero intacto sigue importándose.
    let ok = import_preview(&app, &owner.cookie, &good).await;
    assert_eq!(ok.status, http::StatusCode::OK, "el export legítimo debe seguir valiendo: {ok:?}");
}

// ---------------------------------------------------------------------------
// v10 (4.2.0) — `repayment_model` en los pasivos
// ---------------------------------------------------------------------------

/// Un `.ffbackup` **v9** (pre-4.2.0) no lleva `repayment_model`. Sus pasivos tienen que entrar
/// como `fixed_payments`, que es exactamente el modelo con el que se calcularon los números que
/// el usuario vio cuando exportó: restaurar un backup viejo no puede mover una proyección.
#[tokio::test]
async fn v9_backup_imports_its_liabilities_as_fixed_payments() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": null },
        "categories_used": [
            { "scope": "liability", "name": "Préstamos", "sort_index": 0 },
            { "scope": "expense", "name": "Cuotas", "sort_index": 0 }
        ],
        "assets": [],
        "allocation_rules": [],
        "liabilities": [{
            "category_ref": { "scope": "liability", "name": "Préstamos" },
            "expense_category_ref": { "scope": "expense", "name": "Cuotas" },
            "label": "Hipoteca vieja",
            "principal": "80000.0000",
            "principal_derived_from_plan": false,
            "apr_percent": "3.0000",
            "payment_amount": "500.0000",
            "payment_frequency": "monthly",
            "payment_end_date": null,
            "sort_index": 0
        }],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json(),
        "snapshots": [],
        "transaction_imports": [],
        "transactions": [],
        "categorization_rules": [],
        "recurring_transaction_rules": [],
        "transfer_match_rejections": []
    });
    let b64 = craft_ffbackup_b64(9, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::OK, "v9 import: {applied:?}");
    assert_eq!(applied.json()["imported"]["liabilities"].as_u64(), Some(1));

    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert_eq!(rows[0]["label"], "Hipoteca vieja");
    assert_eq!(
        rows[0]["repayment_model"], "fixed_payments",
        "un pasivo de un backup v9 tiene que quedar en el modelo histórico"
    );
}

/// Roundtrip v10: el modelo sobrevive al export → import. Sin esto, un usuario que restaura su
/// propio backup perdería la configuración de sus préstamos y su proyección cambiaría en
/// silencio (que es el modo de fallo que este repo persigue).
#[tokio::test]
async fn v10_roundtrip_preserves_the_repayment_model() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamos").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": liab_cat,
                "expense_category_id": exp_cat,
                "label": "Hipoteca",
                "principal": "150000",
                "repayment_model": "french",
                "apr_percent": "2.75",
                "payment_amount": "700",
                "payment_frequency": "monthly",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create french: {r:?}");

    let backup = export_ffbackup_b64(&app, &owner.cookie).await;
    // Ensuciar el estado vivo: el pasivo pasa al modelo histórico antes de restaurar.
    let id = r.json()["id"].as_str().unwrap().to_string();
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            serde_json::json!({ "repayment_model": "fixed_payments" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let applied = import_apply(&app, &owner.cookie, &backup).await;
    assert_eq!(applied.status, http::StatusCode::OK, "v10 import: {applied:?}");

    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["repayment_model"], "french", "el roundtrip debe conservar el modelo");
    let apr: rust_decimal::Decimal = rows[0]["apr_percent"].as_str().unwrap().parse().unwrap();
    assert_eq!(apr, "2.75".parse::<rust_decimal::Decimal>().unwrap());
}

/// Un v10 con un pasivo `french` **sin TIN** —combinación que el create rechaza con 400— tiene
/// que importar igualmente: el INSERT del import bypasea la validación a propósito (un backup
/// restaura lo que había, no lo que hoy sería válido) y el engine degenera a 0 % en vez de
/// panicar. Lo único que filtra es el CHECK de la columna.
#[tokio::test]
async fn v10_french_liability_without_apr_still_imports() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let payload = serde_json::json!({
        "user": { "username": "alice", "birth_date": null },
        "categories_used": [{ "scope": "liability", "name": "Préstamos", "sort_index": 0 }],
        "assets": [],
        "allocation_rules": [],
        "liabilities": [{
            "category_ref": { "scope": "liability", "name": "Préstamos" },
            "expense_category_ref": null,
            "label": "Francés sin TIN",
            "principal": "50000.0000",
            "principal_derived_from_plan": false,
            "repayment_model": "french",
            "apr_percent": null,
            "payment_amount": "400.0000",
            "payment_frequency": "monthly",
            "payment_end_date": null,
            "sort_index": 0
        }],
        "budget_entries": [],
        "planning_flows": [],
        "ui_preferences": {},
        "installation_snapshot_informative": installation_snapshot_json(),
        "snapshots": [],
        "transaction_imports": [],
        "transactions": [],
        "categorization_rules": [],
        "recurring_transaction_rules": [],
        "transfer_match_rejections": []
    });
    let b64 = craft_ffbackup_b64(10, &payload, owner.user_id);

    let applied = import_apply(&app, &owner.cookie, &b64).await;
    assert_eq!(applied.status, http::StatusCode::OK, "v10 import: {applied:?}");
    assert_eq!(applied.json()["imported"]["liabilities"].as_u64(), Some(1));

    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert_eq!(rows[0]["repayment_model"], "french");
    assert!(rows[0].get("apr_percent").is_none_or(serde_json::Value::is_null));

    // Y la proyección se sirve sin reventar (la degeneración del engine, extremo a extremo).
    let series = app
        .get_with_cookie("/v1/projection/series?months=24", &owner.cookie)
        .await;
    assert_eq!(series.status, http::StatusCode::OK, "series: {series:?}");
}
