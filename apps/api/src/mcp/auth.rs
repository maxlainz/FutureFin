//! Autenticación Bearer del endpoint `/mcp`.
//!
//! Middleware axum que corta ANTES del servicio MCP. Acepta las DOS credenciales del
//! servidor MCP, despachadas por prefijo del Bearer: tokens de API (`ffp_`,
//! `require_api_token`) y access tokens OAuth (`ffo_`, `require_oauth_access_token`).
//! Tras cualquiera de las dos, la membership se re-resuelve VIVA
//! (`require_installation_member`) — nada se congela en la credencial (D14). La
//! identidad viaja en las extensions del request: rmcp propaga las
//! `http::request::Parts` hasta el `RequestContext` de cada tool.
//!
//! El 401 anuncia la Protected Resource Metadata (RFC 9728 §5.1) para que los clientes
//! OAuth descubran el authorization server. SOLO el 401: un 403 (usuario pending,
//! membership revocada) con `WWW-Authenticate` mandaría a claude a re-autenticarse en
//! bucle — token nuevo, mismo 403, otra vuelta.

use crate::error::ApiError;
use crate::handlers::api_tokens::require_api_token;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::MembershipRole;
use crate::oauth::access::require_oauth_access_token;
use crate::oauth::url::{public_base_url, resource_metadata_url};
use crate::state::AppState;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use std::sync::Arc;
use uuid::Uuid;

/// Qué credencial autenticó el request (para telemetría/futuras auditorías; las tools
/// solo consumen `user_id`/`installation_id`/`role`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum McpCredential {
    ApiToken { token_id: Uuid },
    OAuth { grant_id: Uuid, token_id: Uuid },
}

/// Identidad resuelta del token: SIEMPRE el estado vivo (rol e installation se leen en
/// cada request, nunca se congelan en el token).
#[derive(Debug, Clone)]
pub struct McpIdentity {
    pub user_id: Uuid,
    pub installation_id: Uuid,
    pub role: MembershipRole,
    #[allow(dead_code)]
    pub credential: McpCredential,
}

pub async fn mcp_bearer_auth(state: Arc<AppState>, mut req: Request, next: Next) -> Response {
    // Se clonan los headers antes de los awaits: el body de axum no es `Sync`, así que
    // un `&Request` retenido a través de un await haría el future no-`Send`.
    let authorization = req.headers().get(http::header::AUTHORIZATION).cloned();
    let headers = req.headers().clone();
    match authenticate(&state, authorization).await {
        Ok(identity) => {
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        Err(e) => {
            let status = e.status();
            let mut resp = e.into_response();
            if status == StatusCode::UNAUTHORIZED {
                let value = match public_base_url(&state, &headers) {
                    Ok(base) => format!(
                        r#"Bearer realm="FutureFin", resource_metadata="{}""#,
                        resource_metadata_url(&base)
                    ),
                    Err(_) => "Bearer".to_string(),
                };
                if let Ok(v) = http::HeaderValue::from_str(&value) {
                    resp.headers_mut().insert(http::header::WWW_AUTHENTICATE, v);
                }
            }
            resp
        }
    }
}

async fn authenticate(
    state: &AppState,
    authorization: Option<http::HeaderValue>,
) -> Result<McpIdentity, ApiError> {
    let bearer = authorization
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.strip_prefix("Bearer "))
        .map(str::trim);

    let (user_id, credential) = match bearer {
        Some(t) if t.starts_with(crate::oauth::ACCESS_TOKEN_PREFIX) => {
            let id = require_oauth_access_token(&state.pool, authorization.as_ref()).await?;
            (
                id.user_id,
                McpCredential::OAuth {
                    grant_id: id.grant_id,
                    token_id: id.token_id,
                },
            )
        }
        // Todo lo demás (incl. prefijos desconocidos) pasa por api_tokens, que ya
        // devuelve el 401 indistinto.
        _ => {
            let token = require_api_token(&state.pool, authorization.as_ref()).await?;
            (
                token.user_id,
                McpCredential::ApiToken {
                    token_id: token.token_id,
                },
            )
        }
    };

    let (installation_id, role) = require_installation_member(&state.pool, user_id).await?;
    Ok(McpIdentity {
        user_id,
        installation_id,
        role,
        credential,
    })
}

/// Gate de TODA tool de escritura MCP: rol con permiso de escritura + kill-switch vivo
/// `installation.mcp_write_enabled`. Se ejecuta por request (misma filosofía que el resto de la
/// identidad: revocar o apagar el toggle = corte en la siguiente llamada, sin reinicios).
///
/// Errores: `viewer` → `Forbidden` (código `forbidden`; la variante no lleva mensaje y
/// `sanitised_message` lo fija). Toggle apagado → `BadRequest` con prefijo `mcp_write_disabled:`
/// — BadRequest es la única variante que propaga el mensaje al wire, y el LLM necesita leer el
/// MOTIVO para poder explicárselo al usuario en vez de reintentar a ciegas.
pub(crate) async fn require_mcp_write(
    pool: &sqlx::PgPool,
    id: &McpIdentity,
) -> Result<(), ApiError> {
    if !crate::handlers::membership::role_can_write(id.role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let enabled: bool =
        sqlx::query_scalar(r#"SELECT mcp_write_enabled FROM installation WHERE id = $1"#)
            .bind(id.installation_id)
            .fetch_one(pool)
            .await?;
    if !enabled {
        return Err(ApiError::BadRequest(
            "mcp_write_disabled: la escritura vía MCP está desactivada en esta instalación \
             (Ajustes → MCP, solo el owner puede activarla)"
                .into(),
        ));
    }
    Ok(())
}
