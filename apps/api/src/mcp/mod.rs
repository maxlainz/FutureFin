//! Endpoint MCP embebido (`/mcp`, Streamable HTTP).
//!
//! Un único binario: el servicio Tower de rmcp se monta en el mismo router (y puerto)
//! que el API y la SPA. Auth por token Bearer (`ffp_…`, ver `handlers/api_tokens.rs`)
//! validada por middleware ANTES del protocolo MCP. Deshabilitable con
//! `FUTUREFIN_MCP_ENABLED=0` (el router ni se monta → 404 del fallback).

pub mod auth;
pub mod server;

use crate::state::AppState;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;

pub fn mcp_router(state: Arc<AppState>) -> Router {
    let factory_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(server::FutureFinMcp::new(factory_state.clone())),
        Arc::new(LocalSessionManager::default()),
        // El default de rmcp solo acepta Hosts loopback (anti DNS-rebinding para
        // servidores locales). Aquí el gate es el Bearer y el despliegue objetivo es
        // LAN/Cloudflare Tunnel con Host arbitrario → validación de Host desactivada.
        StreamableHttpServerConfig::default().disable_allowed_hosts(),
    );

    Router::new()
        .route_service("/mcp", service)
        .layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let state = state.clone();
                async move { auth::mcp_bearer_auth(state, req, next).await }
            },
        ))
}
