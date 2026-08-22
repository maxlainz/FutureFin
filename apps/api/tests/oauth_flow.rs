//! Suite de integración del OAuth 2.1 embebido (v3.1.0): metadata de descubrimiento,
//! DCR, flujo authorization_code + PKCE de punta a punta hasta `/mcp`, rotación de
//! refresh tokens con reuse-detection, revocación (panel + RFC 7009) y kill-switch.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::routes;
use futurefin_api::state::AppState;
use axum::extract::Extension;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use std::sync::Arc;

const CLAUDE_CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";

/// Challenge sintáctica válida (43 chars) para tests que no llegan al canje.
fn dummy_challenge() -> String {
    "a".repeat(43)
}

/// (verifier, challenge) PKCE S256.
fn pkce_pair() -> (String, String) {
    use rand_core::{OsRng, RngCore};
    use sha2::{Digest, Sha256};
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let verifier = B64URL.encode(bytes); // 43 chars
    let challenge = B64URL.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Registra un cliente público (el caso claude.ai) y devuelve su client_id.
async fn register_public_client(app: &TestApp) -> String {
    let resp = app
        .post_json(
            "/oauth/register",
            serde_json::json!({
                "redirect_uris": [CLAUDE_CALLBACK],
                "client_name": "Claude",
                "token_endpoint_auth_method": "none",
            }),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
    resp.json()["client_id"].as_str().unwrap().to_string()
}

/// POST /v1/oauth/authorize (approve) y devuelve el `code` extraído del redirect.
async fn authorize_and_get_code(
    app: &TestApp,
    cookie: &str,
    client_id: &str,
    challenge: &str,
    state: Option<&str>,
) -> (String, String) {
    let mut body = serde_json::json!({
        "approve": true,
        "client_id": client_id,
        "redirect_uri": CLAUDE_CALLBACK,
        "response_type": "code",
        "code_challenge": challenge,
        "code_challenge_method": "S256",
    });
    if let Some(s) = state {
        body["state"] = serde_json::json!(s);
    }
    let resp = app
        .post_json_with_cookie("/v1/oauth/authorize", body, cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let redirect_to = resp.json()["redirect_to"].as_str().unwrap().to_string();
    let parsed = url::Url::parse(&redirect_to).expect("redirect_to parses");
    let code = parsed
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .expect("redirect_to carries code");
    (code, redirect_to)
}

/// Canjea un code en /oauth/token (cliente público). Devuelve la respuesta cruda.
async fn exchange_code(
    app: &TestApp,
    client_id: &str,
    code: &str,
    verifier: &str,
) -> common::ResponseParts {
    app.post_form(
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CLAUDE_CALLBACK),
            ("code_verifier", verifier),
            ("client_id", client_id),
        ],
    )
    .await
}

/// Flujo completo hasta obtener tokens: register → authorize → exchange.
async fn full_grant(app: &TestApp, owner: &LoggedInOwner) -> (String, serde_json::Value) {
    let client_id = register_public_client(app).await;
    let (verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(app, &owner.cookie, &client_id, &challenge, None).await;
    let resp = exchange_code(app, &client_id, &code, &verifier).await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    (client_id, resp.json())
}

// ---------------------------------------------------------------------------
// Metadata y descubrimiento
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metadata_endpoints_are_json_not_spa_fallback() {
    let app = TestApp::spawn().await;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-authorization-server/mcp",
    ] {
        let resp = app
            .get_with_headers(path, &[("host", "futurefin.test")])
            .await;
        assert_eq!(resp.status, http::StatusCode::OK, "{path}: {resp:?}");
        let ct = resp
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("application/json"),
            "{path} debe ser JSON (si es text/html, el fallback SPA se lo está tragando): {ct}"
        );
    }
    let as_meta = app
        .get_with_headers(
            "/.well-known/oauth-authorization-server",
            &[("host", "futurefin.test")],
        )
        .await
        .json();
    assert_eq!(
        as_meta["code_challenge_methods_supported"],
        serde_json::json!(["S256"]),
        "S256 only"
    );
    assert_eq!(as_meta["issuer"], "http://futurefin.test");
    assert_eq!(
        as_meta["authorization_endpoint"],
        "http://futurefin.test/oauth/authorize"
    );
    let rs_meta = app
        .get_with_headers(
            "/.well-known/oauth-protected-resource/mcp",
            &[("host", "futurefin.test")],
        )
        .await
        .json();
    assert_eq!(rs_meta["resource"], "http://futurefin.test/mcp");
    assert_eq!(
        rs_meta["authorization_servers"],
        serde_json::json!(["http://futurefin.test"])
    );
}

#[tokio::test]
async fn metadata_issuer_follows_forwarded_headers_and_rejects_bad_host() {
    let app = TestApp::spawn().await;
    let resp = app
        .get_with_headers(
            "/.well-known/oauth-authorization-server",
            &[
                ("host", "interno:8080"),
                ("x-forwarded-host", "ff.example.com"),
                ("x-forwarded-proto", "https"),
            ],
        )
        .await;
    assert_eq!(resp.json()["issuer"], "https://ff.example.com");

    // Host con path (inyección) → 400, nunca metadata envenenada.
    let bad = app
        .get_with_headers(
            "/.well-known/oauth-authorization-server",
            &[("host", "evil.com/path")],
        )
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST, "{bad:?}");
}

#[tokio::test]
async fn mcp_401_advertises_resource_metadata_but_403_does_not() {
    let app = TestApp::spawn().await;
    let _owner = app.register_and_login_owner("alice").await;

    // Sin Bearer → 401 con resource_metadata (RFC 9728 §5.1).
    let unauth = app.mcp_initialize(None).await;
    assert_eq!(unauth.status, http::StatusCode::UNAUTHORIZED);
    let www = unauth
        .headers
        .get(http::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .expect("401 lleva WWW-Authenticate");
    assert!(
        www.contains(r#"resource_metadata="http://futurefin.test/.well-known/oauth-protected-resource/mcp""#),
        "el 401 anuncia la PRM con inserción de path: {www}"
    );

    // Un token OAuth vivo de un usuario que PIERDE la membership → 403 en /mcp, y ese
    // 403 NO lleva WWW-Authenticate: con el header, claude re-haría OAuth en bucle
    // (token nuevo, mismo 403, otra vuelta).
    let owner = LoggedInOwner {
        username: "alice".into(),
        cookie: app
            .post_json(
                "/v1/auth/login",
                serde_json::json!({"username": "alice", "password": "correct horse battery staple"}),
            )
            .await
            .session_cookie()
            .unwrap(),
        user_id: uuid::Uuid::nil(),
    };
    let (_client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().unwrap();
    assert_eq!(app.mcp_initialize(Some(access)).await.status, http::StatusCode::OK);

    sqlx::query("DELETE FROM installation_memberships")
        .execute(&app.pool)
        .await
        .expect("drop memberships");
    let forbidden = app.mcp_initialize(Some(access)).await;
    assert_eq!(forbidden.status, http::StatusCode::FORBIDDEN, "{forbidden:?}");
    assert!(
        forbidden.headers.get(http::header::WWW_AUTHENTICATE).is_none(),
        "el 403 no debe anunciar re-auth (anti-bucle)"
    );
}

#[tokio::test]
async fn pending_user_cannot_consent() {
    let app = TestApp::spawn().await;
    let _owner = app.register_and_login_owner("alice").await;
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
    let bob_cookie = login.session_cookie().unwrap();

    let client_id = register_public_client(&app).await;
    let denied = app
        .post_json_with_cookie(
            "/v1/oauth/authorize",
            serde_json::json!({
                "approve": true, "client_id": client_id, "redirect_uri": CLAUDE_CALLBACK,
                "response_type": "code", "code_challenge": dummy_challenge(),
                "code_challenge_method": "S256",
            }),
            &bob_cookie,
        )
        .await;
    assert_eq!(denied.status, http::StatusCode::FORBIDDEN, "{denied:?}");
    assert_eq!(app.count_rows("oauth_grants").await, 0);
}

// ---------------------------------------------------------------------------
// DCR
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dcr_registers_public_client() {
    let app = TestApp::spawn().await;
    let resp = app
        .post_json(
            "/oauth/register",
            serde_json::json!({
                "redirect_uris": [CLAUDE_CALLBACK, "https://claude.com/api/mcp/auth_callback"],
                "client_name": "Claude",
                "token_endpoint_auth_method": "none",
            }),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
    let body = resp.json();
    assert!(body["client_id"].as_str().unwrap().starts_with("ffc_"));
    assert!(body.get("client_secret").is_none(), "público: sin secreto");
    assert_eq!(body["token_endpoint_auth_method"], "none");

    // Default (sin method) ⇒ client_secret_basic con secreto de un solo vistazo.
    let confidential = app
        .post_json(
            "/oauth/register",
            serde_json::json!({"redirect_uris": ["https://example.com/cb"]}),
        )
        .await
        .json();
    assert_eq!(confidential["token_endpoint_auth_method"], "client_secret_basic");
    assert!(confidential["client_secret"]
        .as_str()
        .unwrap()
        .starts_with("ffcs_"));
    assert_eq!(confidential["client_secret_expires_at"], 0);
}

#[tokio::test]
async fn dcr_rejects_bad_redirect_uris() {
    let app = TestApp::spawn().await;
    let cases: Vec<serde_json::Value> = vec![
        serde_json::json!({}),                                      // ausente
        serde_json::json!({"redirect_uris": []}),                   // vacío
        serde_json::json!({"redirect_uris": ["http://evil.com/cb"]}), // http no-loopback
        serde_json::json!({"redirect_uris": ["ftp://x/cb"]}),       // esquema raro
        serde_json::json!({"redirect_uris": ["https://a.com/cb#frag"]}), // fragmento
        serde_json::json!({"redirect_uris": ["https://user:pw@a.com/cb"]}), // userinfo
        serde_json::json!({"redirect_uris": ["relative/path"]}),    // no absoluta
        serde_json::json!({"redirect_uris": [
            "https://a.com/1", "https://a.com/2", "https://a.com/3",
            "https://a.com/4", "https://a.com/5", "https://a.com/6"]}), // >5
    ];
    for case in cases {
        let resp = app.post_json("/oauth/register", case.clone()).await;
        assert_eq!(resp.status, http::StatusCode::BAD_REQUEST, "caso {case}: {resp:?}");
        assert_eq!(resp.json()["error"], "invalid_redirect_uri", "caso {case}");
    }
    // Loopback http con puerto dinámico (Claude Code CLI) SÍ se acepta.
    let ok = app
        .post_json(
            "/oauth/register",
            serde_json::json!({"redirect_uris": ["http://localhost:53682/callback", "http://127.0.0.1:8976/cb"], "token_endpoint_auth_method": "none"}),
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::CREATED, "{ok:?}");
}

#[tokio::test]
async fn dcr_ignores_unknown_metadata_fields() {
    let app = TestApp::spawn().await;
    // RFC 7591 §3.1: metadata desconocida se ignora, no rompe el registro.
    let resp = app
        .post_json(
            "/oauth/register",
            serde_json::json!({
                "redirect_uris": [CLAUDE_CALLBACK],
                "token_endpoint_auth_method": "none",
                "logo_uri": "https://claude.ai/logo.png",
                "contacts": ["a@b.c"],
                "unknown_future_field": {"nested": true},
            }),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
}

// ---------------------------------------------------------------------------
// Happy path completo y validaciones del canje
// ---------------------------------------------------------------------------

#[tokio::test]
async fn happy_path_code_to_access_token_to_mcp() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (verifier, challenge) = pkce_pair();
    let (code, redirect_to) =
        authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, Some("xyz-state")).await;

    // El redirect lleva state (eco literal) e iss (RFC 9207).
    let parsed = url::Url::parse(&redirect_to).unwrap();
    let q: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
    assert_eq!(q.get("state").map(|v| v.as_ref()), Some("xyz-state"));
    assert_eq!(q.get("iss").map(|v| v.as_ref()), Some("http://futurefin.test"));

    let resp = exchange_code(&app, &client_id, &code, &verifier).await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    assert_eq!(
        resp.headers
            .get(http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let body = resp.json();
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    let access = body["access_token"].as_str().unwrap();
    let refresh = body["refresh_token"].as_str().unwrap();
    assert!(access.starts_with("ffo_"));
    assert!(refresh.starts_with("ffr_"));

    // El access token autentica /mcp de verdad.
    let ok = app.mcp_initialize(Some(access)).await;
    assert_eq!(ok.status, http::StatusCode::OK, "{ok:?}");
}

#[tokio::test]
async fn token_rejects_wrong_code_verifier() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (_verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;

    let (other_verifier, _) = pkce_pair();
    let resp = exchange_code(&app, &client_id, &code, &other_verifier).await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST, "{resp:?}");
    assert_eq!(resp.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn token_rejects_redirect_uri_mismatch_at_exchange() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;

    let resp = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", "https://claude.com/api/mcp/auth_callback"),
                ("code_verifier", &verifier),
                ("client_id", &client_id),
            ],
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(resp.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn authorization_code_reuse_revokes_grant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;

    let first = exchange_code(&app, &client_id, &code, &verifier).await;
    assert_eq!(first.status, http::StatusCode::OK);
    let access = first.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(app.mcp_initialize(Some(&access)).await.status, http::StatusCode::OK);

    // Segundo canje del mismo code = robo ⇒ invalid_grant Y el grant muere entero.
    let second = exchange_code(&app, &client_id, &code, &verifier).await;
    assert_eq!(second.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(second.json()["error"], "invalid_grant");
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoked_reason FROM oauth_grants LIMIT 1")
            .fetch_one(&app.pool)
            .await
            .expect("grant row");
    assert_eq!(reason.as_deref(), Some("code_reuse"));
    assert_eq!(
        app.mcp_initialize(Some(&access)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "el access del primer canje muere con el grant"
    );
}

#[tokio::test]
async fn expired_authorization_code_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;

    sqlx::query("UPDATE oauth_authorization_codes SET expires_at = now() - interval '1 minute'")
        .execute(&app.pool)
        .await
        .expect("expire code");
    let resp = exchange_code(&app, &client_id, &code, &verifier).await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(resp.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn unknown_client_id_at_token_is_401_invalid_client() {
    let app = TestApp::spawn().await;
    // La señal de re-registro de claude: 401 invalid_client, no 400.
    let resp = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", "whatever"),
                ("client_id", "ffc_desconocido"),
            ],
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::UNAUTHORIZED, "{resp:?}");
    assert_eq!(resp.json()["error"], "invalid_client");
}

#[tokio::test]
async fn confidential_client_authenticates_with_basic_and_bad_secret_is_401() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let reg = app
        .post_json(
            "/oauth/register",
            serde_json::json!({
                "redirect_uris": [CLAUDE_CALLBACK],
                "client_name": "Cliente confidencial",
                "token_endpoint_auth_method": "client_secret_basic",
            }),
        )
        .await
        .json();
    let client_id = reg["client_id"].as_str().unwrap().to_string();
    let secret = reg["client_secret"].as_str().unwrap().to_string();

    let (verifier, challenge) = pkce_pair();
    let (code, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;

    // Secreto malo → 401 invalid_client (y el code sigue vivo: no se ha canjeado).
    let bad = app
        .post_form_with_basic_auth(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", CLAUDE_CALLBACK),
                ("code_verifier", &verifier),
            ],
            &client_id,
            "ffcs_incorrecto",
        )
        .await;
    assert_eq!(bad.status, http::StatusCode::UNAUTHORIZED, "{bad:?}");
    assert_eq!(bad.json()["error"], "invalid_client");

    let ok = app
        .post_form_with_basic_auth(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", CLAUDE_CALLBACK),
                ("code_verifier", &verifier),
            ],
            &client_id,
            &secret,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::OK, "{ok:?}");
    let access = ok.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(app.mcp_initialize(Some(&access)).await.status, http::StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Refresh tokens
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_rotates_and_reuse_kills_grant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (client_id, tokens) = full_grant(&app, &owner).await;
    let old_refresh = tokens["refresh_token"].as_str().unwrap().to_string();

    // Rotación: refresh nuevo + access nuevo.
    let rotated = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &old_refresh),
                ("client_id", &client_id),
            ],
        )
        .await;
    assert_eq!(rotated.status, http::StatusCode::OK, "{rotated:?}");
    let new_access = rotated.json()["access_token"].as_str().unwrap().to_string();
    let new_refresh = rotated.json()["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, old_refresh);
    assert_eq!(app.mcp_initialize(Some(&new_access)).await.status, http::StatusCode::OK);

    // Reusar el refresh viejo = robo ⇒ grant revocado y el access nuevo muere también.
    let reused = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &old_refresh),
                ("client_id", &client_id),
            ],
        )
        .await;
    assert_eq!(reused.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(reused.json()["error"], "invalid_grant");
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoked_reason FROM oauth_grants LIMIT 1")
            .fetch_one(&app.pool)
            .await
            .expect("grant row");
    assert_eq!(reason.as_deref(), Some("refresh_token_reuse"));
    assert_eq!(
        app.mcp_initialize(Some(&new_access)).await.status,
        http::StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn refresh_from_another_client_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_client_id, tokens) = full_grant(&app, &owner).await;
    let refresh = tokens["refresh_token"].as_str().unwrap();

    let other_client = register_public_client(&app).await;
    let resp = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
                ("client_id", &other_client),
            ],
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(resp.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn expired_access_token_is_401_on_mcp() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().unwrap();

    sqlx::query("UPDATE oauth_access_tokens SET expires_at = now() - interval '1 minute'")
        .execute(&app.pool)
        .await
        .expect("expire access token");
    assert_eq!(
        app.mcp_initialize(Some(access)).await.status,
        http::StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------
// Authorize (consentimiento SPA)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authorize_details_fatal_errors_never_redirect() {
    let app = TestApp::spawn().await;
    let client_id = register_public_client(&app).await;

    // Cliente desconocido → invalid_request, sin redirect_to (anti open-redirect).
    let unknown = app
        .get_with_headers(
            "/v1/oauth/authorize-details?client_id=ffc_nadie&redirect_uri=https%3A%2F%2Fevil.com%2Fcb&response_type=code&code_challenge=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&code_challenge_method=S256",
            &[("host", "futurefin.test")],
        )
        .await;
    assert_eq!(unknown.status, http::StatusCode::OK);
    let body = unknown.json();
    assert_eq!(body["status"], "invalid_request");
    assert_eq!(body["error_code"], "unknown_client");
    assert!(body.get("redirect_to").is_none());

    // redirect_uri no registrado → mismo contrato fatal.
    let mismatch = app
        .get_with_headers(
            &format!("/v1/oauth/authorize-details?client_id={client_id}&redirect_uri=https%3A%2F%2Fevil.com%2Fcb&response_type=code&code_challenge=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&code_challenge_method=S256"),
            &[("host", "futurefin.test")],
        )
        .await;
    let body = mismatch.json();
    assert_eq!(body["status"], "invalid_request");
    assert_eq!(body["error_code"], "redirect_uri_mismatch");
    assert!(body.get("redirect_to").is_none());
}

#[tokio::test]
async fn authorize_details_redirectable_and_plain_pkce_rejected() {
    let app = TestApp::spawn().await;
    let client_id = register_public_client(&app).await;
    let cb = urlenc(CLAUDE_CALLBACK);

    // plain PKCE → error redirigible al callback registrado.
    let resp = app
        .get_with_headers(
            &format!("/v1/oauth/authorize-details?client_id={client_id}&redirect_uri={cb}&response_type=code&code_challenge=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&code_challenge_method=plain&state=st1"),
            &[("host", "futurefin.test")],
        )
        .await;
    let body = resp.json();
    assert_eq!(body["status"], "redirect_error");
    let redirect_to = body["redirect_to"].as_str().unwrap();
    assert!(redirect_to.starts_with(CLAUDE_CALLBACK));
    assert!(redirect_to.contains("error=invalid_request"));
    assert!(redirect_to.contains("state=st1"));

    // resource ajeno → invalid_target, también redirigible.
    let resp = app
        .get_with_headers(
            &format!("/v1/oauth/authorize-details?client_id={client_id}&redirect_uri={cb}&response_type=code&code_challenge=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&code_challenge_method=S256&resource=https%3A%2F%2Fotro.example%2Fmcp"),
            &[("host", "futurefin.test")],
        )
        .await;
    let body = resp.json();
    assert_eq!(body["status"], "redirect_error");
    assert!(body["redirect_to"].as_str().unwrap().contains("error=invalid_target"));
}

#[tokio::test]
async fn authorize_details_works_without_session_and_reports_already_connected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (_verifier, challenge) = pkce_pair();
    let cb = urlenc(CLAUDE_CALLBACK);
    let details_uri = format!(
        "/v1/oauth/authorize-details?client_id={client_id}&redirect_uri={cb}&response_type=code&code_challenge={challenge}&code_challenge_method=S256"
    );

    // Sin sesión: valida y devuelve el nombre, sin already_connected.
    let anon = app
        .get_with_headers(&details_uri, &[("host", "futurefin.test")])
        .await;
    assert_eq!(anon.status, http::StatusCode::OK);
    let body = anon.json();
    assert_eq!(body["status"], "consent");
    assert_eq!(body["client_name"], "Claude");
    assert_eq!(body["redirect_host"], "claude.ai");
    assert!(body.get("already_connected").is_none());

    // Con sesión y grant previo: already_connected=true.
    authorize_and_get_code(&app, &owner.cookie, &client_id, &challenge, None).await;
    let with_grant = app
        .get_with_headers(
            &details_uri,
            &[("host", "futurefin.test"), ("cookie", &owner.cookie)],
        )
        .await
        .json();
    assert_eq!(with_grant["already_connected"], true);
    assert!(with_grant["connected_at"].is_string());
}

#[tokio::test]
async fn authorize_denied_redirects_with_access_denied_and_no_grant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (_verifier, challenge) = pkce_pair();

    let resp = app
        .post_json_with_cookie(
            "/v1/oauth/authorize",
            serde_json::json!({
                "approve": false, "client_id": client_id, "redirect_uri": CLAUDE_CALLBACK,
                "response_type": "code", "code_challenge": challenge,
                "code_challenge_method": "S256", "state": "st2",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let redirect_to = resp.json()["redirect_to"].as_str().unwrap().to_string();
    assert!(redirect_to.contains("error=access_denied"));
    assert!(redirect_to.contains("state=st2"));
    assert_eq!(app.count_rows("oauth_grants").await, 0, "deny no crea grant");
}

#[tokio::test]
async fn authorize_requires_session() {
    let app = TestApp::spawn().await;
    let _owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;
    let (_verifier, challenge) = pkce_pair();
    let resp = app
        .post_json(
            "/v1/oauth/authorize",
            serde_json::json!({
                "approve": true, "client_id": client_id, "redirect_uri": CLAUDE_CALLBACK,
                "response_type": "code", "code_challenge": challenge, "code_challenge_method": "S256",
            }),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reconsent_reuses_active_grant_row() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let client_id = register_public_client(&app).await;

    let (v1, c1) = pkce_pair();
    let (code1, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &c1, None).await;
    assert_eq!(exchange_code(&app, &client_id, &code1, &v1).await.status, http::StatusCode::OK);

    let (v2, c2) = pkce_pair();
    let (code2, _) = authorize_and_get_code(&app, &owner.cookie, &client_id, &c2, None).await;
    assert_eq!(exchange_code(&app, &client_id, &code2, &v2).await.status, http::StatusCode::OK);

    assert_eq!(
        app.count_rows("oauth_grants").await,
        1,
        "re-consentir la misma app reutiliza el grant (el panel no muestra duplicados)"
    );
}

// ---------------------------------------------------------------------------
// Panel de conexiones y revocación
// ---------------------------------------------------------------------------

#[tokio::test]
async fn panel_revocation_cuts_mcp_immediately() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().unwrap();
    assert_eq!(app.mcp_initialize(Some(access)).await.status, http::StatusCode::OK);

    let list = app
        .get_with_cookie("/v1/oauth/connections", &owner.cookie)
        .await;
    assert_eq!(list.status, http::StatusCode::OK);
    let connections = list.json();
    let conn = &connections.as_array().unwrap()[0];
    assert_eq!(conn["client_name"], "Claude");
    assert_eq!(conn["redirect_host"], "claude.ai");
    let id = conn["id"].as_str().unwrap().to_string();

    let del = app
        .delete_with_cookie(&format!("/v1/oauth/connections/{id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    assert_eq!(
        app.mcp_initialize(Some(access)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "revocar desde el panel corta al instante"
    );
    // El refresh tampoco sobrevive (grant muerto).
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let refreshed = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
                ("client_id", &_client_id),
            ],
        )
        .await;
    assert_eq!(refreshed.status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn connections_are_isolated_between_users() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "mario", "member").await;
    let (_client_id, _tokens) = full_grant(&app, &owner).await;

    let mario_list = app
        .get_with_cookie("/v1/oauth/connections", &member.cookie)
        .await
        .json();
    assert_eq!(mario_list.as_array().unwrap().len(), 0, "mario no ve el grant de alice");

    let alice_list = app
        .get_with_cookie("/v1/oauth/connections", &owner.cookie)
        .await
        .json();
    let id = alice_list.as_array().unwrap()[0]["id"].as_str().unwrap().to_string();
    let del = app
        .delete_with_cookie(&format!("/v1/oauth/connections/{id}"), &member.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NOT_FOUND, "id ajeno = mismo 404 que inexistente");
}

#[tokio::test]
async fn rfc7009_revoke_with_refresh_token_kills_grant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().unwrap();
    let refresh = tokens["refresh_token"].as_str().unwrap();

    let resp = app
        .post_form(
            "/oauth/revoke",
            &[("token", refresh), ("client_id", &client_id)],
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    assert_eq!(
        app.mcp_initialize(Some(access)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "revocar el refresh mata el grant entero (RFC 7009 §2.1)"
    );

    // Token desconocido ⇒ 200 igualmente (RFC 7009 §2.2).
    let unknown = app
        .post_form(
            "/oauth/revoke",
            &[("token", "ffr_desconocido"), ("client_id", &client_id)],
        )
        .await;
    assert_eq!(unknown.status, http::StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Kill-switch, shadowing y backup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_protocol_disabled_with_mcp_but_connections_panel_survives() {
    // Router a mano con mcp_enabled=false (patrón de mcp_http.rs).
    let (pool, _schema) = common::isolated_pool().await;
    let state = Arc::new(AppState::new(
        env!("CARGO_PKG_VERSION"),
        pool.clone(),
        false,
        30,
        false,
        None,
    ));
    let router = Router::new()
        .merge(routes::app_router(&state))
        .layer(Extension(state.clone()));
    let app = TestApp {
        router,
        pool,
        schema: _schema,
        state,
    };
    let owner = app.register_and_login_owner("alice").await;

    // Protocolo desmontado: 404 (en producción caería al fallback SPA).
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-authorization-server",
    ] {
        let resp = app.get_with_headers(path, &[("host", "futurefin.test")]).await;
        assert_eq!(resp.status, http::StatusCode::NOT_FOUND, "{path}");
    }
    let token = app.post_form("/oauth/token", &[("grant_type", "authorization_code")]).await;
    assert_eq!(token.status, http::StatusCode::NOT_FOUND);
    let register = app
        .post_json("/oauth/register", serde_json::json!({"redirect_uris": [CLAUDE_CALLBACK]}))
        .await;
    assert_eq!(register.status, http::StatusCode::NOT_FOUND);
    let details = app
        .get_with_headers("/v1/oauth/authorize-details?client_id=x", &[("host", "h")])
        .await;
    assert_eq!(details.status, http::StatusCode::NOT_FOUND);

    // El panel sobrevive (precedente /v1/api-tokens): sin él no podrías revocar
    // grants existentes con MCP apagado.
    let list = app
        .get_with_cookie("/v1/oauth/connections", &owner.cookie)
        .await;
    assert_eq!(list.status, http::StatusCode::OK, "{list:?}");
}

#[tokio::test]
async fn get_oauth_authorize_is_not_handled_by_the_api() {
    let app = TestApp::spawn().await;
    // En el harness (sin fallback SPA) debe ser 404 — un 405 significaría que alguien
    // registró una ruta backend en /oauth/authorize y axum ya no cae al fallback:
    // la pantalla de consentimiento moriría en producción.
    let resp = app.get("/oauth/authorize").await;
    assert_eq!(
        resp.status,
        http::StatusCode::NOT_FOUND,
        "jamás registres una ruta backend en /oauth/authorize"
    );
}

#[tokio::test]
async fn backup_export_works_with_oauth_grants_present() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_client_id, _tokens) = full_grant(&app, &owner).await;

    // Anti-deriva SQL (incidente v1.0.10): el export sigue funcionando con tablas OAuth
    // pobladas, y las credenciales quedan fuera del .ffbackup por construcción.
    let export = app
        .post_json_with_cookie(
            "/v1/backup/user-export",
            serde_json::json!({"password": "correct horse battery staple"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(export.status, http::StatusCode::OK, "{export:?}");
    assert_eq!(app.count_rows("oauth_grants").await, 1);
}

// ---------------------------------------------------------------------------
// Cruce de esquemas
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_and_api_token_schemes_do_not_cross() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().unwrap();

    // Un ffo_ válido en /mcp funciona, pero el refresh (ffr_) NO es un Bearer válido.
    let refresh = tokens["refresh_token"].as_str().unwrap();
    assert_eq!(
        app.mcp_initialize(Some(refresh)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "un refresh token no autentica /mcp"
    );
    // Y un ffo_ truncado/alterado tampoco.
    let tampered = format!("{}x", &access[..access.len() - 1]);
    assert_eq!(
        app.mcp_initialize(Some(&tampered)).await.status,
        http::StatusCode::UNAUTHORIZED
    );
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// REGRESIÓN — cambiar la contraseña corta también las credenciales OAuth.
///
/// El commit que introdujo el cambio de contraseña afirma revocar «las cuatro credenciales»
/// (cookie, `ffp_`, `ffo_`, `ffr_`). Las dos primeras las fija `account_and_members.rs`; estas
/// no las fijaba nada, y el corte de OAuth no es directo sino por JOIN contra `oauth_grants`:
/// un refactor de `require_oauth_access_token` que quitara ese JOIN pasaría verde dejando vivo
/// un access token de una cuenta que su dueño acaba de intentar recuperar.
#[tokio::test]
async fn changing_the_password_kills_oauth_access_and_refresh() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (client_id, tokens) = full_grant(&app, &owner).await;
    let access = tokens["access_token"].as_str().expect("access").to_string();
    let refresh = tokens["refresh_token"].as_str().expect("refresh").to_string();
    assert_eq!(
        app.mcp_initialize(Some(&access)).await.status,
        http::StatusCode::OK,
        "el access debe funcionar antes del cambio"
    );

    let cambio = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({
                "current_password": "correct horse battery staple",
                "new_password": "una contraseña distinta y larga",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(cambio.status, http::StatusCode::NO_CONTENT, "{cambio:?}");

    assert_eq!(
        app.mcp_initialize(Some(&access)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "el access token OAuth DEBE morir con el cambio de contraseña"
    );

    // Y el refresh no puede acuñar uno nuevo: es la mitad que de verdad importa, porque un
    // access caduca solo y un refresh no.
    let renovar = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", client_id.as_str()),
            ],
        )
        .await;
    assert_eq!(
        renovar.status,
        http::StatusCode::BAD_REQUEST,
        "el refresh no puede emitir un access nuevo tras el cambio: {renovar:?}"
    );
    assert_eq!(renovar.json()["error"], "invalid_grant", "{}", renovar.json());
}

/// REGRESIÓN — revocar la membresía corta las credenciales OAuth de esa persona.
///
/// Mismo razonamiento que el test de arriba, por el otro camino: el corte no toca los tokens,
/// revoca el grant, y todo depende de que las dos consultas sigan haciendo el JOIN.
#[tokio::test]
async fn revoking_a_membership_kills_that_users_oauth_grant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let (client_id, tokens) = full_grant(&app, &bob).await;
    let access = tokens["access_token"].as_str().expect("access").to_string();
    let refresh = tokens["refresh_token"].as_str().expect("refresh").to_string();
    assert_eq!(app.mcp_initialize(Some(&access)).await.status, http::StatusCode::OK);

    let out = app
        .delete_with_cookie(
            &format!("/v1/installation/members/{}", bob.user_id),
            &owner.cookie,
        )
        .await;
    assert_eq!(out.status, http::StatusCode::NO_CONTENT, "{out:?}");

    assert_eq!(
        app.mcp_initialize(Some(&access)).await.status,
        http::StatusCode::UNAUTHORIZED,
        "el access del expulsado debe dejar de valer"
    );
    let renovar = app
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", client_id.as_str()),
            ],
        )
        .await;
    assert_eq!(
        renovar.status,
        http::StatusCode::BAD_REQUEST,
        "el refresh del expulsado no puede acuñar nada: {renovar:?}"
    );
}
