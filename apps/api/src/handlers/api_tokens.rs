//! Tokens de API por usuario (Bearer `ffp_…`) — la credencial del servidor MCP embebido.
//!
//! El secreto se genera una vez (`POST /v1/api-tokens`) y solo se persiste su SHA-256:
//! el lookup es O(1) por hash exacto y no hay comparación de secretos en Rust. El token
//! hereda la identidad del usuario y su rol VIVO vía `require_installation_member` (no se
//! congela el rol en la fila: revocar la membership mata el token al instante, misma
//! filosofía que las sesiones en DB). La gestión (crear/listar/revocar) va autenticada
//! por cookie de sesión como el resto del API; cualquier miembro puede crear los suyos
//! (un token no puede hacer nada que su dueño no pueda ya).
//!
//! Desde la Fase 3 (issue #84) el token lleva además un **scope** (`read_write` | `read_only`)
//! que solo RESTA: `read_only` corta las 31 tools de escritura de `/mcp` sin tocar el rol de la
//! persona, que sigue escribiendo en la web. Se lee VIVO en cada request —en el mismo SELECT que
//! autentica, sobre la misma fila que autoriza— así que no contradice D14: no hay nada congelado
//! en el secreto, solo una columna que se consulta cada vez, igual que `revoked_at`.

use crate::auth::secret::{generate_opaque_secret, sha256_hex};
use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Prefijo reconocible del secreto (útil para secret-scanning y para descartar
/// Bearers ajenos sin tocar la DB).
pub const TOKEN_PREFIX: &str = "ffp_";
/// Chars del token que se guardan en claro para identificarlo en la UI.
const VISIBLE_PREFIX_LEN: usize = 12;
/// Tokens activos (no revocados, no expirados) máximos por usuario.
const MAX_ACTIVE_TOKENS_PER_USER: i64 = 10;

/// Qué puede hacer una credencial de `/mcp`, con independencia del rol de su dueño.
///
/// `read_write` es el default de la columna y reproduce el comportamiento anterior al scope.
/// `read_only` es una restricción pura: nunca concede nada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
    ReadWrite,
    ReadOnly,
}

impl TokenScope {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenScope::ReadWrite => "read_write",
            TokenScope::ReadOnly => "read_only",
        }
    }

    /// Lee el valor de la columna. Un valor desconocido **falla cerrado** (`read_only`): el CHECK
    /// de la tabla ya impide que exista, así que llegar aquí significa que algo va mal — y en ese
    /// caso conceder escritura es el error caro.
    pub fn from_db(raw: &str) -> Self {
        match raw {
            "read_write" => TokenScope::ReadWrite,
            "read_only" => TokenScope::ReadOnly,
            other => {
                tracing::warn!(scope = other, "api_tokens.scope desconocido; se trata como read_only");
                TokenScope::ReadOnly
            }
        }
    }

    pub fn can_write(self) -> bool {
        matches!(self, TokenScope::ReadWrite)
    }
}

#[derive(Debug, Clone)]
pub struct ApiTokenIdentity {
    pub user_id: Uuid,
    pub token_id: Uuid,
    /// Scope VIVO leído en el mismo SELECT que autentica.
    pub scope: TokenScope,
}

/// Valida un header `Authorization: Bearer ffp_…` contra `api_tokens`.
///
/// Cualquier fallo (header ausente/malformado, prefijo desconocido, hash sin fila,
/// token revocado o expirado) es el mismo 401 — no se distingue para no filtrar
/// qué tokens existen. `last_used_at` se actualiza con throttle de 60 s: telemetría
/// de autenticación (análoga a `sessions`), no una mutación de dominio.
pub async fn require_api_token(
    pool: &PgPool,
    authorization: Option<&http::HeaderValue>,
) -> Result<ApiTokenIdentity, ApiError> {
    let Some(raw) = authorization.and_then(|v| v.to_str().ok()) else {
        return Err(ApiError::Unauthorized);
    };
    let Some(token) = raw.strip_prefix("Bearer ").map(str::trim) else {
        return Err(ApiError::Unauthorized);
    };
    if !token.starts_with(TOKEN_PREFIX) {
        return Err(ApiError::Unauthorized);
    }

    let hash = sha256_hex(token.as_bytes());
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT id, user_id, scope FROM api_tokens
           WHERE token_hash = $1
             AND revoked_at IS NULL
             AND (expires_at IS NULL OR expires_at > now())"#,
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;
    let Some((token_id, user_id, scope)) = row else {
        return Err(ApiError::Unauthorized);
    };

    let _ = sqlx::query(
        r#"UPDATE api_tokens SET last_used_at = now()
           WHERE id = $1
             AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')"#,
    )
    .bind(token_id)
    .execute(pool)
    .await;

    Ok(ApiTokenIdentity {
        user_id,
        token_id,
        scope: TokenScope::from_db(&scope),
    })
}

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct ApiTokenResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub label: String,
    /// Primeros caracteres del secreto (`ffp_XXXXXXXX`) — identifica el token sin exponerlo.
    pub token_prefix: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    /// `read_write` (default histórico) o `read_only`. Un token `read_only` autentica igual y
    /// lee igual, pero ninguna tool de escritura de `/mcp` lo acepta.
    pub scope: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiTokenBody {
    pub label: String,
    /// Omitido = el token no expira.
    #[serde(default)]
    pub expires_in_days: Option<u32>,
    /// `read_write` | `read_only`. Omitido = `read_write`, que es el comportamiento de todos los
    /// tokens emitidos antes de que existiera el scope. Se valida a mano (y no por serde) para
    /// devolver `token_scope_invalid` en vez del error de deserialización genérico.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateApiTokenResponse {
    #[serde(flatten)]
    pub token_info: ApiTokenResponse,
    /// El secreto completo. SOLO viaja en esta respuesta; no vuelve a mostrarse.
    pub token: String,
}

const TOKEN_COLUMNS: &str =
    "id, label, token_prefix, created_at, expires_at, last_used_at, revoked_at, scope";

#[utoipa::path(
    get,
    path = "/v1/api-tokens",
    tag = "api-tokens",
    responses(
        (status = 200, description = "The caller's API tokens (never the secret)", body = Vec<ApiTokenResponse>),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_api_tokens(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<ApiTokenResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;

    let sql = format!(
        r#"SELECT {TOKEN_COLUMNS} FROM api_tokens
           WHERE user_id = $1
           ORDER BY created_at DESC"#
    );
    let rows: Vec<ApiTokenResponse> = sqlx::query_as(&sql)
        .bind(user.id.0)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

#[utoipa::path(
    post,
    path = "/v1/api-tokens",
    tag = "api-tokens",
    request_body = CreateApiTokenBody,
    responses(
        (status = 201, description = "Created. The `token` field carries the secret exactly once.", body = CreateApiTokenResponse),
        (status = 400, description = "Validation error or active-token limit reached"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn create_api_token(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateApiTokenBody>,
) -> Result<(axum::http::StatusCode, Json<CreateApiTokenResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;

    let label = body.label.trim();
    if label.is_empty() || label.len() > 64 {
        return Err(ApiError::BadRequest(
            "token_label_length: label must be between 1 and 64 characters".into(),
        ));
    }
    if let Some(days) = body.expires_in_days {
        if !(1..=3650).contains(&days) {
            return Err(ApiError::BadRequest(
                "token_expiry_out_of_range: expires_in_days must be between 1 and 3650".into(),
            ));
        }
    }
    // Literal completo, nunca compuesto con `format!`: `error_codes_parity` extrae los códigos
    // del fuente y uno compuesto degradaría en silencio al mensaje genérico de la SPA.
    let scope = match body.scope.as_deref() {
        None | Some("read_write") => TokenScope::ReadWrite,
        Some("read_only") => TokenScope::ReadOnly,
        Some(_) => {
            return Err(ApiError::BadRequest(
                "token_scope_invalid: scope must be 'read_write' or 'read_only'".into(),
            ))
        }
    };

    let active: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM api_tokens
           WHERE user_id = $1
             AND revoked_at IS NULL
             AND (expires_at IS NULL OR expires_at > now())"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;
    if active >= MAX_ACTIVE_TOKENS_PER_USER {
        return Err(ApiError::BadRequest(format!(
            "token_limit_reached: at most {MAX_ACTIVE_TOKENS_PER_USER} active tokens per user"
        )));
    }

    let token = generate_opaque_secret(TOKEN_PREFIX);
    let token_hash = sha256_hex(token.as_bytes());
    let token_prefix: String = token.chars().take(VISIBLE_PREFIX_LEN).collect();

    let sql = format!(
        r#"INSERT INTO api_tokens (user_id, label, token_hash, token_prefix, expires_at, scope)
           VALUES ($1, $2, $3, $4, now() + make_interval(days => $5), $6)
           RETURNING {TOKEN_COLUMNS}"#
    );
    // make_interval(days => NULL) => NULL → expires_at queda NULL cuando no se pide caducidad.
    let row: ApiTokenResponse = sqlx::query_as(&sql)
        .bind(user.id.0)
        .bind(label)
        .bind(&token_hash)
        .bind(&token_prefix)
        .bind(body.expires_in_days.map(|d| d as i32))
        .bind(scope.as_str())
        .fetch_one(&state.pool)
        .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApiTokenResponse {
            token_info: row,
            token,
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/v1/api-tokens/{id}",
    tag = "api-tokens",
    params(
        ("id" = Uuid, Path, description = "Token id"),
    ),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Token missing or not owned by the caller"),
    )
)]
pub async fn revoke_api_token(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    require_installation_member(&state.pool, user.id.0).await?;

    // Solo tokens propios; un id ajeno devuelve el mismo 404 que uno inexistente.
    let res = sqlx::query(
        r#"UPDATE api_tokens SET revoked_at = now()
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

pub fn api_tokens_router() -> Router {
    Router::new()
        .route("/", get(list_api_tokens).post(create_api_token))
        .route("/{id}", axum::routing::delete(revoke_api_token))
}
