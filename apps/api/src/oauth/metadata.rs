//! Documentos de descubrimiento OAuth: RFC 9728 (Protected Resource Metadata) y
//! RFC 8414 (Authorization Server Metadata). SELECT-free y mutación-free (D5): solo
//! reflejan la URL pública derivada del request (o `FUTUREFIN_PUBLIC_URL`).

use super::error::OAuthError;
use super::url::{mcp_resource_url, public_base_url};
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use http::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

/// RFC 9728 — apunta al AS (nosotros mismos). Servida en
/// `/.well-known/oauth-protected-resource` y con el sufijo `/mcp` (inserción de path §3.1).
pub async fn protected_resource(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(Json(json!({
        "resource": mcp_resource_url(&base),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
    })))
}

/// RFC 8414. Sin `scopes_supported` a propósito: MCP v1 es read-only entero y no hay
/// scopes con función. `S256` es el único PKCE admitido (claude.ai rechaza `plain`).
pub async fn authorization_server(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "revocation_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "authorization_response_iss_parameter_supported": true,
    })))
}
