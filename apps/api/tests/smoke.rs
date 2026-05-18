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
