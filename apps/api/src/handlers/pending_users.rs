use crate::error::ApiError;
use crate::handlers::auth::UserResponse;
use crate::handlers::installation::{singleton_installation_id, user_is_installation_owner};
use crate::handlers::membership::MembershipRole;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use futurefin_domain::UserId;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApproveMemberRole {
    Member,
    Viewer,
}

impl ApproveMemberRole {
    fn to_membership(self) -> MembershipRole {
        match self {
            ApproveMemberRole::Member => MembershipRole::Member,
            ApproveMemberRole::Viewer => MembershipRole::Viewer,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApprovePendingUserBody {
    pub role: ApproveMemberRole,
}

#[derive(Debug, FromRow)]
struct PendingUserRow {
    id: Uuid,
    username: String,
    birth_date: Option<NaiveDate>,
}

#[utoipa::path(
    get,
    path = "/v1/installation/pending-users",
    tag = "installation",
    responses(
        (status = 200, description = "Registered users without installation membership", body = [UserResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation owner"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_pending_users(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let Some(iid) = singleton_installation_id(&state.pool).await? else {
        return Err(ApiError::NotFound);
    };
    if !user_is_installation_owner(&state.pool, user.id.0, iid).await? {
        return Err(ApiError::Forbidden);
    }

    let rows: Vec<PendingUserRow> = sqlx::query_as(
        r#"SELECT u.id, u.username, u.birth_date
           FROM users u
           WHERE NOT EXISTS (
               SELECT 1
               FROM installation_memberships m
               WHERE
                   m.user_id = u.id
                   AND m.installation_id = $1
           )
           ORDER BY u.created_at ASC"#,
    )
    .bind(iid)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| UserResponse {
                id: UserId(r.id),
                username: r.username,
                birth_date: r.birth_date,
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/installation/pending-users/{user_id}/approve",
    tag = "installation",
    request_body = ApprovePendingUserBody,
    responses(
        (status = 204, description = "Membership granted"),
        (status = 400, description = "User already has access"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation owner"),
        (status = 404, description = "Installation or user missing"),
    )
)]
pub async fn approve_pending_user(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path(target_user_id): Path<Uuid>,
    Json(body): Json<ApprovePendingUserBody>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let Some(iid) = singleton_installation_id(&state.pool).await? else {
        return Err(ApiError::NotFound);
    };
    if !user_is_installation_owner(&state.pool, user.id.0, iid).await? {
        return Err(ApiError::Forbidden);
    }

    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)"#,
    )
    .bind(target_user_id)
    .fetch_one(&state.pool)
    .await?;

    if !exists {
        return Err(ApiError::NotFound);
    }

    let already: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1
               FROM installation_memberships
               WHERE
                   installation_id = $1
                   AND user_id = $2
           )"#,
    )
    .bind(iid)
    .bind(target_user_id)
    .fetch_one(&state.pool)
    .await?;

    if already {
        return Err(ApiError::Conflict);
    }

    let role = body.role.to_membership();

    sqlx::query(
        r#"INSERT INTO installation_memberships (installation_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(iid)
    .bind(target_user_id)
    .bind(role.as_str())
    .execute(&state.pool)
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn pending_users_router() -> Router {
    Router::new()
        .route("/", get(list_pending_users))
        .route("/{user_id}/approve", post(approve_pending_user))
}
