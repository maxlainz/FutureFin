//! Servidor de autorización OAuth 2.1 embebido (v3.1.0) — la puerta del conector MCP de
//! claude.ai web. FutureFin es a la vez *authorization server* y *resource server* del
//! endpoint `/mcp`: no hay IdP externo ni claves de firma (D3/D14 — credenciales opacas
//! vivas en DB, jamás JWT).
//!
//! Este módulo es el PROTOCOLO (rutas raíz, fuera de OpenAPI, contrato fijado por las
//! RFC 8414/9728/7591/7009 y la spec de autorización MCP). La pantalla de consentimiento
//! y el panel de conexiones viven en `handlers/oauth_consent.rs` (`/v1/oauth/*`, cookie).
//!
//! OJO: jamás registres una ruta backend en `/oauth/authorize` — la sirve el fallback
//! SPA, y un method-mismatch (405) en axum NO cae al fallback: mataría la pantalla de
//! consentimiento en producción (test `get_oauth_authorize_is_not_handled_by_the_api`).

pub mod access;
pub mod authorize;
pub mod client_auth;
pub mod error;
pub mod metadata;
pub mod register;
pub mod token;
pub mod url;

use axum::routing::{get, post};
use axum::Router;

/// Prefijos reconocibles (coherentes con `ffp_` de api_tokens; útiles para
/// secret-scanning y para despachar Bearers sin tocar la DB).
pub const CLIENT_ID_PREFIX: &str = "ffc_";
pub const CLIENT_SECRET_PREFIX: &str = "ffcs_";
pub const ACCESS_TOKEN_PREFIX: &str = "ffo_";
pub const REFRESH_TOKEN_PREFIX: &str = "ffr_";

/// Rutas de protocolo (nivel raíz). Solo se montan con `state.mcp_enabled` — OAuth hoy
/// no sirve a nada más que a MCP. Las metadata van también con el sufijo `/mcp` porque
/// RFC 9728/8414 §3.1 insertan el path del recurso tras el `/.well-known/…` — montar
/// solo la raíz es la causa #1 de "connection failed" en claude.ai.
pub fn oauth_protocol_router() -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(metadata::protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(metadata::protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata::authorization_server),
        )
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(metadata::authorization_server),
        )
        .route("/oauth/register", post(register::register_client))
        .route("/oauth/token", post(token::token))
        .route("/oauth/revoke", post(token::revoke))
}
