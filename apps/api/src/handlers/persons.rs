//! CRUD for installation persons (nombre, primaria, fecha nacimiento).

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub display_name: String,
    pub is_primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePersonBody {
    pub display_name: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub birth_date: Option<NaiveDate>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPersonBody {
    pub display_name: Option<String>,
    pub is_primary: Option<bool>,
    /// Omit = sin cambio; `null` en JSON borra la fecha; `"YYYY-MM-DD"` = asignar.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub birth_date: Option<Value>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct PersonRow {
    id: Uuid,
    display_name: String,
    is_primary: bool,
    birth_date: Option<NaiveDate>,
    sort_index: i32,
}

fn row_to_response(r: PersonRow) -> PersonResponse {
    PersonResponse {
        id: r.id,
        display_name: r.display_name,
        is_primary: r.is_primary,
        birth_date: r.birth_date,
        sort_index: r.sort_index,
    }
}

async fn clear_other_primaries_tx(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: Uuid,
    except_id: Option<Uuid>,
) -> Result<(), ApiError> {
    match except_id {
        Some(eid) => {
            sqlx::query(
                r#"UPDATE persons SET is_primary = false, updated_at = now()
                   WHERE installation_id = $1 AND id <> $2"#,
            )
            .bind(installation_id)
            .bind(eid)
            .execute(&mut **tx)
            .await?;
        }
        None => {
            sqlx::query(
                r#"UPDATE persons SET is_primary = false, updated_at = now()
                   WHERE installation_id = $1"#,
            )
            .bind(installation_id)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn promote_first_remaining_primary_tx(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: Uuid,
) -> Result<(), ApiError> {
    let next: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM persons
           WHERE installation_id = $1
           ORDER BY sort_index ASC, display_name ASC
           LIMIT 1"#,
    )
    .bind(installation_id)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(pid) = next {
        sqlx::query(
            r#"UPDATE persons SET is_primary = true, updated_at = now() WHERE id = $1"#,
        )
        .bind(pid)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn parse_birth_patch(v: &Value) -> Result<Option<NaiveDate>, ApiError> {
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

fn normalize_display_name(raw: &str) -> Result<String, ApiError> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 128 {
        return Err(ApiError::BadRequest(
            "display_name must be 1–128 characters".into(),
        ));
    }
    Ok(s.to_string())
}

#[utoipa::path(
    get,
    path = "/v1/persons",
    tag = "persons",
    responses(
        (status = 200, description = "Household persons", body = [PersonResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_persons(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<PersonResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let rows: Vec<PersonRow> = sqlx::query_as(
        r#"SELECT id, display_name, is_primary, birth_date, sort_index
           FROM persons WHERE installation_id = $1
           ORDER BY sort_index ASC, display_name ASC"#,
    )
    .bind(iid)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

#[utoipa::path(
    post,
    path = "/v1/persons",
    tag = "persons",
    request_body = CreatePersonBody,
    responses(
        (status = 201, description = "Created", body = PersonResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Cannot write"),
    )
)]
pub async fn create_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreatePersonBody>,
) -> Result<(axum::http::StatusCode, Json<PersonResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let name = normalize_display_name(&body.display_name)?;
    let sort_index = body.sort_index.unwrap_or(0);
    let is_primary = body.is_primary;

    let mut tx = state.pool.begin().await?;

    if is_primary {
        clear_other_primaries_tx(&mut tx, iid, None).await?;
    }

    let row: PersonRow = sqlx::query_as(
        r#"INSERT INTO persons (installation_id, display_name, is_primary, birth_date, sort_index)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, display_name, is_primary, birth_date, sort_index"#,
    )
    .bind(iid)
    .bind(&name)
    .bind(is_primary)
    .bind(body.birth_date)
    .bind(sort_index)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| person_insert_err(e))?;

    tx.commit().await?;
    Ok((axum::http::StatusCode::CREATED, Json(row_to_response(row))))
}

fn person_insert_err(e: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db) = e {
        if db.code().as_deref() == Some("23505") {
            return ApiError::Conflict;
        }
    }
    ApiError::Db(e)
}

#[utoipa::path(
    patch,
    path = "/v1/persons/{id}",
    tag = "persons",
    params(("id" = Uuid, Path, description = "Person id")),
    request_body = PatchPersonBody,
    responses(
        (status = 200, description = "Updated", body = PersonResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Cannot write"),
        (status = 404, description = "Unknown person"),
    )
)]
pub async fn patch_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(pid): Path<Uuid>,
    Json(body): Json<PatchPersonBody>,
) -> Result<Json<PersonResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let mut tx = state.pool.begin().await?;

    let current: Option<PersonRow> = sqlx::query_as(
        r#"SELECT id, display_name, is_primary, birth_date, sort_index
           FROM persons WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(pid)
    .bind(iid)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(mut row) = current else {
        return Err(ApiError::NotFound);
    };

    if let Some(ref name) = body.display_name {
        row.display_name = normalize_display_name(name)?;
    }

    if let Some(si) = body.sort_index {
        row.sort_index = si;
    }

    if let Some(v) = &body.birth_date {
        row.birth_date = parse_birth_patch(v)?;
    }

    if let Some(true) = body.is_primary {
        clear_other_primaries_tx(&mut tx, iid, Some(pid)).await?;
        row.is_primary = true;
    } else if let Some(false) = body.is_primary {
        row.is_primary = false;
    }

    sqlx::query(
        r#"UPDATE persons SET display_name = $1, is_primary = $2, birth_date = $3,
               sort_index = $4, updated_at = now()
           WHERE id = $5 AND installation_id = $6"#,
    )
    .bind(&row.display_name)
    .bind(row.is_primary)
    .bind(row.birth_date)
    .bind(row.sort_index)
    .bind(pid)
    .bind(iid)
    .execute(&mut *tx)
    .await?;

    if body.is_primary == Some(false) {
        let any_primary: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                SELECT 1 FROM persons WHERE installation_id = $1 AND is_primary = true)"#,
        )
        .bind(iid)
        .fetch_one(&mut *tx)
        .await?;
        if !any_primary {
            promote_first_remaining_primary_tx(&mut tx, iid).await?;
        }
    }

    let updated: PersonRow = sqlx::query_as(
        r#"SELECT id, display_name, is_primary, birth_date, sort_index
           FROM persons WHERE id = $1"#,
    )
    .bind(pid)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Json(row_to_response(updated)))
}

#[utoipa::path(
    delete,
    path = "/v1/persons/{id}",
    tag = "persons",
    params(("id" = Uuid, Path, description = "Person id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Cannot write"),
        (status = 404, description = "Unknown person"),
    )
)]
pub async fn delete_person(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(pid): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let mut tx = state.pool.begin().await?;

    let current: Option<(Uuid, bool)> = sqlx::query_as(
        r#"SELECT id, is_primary FROM persons WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(pid)
    .bind(iid)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((found_id, was_primary)) = current else {
        return Err(ApiError::NotFound);
    };

    sqlx::query(r#"DELETE FROM persons WHERE id = $1"#)
        .bind(found_id)
        .execute(&mut *tx)
        .await?;

    if was_primary {
        promote_first_remaining_primary_tx(&mut tx, iid).await?;
    }

    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn persons_router() -> Router {
    Router::new()
        .route("/", get(list_persons).post(create_person))
        .route("/{id}", patch(patch_person).delete(delete_person))
}
