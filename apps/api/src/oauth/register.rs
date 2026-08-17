//! Dynamic Client Registration (RFC 7591) — registro PÚBLICO y sin autenticación.
//!
//! Es el flujo por defecto del conector de claude.ai y de Claude Code CLI (callback
//! localhost con puerto dinámico). Una fila en `oauth_clients` no da acceso a nada:
//! el gate real es el login + consentimiento explícito del usuario (`oauth_grants`).

use super::error::OAuthError;
use super::{CLIENT_ID_PREFIX, CLIENT_SECRET_PREFIX};
use crate::auth::secret::{generate_opaque_id, generate_opaque_secret, sha256_hex};
use axum::extract::rejection::JsonRejection;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use crate::state::AppState;
use std::sync::Arc;
use uuid::Uuid;

/// Cupo máximo de clientes registrados tras el GC (anti-flood del registro abierto).
const MAX_REGISTERED_CLIENTS: i64 = 1000;
/// Longitud máxima de cada redirect_uri y de client_uri.
const MAX_URI_LEN: usize = 512;

/// EXCEPCIÓN DOCUMENTADA a la regla strict-deserialization (§2.6 de change-control):
/// RFC 7591 §3.1 OBLIGA a ignorar metadata de cliente desconocida. No añadas
/// `deny_unknown_fields` aquí — rompería el registro de claude.ai, que manda campos
/// extra con normalidad.
#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub redirect_uris: Option<Vec<String>>,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Option<Vec<String>>,
    pub response_types: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub client_id: String,
    pub client_id_issued_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// 0 = el secreto no caduca (RFC 7591 §3.2.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_expires_at: Option<i64>,
    pub redirect_uris: Vec<String>,
    pub client_name: String,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
}

pub async fn register_client(
    Extension(state): Extension<Arc<AppState>>,
    body: Result<Json<RegisterBody>, JsonRejection>,
) -> Result<(StatusCode, Json<RegisterResponse>), OAuthError> {
    let Json(body) = body.map_err(|e| OAuthError::invalid_client_metadata(e.to_string()))?;

    // Validaciones ANTES de tocar la DB.
    let redirect_uris = validate_redirect_uris(body.redirect_uris.as_deref())?;
    let client_name = match body.client_name.as_deref().map(str::trim) {
        None | Some("") => "Cliente OAuth".to_string(),
        Some(n) if n.chars().count() <= 120 => n.to_string(),
        Some(_) => {
            return Err(OAuthError::invalid_client_metadata(
                "client_name must be at most 120 characters",
            ))
        }
    };
    let client_uri = match body.client_uri.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(u) => {
            let parsed = url::Url::parse(u)
                .map_err(|_| OAuthError::invalid_client_metadata("client_uri must be a valid URL"))?;
            if !matches!(parsed.scheme(), "http" | "https") || u.len() > MAX_URI_LEN {
                return Err(OAuthError::invalid_client_metadata("client_uri must be http(s)"));
            }
            Some(u.to_string())
        }
    };
    // Omitido ⇒ `client_secret_basic` (default RFC 7591 §2) y se emite secreto.
    let auth_method = match body.token_endpoint_auth_method.as_deref() {
        None => "client_secret_basic",
        Some(m @ ("none" | "client_secret_basic" | "client_secret_post")) => m,
        Some(other) => {
            return Err(OAuthError::invalid_client_metadata(format!(
                "unsupported token_endpoint_auth_method: {other}"
            )))
        }
    };
    let grant_types = match body.grant_types {
        None => vec!["authorization_code".to_string(), "refresh_token".to_string()],
        Some(gs) => {
            if gs.is_empty()
                || !gs
                    .iter()
                    .all(|g| matches!(g.as_str(), "authorization_code" | "refresh_token"))
            {
                return Err(OAuthError::invalid_client_metadata(
                    "supported grant_types: authorization_code, refresh_token",
                ));
            }
            gs
        }
    };
    let response_types = match body.response_types {
        None => vec!["code".to_string()],
        Some(rs) => {
            if !rs.iter().all(|r| r == "code") || rs.is_empty() {
                return Err(OAuthError::invalid_client_metadata(
                    "supported response_types: code",
                ));
            }
            rs
        }
    };

    gc_orphan_clients(&state.pool).await?;

    let client_id = generate_opaque_id(CLIENT_ID_PREFIX);
    let (client_secret, client_secret_hash) = if auth_method == "none" {
        (None, None)
    } else {
        let secret = generate_opaque_secret(CLIENT_SECRET_PREFIX);
        let hash = sha256_hex(secret.as_bytes());
        (Some(secret), Some(hash))
    };

    let (_row_id, issued_at): (Uuid, i64) = sqlx::query_as(
        r#"INSERT INTO oauth_clients
               (client_id, client_secret_hash, client_name, client_uri, redirect_uris,
                token_endpoint_auth_method)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, extract(epoch FROM created_at)::bigint"#,
    )
    .bind(&client_id)
    .bind(&client_secret_hash)
    .bind(&client_name)
    .bind(&client_uri)
    .bind(&redirect_uris)
    .bind(auth_method)
    .fetch_one(&state.pool)
    .await?;

    let secret_expires = client_secret.as_ref().map(|_| 0);
    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            client_id,
            client_id_issued_at: issued_at,
            client_secret,
            client_secret_expires_at: secret_expires,
            redirect_uris,
            client_name,
            token_endpoint_auth_method: auth_method.to_string(),
            grant_types,
            response_types,
        }),
    ))
}

/// https:// siempre; http:// solo loopback (localhost/127.0.0.1/[::1], cualquier puerto
/// y path — el caso del callback de puerto dinámico de Claude Code CLI). Sin fragmento
/// ni userinfo, absolutas, ≤512 chars, entre 1 y 5.
fn validate_redirect_uris(uris: Option<&[String]>) -> Result<Vec<String>, OAuthError> {
    let Some(uris) = uris.filter(|u| !u.is_empty()) else {
        return Err(OAuthError::invalid_redirect_uri("redirect_uris is required"));
    };
    if uris.len() > 5 {
        return Err(OAuthError::invalid_redirect_uri("at most 5 redirect_uris"));
    }
    for uri in uris {
        if uri.len() > MAX_URI_LEN {
            return Err(OAuthError::invalid_redirect_uri("redirect_uri too long"));
        }
        let parsed = url::Url::parse(uri)
            .map_err(|_| OAuthError::invalid_redirect_uri(format!("invalid redirect_uri: {uri}")))?;
        if parsed.fragment().is_some() {
            return Err(OAuthError::invalid_redirect_uri(
                "redirect_uri must not contain a fragment",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(OAuthError::invalid_redirect_uri(
                "redirect_uri must not contain userinfo",
            ));
        }
        match parsed.scheme() {
            "https" => {}
            "http" => {
                let loopback = matches!(
                    parsed.host_str(),
                    Some("localhost") | Some("127.0.0.1") | Some("[::1]") | Some("::1")
                );
                if !loopback {
                    return Err(OAuthError::invalid_redirect_uri(
                        "http redirect_uris are only allowed for loopback hosts",
                    ));
                }
            }
            _ => {
                return Err(OAuthError::invalid_redirect_uri(
                    "redirect_uri scheme must be https (or http on loopback)",
                ))
            }
        }
    }
    Ok(uris.to_vec())
}

/// GC perezoso del registro abierto: borra clientes de >24 h sin NINGÚN grant (nunca
/// uno consentido). Vive en este POST y no en un GET — D5, reads never mutate.
async fn gc_orphan_clients(pool: &PgPool) -> Result<(), OAuthError> {
    sqlx::query(
        r#"DELETE FROM oauth_clients c
           WHERE c.created_at < now() - interval '24 hours'
             AND NOT EXISTS (SELECT 1 FROM oauth_grants g WHERE g.client_id = c.id)"#,
    )
    .execute(pool)
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM oauth_clients")
        .fetch_one(pool)
        .await?;
    if count >= MAX_REGISTERED_CLIENTS {
        return Err(OAuthError::temporarily_unavailable(
            "client registration quota exhausted, retry later",
        ));
    }
    Ok(())
}
