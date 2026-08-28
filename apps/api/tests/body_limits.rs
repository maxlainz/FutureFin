//! Fase 1.4 — Límite de tamaño de request.
//!
//! - Endpoints normales: ≤ 1 MiB (default).
//! - Endpoints de import de backup: ≤ 16 MiB.
//! - `/mcp`: ≤ 1 MiB, pero **por un camino distinto** (ver abajo).
//!
//! El extractor `Json<T>` respeta `DefaultBodyLimit` y responde 413 cuando el body excede el
//! tope. `/mcp` NO usa extractor: es un `route_service` de rmcp que lee el body con su propio
//! tope, así que `DefaultBodyLimit` no lo alcanzaba y el límite real eran los 4 MiB por defecto
//! del SDK — el invariante «1 MiB global» era falso justo ahí (issue #85, hallazgo 6).

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

/// El tope de `/mcp` se fija en `mcp::MCP_MAX_REQUEST_BODY_BYTES` porque `DefaultBodyLimit` no
/// llega hasta ahí. 2 MiB está por encima del global (1 MiB) y por debajo del default de rmcp
/// (4 MiB): es exactamente el hueco por el que pasaba antes.
#[tokio::test]
async fn oversized_mcp_body_returns_413() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "body limit"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let token = created.json()["token"].as_str().unwrap().to_string();

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {
                "name": "futurefin-tests",
                "version": "0",
                "padding": "a".repeat(2 * 1024 * 1024),
            }
        }
    });
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/mcp")
        .header(http::header::HOST, "futurefin.test")
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "initialize")
        .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.request(req).await;
    assert_eq!(
        resp.status,
        http::StatusCode::PAYLOAD_TOO_LARGE,
        "esperado 413 con body > 1 MiB en /mcp: {resp:?}"
    );
}
