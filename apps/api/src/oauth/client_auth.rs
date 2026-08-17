//! Autenticación del cliente en `/oauth/token` y `/oauth/revoke` (RFC 6749 §2.3).
//!
//! - `none`: cliente público (claude.ai, Claude Code CLI) — identifica el `client_id`
//!   del body; la protección real del canje es PKCE.
//! - `client_secret_post`: `client_id` + `client_secret` en el body.
//! - `client_secret_basic`: `Authorization: Basic base64(client_id:client_secret)`.
//!
//! Somos tolerantes con el CANAL del secreto (Basic o body) pero estrictos con su
//! presencia: un cliente confidencial sin secreto válido es 401 `invalid_client`.

use super::error::OAuthError;
use crate::auth::secret::sha256_hex;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use http::HeaderMap;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug)]
pub struct AuthenticatedClient {
    /// PK interna (`oauth_clients.id`) — la que referencian los grants.
    pub row_id: Uuid,
    #[allow(dead_code)]
    pub client_id: String,
}

pub async fn authenticate_client(
    pool: &PgPool,
    headers: &HeaderMap,
    body_client_id: Option<&str>,
    body_client_secret: Option<&str>,
) -> Result<AuthenticatedClient, OAuthError> {
    let basic = basic_credentials(headers);
    let client_id = basic
        .as_ref()
        .map(|(id, _)| id.as_str())
        .or(body_client_id)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OAuthError::invalid_client("client authentication required"))?;

    let row: Option<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        r#"SELECT id, client_id, client_secret_hash, token_endpoint_auth_method
           FROM oauth_clients WHERE client_id = $1"#,
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await?;
    // Cliente desconocido ⇒ 401 `invalid_client`: la señal con la que claude re-registra.
    let Some((row_id, client_id, secret_hash, auth_method)) = row else {
        return Err(OAuthError::invalid_client("unknown client"));
    };

    if auth_method != "none" {
        let presented = basic
            .as_ref()
            .map(|(_, s)| s.as_str())
            .or(body_client_secret)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| OAuthError::invalid_client("client secret required"))?;
        let expected = secret_hash
            .ok_or_else(|| OAuthError::invalid_client("client has no secret"))?;
        if sha256_hex(presented.as_bytes()) != expected {
            return Err(OAuthError::invalid_client("invalid client secret"));
        }
    }

    let _ = sqlx::query(
        r#"UPDATE oauth_clients SET last_used_at = now()
           WHERE id = $1
             AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')"#,
    )
    .bind(row_id)
    .execute(pool)
    .await;

    Ok(AuthenticatedClient { row_id, client_id })
}

fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?
        .trim();
    let decoded = B64.decode(raw).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}
