use crate::error::ApiError;
use crate::handlers::auth::SESSION_COOKIE;
use axum_extra::extract::cookie::CookieJar;
use futurefin_domain::UserId;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug)]
pub struct SessionUser {
    pub id: UserId,
    pub username: String,
}

pub async fn require_session_user(jar: &CookieJar, pool: &PgPool) -> Result<SessionUser, ApiError> {
    let Some(c) = jar.get(SESSION_COOKIE) else {
        return Err(ApiError::Unauthorized);
    };
    let sid = Uuid::parse_str(c.value()).map_err(|_| ApiError::Unauthorized)?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        r#"SELECT u.id, u.username
           FROM sessions s
           JOIN users u ON u.id = s.user_id
           WHERE s.id = $1 AND s.expires_at > now()"#,
    )
    .bind(sid)
    .fetch_optional(pool)
    .await?;
    let Some((id, username)) = row else {
        return Err(ApiError::Unauthorized);
    };
    Ok(SessionUser {
        id: UserId(id),
        username,
    })
}
