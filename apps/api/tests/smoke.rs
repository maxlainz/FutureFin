//! Smoke test: el router se monta, las migraciones aplican, los endpoints triviales contestan.
//! Sirve como verificación de que la infraestructura de tests funciona.

mod common;

use common::TestApp;

#[tokio::test]
async fn health_returns_ok() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/health").await;
    assert_eq!(resp.status, http::StatusCode::OK);
    let body = resp.json();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "futurefin");
}

#[tokio::test]
async fn ready_returns_ok_when_db_reachable() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/ready").await;
    assert_eq!(resp.status, http::StatusCode::OK);
}

#[tokio::test]
async fn unauthenticated_protected_endpoint_returns_401() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/installation/session-context").await;
    assert_eq!(resp.status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn register_login_me_roundtrip() {
    let app = TestApp::spawn().await;

    let register_resp = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({
                "username": "alice",
                "password": "correct horse battery staple",
                "birth_date": "1990-04-15",
            }),
        )
        .await;
    assert_eq!(register_resp.status, http::StatusCode::CREATED);

    let login_resp = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({
                "username": "alice",
                "password": "correct horse battery staple",
            }),
        )
        .await;
    assert_eq!(login_resp.status, http::StatusCode::OK);
    let cookie = login_resp
        .session_cookie()
        .expect("login should set ff_session cookie");

    let me_resp = app.get_with_cookie("/v1/auth/me", &cookie).await;
    assert_eq!(me_resp.status, http::StatusCode::OK);
    let me = me_resp.json();
    assert_eq!(me["username"], "alice");
    assert_eq!(me["birth_date"], "1990-04-15");
}

#[tokio::test]
async fn first_register_bootstraps_installation_with_owner() {
    let app = TestApp::spawn().await;

    let register_resp = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({
                "username": "bob",
                "password": "correct horse battery staple",
                "birth_date": "1985-09-01",
            }),
        )
        .await;
    assert_eq!(register_resp.status, http::StatusCode::CREATED);

    let login_resp = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({
                "username": "bob",
                "password": "correct horse battery staple",
            }),
        )
        .await;
    let cookie = login_resp
        .session_cookie()
        .expect("login should set ff_session cookie");

    let ctx_resp = app
        .get_with_cookie("/v1/installation/session-context", &cookie)
        .await;
    assert_eq!(ctx_resp.status, http::StatusCode::OK);
    let ctx = ctx_resp.json();
    assert_eq!(ctx["installation_initialized"], true);
    assert_eq!(ctx["access"]["role"], "owner");
}

// ---------------------------------------------------------------------------
// Códigos de error estables (3.10.0)
// ---------------------------------------------------------------------------

/// El caso que originó el catálogo: registrarse con un usuario ya existente devolvía un 409 sin
/// nada más que «resource conflict», y la SPA lo pintaba tal cual. El `code` es lo que permite a
/// la interfaz decir «ese nombre de usuario ya está registrado».
#[tokio::test]
async fn duplicate_username_returns_a_specific_code_not_a_bare_conflict() {
    let app = TestApp::spawn().await;
    let body = serde_json::json!({
        "username": "alice",
        "password": "correct horse battery staple",
        "birth_date": "1990-01-01",
    });
    let first = app.post_json("/v1/auth/register", body.clone()).await;
    assert_eq!(first.status, http::StatusCode::CREATED);

    let second = app.post_json("/v1/auth/register", body).await;
    assert_eq!(second.status, http::StatusCode::CONFLICT);
    let json = second.json();
    assert_eq!(json["code"], "username_taken", "código granular: {json}");
    // La clase HTTP sigue publicándose igual que antes de 3.10.0.
    assert_eq!(json["error"], "conflict");
    assert!(
        json["message"].as_str().unwrap().starts_with("username_taken: "),
        "el mensaje conserva el prefijo del que sale el código: {json}"
    );
}

/// Los errores sin mensaje propio (401/403/404) siguen cayendo a su clase HTTP: no todo error
/// necesita código granular, y uno inventado sería peor que ninguno.
#[tokio::test]
async fn errors_without_a_message_fall_back_to_their_http_class() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/assets").await; // sin cookie de sesión
    assert_eq!(resp.status, http::StatusCode::UNAUTHORIZED);
    assert_eq!(resp.json()["code"], "unauthorized");
}

/// Una validación corriente viaja con su código, listo para traducir.
#[tokio::test]
async fn validation_errors_carry_their_code() {
    let app = TestApp::spawn().await;
    let resp = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({
                "username": "bob",
                "password": "corta",
                "birth_date": "1990-01-01",
            }),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(resp.json()["code"], "password_length");
}
