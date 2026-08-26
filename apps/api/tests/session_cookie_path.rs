//! `Path` de la cookie de sesión (`handlers/auth.rs`).
//!
//! Bajo el Ingress de Home Assistant todos los add-ons comparten origen: un `Path=/` mandaría
//! `ff_session` a las rutas de ingress de los demás. La cookie se acota al prefijo de la request
//! — y el logout tiene que borrar **esa misma**, porque el navegador solo casa el borrado si el
//! `Path` coincide. Sin cabeceras de proxy, `Path=/` exactamente como siempre.

mod common;

use common::TestApp;

/// El `Set-Cookie` completo de `ff_session` (no solo el valor, que es lo que da
/// `ResponseParts::session_cookie`).
fn set_cookie(resp: &common::ResponseParts) -> String {
    resp.headers
        .get_all(http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().expect("set-cookie utf-8").to_string())
        .find(|s| s.starts_with("ff_session="))
        .expect("la respuesta debe traer Set-Cookie de ff_session")
}

const PASSWORD: &str = "correct horse battery staple";

async fn register(app: &TestApp, username: &str) {
    let reg = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({
                "username": username,
                "password": PASSWORD,
                "birth_date": "1990-01-01",
            }),
        )
        .await;
    assert_eq!(reg.status, http::StatusCode::CREATED, "register: {reg:?}");
}

fn login_body(username: &str) -> serde_json::Value {
    serde_json::json!({"username": username, "password": PASSWORD})
}

#[tokio::test]
async fn login_without_proxy_headers_scopes_the_cookie_to_root() {
    let app = TestApp::spawn().await;
    register(&app, "alice").await;
    let login = app.post_json("/v1/auth/login", login_body("alice")).await;
    assert_eq!(login.status, http::StatusCode::OK);
    let raw = set_cookie(&login);
    assert!(raw.contains("Path=/;") || raw.ends_with("Path=/"), "cookie: {raw}");
    assert!(!raw.contains("Path=/a"), "cookie: {raw}");
}

#[tokio::test]
async fn login_under_ingress_scopes_the_cookie_to_the_prefix() {
    let app = TestApp::spawn().await;
    register(&app, "alice").await;
    let login = app
        .post_with_headers(
            "/v1/auth/login",
            &[("x-ingress-path", "/a/b")],
            Some(login_body("alice")),
            None,
        )
        .await;
    assert_eq!(login.status, http::StatusCode::OK, "login: {login:?}");
    let raw = set_cookie(&login);
    assert!(raw.contains("Path=/a/b"), "cookie: {raw}");
}

#[tokio::test]
async fn logout_under_ingress_removes_the_cookie_on_the_same_path() {
    let app = TestApp::spawn().await;
    register(&app, "alice").await;
    let login = app
        .post_with_headers(
            "/v1/auth/login",
            &[("x-ingress-path", "/a/b")],
            Some(login_body("alice")),
            None,
        )
        .await;
    let cookie = login.session_cookie().expect("login sets ff_session");

    let logout = app
        .post_with_headers(
            "/v1/auth/logout",
            &[("x-ingress-path", "/a/b")],
            None,
            Some(&cookie),
        )
        .await;
    assert_eq!(logout.status, http::StatusCode::NO_CONTENT, "logout: {logout:?}");
    let raw = set_cookie(&logout);
    assert!(raw.contains("Path=/a/b"), "removal cookie: {raw}");
    // Es un borrado, no una cookie viva.
    assert!(
        raw.contains("Max-Age=0") || raw.contains("Expires="),
        "removal cookie: {raw}"
    );
}
