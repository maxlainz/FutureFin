use crate::error::ApiError;
use crate::handlers::household_access::{membership_role, role_can_read, role_can_write};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::routing::{get, patch};
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub household_id: Uuid,
    pub display_name: String,
    pub is_primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePersonBody {
    pub display_name: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub birth_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePersonBody {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub is_primary: Option<bool>,
}

#[derive(Debug, FromRow)]
struct PersonRow {
    id: Uuid,
    household_id: Uuid,
    display_name: String,
    is_primary: bool,
    birth_date: Option<NaiveDate>,
}

fn validate_display_name(name: &str) -> Result<(), ApiError> {
    let len = name.chars().count();
    if !(1..=128).contains(&len) {
        return Err(ApiError::BadRequest(
            "display_name must be between 1 and 128 characters".into(),
        ));
    }
    Ok(())
}

async fn ensure_membership_read(
    pool: &PgPool,
    user_id: Uuid,
    household_id: Uuid,
) -> Result<String, ApiError> {
    let role = membership_role(pool, user_id, household_id).await?;
    let Some(r) = role else {
        return Err(ApiError::NotFound);
    };
    if !role_can_read(&r) {
        return Err(ApiError::NotFound);
    }
    Ok(r)
}

async fn ensure_membership_write(
    pool: &PgPool,
    user_id: Uuid,
    household_id: Uuid,
) -> Result<(), ApiError> {
    let role = membership_role(pool, user_id, household_id).await?;
    let Some(r) = role else {
        return Err(ApiError::NotFound);
    };
    if !role_can_write(&r) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn clear_other_primaries(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hid: Uuid,
    except: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"UPDATE persons SET is_primary = false
           WHERE household_id = $1 AND id <> $2 AND is_primary = true"#,
    )
    .bind(hid)
    .bind(except)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/households/{household_id}/persons",
    tag = "persons",
    responses(
        (status = 200, description = "Household members", body = [PersonResponse]),
        (status = 401, description = "No valid session"),
        (status = 404, description = "Household not found or no access"),
    )
)]
pub async fn list_persons(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path(household_id): Path<Uuid>,
) -> Result<Json<Vec<PersonResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    ensure_membership_read(&state.pool, user.id.0, household_id).await?;

    let rows: Vec<PersonRow> = sqlx::query_as(
        r#"SELECT id, household_id, display_name, is_primary, birth_date
           FROM persons WHERE household_id = $1
           ORDER BY is_primary DESC, display_name ASC"#,
    )
    .bind(household_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| PersonResponse {
                id: r.id,
                household_id: r.household_id,
                display_name: r.display_name,
                is_primary: r.is_primary,
                birth_date: r.birth_date,
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/households/{household_id}/persons",
    tag = "persons",
    request_body = CreatePersonBody,
    responses(
        (status = 201, description = "Person created", body = PersonResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer cannot modify"),
        (status = 404, description = "Household not found or no access"),
        (status = 409, description = "Primary constraint conflict"),
    )
)]
pub async fn create_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path(household_id): Path<Uuid>,
    Json(body): Json<CreatePersonBody>,
) -> Result<(axum::http::StatusCode, Json<PersonResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    validate_display_name(&body.display_name)?;
    ensure_membership_write(&state.pool, user.id.0, household_id).await?;

    let mut tx = state.pool.begin().await?;

    let pid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO persons (household_id, display_name, is_primary, birth_date)
           VALUES ($1, $2, $3, $4)
           RETURNING id"#,
    )
    .bind(household_id)
    .bind(&body.display_name)
    .bind(body.is_primary)
    .bind(body.birth_date)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_person_db_error)?;

    if body.is_primary {
        clear_other_primaries(&mut tx, household_id, pid).await?;
    }

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(PersonResponse {
            id: pid,
            household_id,
            display_name: body.display_name,
            is_primary: body.is_primary,
            birth_date: body.birth_date,
        }),
    ))
}

fn map_person_db_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db) = err {
        if db.code().as_deref() == Some("23505") {
            return ApiError::Conflict;
        }
    }
    ApiError::Db(err)
}

#[utoipa::path(
    patch,
    path = "/v1/households/{household_id}/persons/{person_id}",
    tag = "persons",
    request_body = UpdatePersonBody,
    responses(
        (status = 200, description = "Updated", body = PersonResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer cannot modify"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Primary constraint conflict"),
    )
)]
pub async fn update_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path((household_id, person_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdatePersonBody>,
) -> Result<Json<PersonResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    ensure_membership_write(&state.pool, user.id.0, household_id).await?;

    if body.display_name.is_none() && body.is_primary.is_none() {
        return Err(ApiError::BadRequest(
            "provide display_name and/or is_primary".into(),
        ));
    }

    let existing: Option<PersonRow> = sqlx::query_as(
        r#"SELECT id, household_id, display_name, is_primary, birth_date
           FROM persons WHERE id = $1 AND household_id = $2"#,
    )
    .bind(person_id)
    .bind(household_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(mut row) = existing else {
        return Err(ApiError::NotFound);
    };

    if let Some(ref name) = body.display_name {
        validate_display_name(name)?;
        row.display_name.clone_from(name);
    }
    if let Some(p) = body.is_primary {
        row.is_primary = p;
    }

    let mut tx = state.pool.begin().await?;

    sqlx::query(
        r#"UPDATE persons SET display_name = $1, is_primary = $2
           WHERE id = $3 AND household_id = $4"#,
    )
    .bind(&row.display_name)
    .bind(row.is_primary)
    .bind(person_id)
    .bind(household_id)
    .execute(&mut *tx)
    .await
    .map_err(map_person_db_error)?;

    if row.is_primary {
        clear_other_primaries(&mut tx, household_id, person_id).await?;
    }

    tx.commit().await?;

    let updated: PersonRow = sqlx::query_as(
        r#"SELECT id, household_id, display_name, is_primary, birth_date
           FROM persons WHERE id = $1"#,
    )
    .bind(person_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(PersonResponse {
        id: updated.id,
        household_id: updated.household_id,
        display_name: updated.display_name,
        is_primary: updated.is_primary,
        birth_date: updated.birth_date,
    }))
}

#[utoipa::path(
    delete,
    path = "/v1/households/{household_id}/persons/{person_id}",
    tag = "persons",
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer cannot modify"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path((household_id, person_id)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    ensure_membership_write(&state.pool, user.id.0, household_id).await?;

    let result = sqlx::query(r#"DELETE FROM persons WHERE id = $1 AND household_id = $2"#)
        .bind(person_id)
        .bind(household_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn persons_router() -> Router {
    Router::new()
        .route("/{household_id}/persons", get(list_persons).post(create_person))
        .route(
            "/{household_id}/persons/{person_id}",
            patch(update_person).delete(delete_person),
        )
}
