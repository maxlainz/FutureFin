//! Token endpoint (`POST /oauth/token`) y revocación (`POST /oauth/revoke`, RFC 7009).
//!
//! Los dos grant types (authorization_code + PKCE, refresh_token con rotación) corren en
//! una transacción con `FOR UPDATE` sobre la fila de la credencial: el reuso de un code o
//! de un refresh ya consumido es la señal de robo y revoca el grant ENTERO (OAuth 2.1
//! §4.3.1 / §7.5). Las expiries las calcula Postgres, nunca Rust.

use super::client_auth::authenticate_client;
use super::error::OAuthError;
use super::{ACCESS_TOKEN_PREFIX, REFRESH_TOKEN_PREFIX};
use crate::auth::secret::{generate_opaque_secret, sha256_hex};
use crate::state::AppState;
use axum::extract::rejection::FormRejection;
use axum::extract::Extension;
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const ACCESS_TOKEN_TTL_SECS: i64 = 3600;
const ACCESS_TOKEN_INTERVAL: &str = "1 hour";
/// Sliding por construcción: cada rotación emite un refresh nuevo con 90 días.
const REFRESH_TOKEN_INTERVAL: &str = "90 days";

/// Mismo criterio que DCR: el token endpoint ignora campos desconocidos (los clientes
/// mandan extras como `audience` con normalidad) — no añadas `deny_unknown_fields`.
#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub refresh_token: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

pub async fn token(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    form: Result<Form<TokenForm>, FormRejection>,
) -> Result<Response, OAuthError> {
    let Form(form) = form.map_err(|e| OAuthError::invalid_request(e.to_string()))?;
    let client = authenticate_client(
        &state.pool,
        &headers,
        form.client_id.as_deref(),
        form.client_secret.as_deref(),
    )
    .await?;

    let body = match form.grant_type.as_deref() {
        Some("authorization_code") => exchange_code(&state.pool, client.row_id, &form).await?,
        Some("refresh_token") => refresh(&state.pool, client.row_id, &form).await?,
        _ => return Err(OAuthError::unsupported_grant_type()),
    };

    let mut resp = Json(body).into_response();
    // RFC 6749 §5.1: la respuesta transporta secretos.
    resp.headers_mut().insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

async fn exchange_code(
    pool: &PgPool,
    client_row_id: Uuid,
    form: &TokenForm,
) -> Result<TokenResponse, OAuthError> {
    let Some(code) = form.code.as_deref().filter(|c| !c.is_empty()) else {
        return Err(OAuthError::invalid_request("code is required"));
    };

    let mut tx = pool.begin().await?;
    let row: Option<CodeRow> = sqlx::query_as(
        r#"SELECT c.code_hash, c.grant_id, c.redirect_uri, c.code_challenge, c.resource,
                  c.scope, c.expires_at, c.consumed_at,
                  g.client_id AS grant_client_id, g.revoked_at AS grant_revoked_at
           FROM oauth_authorization_codes c
           JOIN oauth_grants g ON g.id = c.grant_id
           WHERE c.code_hash = $1
           FOR UPDATE OF c"#,
    )
    .bind(sha256_hex(code.as_bytes()))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Err(OAuthError::invalid_grant("unknown authorization code"));
    };

    if row.grant_client_id != client_row_id {
        return Err(OAuthError::invalid_grant("code was issued to another client"));
    }
    if row.grant_revoked_at.is_some() {
        return Err(OAuthError::invalid_grant("grant has been revoked"));
    }
    if row.consumed_at.is_some() {
        // Reuso de code = señal de robo: muere el grant entero (OAuth 2.1 §7.5).
        revoke_grant(&mut tx, row.grant_id, "code_reuse").await?;
        tx.commit().await?;
        return Err(OAuthError::invalid_grant("authorization code already used"));
    }
    if row.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant("authorization code expired"));
    }
    if form.redirect_uri.as_deref() != Some(row.redirect_uri.as_str()) {
        return Err(OAuthError::invalid_grant("redirect_uri mismatch"));
    }
    // PKCE S256: challenge == base64url(sha256(verifier)).
    let Some(verifier) = form
        .code_verifier
        .as_deref()
        .filter(|v| (43..=128).contains(&v.len()))
    else {
        return Err(OAuthError::invalid_grant("code_verifier is required"));
    };
    let digest = {
        use sha2::{Digest, Sha256};
        B64URL.encode(Sha256::digest(verifier.as_bytes()))
    };
    if digest != row.code_challenge {
        return Err(OAuthError::invalid_grant("PKCE verification failed"));
    }
    // RFC 8707: si el canje trae resource, debe ser el mismo que consintió el usuario.
    if let Some(resource) = form.resource.as_deref().filter(|s| !s.is_empty()) {
        if Some(resource.trim_end_matches('/'))
            != row.resource.as_deref().map(|r| r.trim_end_matches('/'))
        {
            return Err(OAuthError::invalid_target(
                "resource does not match the authorization request",
            ));
        }
    }

    sqlx::query("UPDATE oauth_authorization_codes SET consumed_at = now() WHERE code_hash = $1")
        .bind(&row.code_hash)
        .execute(&mut *tx)
        .await?;
    let (body, _new_refresh_id) = issue_token_pair(&mut tx, row.grant_id, row.scope).await?;
    tx.commit().await?;
    Ok(body)
}

async fn refresh(
    pool: &PgPool,
    client_row_id: Uuid,
    form: &TokenForm,
) -> Result<TokenResponse, OAuthError> {
    let Some(refresh_token) = form
        .refresh_token
        .as_deref()
        .filter(|t| t.starts_with(REFRESH_TOKEN_PREFIX))
    else {
        return Err(OAuthError::invalid_request("refresh_token is required"));
    };

    let mut tx = pool.begin().await?;
    let row: Option<RefreshRow> = sqlx::query_as(
        r#"SELECT r.id, r.grant_id, r.expires_at, r.consumed_at, r.revoked_at,
                  g.client_id AS grant_client_id, g.revoked_at AS grant_revoked_at,
                  g.scope AS grant_scope
           FROM oauth_refresh_tokens r
           JOIN oauth_grants g ON g.id = r.grant_id
           WHERE r.token_hash = $1
           FOR UPDATE OF r"#,
    )
    .bind(sha256_hex(refresh_token.as_bytes()))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Err(OAuthError::invalid_grant("unknown refresh token"));
    };

    if row.grant_client_id != client_row_id {
        return Err(OAuthError::invalid_grant(
            "refresh token was issued to another client",
        ));
    }
    if row.grant_revoked_at.is_some() || row.revoked_at.is_some() {
        return Err(OAuthError::invalid_grant("grant has been revoked"));
    }
    if row.consumed_at.is_some() {
        // Rotación: un refresh ya consumido que reaparece = robo ⇒ muere el grant.
        revoke_grant(&mut tx, row.grant_id, "refresh_token_reuse").await?;
        tx.commit().await?;
        return Err(OAuthError::invalid_grant("refresh token already used"));
    }
    if row.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant("refresh token expired"));
    }

    let (body, new_refresh_id) = issue_token_pair(&mut tx, row.grant_id, row.grant_scope).await?;
    // Consumir el refresh viejo y encadenarlo al nuevo (auditoría de la rotación).
    sqlx::query(
        r#"UPDATE oauth_refresh_tokens SET consumed_at = now(), replaced_by = $2
           WHERE id = $1"#,
    )
    .bind(row.id)
    .bind(new_refresh_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(body)
}

/// Inserta el par access (`ffo_`, 1 h) + refresh (`ffr_`, 90 días) del grant.
/// Devuelve también el id del refresh nuevo para encadenar la rotación.
async fn issue_token_pair(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    grant_id: Uuid,
    scope: Option<String>,
) -> Result<(TokenResponse, Uuid), OAuthError> {
    let access_token = generate_opaque_secret(ACCESS_TOKEN_PREFIX);
    sqlx::query(
        r#"INSERT INTO oauth_access_tokens (grant_id, token_hash, expires_at)
           VALUES ($1, $2, now() + $3::interval)"#,
    )
    .bind(grant_id)
    .bind(sha256_hex(access_token.as_bytes()))
    .bind(ACCESS_TOKEN_INTERVAL)
    .execute(&mut **tx)
    .await?;

    let refresh_token = generate_opaque_secret(REFRESH_TOKEN_PREFIX);
    let (refresh_id,): (Uuid,) = sqlx::query_as(
        r#"INSERT INTO oauth_refresh_tokens (grant_id, token_hash, expires_at)
           VALUES ($1, $2, now() + $3::interval)
           RETURNING id"#,
    )
    .bind(grant_id)
    .bind(sha256_hex(refresh_token.as_bytes()))
    .bind(REFRESH_TOKEN_INTERVAL)
    .fetch_one(&mut **tx)
    .await?;

    Ok((
        TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in: ACCESS_TOKEN_TTL_SECS,
            refresh_token,
            scope,
        },
        refresh_id,
    ))
}

async fn revoke_grant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    grant_id: Uuid,
    reason: &str,
) -> Result<(), OAuthError> {
    sqlx::query(
        r#"UPDATE oauth_grants SET revoked_at = now(), revoked_reason = $2
           WHERE id = $1 AND revoked_at IS NULL"#,
    )
    .bind(grant_id)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// RFC 7009. Un refresh token revoca el grant ENTERO (§2.1: el server debería revocar
/// los tokens relacionados — "desconectar" en claude corta todo); un access token
/// revoca solo su fila. Token desconocido ⇒ 200 igualmente (§2.2).
#[derive(Debug, Deserialize)]
pub struct RevokeForm {
    pub token: Option<String>,
    #[allow(dead_code)]
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

pub async fn revoke(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    form: Result<Form<RevokeForm>, FormRejection>,
) -> Result<Response, OAuthError> {
    let Form(form) = form.map_err(|e| OAuthError::invalid_request(e.to_string()))?;
    let client = authenticate_client(
        &state.pool,
        &headers,
        form.client_id.as_deref(),
        form.client_secret.as_deref(),
    )
    .await?;
    let Some(token) = form.token.as_deref().filter(|t| !t.is_empty()) else {
        return Err(OAuthError::invalid_request("token is required"));
    };
    let hash = sha256_hex(token.as_bytes());

    if token.starts_with(REFRESH_TOKEN_PREFIX) {
        sqlx::query(
            r#"UPDATE oauth_grants g SET revoked_at = now(), revoked_reason = 'rfc7009'
               FROM oauth_refresh_tokens r
               WHERE r.token_hash = $1 AND r.grant_id = g.id AND g.client_id = $2
                 AND g.revoked_at IS NULL"#,
        )
        .bind(&hash)
        .bind(client.row_id)
        .execute(&state.pool)
        .await?;
    } else if token.starts_with(ACCESS_TOKEN_PREFIX) {
        sqlx::query(
            r#"UPDATE oauth_access_tokens t SET revoked_at = now()
               FROM oauth_grants g
               WHERE t.token_hash = $1 AND t.grant_id = g.id AND g.client_id = $2
                 AND t.revoked_at IS NULL"#,
        )
        .bind(&hash)
        .bind(client.row_id)
        .execute(&state.pool)
        .await?;
    }

    let mut resp = http::StatusCode::OK.into_response();
    resp.headers_mut().insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

#[derive(sqlx::FromRow)]
struct CodeRow {
    code_hash: String,
    grant_id: Uuid,
    redirect_uri: String,
    code_challenge: String,
    resource: Option<String>,
    scope: Option<String>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
    grant_client_id: Uuid,
    grant_revoked_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct RefreshRow {
    id: Uuid,
    grant_id: Uuid,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    grant_client_id: Uuid,
    grant_revoked_at: Option<DateTime<Utc>>,
    grant_scope: Option<String>,
}
