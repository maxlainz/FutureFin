//! Anti-clickjacking condicionado al peer (`handlers/frame.rs`).
//!
//! El contrato tiene dos mitades y las dos importan: sin peer de confianza la respuesta lleva
//! `X-Frame-Options: DENY` **aunque llegue `X-Ingress-Path`** (si no, mandar un header bastaría
//! para desactivar la protección desde fuera), y con peer de confianza + header se relaja a
//! `Content-Security-Policy: frame-ancestors 'self'` **sin** `X-Frame-Options` (un `DENY`
//! superviviente ganaría sobre la CSP y el add-on saldría en blanco dentro del Ingress de HA).

mod common;

use common::{TestApp, TestConfig};

const CSP: &str = "content-security-policy";
const XFO: &str = "x-frame-options";

fn header<'a>(resp: &'a common::ResponseParts, name: &str) -> Option<&'a str> {
    resp.headers.get(name).map(|v| v.to_str().expect("header utf-8"))
}

#[tokio::test]
async fn default_response_denies_framing() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/health").await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(header(&resp, XFO), Some("DENY"));
    assert_eq!(header(&resp, CSP), None);
}

#[tokio::test]
async fn ingress_header_from_untrusted_peer_still_denies() {
    let app = TestApp::spawn().await;
    let resp = app
        .get_with_headers("/v1/health", &[("x-ingress-path", "/a")])
        .await;
    assert_eq!(header(&resp, XFO), Some("DENY"));
    assert_eq!(header(&resp, CSP), None);
}

#[tokio::test]
async fn ingress_header_from_trusted_peer_relaxes_to_frame_ancestors_self() {
    let app = TestApp::spawn_with(TestConfig {
        trusted_peers_any: true,
        ..Default::default()
    })
    .await;
    let resp = app
        .get_with_headers("/v1/health", &[("x-ingress-path", "/a")])
        .await;
    assert_eq!(header(&resp, CSP), Some("frame-ancestors 'self'"));
    assert_eq!(header(&resp, XFO), None);
}

#[tokio::test]
async fn trusted_peer_without_the_header_still_denies() {
    let app = TestApp::spawn_with(TestConfig {
        trusted_peers_any: true,
        ..Default::default()
    })
    .await;
    let resp = app.get("/v1/health").await;
    assert_eq!(header(&resp, XFO), Some("DENY"));
    assert_eq!(header(&resp, CSP), None);
}
