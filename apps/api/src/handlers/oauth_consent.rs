//! Endpoints `/v1/oauth/*` que consume la SPA: la pantalla de consentimiento
//! (`/oauth/authorize`, servida por el fallback SPA) y el panel de conexiones de
//! Ajustes → Acceso. Autenticación por cookie `ff_session`, como el resto de `/v1`.
//!
//! La validación del authorization request vive en `oauth::authorize` — aquí solo se
//! consume. CSRF del POST: cubierto por partida doble — la cookie es SameSite=Lax (un
//! POST cross-site no la lleva) y el body es JSON (no es "simple request": exige
//! preflight, que la lista blanca CORS bloquea). NO cambies el body a form-urlencoded.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::session::require_session_user;
use crate::oauth::authorize::{
    error_redirect, issue_authorization_code, success_redirect, validate_authorize_params,
    AuthorizeParamError, AuthorizeParams,
};
use crate::oauth::url::public_base_url;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizeDetailsStatus {
    /// Parámetros válidos: pintar la pantalla de consentimiento.
    Consent,
    /// Error FATAL (cliente/redirect inválidos): pintar el error, JAMÁS redirigir.
    InvalidRequest,
    /// Error redirigible: navegar a `redirect_to` (lleva `?error=…`).
    RedirectError,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorizeDetailsResponse {
    pub status: AuthorizeDetailsStatus,
    /// Nombre que declaró la app al registrarse — texto NO verificado.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    /// Host del redirect_uri — el único dato VERIFICADO que ve el usuario.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Solo se rellena con sesión válida: ya existe un grant activo de esta app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_connected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_to: Option<String>,
}

/// Sin sesión A PROPÓSITO: solo devuelve metadata que el propio cliente registró
/// (nombre + host del redirect), nada del usuario. A cambio, un redirect_uri que no
/// cuadra se ve ANTES de teclear la contraseña.
#[utoipa::path(
    get,
    path = "/v1/oauth/authorize-details",
    tag = "oauth",
    responses(
        (status = 200, description = "Validation result for the consent screen", body = AuthorizeDetailsResponse),
    )
)]
pub async fn authorize_details(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(params): Query<AuthorizeParams>,
) -> Result<Json<AuthorizeDetailsResponse>, ApiError> {
    let base = public_base_url(&state, &headers)
        .map_err(|_| ApiError::BadRequest("cannot derive public URL".into()))?;

    match validate_authorize_params(&state.pool, &base, &params).await {
        Ok(v) => {
            let (already_connected, connected_at) =
                match require_session_user(&jar, &state.pool).await {
                    Ok(user) => {
                        let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
                            r#"SELECT created_at FROM oauth_grants
                               WHERE client_id = $1 AND user_id = $2 AND revoked_at IS NULL"#,
                        )
                        .bind(v.client_row_id)
                        .bind(user.id.0)
                        .fetch_optional(&state.pool)
                        .await?;
                        (Some(row.is_some()), row.map(|(t,)| t))
                    }
                    Err(_) => (None, None),
                };
            Ok(Json(AuthorizeDetailsResponse {
                status: AuthorizeDetailsStatus::Consent,
                redirect_host: host_label(&v.redirect_uri),
                client_name: Some(v.client_name),
                client_uri: v.client_uri,
                resource: Some(v.resource),
                already_connected,
                connected_at,
                error_code: None,
                redirect_to: None,
            }))
        }
        Err(AuthorizeParamError::Fatal { code }) => Ok(Json(AuthorizeDetailsResponse {
            status: AuthorizeDetailsStatus::InvalidRequest,
            client_name: None,
            client_uri: None,
            redirect_host: None,
            resource: None,
            already_connected: None,
            connected_at: None,
            error_code: Some(code.to_string()),
            redirect_to: None,
        })),
        Err(AuthorizeParamError::Redirectable {
            code,
            description,
            redirect_uri,
            state: oauth_state,
        }) => {
            let redirect_to = error_redirect(
                &redirect_uri,
                code,
                Some(&description),
                oauth_state.as_deref(),
                &base,
            )
            .map_err(|_| ApiError::BadRequest("invalid redirect_uri".into()))?;
            Ok(Json(AuthorizeDetailsResponse {
                status: AuthorizeDetailsStatus::RedirectError,
                client_name: None,
                client_uri: None,
                redirect_host: None,
                resource: None,
                already_connected: None,
                connected_at: None,
                error_code: Some(code.to_string()),
                redirect_to: Some(redirect_to),
            }))
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorizeDecisionBody {
    pub approve: bool,
    #[serde(flatten)]
    pub params: AuthorizeParams,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorizeDecisionResponse {
    /// URL a la que la SPA debe navegar (lleva `code+state+iss` o `error=…`).
    pub redirect_to: String,
}

#[utoipa::path(
    post,
    path = "/v1/oauth/authorize",
    tag = "oauth",
    request_body = AuthorizeDecisionBody,
    responses(
        (status = 200, description = "Where to navigate next", body = AuthorizeDecisionResponse),
        (status = 400, description = "Fatal parameter error (unknown client / redirect mismatch)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn authorize_decision(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<AuthorizeDecisionBody>,
) -> Result<Json<AuthorizeDecisionResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;
    let base = public_base_url(&state, &headers)
        .map_err(|_| ApiError::BadRequest("cannot derive public URL".into()))?;

    let validated = match validate_authorize_params(&state.pool, &base, &body.params).await {
        Ok(v) => v,
        Err(AuthorizeParamError::Fatal { code }) => {
            return Err(ApiError::BadRequest(format!("authorize_error: {code}")));
        }
        Err(AuthorizeParamError::Redirectable {
            code,
            description,
            redirect_uri,
            state: oauth_state,
        }) => {
            let redirect_to = error_redirect(
                &redirect_uri,
                code,
                Some(&description),
                oauth_state.as_deref(),
                &base,
            )
            .map_err(|_| ApiError::BadRequest("invalid redirect_uri".into()))?;
            return Ok(Json(AuthorizeDecisionResponse { redirect_to }));
        }
    };

    let redirect_to = if body.approve {
        let code = issue_authorization_code(&state.pool, &validated, user.id.0).await?;
        success_redirect(
            &validated.redirect_uri,
            &code,
            validated.state.as_deref(),
            &base,
        )
        .map_err(|_| ApiError::BadRequest("invalid redirect_uri".into()))?
    } else {
        // Deny: el cliente recibe access_denied (no dejar a claude colgado esperando).
        error_redirect(
            &validated.redirect_uri,
            "access_denied",
            Some("el usuario ha cancelado la autorización"),
            validated.state.as_deref(),
            &base,
        )
        .map_err(|_| ApiError::BadRequest("invalid redirect_uri".into()))?
    };
    Ok(Json(AuthorizeDecisionResponse { redirect_to }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthConnectionResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// Nombre que declaró la app — texto NO verificado.
    pub client_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    /// Host del primer redirect_uri registrado (dato verificado).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_host: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
}

#[utoipa::path(
    get,
    path = "/v1/oauth/connections",
    tag = "oauth",
    responses(
        (status = 200, description = "The caller's active OAuth connections", body = Vec<OAuthConnectionResponse>),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_connections(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<OAuthConnectionResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;

    let rows: Vec<(Uuid, String, Option<String>, Vec<String>, DateTime<Utc>, Option<DateTime<Utc>>)> =
        sqlx::query_as(
            r#"SELECT g.id, c.client_name, c.client_uri, c.redirect_uris,
                      g.created_at, g.last_used_at
               FROM oauth_grants g
               JOIN oauth_clients c ON c.id = g.client_id
               WHERE g.user_id = $1 AND g.revoked_at IS NULL
               ORDER BY g.created_at DESC"#,
        )
        .bind(user.id.0)
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(
        rows.into_iter()
            .map(
                |(id, client_name, client_uri, redirect_uris, created_at, last_used_at)| {
                    OAuthConnectionResponse {
                        id,
                        client_name,
                        client_uri,
                        redirect_host: redirect_uris.first().and_then(|u| host_label(u)),
                        created_at,
                        last_used_at,
                    }
                },
            )
            .collect(),
    ))
}

#[utoipa::path(
    delete,
    path = "/v1/oauth/connections/{id}",
    tag = "oauth",
    params(("id" = Uuid, Path, description = "Connection (grant) id")),
    responses(
        (status = 204, description = "Revoked — the app loses access immediately"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Connection missing or not owned by the caller"),
    )
)]
pub async fn revoke_connection(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;

    // Solo grants propios; un id ajeno devuelve el mismo 404 que uno inexistente.
    let res = sqlx::query(
        r#"UPDATE oauth_grants SET revoked_at = now(), revoked_reason = 'user_panel'
           WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL"#,
    )
    .bind(id)
    .bind(user.id.0)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn host_label(uri: &str) -> Option<String> {
    url::Url::parse(uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

/// El panel de conexiones se monta SIEMPRE (precedente `/v1/api-tokens`: con MCP
/// apagado sigues pudiendo revocar credenciales que ya existen). Los endpoints del
/// flujo de consentimiento solo con MCP habilitado.
pub fn oauth_consent_router(mcp_enabled: bool) -> Router {
    let mut r = Router::new()
        .route("/connections", get(list_connections))
        .route("/connections/{id}", delete(revoke_connection));
    if mcp_enabled {
        r = r
            .route("/authorize-details", get(authorize_details))
            .route("/authorize", post(authorize_decision));
    }
    r
}
