use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CategoryScope {
    Asset,
    Liability,
    Income,
    Expense,
}

impl CategoryScope {
    fn as_str(self) -> &'static str {
        match self {
            CategoryScope::Asset => "asset",
            CategoryScope::Liability => "liability",
            CategoryScope::Income => "income",
            CategoryScope::Expense => "expense",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s.trim() {
            "asset" => Ok(Self::Asset),
            "liability" => Ok(Self::Liability),
            "income" => Ok(Self::Income),
            "expense" => Ok(Self::Expense),
            _ => Err(ApiError::BadRequest("category_scope_invalid: category scope must be one of asset, liability, income, expense".into())),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub scope: CategoryScope,
    pub name: String,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCategoryBody {
    pub scope: CategoryScope,
    pub name: String,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchCategoryBody {
    pub name: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ListCategoriesQuery {
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteCategoryQuery {
    /// Required when the category is still referenced by assets, liabilities, budget or planning flows (same scope as source).
    #[serde(default)]
    pub remap_to: Option<Uuid>,
}

async fn category_reference_count(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<i64, ApiError> {
    // Nota: las `categorization_rules` NO cuentan aquí (su `assign_category_id` es ON DELETE SET
    // NULL → una regla degradada nunca bloquea el borrado de una categoría). Las `transactions` y
    // las `recurring_transaction_rules` sí (su `category_id` es ON DELETE RESTRICT → deben
    // remapearse antes de borrar).
    let n: Option<i64> = sqlx::query_scalar(
        r#"SELECT (
               COALESCE((SELECT COUNT(*)::bigint FROM assets
                         WHERE installation_id = $1 AND category_id = $2), 0)
             + COALESCE((SELECT COUNT(*)::bigint FROM liabilities
                         WHERE installation_id = $1 AND category_id = $2), 0)
             + COALESCE((SELECT COUNT(*)::bigint FROM budget_entries
                         WHERE installation_id = $1 AND category_id = $2), 0)
             + COALESCE((SELECT COUNT(*)::bigint FROM planning_flows
                         WHERE installation_id = $1 AND category_id = $2), 0)
             + COALESCE((SELECT COUNT(*)::bigint FROM transactions
                         WHERE installation_id = $1 AND category_id = $2), 0)
             + COALESCE((SELECT COUNT(*)::bigint FROM recurring_transaction_rules
                         WHERE installation_id = $1 AND category_id = $2), 0)
            )"#,
    )
    .bind(installation_id)
    .bind(category_id)
    .fetch_one(pool)
    .await?;
    Ok(n.unwrap_or(0))
}

async fn category_scope_row(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<Option<String>, ApiError> {
    let s: Option<String> = sqlx::query_scalar(
        r#"SELECT scope FROM categories WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    Ok(s)
}

#[derive(Debug, FromRow)]
struct CategoryRow {
    id: Uuid,
    scope: String,
    name: String,
    sort_index: i32,
}

fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "name_empty: name must not be empty".into(),
        ));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "name_too_long: name must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn row_to_response(r: CategoryRow) -> Result<CategoryResponse, ApiError> {
    Ok(CategoryResponse {
        id: r.id,
        scope: CategoryScope::parse(&r.scope)?,
        name: r.name,
        sort_index: r.sort_index,
    })
}

#[utoipa::path(
    get,
    path = "/v1/categories",
    tag = "categories",
    params(
        ("scope" = Option<String>, Query, description = "Filter: asset | liability | income | expense"),
    ),
    responses(
        (status = 200, description = "Categories for the installation", body = [CategoryResponse]),
        (status = 400, description = "Invalid scope filter"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation not initialized"),
    )
)]
pub async fn list_categories(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ListCategoriesQuery>,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_categories_core(&state.pool, iid, q.scope.as_deref()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_categories`. Per-installation
/// (las categorías no tienen owner — no acepta `view`).
pub(crate) async fn list_categories_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    scope: Option<&str>,
) -> Result<Vec<CategoryResponse>, ApiError> {
    let scope_filter: Option<String> = match scope.map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(CategoryScope::parse(s)?.as_str().to_string()),
    };

    let rows: Vec<CategoryRow> = if let Some(ref sc) = scope_filter {
        sqlx::query_as(
            r#"SELECT id, scope, name, sort_index
               FROM categories
               WHERE installation_id = $1 AND scope = $2
               ORDER BY sort_index ASC, name ASC"#,
        )
        .bind(iid)
        .bind(sc)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            r#"SELECT id, scope, name, sort_index
               FROM categories
               WHERE installation_id = $1
               ORDER BY scope ASC, sort_index ASC, name ASC"#,
        )
        .bind(iid)
        .fetch_all(pool)
        .await?
    };

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_response(r)?);
    }
    Ok(out)
}

#[utoipa::path(
    post,
    path = "/v1/categories",
    tag = "categories",
    request_body = CreateCategoryBody,
    responses(
        (status = 201, description = "Created", body = CategoryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation not initialized"),
        (status = 409, description = "Duplicate name in scope"),
    )
)]
pub async fn create_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateCategoryBody>,
) -> Result<(axum::http::StatusCode, Json<CategoryResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_category_core(&state.pool, iid, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_category`. Las categorías
/// no invalidan la cache de proyección (contrato histórico: ningún handler del módulo lo hace).
/// 409 en duplicado (unique instalación+scope+nombre) vía el mapeo global de sqlx.
pub(crate) async fn create_category_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    body: CreateCategoryBody,
) -> Result<CategoryResponse, ApiError> {
    let name = normalize_name(&body.name)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let row: CategoryRow = sqlx::query_as(
        r#"INSERT INTO categories (installation_id, scope, name, sort_index)
           VALUES ($1, $2, $3, $4)
           RETURNING id, scope, name, sort_index"#,
    )
    .bind(iid)
    .bind(body.scope.as_str())
    .bind(&name)
    .bind(sort_index)
    .fetch_one(pool)
    .await?;

    row_to_response(row)
}

#[utoipa::path(
    patch,
    path = "/v1/categories/{id}",
    tag = "categories",
    request_body = PatchCategoryBody,
    params(
        ("id" = Uuid, Path, description = "Category id"),
    ),
    responses(
        (status = 200, description = "Updated", body = CategoryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Category missing"),
        (status = 409, description = "Duplicate name in scope"),
    )
)]
pub async fn patch_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchCategoryBody>,
) -> Result<Json<CategoryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    if body.name.is_none() && body.sort_index.is_none() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide name and/or sort_index".into(),
        ));
    }

    let row: Option<CategoryRow> = sqlx::query_as(
        r#"SELECT id, scope, name, sort_index
           FROM categories
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };

    let new_name = match &body.name {
        Some(s) => normalize_name(s)?,
        None => current.name.clone(),
    };
    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: CategoryRow = sqlx::query_as(
        r#"UPDATE categories
           SET name = $1, sort_index = $2
           WHERE id = $3 AND installation_id = $4
           RETURNING id, scope, name, sort_index"#,
    )
    .bind(&new_name)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row_to_response(updated)?))
}

#[utoipa::path(
    delete,
    path = "/v1/categories/{id}",
    tag = "categories",
    params(
        ("id" = Uuid, Path, description = "Category id"),
        ("remap_to" = Option<Uuid>, Query, description = "Target category id (same scope) when rows still reference the deleted category"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Invalid remap or category still in use without remap_to"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Category missing"),
    )
)]
pub async fn delete_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Query(q): Query<DeleteCategoryQuery>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let Some(scope_src) = category_scope_row(&state.pool, iid, id).await? else {
        return Err(ApiError::NotFound);
    };

    let refs = category_reference_count(&state.pool, iid, id).await?;

    if refs > 0 {
        let Some(target) = q.remap_to else {
            return Err(ApiError::BadRequest(
                "category_in_use: category is in use; pass remap_to query parameter with another category id of the same scope"
                    .into(),
            ));
        };
        if target == id {
            return Err(ApiError::BadRequest(
                "remap_to_same_category: remap_to must differ from the category being deleted".into(),
            ));
        }
        let Some(scope_tgt) = category_scope_row(&state.pool, iid, target).await? else {
            return Err(ApiError::BadRequest(
                "remap_to_not_found: remap_to category was not found in this installation".into(),
            ));
        };
        if scope_src != scope_tgt {
            return Err(ApiError::BadRequest(
                "remap_to_scope_mismatch: remap_to category must have the same scope as the deleted category".into(),
            ));
        }

        let mut tx = state.pool.begin().await?;

        sqlx::query(
            r#"UPDATE assets SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE liabilities SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE budget_entries SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE planning_flows SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // Las transacciones y las reglas recurrentes referencian la categoría con ON DELETE
        // RESTRICT → hay que remaparlas (las `categorization_rules` no: su FK es SET NULL y se
        // degradan solas).
        sqlx::query(
            r#"UPDATE transactions SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE recurring_transaction_rules SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // `liabilities.expense_category_id` (3.4.0) es SET NULL — no cuenta en
        // `category_reference_count` ni bloquea el borrado — pero cuando el usuario remapea una
        // categoría de gasto, la atribución de las cuotas debe seguirla en vez de degradarse a
        // NULL por el FK al borrar.
        if scope_src == "expense" {
            sqlx::query(
                r#"UPDATE liabilities SET expense_category_id = $1, updated_at = now()
                   WHERE installation_id = $2 AND expense_category_id = $3"#,
            )
            .bind(target)
            .bind(iid)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }

        let del = sqlx::query(r#"DELETE FROM categories WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        if del.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
        return Ok(axum::http::StatusCode::NO_CONTENT);
    }

    let res = sqlx::query(r#"DELETE FROM categories WHERE id = $1 AND installation_id = $2"#)
        .bind(id)
        .bind(iid)
        .execute(&state.pool)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn categories_router() -> Router {
    Router::new()
        .route("/", get(list_categories).post(create_category))
        .route("/{id}", patch(patch_category).delete(delete_category))
}
