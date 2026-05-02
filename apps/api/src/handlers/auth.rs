use crate::auth::password;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use chrono::{Duration, Utc};
use cookie::{SameSite, time::Duration as CookieDuration};
use futurefin_domain::UserId;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "ff_session";

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: UserId,
    pub username: String,
}

#[derive(Debug, FromRow)]
struct UserAuthRow {
    id: Uuid,
    username: String,
    password_hash: String,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
}

fn validate_username(username: &str) -> Result<(), ApiError> {
    let trimmed = username.trim();
    if trimmed != username {
        return Err(ApiError::BadRequest(
            "username must not have leading or trailing whitespace".into(),
        ));
    }
    let len = username.chars().count();
    if !(3..=64).contains(&len) {
        return Err(ApiError::BadRequest(
            "username must be between 3 and 64 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ApiError::BadRequest(
            "username may only contain letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(())
}

fn map_unique_violation(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db) = err {
        if db.code().as_deref() == Some("23505") {
            return ApiError::Conflict;
        }
    }
    ApiError::Db(err)
}

#[utoipa::path(
    post,
    path = "/v1/auth/register",
    tag = "auth",
    request_body = RegisterBody,
    responses(
        (status = 201, description = "Account created", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Username already taken"),
    )
)]
pub async fn register(
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Result<(axum::http::StatusCode, Json<UserResponse>), ApiError> {
    validate_username(&body.username)?;
    let hash = password::hash_password(&body.password)?;
    let row: UserRow = sqlx::query_as(
        r#"INSERT INTO users (username, password_hash)
           VALUES ($1, $2)
           RETURNING id, username"#,
    )
    .bind(&body.username)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    .map_err(map_unique_violation)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(UserResponse {
            id: UserId(row.id),
            username: row.username,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "auth",
    request_body = LoginBody,
    responses(
        (status = 200, description = "Logged in; session cookie set", body = UserResponse),
        (status = 401, description = "Invalid credentials"),
    )
)]
pub async fn login(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Result<(CookieJar, Json<UserResponse>), ApiError> {
    validate_username(&body.username)?;
    let user: Option<UserAuthRow> =
        sqlx::query_as(r#"SELECT id, username, password_hash FROM users WHERE username = $1"#)
            .bind(&body.username)
            .fetch_optional(&state.pool)
            .await?;
    let Some(user) = user else {
        return Err(ApiError::Unauthorized);
    };
    password::verify_password(&body.password, &user.password_hash)?;
    let expires_at = Utc::now() + Duration::days(state.session_ttl_days);
    let sid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO sessions (user_id, expires_at) VALUES ($1, $2) RETURNING id"#,
    )
    .bind(user.id)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await?;
    let cookie = Cookie::build((SESSION_COOKIE, sid.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::days(state.session_ttl_days))
        .secure(state.cookie_secure)
        .build();
    let jar = jar.add(cookie);
    Ok((
        jar,
        Json(UserResponse {
            id: UserId(user.id),
            username: user.username,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Session cleared"),
    )
)]
pub async fn logout(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<(CookieJar, axum::http::StatusCode), ApiError> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        if let Ok(sid) = Uuid::parse_str(c.value()) {
            sqlx::query(r#"DELETE FROM sessions WHERE id = $1"#)
                .bind(sid)
                .execute(&state.pool)
                .await?;
        }
    }
    let jar = jar.remove(
        Cookie::build((SESSION_COOKIE, ""))
            .path("/")
            .build(),
    );
    Ok((jar, axum::http::StatusCode::NO_CONTENT))
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current user", body = UserResponse),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn me(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<UserResponse>, ApiError> {
    let Some(c) = jar.get(SESSION_COOKIE) else {
        return Err(ApiError::Unauthorized);
    };
    let sid = Uuid::parse_str(c.value()).map_err(|_| ApiError::Unauthorized)?;
    let row: Option<UserRow> = sqlx::query_as(
        r#"SELECT u.id, u.username
           FROM sessions s
           JOIN users u ON u.id = s.user_id
           WHERE s.id = $1 AND s.expires_at > now()"#,
    )
    .bind(sid)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::Unauthorized);
    };
    Ok(Json(UserResponse {
        id: UserId(row.id),
        username: row.username,
    }))
}
