//! Tokens de API (`/v1/api-tokens`) — la credencial Bearer del servidor MCP.
//!
//! Cubre: creación (secreto `ffp_` una sola vez), listado sin secreto, revocación,
//! caducidad, límite de tokens activos, aislamiento entre usuarios y el contrato de
//! `require_api_token` a través de `/mcp` (401 para todo Bearer inválido).

mod common;

use common::TestApp;

/// Petición MCP mínima (initialize) con Bearer opcional. Sirve para probar el gate de
/// auth sin depender del protocolo (cualquier 401 corta antes de rmcp).
async fn mcp_initialize(app: &TestApp, bearer: Option<&str>) -> common::ResponseParts {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    });
    let mut builder = http::Request::builder()
        .method(http::Method::POST)
        .uri("/mcp")
        // Todo cliente HTTP/1.1 real manda Host; el oneshot del harness no lo añade solo
        // y rmcp lo exige aunque la validación de allowed_hosts esté desactivada.
        .header(http::header::HOST, "futurefin.test")
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream");
    if let Some(b) = bearer {
        builder = builder.header(http::header::AUTHORIZATION, format!("Bearer {b}"));
    }
    app.request(
        builder
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .expect("build MCP request"),
    )
    .await
}

#[tokio::test]
async fn create_token_returns_secret_once_and_list_never_repeats_it() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "Claude portátil"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let body = created.json();
    let token = body["token"].as_str().expect("token in create response");
    assert!(token.starts_with("ffp_"), "token con prefijo ffp_: {token}");
    assert!(token.len() > 40, "token largo (32 bytes b64url): {token}");
    let prefix = body["token_prefix"].as_str().expect("token_prefix");
    assert_eq!(prefix, &token[..12], "token_prefix = primeros 12 chars");
    assert_eq!(body["label"], "Claude portátil");
    assert!(body["expires_at"].is_null() || body.get("expires_at").is_none());

    // El listado no contiene el secreto, ni el hash — solo el prefijo.
    let list = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK);
    let text = String::from_utf8_lossy(&list.body).to_string();
    assert!(!text.contains(token), "el secreto no puede reaparecer en el listado");
    let arr = list.json();
    assert_eq!(arr.as_array().unwrap().len(), 1);
    assert_eq!(arr[0]["token_prefix"], prefix);
    assert!(arr[0].get("token").is_none(), "sin campo token en el listado");
    assert!(arr[0].get("token_hash").is_none(), "sin hash en el listado");
}

#[tokio::test]
async fn valid_token_authenticates_mcp_and_revoked_token_stops_working() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "cli"}),
            &owner.cookie,
        )
        .await;
    let body = created.json();
    let token = body["token"].as_str().unwrap().to_string();
    let token_id = body["id"].as_str().unwrap().to_string();

    let ok = mcp_initialize(&app, Some(&token)).await;
    assert_eq!(ok.status, http::StatusCode::OK, "token válido → 200: {ok:?}");

    // Revocar → 204; segunda revocación del mismo id → 404 (ya revocado).
    let del = app
        .delete_with_cookie(&format!("/v1/api-tokens/{token_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    let again = app
        .delete_with_cookie(&format!("/v1/api-tokens/{token_id}"), &owner.cookie)
        .await;
    assert_eq!(again.status, http::StatusCode::NOT_FOUND);

    let denied = mcp_initialize(&app, Some(&token)).await;
    assert_eq!(denied.status, http::StatusCode::UNAUTHORIZED, "revocado → 401");
    assert!(
        denied.headers.get(http::header::WWW_AUTHENTICATE).is_some(),
        "401 con WWW-Authenticate"
    );

    // El token revocado sigue listado (auditoría) con revoked_at poblado.
    let list = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    let arr = list.json();
    assert_eq!(arr.as_array().unwrap().len(), 1);
    assert!(arr[0]["revoked_at"].is_string(), "revoked_at visible: {arr}");
}

#[tokio::test]
async fn invalid_bearers_are_all_401() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Instalación creada; ningún token emitido.
    let _ = owner;

    // Sin header.
    let r = mcp_initialize(&app, None).await;
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED);
    // Prefijo desconocido.
    let r = mcp_initialize(&app, Some("sk-otracosa")).await;
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED);
    // Prefijo correcto, secreto inexistente.
    let r = mcp_initialize(&app, Some("ffp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).await;
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_token_is_401() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "expira", "expires_in_days": 1}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let body = created.json();
    let token = body["token"].as_str().unwrap().to_string();
    assert!(body["expires_at"].is_string(), "expires_at poblado");

    // Vigente → 200.
    let ok = mcp_initialize(&app, Some(&token)).await;
    assert_eq!(ok.status, http::StatusCode::OK);

    // Vencer el token por detrás (UPDATE directo, como haría el paso del tiempo).
    sqlx::query("UPDATE api_tokens SET expires_at = now() - interval '1 minute'")
        .execute(&app.pool)
        .await
        .expect("expire token");
    let denied = mcp_initialize(&app, Some(&token)).await;
    assert_eq!(denied.status, http::StatusCode::UNAUTHORIZED, "expirado → 401");
}

#[tokio::test]
async fn pending_user_token_is_403_on_mcp() {
    let app = TestApp::spawn().await;
    let _owner = app.register_and_login_owner("alice").await;

    // Segundo usuario registrado pero NO aprobado (pending): puede crear sesión...
    let reg = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({"username": "bob", "password": "correct horse battery staple", "birth_date": "1991-01-01"}),
        )
        .await;
    assert_eq!(reg.status, http::StatusCode::CREATED);
    let login = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "bob", "password": "correct horse battery staple"}),
        )
        .await;
    let cookie = login.session_cookie().expect("bob session");

    // ...pero /v1/api-tokens exige membership → 403 también al crear.
    let create = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "pending"}),
            &cookie,
        )
        .await;
    assert_eq!(
        create.status,
        http::StatusCode::FORBIDDEN,
        "usuario pending no puede crear tokens: {create:?}"
    );
}

#[tokio::test]
async fn viewer_can_create_token_and_it_authenticates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vera", "viewer").await;

    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "viewer token"}),
            &viewer.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let token = created.json()["token"].as_str().unwrap().to_string();

    let ok = mcp_initialize(&app, Some(&token)).await;
    assert_eq!(ok.status, http::StatusCode::OK, "token de viewer autentica: {ok:?}");
}

#[tokio::test]
async fn tokens_are_isolated_between_users() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "mario", "member").await;

    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "de alice"}),
            &owner.cookie,
        )
        .await;
    let token_id = created.json()["id"].as_str().unwrap().to_string();

    // El member no ve el token de alice…
    let list = app.get_with_cookie("/v1/api-tokens", &member.cookie).await;
    assert_eq!(list.json().as_array().unwrap().len(), 0);
    // …ni puede revocarlo (mismo 404 que un id inexistente).
    let del = app
        .delete_with_cookie(&format!("/v1/api-tokens/{token_id}"), &member.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn validation_errors_and_active_token_limit() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Label vacío / demasiado largo / expires fuera de rango → 400.
    for body in [
        serde_json::json!({"label": "   "}),
        serde_json::json!({"label": "x".repeat(65)}),
        serde_json::json!({"label": "ok", "expires_in_days": 0}),
        serde_json::json!({"label": "ok", "expires_in_days": 4000}),
    ] {
        let r = app.post_json_with_cookie("/v1/api-tokens", body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    }

    // 10 tokens activos OK; el 11º → 400 token_limit_reached.
    for i in 0..10 {
        let r = app
            .post_json_with_cookie(
                "/v1/api-tokens",
                serde_json::json!({"label": format!("t{i}")}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "token {i}: {r:?}");
    }
    let over = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "el 11"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(over.status, http::StatusCode::BAD_REQUEST);
    let msg = String::from_utf8_lossy(&over.body).to_string();
    assert!(msg.contains("token_limit_reached"), "{msg}");

    // Revocar uno libera hueco.
    let list = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    let first_id = list.json()[0]["id"].as_str().unwrap().to_string();
    let del = app
        .delete_with_cookie(&format!("/v1/api-tokens/{first_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    let retry = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "ahora sí"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(retry.status, http::StatusCode::CREATED);
}
