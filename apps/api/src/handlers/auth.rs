use crate::auth::password;
use crate::error::ApiError;
use crate::handlers::installation::bootstrap_installation_as_owner_if_empty;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use cookie::{SameSite, time::Duration as CookieDuration};
use futurefin_domain::UserId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchMeBody {
    /// `null` borra la fecha; `"YYYY-MM-DD"` la fija. Omitir el campo no actualiza.
    #[serde(default)]
    #[schema(nullable = true, value_type = Object)]
    pub birth_date: Option<Value>,
}

#[derive(Debug, FromRow)]
struct UserAuthRow {
    id: Uuid,
    username: String,
    password_hash: String,
    birth_date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    birth_date: Option<NaiveDate>,
}

pub(crate) fn validate_username(username: &str) -> Result<(), ApiError> {
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

fn parse_me_birth_patch(v: &Value) -> Result<Option<NaiveDate>, ApiError> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .map(Some)
            .map_err(|_| ApiError::BadRequest("birth_date must be YYYY-MM-DD".into())),
        _ => Err(ApiError::BadRequest(
            "birth_date must be null or a date string".into(),
        )),
    }
}

fn validate_birth_date(d: NaiveDate) -> Result<(), ApiError> {
    let today = Utc::now().date_naive();
    if d > today {
        return Err(ApiError::BadRequest(
            "birth_date cannot be in the future".into(),
        ));
    }
    if d.year() < 1900 {
        return Err(ApiError::BadRequest(
            "birth_date year must be >= 1900".into(),
        ));
    }
    Ok(())
}

fn user_row_to_response(row: UserRow) -> UserResponse {
    UserResponse {
        id: UserId(row.id),
        username: row.username,
        birth_date: row.birth_date,
    }
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
    let mut tx = state.pool.begin().await?;
    let row: UserRow = sqlx::query_as(
        r#"INSERT INTO users (username, password_hash)
           VALUES ($1, $2)
           RETURNING id, username, birth_date"#,
    )
    .bind(&body.username)
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_unique_violation)?;

    match bootstrap_installation_as_owner_if_empty(&mut tx, &row.id).await
    {
        Ok(()) => {}
        Err(ApiError::Conflict) => {
            return Err(ApiError::Conflict);
        }
        Err(e) => return Err(e),
    }

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(user_row_to_response(row)),
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
    let user: Option<UserAuthRow> = sqlx::query_as(
        r#"SELECT id, username, password_hash, birth_date FROM users WHERE username = $1"#,
    )
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
            birth_date: user.birth_date,
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
    let user = require_session_user(&jar, &state.pool).await?;
    let row: UserRow = sqlx::query_as(
        r#"SELECT id, username, birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(user_row_to_response(row)))
}

#[utoipa::path(
    patch,
    path = "/v1/auth/me",
    tag = "auth",
    request_body = PatchMeBody,
    responses(
        (status = 200, description = "Perfil actualizado", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn patch_me(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PatchMeBody>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;

    if let Some(ref raw) = body.birth_date {
        let parsed = parse_me_birth_patch(raw)?;
        if let Some(d) = parsed {
            validate_birth_date(d)?;
        }
        sqlx::query(
            r#"UPDATE users SET birth_date = $1 WHERE id = $2"#,
        )
        .bind(parsed)
        .bind(user.id.0)
        .execute(&state.pool)
        .await?;
    }

    let row: UserRow = sqlx::query_as(
        r#"SELECT id, username, birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(user_row_to_response(row)))
}
