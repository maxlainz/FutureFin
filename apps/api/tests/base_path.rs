//! Prefijo público por request en el shell de la SPA (`handlers/spa.rs` + `prefix.rs`), visto
//! desde el router entero.
//!
//! El invariante maestro va primero: **sin cabeceras de proxy el HTML sale byte a byte** como el
//! del disco (modo compose intacto, y cacheable con `no-cache` sin `Vary`). Con prefijo, el shell
//! depende de cabeceras → `no-store` + `Vary`, y todos los refs absolutos se reescriben.

mod common;

use common::{TestApp, TestConfig};

const SHELL: &str = concat!(
    "<!doctype html>\n<html lang=\"es\">\n  <head>\n",
    "    <script type=\"module\" src=\"/assets/x.js\"></script>\n",
    "  </head>\n  <body><div id=\"root\"></div></body>\n</html>\n",
);

async fn app_with_shell() -> TestApp {
    TestApp::spawn_with(TestConfig {
        with_spa_index: Some(SHELL.to_string()),
        ..Default::default()
    })
    .await
}

fn body(resp: &common::ResponseParts) -> String {
    String::from_utf8(resp.body.clone()).expect("body utf-8")
}

#[tokio::test]
async fn root_without_proxy_headers_is_the_shell_verbatim() {
    let app = app_with_shell().await;
    let resp = app.get("/").await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(
        resp.headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(body(&resp), SHELL, "el shell debe salir byte a byte");
    assert!(!body(&resp).contains("__FF_BASE__"));
    assert_eq!(
        resp.headers
            .get(http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    // `Vary` va también en la rama verbatim: es una variante más, y sin él un caché intermedio
    // devolvería este shell anónimo a una petición que sí trae `X-Remote-User-Id`.
    assert_eq!(
        resp.headers
            .get(http::header::VARY)
            .and_then(|v| v.to_str().ok()),
        Some("X-Ingress-Path, X-Forwarded-Prefix, X-Remote-User-Id")
    );
}

#[tokio::test]
async fn forwarded_prefix_rewrites_assets_and_injects_the_base() {
    let app = app_with_shell().await;
    let resp = app
        .get_with_headers("/", &[("x-forwarded-prefix", "/f")])
        .await;
    assert_eq!(resp.status, http::StatusCode::OK);
    let html = body(&resp);
    assert!(html.contains(r#"window.__FF_BASE__="/f""#), "html: {html}");
    assert!(html.contains(r#"src="/f/assets/x.js""#), "html: {html}");
    assert_eq!(
        resp.headers
            .get(http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert!(resp.headers.get(http::header::VARY).is_some());
}

#[tokio::test]
async fn ingress_path_wins_over_forwarded_prefix() {
    let app = app_with_shell().await;
    let resp = app
        .get_with_headers(
            "/",
            &[("x-ingress-path", "/i"), ("x-forwarded-prefix", "/f")],
        )
        .await;
    let html = body(&resp);
    assert!(html.contains(r#"window.__FF_BASE__="/i""#), "html: {html}");
    assert!(html.contains(r#"src="/i/assets/x.js""#), "html: {html}");
    assert!(!html.contains("/f/"));
}

#[tokio::test]
async fn invalid_ingress_path_falls_through_to_the_valid_header() {
    let app = app_with_shell().await;
    let resp = app
        .get_with_headers(
            "/",
            &[("x-ingress-path", "no-slash"), ("x-forwarded-prefix", "/f")],
        )
        .await;
    let html = body(&resp);
    assert!(html.contains(r#"window.__FF_BASE__="/f""#), "html: {html}");
    assert!(!html.contains("no-slash"));
}

/// Shell + SSO activado + peer de confianza: el flag depende del MISMO predicado que el
/// endpoint (`handlers/sso.rs`). Una identidad que no es un UUID no abre sesión, así que el
/// shell no puede anunciarla — antes decía `true` y la SPA se lanzaba a un POST condenado al 400.
async fn app_with_sso() -> TestApp {
    TestApp::spawn_with(TestConfig {
        with_spa_index: Some(SHELL.to_string()),
        trusted_header_auth: true,
        trusted_peers_any: true,
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn a_non_uuid_identity_header_does_not_advertise_sso() {
    let app = app_with_sso().await;
    // Con prefijo para que el shell se inyecte y el flag sea observable (sin prefijo y sin SSO
    // la respuesta es el fichero verbatim, que no lleva bootstrap ninguno).
    let resp = app
        .get_with_headers(
            "/",
            &[
                ("x-ingress-path", "/i"),
                ("x-remote-user-id", "no-soy-un-uuid"),
            ],
        )
        .await;
    let html = body(&resp);
    assert!(html.contains("window.__FF_SSO__=false;"), "html: {html}");
}

#[tokio::test]
async fn a_uuid_identity_header_advertises_sso() {
    let app = app_with_sso().await;
    let resp = app
        .get_with_headers(
            "/",
            &[("x-remote-user-id", "11111111-2222-3333-4444-555555555555")],
        )
        .await;
    let html = body(&resp);
    assert!(html.contains("window.__FF_SSO__=true;"), "html: {html}");
}
