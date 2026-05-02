use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct AssetResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub current_value: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAssetBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub current_value: Decimal,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
    #[serde(default)]
    pub is_liquid: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchAssetBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub name: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub current_value: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
    pub is_liquid: Option<bool>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct AssetRow {
    id: Uuid,
    category_id: Uuid,
    name: String,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    notes: Option<String>,
    sort_index: i32,
    owner_user_id: Option<Uuid>,
}

fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "name must not be empty".into(),
        ));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "name must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 4000 {
                return Err(ApiError::BadRequest(
                    "notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("{field} must be >= 0")));
    }
    Ok(())
}

async fn assert_asset_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM categories
            WHERE
                id = $1
                AND installation_id = $2
                AND scope = 'asset'
        )"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;

    if !ok {
        return Err(ApiError::BadRequest(
            "category_id must reference an asset category in this installation".into(),
        ));
    }
    Ok(())
}

fn row_to_response(r: AssetRow) -> AssetResponse {
    AssetResponse {
        id: r.id,
        category_id: r.category_id,
        name: r.name,
        current_value: r.current_value,
        purchase_price: r.purchase_price,
        is_liquid: r.is_liquid,
        notes: r.notes,
        sort_index: r.sort_index,
    }
}

#[utoipa::path(
    get,
    path = "/v1/assets",
    tag = "assets",
    params(
        ("view" = Option<String>, Query, description = "`mine` = rows attributed to the signed-in user; omit or other value = full household."),
    ),
    responses(
        (status = 200, description = "Assets for the installation", body = [AssetResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_assets(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<AssetResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let rows: Vec<AssetRow> = match q.resolve() {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT id, category_id, name, current_value, purchase_price,
                          is_liquid, notes, sort_index, owner_user_id
                   FROM assets
                   WHERE installation_id = $1
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .fetch_all(&state.pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT id, category_id, name, current_value, purchase_price,
                          is_liquid, notes, sort_index, owner_user_id
                   FROM assets
                   WHERE installation_id = $1 AND owner_user_id = $2
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .bind(user.id.0)
            .fetch_all(&state.pool)
            .await?
        }
    };

    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

#[utoipa::path(
    post,
    path = "/v1/assets",
    tag = "assets",
    request_body = CreateAssetBody,
    responses(
        (status = 201, description = "Created", body = AssetResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_asset(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateAssetBody>,
) -> Result<(axum::http::StatusCode, Json<AssetResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    assert_asset_category(&state.pool, iid, body.category_id).await?;

    let name = normalize_name(&body.name)?;
    assert_non_negative(body.current_value, "current_value")?;
    if let Some(pp) = body.purchase_price {
        assert_non_negative(pp, "purchase_price")?;
    }
    let notes = normalize_notes(&body.notes)?;
    let is_liquid = body.is_liquid.unwrap_or(true);
    let sort_index = body.sort_index.unwrap_or(0);

    let row: AssetRow = sqlx::query_as(
        r#"INSERT INTO assets (
               installation_id, category_id, name, current_value,
               purchase_price, is_liquid, notes, sort_index, owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, notes, sort_index, owner_user_id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&name)
    .bind(body.current_value)
    .bind(body.purchase_price)
    .bind(is_liquid)
    .bind(&notes)
    .bind(sort_index)
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    Ok((axum::http::StatusCode::CREATED, Json(row_to_response(row))))
}

#[utoipa::path(
    patch,
    path = "/v1/assets/{id}",
    tag = "assets",
    request_body = PatchAssetBody,
    params(
        ("id" = Uuid, Path, description = "Asset id"),
    ),
    responses(
        (status = 200, description = "Updated", body = AssetResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Asset missing"),
    )
)]
pub async fn patch_asset(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssetBody>,
) -> Result<Json<AssetResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    if body.category_id.is_none()
        && body.name.is_none()
        && body.current_value.is_none()
        && body.purchase_price.is_none()
        && body.is_liquid.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<AssetRow> = sqlx::query_as(
        r#"SELECT id, category_id, name, current_value, purchase_price,
                  is_liquid, notes, sort_index, owner_user_id
           FROM assets
           WHERE id = $1 AND installation_id = $2"#,
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
        assert_asset_category(&state.pool, iid, new_cat).await?;
    }

    let new_name = match &body.name {
        Some(s) => normalize_name(s)?,
        None => current.name.clone(),
    };

    let new_val = match body.current_value {
        Some(v) => {
            assert_non_negative(v, "current_value")?;
            v
        }
        None => current.current_value,
    };

    let new_pp = match body.purchase_price {
        Some(v) => {
            assert_non_negative(v, "purchase_price")?;
            Some(v)
        }
        None => current.purchase_price,
    };

    let new_liquid = body.is_liquid.unwrap_or(current.is_liquid);

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: AssetRow = sqlx::query_as(
        r#"UPDATE assets
           SET category_id = $1,
               name = $2,
               current_value = $3,
               purchase_price = $4,
               is_liquid = $5,
               notes = $6,
               sort_index = $7,
               updated_at = now()
           WHERE id = $8 AND installation_id = $9
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, notes, sort_index, owner_user_id"#,
    )
    .bind(new_cat)
    .bind(&new_name)
    .bind(new_val)
    .bind(new_pp)
    .bind(new_liquid)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row_to_response(updated)))
}

#[utoipa::path(
    delete,
    path = "/v1/assets/{id}",
    tag = "assets",
    params(
        ("id" = Uuid, Path, description = "Asset id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Asset missing"),
    )
)]
pub async fn delete_asset(
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
        sqlx::query(r#"DELETE FROM assets WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&state.pool)
            .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn assets_router() -> Router {
    Router::new()
        .route("/", get(list_assets).post(create_asset))
        .route("/{id}", patch(patch_asset).delete(delete_asset))
}
