//! Fase 1.4 — Límite de tamaño de request.
//!
//! - Endpoints normales: ≤ 1 MB (default).
//! - Endpoints de import de backup: ≤ 16 MB.
//! El extractor `Json<T>` respeta `DefaultBodyLimit` y responde 413 cuando el body excede el tope.

mod common;

use axum::body::Body;
use common::TestApp;
use http::Request;

#[tokio::test]
async fn oversized_register_body_returns_413() {
    let app = TestApp::spawn().await;
    let huge_notes = "a".repeat(2 * 1024 * 1024); // 2 MB
    let body = serde_json::json!({
        "username": "alice",
        "password": "correct horse battery staple",
        "birth_date": "1990-01-01",
        "_padding": huge_notes,
    });
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/v1/auth/register")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.request(req).await;
    assert_eq!(
        resp.status,
        http::StatusCode::PAYLOAD_TOO_LARGE,
        "esperado 413 con body > 1 MB"
    );
}

#[tokio::test]
async fn import_endpoint_accepts_body_above_default_limit() {
    let app = TestApp::spawn().await;
    let _owner = app.register_and_login_owner("alice").await;
    // Body ~5 MB (mucho mayor que el límite global de 1 MB pero por debajo del de import).
    let huge_file = "a".repeat(5 * 1024 * 1024);
    let body = serde_json::json!({
        "file_b64": huge_file,
        "password": "wrong",
    });
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/v1/backup/user-import/preview")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.request(req).await;
    // Sin cookie de sesión → 401, NO 413. Esto demuestra que el body llegó al handler.
    assert_eq!(
        resp.status,
        http::StatusCode::UNAUTHORIZED,
        "esperado 401 (sin sesión), no 413; el endpoint de import debe aceptar > 1 MB"
    );
}
