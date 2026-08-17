//! Validación del Bearer `ffo_…` — el espejo OAuth de `require_api_token` (D14):
//! cualquier fallo es el MISMO 401 indistinto, lookup O(1) por hash exacto, y el JOIN
//! con `oauth_grants` hace que revocar el grant mate todos sus access tokens sin
//! tocarlos (una fila que actualizar para cortar todo, como borrar una sesión).
//!
//! Devuelve `ApiError` (no `OAuthError`): esto alimenta al middleware de `/mcp`, que
//! habla el formato de error del API propio.

use super::ACCESS_TOKEN_PREFIX;
use crate::auth::secret::sha256_hex;
use crate::error::ApiError;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct OAuthIdentity {
    pub user_id: Uuid,
    pub grant_id: Uuid,
    pub token_id: Uuid,
}

pub async fn require_oauth_access_token(
    pool: &PgPool,
    authorization: Option<&http::HeaderValue>,
) -> Result<OAuthIdentity, ApiError> {
    let Some(raw) = authorization.and_then(|v| v.to_str().ok()) else {
        return Err(ApiError::Unauthorized);
    };
    let Some(token) = raw.strip_prefix("Bearer ").map(str::trim) else {
        return Err(ApiError::Unauthorized);
    };
    if !token.starts_with(ACCESS_TOKEN_PREFIX) {
        return Err(ApiError::Unauthorized);
    }

    let hash = sha256_hex(token.as_bytes());
    let row: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT t.id, g.id, g.user_id
           FROM oauth_access_tokens t
           JOIN oauth_grants g ON g.id = t.grant_id
           WHERE t.token_hash = $1
             AND t.revoked_at IS NULL
             AND t.expires_at > now()
             AND g.revoked_at IS NULL"#,
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;
    let Some((token_id, grant_id, user_id)) = row else {
        return Err(ApiError::Unauthorized);
    };

    // Telemetría del panel («Último uso»), con el throttle de 60 s de api_tokens —
    // excepción acotada a D5, no una mutación de dominio.
    let _ = sqlx::query(
        r#"UPDATE oauth_grants SET last_used_at = now()
           WHERE id = $1
             AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')"#,
    )
    .bind(grant_id)
    .execute(pool)
    .await;

    Ok(OAuthIdentity {
        user_id,
        grant_id,
        token_id,
    })
}
