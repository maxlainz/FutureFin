use crate::error::ApiError;
use crate::handlers::budget::assert_budget_category;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlanningFlowDirection {
    Inflow,
    Outflow,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanningFlowResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub direction: PlanningFlowDirection,
    pub title: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expected_amount: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub due_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlanningFlowBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub title: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expected_amount: Decimal,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPlanningFlowBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub title: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_amount: Option<Decimal>,
    /// Omit = leave unchanged; `null` = clear; `"YYYY-MM-DD"` = set.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub due_date: Option<Value>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct PlanningFlowJoinRow {
    id: Uuid,
    category_id: Uuid,
    scope: String,
    title: String,
    expected_amount: Decimal,
    due_date: Option<NaiveDate>,
    notes: Option<String>,
    sort_index: i32,
}

fn scope_to_direction(scope: &str) -> Result<PlanningFlowDirection, ApiError> {
    match scope {
        "income" => Ok(PlanningFlowDirection::Inflow),
        "expense" => Ok(PlanningFlowDirection::Outflow),
        _ => Err(ApiError::BadRequest(
            "planning flow category must be income or expense scope".into(),
        )),
    }
}

fn normalize_title(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "title must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn patch_due_date_from_json(
    v: &Value,
) -> Result<Option<NaiveDate>, ApiError> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v
        .as_str()
        .ok_or_else(|| ApiError::BadRequest("due_date must be a string or null".into()))?;
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d").map(Some).map_err(|_| {
        ApiError::BadRequest("due_date must be YYYY-MM-DD".into())
    })
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                Ok(None)
            } else if t.len() > 4000 {
                Err(ApiError::BadRequest(
                    "notes must be at most 4000 characters".into(),
                ))
            } else {
                Ok(Some(t.into()))
            }
        }
    }
}

fn row_to_response(r: PlanningFlowJoinRow) -> Result<PlanningFlowResponse, ApiError> {
    Ok(PlanningFlowResponse {
        id: r.id,
        category_id: r.category_id,
        direction: scope_to_direction(r.scope.as_str())?,
        title: r.title,
        expected_amount: r.expected_amount,
        due_date: r.due_date,
        notes: r.notes,
        sort_index: r.sort_index,
    })
}

#[utoipa::path(
    get,
    path = "/v1/planning/flows",
    tag = "planning",
    params(
        ("view" = Option<String>, Query, description = "`mine` = flows attributed to the signed-in user; omit = household."),
    ),
    responses(
        (status = 200, description = "Planning flows", body = Vec<PlanningFlowResponse>),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_planning_flows(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<PlanningFlowResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let rows: Vec<PlanningFlowJoinRow> = match q.resolve() {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                          p.expected_amount, p.due_date, p.notes, p.sort_index
                   FROM planning_flows p
                   JOIN categories c ON c.id = p.category_id
                   WHERE p.installation_id = $1
                   ORDER BY p.sort_index ASC, p.due_date ASC NULLS LAST, p.title ASC"#,
            )
            .bind(iid)
            .fetch_all(&state.pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                          p.expected_amount, p.due_date, p.notes, p.sort_index
                   FROM planning_flows p
                   JOIN categories c ON c.id = p.category_id
                   WHERE p.installation_id = $1 AND p.owner_user_id = $2
                   ORDER BY p.sort_index ASC, p.due_date ASC NULLS LAST, p.title ASC"#,
            )
            .bind(iid)
            .bind(user.id.0)
            .fetch_all(&state.pool)
            .await?
        }
    };

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_response(r)?);
    }

    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/v1/planning/flows",
    tag = "planning",
    request_body = CreatePlanningFlowBody,
    responses(
        (status = 201, description = "Created", body = PlanningFlowResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreatePlanningFlowBody>,
) -> Result<(axum::http::StatusCode, Json<PlanningFlowResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    assert_budget_category(&state.pool, iid, body.category_id).await?;

    if body.expected_amount <= Decimal::ZERO {
        return Err(ApiError::BadRequest(
            "expected_amount must be greater than zero".into(),
        ));
    }

    let title = normalize_title(&body.title)?;
    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO planning_flows (
               installation_id, category_id, title, expected_amount, due_date, notes,
               sort_index, owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&title)
    .bind(body.expected_amount)
    .bind(body.due_date)
    .bind(&notes)
    .bind(sort_index)
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    let row: PlanningFlowJoinRow = sqlx::query_as(
        r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                  p.expected_amount, p.due_date, p.notes, p.sort_index
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE p.id = $1"#,
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(row_to_response(row)?),
    ))
}

#[utoipa::path(
    patch,
    path = "/v1/planning/flows/{id}",
    tag = "planning",
    request_body = PatchPlanningFlowBody,
    params(
        ("id" = Uuid, Path, description = "Planning flow id"),
    ),
    responses(
        (status = 200, description = "Updated", body = PlanningFlowResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Flow missing"),
    )
)]
pub async fn patch_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchPlanningFlowBody>,
) -> Result<Json<PlanningFlowResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    if body.category_id.is_none()
        && body.title.is_none()
        && body.expected_amount.is_none()
        && body.due_date.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<PlanningFlowJoinRow> = sqlx::query_as(
        r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                  p.expected_amount, p.due_date, p.notes, p.sort_index
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE p.id = $1 AND p.installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };

    let new_cat = body.category_id.unwrap_or(current.category_id);
    if new_cat != current.category_id {
        assert_budget_category(&state.pool, iid, new_cat).await?;
    }

    let new_title = match &body.title {
        Some(t) => normalize_title(t)?,
        None => current.title.clone(),
    };

    let new_amount = match body.expected_amount {
        Some(a) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "expected_amount must be greater than zero".into(),
                ));
            }
            a
        }
        None => current.expected_amount,
    };

    let new_due = match &body.due_date {
        Some(v) => patch_due_date_from_json(v)?,
        None => current.due_date,
    };

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: PlanningFlowJoinRow = sqlx::query_as(
        r#"UPDATE planning_flows
           SET category_id = $1,
               title = $2,
               expected_amount = $3,
               due_date = $4,
               notes = $5,
               sort_index = $6,
               updated_at = now()
           WHERE id = $7 AND installation_id = $8
           RETURNING planning_flows.id,
                     planning_flows.category_id,
                     (
                         SELECT c.scope
                         FROM categories c
                         WHERE c.id = planning_flows.category_id
                     ) AS scope,
                     planning_flows.title,
                     planning_flows.expected_amount,
                     planning_flows.due_date,
                     planning_flows.notes,
                     planning_flows.sort_index"#,
    )
    .bind(new_cat)
    .bind(&new_title)
    .bind(new_amount)
    .bind(new_due)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row_to_response(updated)?))
}

#[utoipa::path(
    delete,
    path = "/v1/planning/flows/{id}",
    tag = "planning",
    params(
        ("id" = Uuid, Path, description = "Planning flow id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Flow missing"),
    )
)]
pub async fn delete_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let res =
        sqlx::query(r#"DELETE FROM planning_flows WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&state.pool)
            .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn planning_router() -> Router {
    Router::new()
        .route("/flows", get(list_planning_flows).post(create_planning_flow))
        .route(
            "/flows/{id}",
            patch(patch_planning_flow).delete(delete_planning_flow),
        )
}
