//! Autenticación Bearer del endpoint `/mcp`.
//!
//! Middleware axum que corta ANTES del servicio MCP: valida el token (`require_api_token`)
//! y la membership viva (`require_installation_member`), y deja un [`McpIdentity`] en las
//! extensions del request HTTP. rmcp propaga las `http::request::Parts` (extensions
//! incluidas) hasta el `RequestContext` de cada tool, así que la identidad viaja sin
//! estado global. Un fallo responde el 401/403 JSON habitual (`{error, message}`) más
//! `WWW-Authenticate: Bearer` para clientes MCP genéricos.

use crate::error::ApiError;
use crate::handlers::api_tokens::require_api_token;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::MembershipRole;
use crate::state::AppState;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use uuid::Uuid;

/// Identidad resuelta del token: SIEMPRE el estado vivo (rol e installation se leen en
/// cada request, nunca se congelan en el token).
#[derive(Debug, Clone)]
pub struct McpIdentity {
    pub user_id: Uuid,
    pub installation_id: Uuid,
    pub role: MembershipRole,
    #[allow(dead_code)]
    pub token_id: Uuid,
}

pub async fn mcp_bearer_auth(state: Arc<AppState>, mut req: Request, next: Next) -> Response {
    // Se clona el header antes de los awaits: el body de axum no es `Sync`, así que un
    // `&Request` retenido a través de un await haría el future no-`Send`.
    let authorization = req.headers().get(http::header::AUTHORIZATION).cloned();
    match authenticate(&state, authorization).await {
        Ok(identity) => {
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        Err(e) => {
            let mut resp = e.into_response();
            resp.headers_mut().insert(
                http::header::WWW_AUTHENTICATE,
                http::HeaderValue::from_static("Bearer"),
            );
            resp
        }
    }
}

async fn authenticate(
    state: &AppState,
    authorization: Option<http::HeaderValue>,
) -> Result<McpIdentity, ApiError> {
    let token = require_api_token(&state.pool, authorization.as_ref()).await?;
    let (installation_id, role) =
        require_installation_member(&state.pool, token.user_id).await?;
    Ok(McpIdentity {
        user_id: token.user_id,
        installation_id,
        role,
        token_id: token.token_id,
    })
}
