//! Integración de los snapshots históricos (`/v1/history/snapshots*`) — WP3.
//!
//! Cubre captura (upsert por día, términos copiados, exclusión de compartidas/expiradas),
//! backfill CRUD (roundtrip con filtro `year` + cascade), validaciones 400, conflicto 409,
//! guardia cross-user 404, viewer 403, que los GET nunca mutan y que las mutaciones de
//! snapshots NO tocan la cache de proyección.
//!
//! Los Decimals viajan como string ("1000.0000"): se comparan valores parseados, no strings.

mod common;

use chrono::{Datelike, Duration, NaiveDate, Utc};
use common::TestApp;
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use uuid::Uuid;

fn parse_dec(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("expected decimal string, got {v:?}"))
        .parse::<f64>()
        .expect("parse decimal string")
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 0.01, "expected ~{b}, got {a}");
}

/// Encuentra el snapshot de un `kind` dentro de un array de snapshots.
fn find_kind<'a>(arr: &'a serde_json::Value, kind: &str) -> &'a serde_json::Value {
    arr.as_array()
        .expect("snapshots array")
        .iter()
        .find(|s| s["kind"] == kind)
        .unwrap_or_else(|| panic!("no snapshot of kind {kind} in {arr:?}"))
}

async fn installation_id(app: &TestApp) -> Uuid {
    sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("installation id")
}

// ---------------------------------------------------------------------------
// Captura
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capture_creates_both_kinds_with_copied_terms() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": asset_cat, "name": "Bolsa", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;

    let future = (Utc::now().date_naive() + Duration::days(365))
        .format("%Y-%m-%d")
        .to_string();
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": liab_cat,
            "label": "Hipoteca",
            "principal": "5000",
            "apr_percent": "3.5",
            "payment_amount": "200",
            "payment_frequency": "monthly",
            "payment_end_date": future,
        }),
        &owner.cookie,
    )
    .await;

    let resp = app
        .post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "capture: {resp:?}");
    let body = resp.json();
    let snaps = &body["snapshots"];
    assert_eq!(snaps.as_array().unwrap().len(), 2);

    let asset_snap = find_kind(snaps, "asset");
    assert_eq!(asset_snap["source"], "capture");
    approx(parse_dec(&asset_snap["total"]), 10000.0);
    let a_items = asset_snap["items"].as_array().unwrap();
    assert_eq!(a_items.len(), 1);
    assert_eq!(a_items[0]["label"], "Bolsa");
    approx(parse_dec(&a_items[0]["value"]), 10000.0);
    assert!(a_items[0].get("payment_amount").is_none(), "asset item must not carry terms");

    let liab_snap = find_kind(snaps, "liability");
    approx(parse_dec(&liab_snap["total"]), 5000.0);
    let l_items = liab_snap["items"].as_array().unwrap();
    assert_eq!(l_items.len(), 1);
    approx(parse_dec(&l_items[0]["apr_percent"]), 3.5);
    approx(parse_dec(&l_items[0]["payment_amount"]), 200.0);
    assert_eq!(l_items[0]["payment_frequency"], "monthly");
}

#[tokio::test]
async fn capture_same_day_upserts_and_replaces_items() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;

    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Alpha", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;
    let first = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["asset"]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(first.status, http::StatusCode::OK);
    assert_eq!(find_kind(&first.json()["snapshots"], "asset")["items"].as_array().unwrap().len(), 1);
    assert_eq!(app.count_rows("history_snapshots").await, 1);

    // Añadir un segundo asset y volver a capturar el MISMO día.
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Beta", "current_value": "5000"}),
        &owner.cookie,
    )
    .await;
    let second = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["asset"]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(second.status, http::StatusCode::OK);
    let sbody = second.json();
    let snap = find_kind(&sbody["snapshots"], "asset");
    approx(parse_dec(&snap["total"]), 15000.0);
    assert_eq!(snap["items"].as_array().unwrap().len(), 2);

    // Upsert: sigue habiendo una sola cabecera; los items se reemplazaron (2, no 3).
    assert_eq!(app.count_rows("history_snapshots").await, 1);
    assert_eq!(app.count_rows("history_snapshot_items").await, 2);
}

#[tokio::test]
async fn capture_excludes_shared_and_expired_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    let iid = installation_id(&app).await;

    // Asset propio.
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": asset_cat, "name": "Mío", "current_value": "8000"}),
        &owner.cookie,
    )
    .await;
    // Asset compartido (owner_user_id NULL) — no hay ruta API que lo cree, se siembra por SQL.
    sqlx::query(
        "INSERT INTO assets (installation_id, category_id, name, current_value, owner_user_id)
         VALUES ($1, $2, 'Compartido', 99999, NULL)",
    )
    .bind(iid)
    .bind(Uuid::parse_str(&asset_cat).unwrap())
    .execute(&app.pool)
    .await
    .expect("seed shared asset");

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(30)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(180)).format("%Y-%m-%d").to_string();
    // Pasivo expirado.
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": liab_cat, "label": "Vencido", "principal": "1000",
            "payment_amount": "100", "payment_frequency": "monthly", "payment_end_date": past,
        }),
        &owner.cookie,
    )
    .await;
    // Pasivo vivo.
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": liab_cat, "label": "Vivo", "principal": "4000",
            "payment_amount": "150", "payment_frequency": "monthly", "payment_end_date": future,
        }),
        &owner.cookie,
    )
    .await;

    let resp = app
        .post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK);
    let body = resp.json();

    let asset_snap = find_kind(&body["snapshots"], "asset");
    let a_items = asset_snap["items"].as_array().unwrap();
    assert_eq!(a_items.len(), 1, "asset compartido debe excluirse");
    assert_eq!(a_items[0]["label"], "Mío");

    let liab_snap = find_kind(&body["snapshots"], "liability");
    let l_items = liab_snap["items"].as_array().unwrap();
    assert_eq!(l_items.len(), 1, "pasivo expirado debe excluirse");
    assert_eq!(l_items[0]["label"], "Vivo");
}

// ---------------------------------------------------------------------------
// Backfill CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backfill_crud_roundtrip() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let base_year = Utc::now().date_naive().year() - 2;
    let d1 = NaiveDate::from_ymd_opt(base_year, 3, 15).unwrap().format("%Y-%m-%d").to_string();
    let d2 = NaiveDate::from_ymd_opt(base_year, 6, 20).unwrap().format("%Y-%m-%d").to_string();

    // Crear con un item SIN item_id → el servidor lo genera y lo devuelve.
    let created = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": d1,
                "items": [{"label": "Cash", "value": "1000"}],
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "backfill create: {created:?}");
    let cbody = created.json();
    assert_eq!(cbody["source"], "backfill");
    assert_eq!(cbody["snapshot_date_ymd"], d1);
    approx(parse_dec(&cbody["total"]), 1000.0);
    let snap_id = cbody["id"].as_str().unwrap().to_string();
    let echoed_item_id = cbody["items"][0]["item_id"].as_str().expect("item_id echoed").to_string();
    Uuid::parse_str(&echoed_item_id).expect("echoed item_id is a uuid");

    // Filtro por año: base_year → 1; otro año → 0.
    let this_year = app
        .get_with_cookie(&format!("/v1/history/snapshots?year={base_year}"), &owner.cookie)
        .await;
    assert_eq!(this_year.status, http::StatusCode::OK);
    assert_eq!(this_year.json().as_array().unwrap().len(), 1);
    let other_year = app
        .get_with_cookie(&format!("/v1/history/snapshots?year={}", base_year - 1), &owner.cookie)
        .await;
    assert_eq!(other_year.json().as_array().unwrap().len(), 0);

    // PUT: reemplazo de items + mover fecha, reutilizando el item_id.
    let put = app
        .request(
            http::Request::builder()
                .method(http::Method::PUT)
                .uri(format!("/v1/history/snapshots/{snap_id}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, &owner.cookie)
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "snapshot_date": d2,
                        "items": [{"item_id": echoed_item_id, "label": "Cash", "value": "2000"}],
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(put.status, http::StatusCode::OK, "put: {put:?}");
    let pbody = put.json();
    assert_eq!(pbody["snapshot_date_ymd"], d2);
    approx(parse_dec(&pbody["total"]), 2000.0);
    assert_eq!(pbody["items"][0]["item_id"].as_str().unwrap(), echoed_item_id);

    // DELETE → 204 + cascade.
    let del = app
        .delete_with_cookie(&format!("/v1/history/snapshots/{snap_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("history_snapshots").await, 0);
    assert_eq!(app.count_rows("history_snapshot_items").await, 0, "items en cascada");
}

#[tokio::test]
async fn put_with_only_date_preserves_items() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let base_year = Utc::now().date_naive().year() - 2;
    let d1 = NaiveDate::from_ymd_opt(base_year, 3, 15).unwrap().format("%Y-%m-%d").to_string();
    let d2 = NaiveDate::from_ymd_opt(base_year, 8, 20).unwrap().format("%Y-%m-%d").to_string();

    // Snapshot con 2 items, total 3500.
    let created = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": d1,
                "items": [
                    {"label": "Cash", "value": "1000"},
                    {"label": "Bolsa", "value": "2500"},
                ],
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "create: {created:?}");
    let snap_id = created.json()["id"].as_str().unwrap().to_string();
    approx(parse_dec(&created.json()["total"]), 3500.0);
    assert_eq!(app.count_rows("history_snapshot_items").await, 2);

    // PUT con SOLO snapshot_date (sin `items`) → conserva los items intactos, solo mueve la fecha.
    let put = app
        .request(
            http::Request::builder()
                .method(http::Method::PUT)
                .uri(format!("/v1/history/snapshots/{snap_id}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, &owner.cookie)
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({ "snapshot_date": d2 })).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(put.status, http::StatusCode::OK, "put date-only: {put:?}");
    let pbody = put.json();
    assert_eq!(pbody["snapshot_date_ymd"], d2, "la fecha debe cambiar");
    assert_eq!(pbody["items"].as_array().unwrap().len(), 2, "items conservados (no borrados)");
    approx(parse_dec(&pbody["total"]), 3500.0);
    assert_eq!(app.count_rows("history_snapshot_items").await, 2, "items intactos en BD");

    // Guard del otro lado del contrato: `items: []` presente SÍ es reemplazo completo → 0 items.
    let put_empty = app
        .request(
            http::Request::builder()
                .method(http::Method::PUT)
                .uri(format!("/v1/history/snapshots/{snap_id}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, &owner.cookie)
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({ "items": [] })).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(put_empty.status, http::StatusCode::OK, "put items:[]: {put_empty:?}");
    assert_eq!(put_empty.json()["items"].as_array().unwrap().len(), 0, "items:[] reemplaza a vacío");
    assert_eq!(app.count_rows("history_snapshot_items").await, 0);
}

// ---------------------------------------------------------------------------
// Validaciones 400 / 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backfill_future_date_rejected_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let future = (Utc::now().date_naive() + Duration::days(3)).format("%Y-%m-%d").to_string();
    let resp = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({"kind": "asset", "snapshot_date": future, "items": []}),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert!(
        resp.json()["message"].as_str().unwrap().contains("snapshot_date_in_future"),
        "{:?}",
        resp.json()
    );
}

#[tokio::test]
async fn backfill_duplicate_item_id_rejected_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let dup = Uuid::new_v4().to_string();
    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 1, 5).unwrap().format("%Y-%m-%d").to_string();
    let resp = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": d,
                "items": [
                    {"item_id": dup, "label": "A", "value": "1"},
                    {"item_id": dup, "label": "B", "value": "2"},
                ],
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert!(resp.json()["message"].as_str().unwrap().contains("duplicate_item_id"));
}

#[tokio::test]
async fn backfill_terms_on_asset_kind_rejected_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 1, 5).unwrap().format("%Y-%m-%d").to_string();
    let resp = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": "asset",
                "snapshot_date": d,
                "items": [{"label": "A", "value": "1", "payment_amount": "50"}],
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert!(resp.json()["message"].as_str().unwrap().contains("terms_only_for_liabilities"));
}

#[tokio::test]
async fn backfill_existing_date_conflicts_409() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 4, 1).unwrap().format("%Y-%m-%d").to_string();
    let body = serde_json::json!({
        "kind": "asset", "snapshot_date": d, "items": [{"label": "Cash", "value": "1000"}],
    });
    let first = app.post_json_with_cookie("/v1/history/snapshots", body.clone(), &owner.cookie).await;
    assert_eq!(first.status, http::StatusCode::CREATED);
    let second = app.post_json_with_cookie("/v1/history/snapshots", body, &owner.cookie).await;
    assert_eq!(second.status, http::StatusCode::CONFLICT, "misma (user,kind,fecha) → 409: {second:?}");
}

// ---------------------------------------------------------------------------
// Guardias de autorización
// ---------------------------------------------------------------------------

#[tokio::test]
async fn other_users_snapshot_is_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;

    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 2, 2).unwrap().format("%Y-%m-%d").to_string();
    let created = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({"kind": "asset", "snapshot_date": d, "items": [{"label": "Cash", "value": "1000"}]}),
            &owner.cookie,
        )
        .await;
    let snap_id = created.json()["id"].as_str().unwrap().to_string();

    // Bob (member, con permiso de escritura) no puede ver ni tocar el snapshot de Alice → 404.
    let put = app
        .request(
            http::Request::builder()
                .method(http::Method::PUT)
                .uri(format!("/v1/history/snapshots/{snap_id}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, &bob.cookie)
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({"items": [{"label": "X", "value": "9"}]})).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(put.status, http::StatusCode::NOT_FOUND, "PUT cross-user: {put:?}");

    let del = app.delete_with_cookie(&format!("/v1/history/snapshots/{snap_id}"), &bob.cookie).await;
    assert_eq!(del.status, http::StatusCode::NOT_FOUND, "DELETE cross-user");

    // El snapshot de Alice sigue intacto.
    assert_eq!(app.count_rows("history_snapshots").await, 1);
}

#[tokio::test]
async fn viewer_role_cannot_mutate_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "viewer_user", "viewer").await;

    // Snapshot propiedad del owner (para probar PUT/DELETE del viewer).
    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 5, 5).unwrap().format("%Y-%m-%d").to_string();
    let created = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({"kind": "asset", "snapshot_date": d, "items": [{"label": "Cash", "value": "1000"}]}),
            &owner.cookie,
        )
        .await;
    let snap_id = created.json()["id"].as_str().unwrap().to_string();

    let cap = app
        .post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({}), &viewer.cookie)
        .await;
    assert_eq!(cap.status, http::StatusCode::FORBIDDEN, "viewer capture");

    let create = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({"kind": "asset", "snapshot_date": d, "items": []}),
            &viewer.cookie,
        )
        .await;
    assert_eq!(create.status, http::StatusCode::FORBIDDEN, "viewer backfill create");

    let put = app
        .request(
            http::Request::builder()
                .method(http::Method::PUT)
                .uri(format!("/v1/history/snapshots/{snap_id}"))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, &viewer.cookie)
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({"items": []})).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(put.status, http::StatusCode::FORBIDDEN, "viewer put");

    let del = app.delete_with_cookie(&format!("/v1/history/snapshots/{snap_id}"), &viewer.cookie).await;
    assert_eq!(del.status, http::StatusCode::FORBIDDEN, "viewer delete");
}

// ---------------------------------------------------------------------------
// Invariantes: GET no muta; snapshots no tocan la cache de proyección
// ---------------------------------------------------------------------------

#[tokio::test]
async fn history_get_never_mutates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "X", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;
    app.post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({"kinds": ["asset"]}), &owner.cookie).await;

    let headers_before = app.count_rows("history_snapshots").await;
    let items_before = app.count_rows("history_snapshot_items").await;
    assert_eq!(headers_before, 1);

    for uri in [
        "/v1/history/snapshots",
        "/v1/history/snapshots?kind=asset",
        &format!("/v1/history/snapshots?year={}", Utc::now().date_naive().year()),
    ] {
        let r = app.get_with_cookie(uri, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::OK, "GET {uri}: {r:?}");
    }

    assert_eq!(app.count_rows("history_snapshots").await, headers_before, "GET no debe crear/borrar cabeceras");
    assert_eq!(app.count_rows("history_snapshot_items").await, items_before, "GET no debe tocar items");
}

#[tokio::test]
async fn snapshot_mutations_do_not_touch_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "X", "current_value": "10000"}),
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
        assert!(cache.contains_key(&key), "la entrada household debe estar caliente antes de las mutaciones");
    }

    // Mutaciones de snapshots: capture + backfill + delete.
    app.post_json_with_cookie("/v1/history/snapshots/capture", serde_json::json!({"kinds": ["asset"]}), &owner.cookie).await;
    let d = NaiveDate::from_ymd_opt(Utc::now().date_naive().year() - 2, 7, 7).unwrap().format("%Y-%m-%d").to_string();
    let created = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({"kind": "asset", "snapshot_date": d, "items": [{"label": "Cash", "value": "1000"}]}),
            &owner.cookie,
        )
        .await;
    let snap_id = created.json()["id"].as_str().unwrap().to_string();
    app.delete_with_cookie(&format!("/v1/history/snapshots/{snap_id}"), &owner.cookie).await;

    // Dar margen a cualquier tarea de fondo (no debería haber ninguna que invalide).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let cache = app.state.projection_cache.read().await;
    assert!(
        cache.contains_key(&key),
        "las mutaciones de snapshots NO deben invalidar la cache de proyección (no son inputs del engine)"
    );
}
