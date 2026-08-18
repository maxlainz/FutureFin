use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::{assets_projection_context, refresh_projection_after_mutation};
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
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
    /// Aporte estimado del primer mes derivado de las reglas de asignación del sobrante.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub contribution_nominal_monthly: Decimal,
    /// Tope absoluto en € si alguna regla de asignación apunta a este activo con
    /// `cap_kind='amount'` (el más restrictivo si hay varias). Solo se devuelve cuando es
    /// un tope concreto en euros — los topes relativos (`months_expense`, `income_multiple`)
    /// no aparecen aquí porque varían con el presupuesto.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub contribution_target_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
    /// Usuario dueño de la fila (`NULL` = compartida). Dato de display para la UI
    /// (el trigger del modal de snapshot). No es una frontera de seguridad.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
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
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
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
    /// Omitir sin cambio; `null` borra el precio de compra.
    #[serde(default)]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub purchase_price: Option<serde_json::Value>,
    pub is_liquid: Option<bool>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
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
    expected_annual_return_percent: Option<Decimal>,
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

/// PATCH: clave ausente → conservar `current`; `null` JSON → `None` en BD; valor → sustituir.
fn merge_optional_decimal_patch(
    patch: &Option<serde_json::Value>,
    current: Option<Decimal>,
    field: &'static str,
) -> Result<Option<Decimal>, ApiError> {
    match patch {
        None => Ok(current),
        Some(v) => {
            if v.is_null() {
                return Ok(None);
            }
            let d: Decimal = if let serde_json::Value::String(s) = v {
                s.trim().parse().map_err(|_| {
                    ApiError::BadRequest(format!("{field} must be a valid decimal string"))
                })?
            } else {
                serde_json::from_value(v.clone()).map_err(|_| {
                    ApiError::BadRequest(format!("{field} must be a valid decimal"))
                })?
            };
            assert_non_negative(d, field)?;
            Ok(Some(d))
        }
    }
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

fn row_to_response(
    r: AssetRow,
    contribution_nominal_monthly: Decimal,
    contribution_target_amount: Option<Decimal>,
) -> AssetResponse {
    AssetResponse {
        id: r.id,
        category_id: r.category_id,
        name: r.name,
        current_value: r.current_value,
        purchase_price: r.purchase_price,
        is_liquid: r.is_liquid,
        expected_annual_return_percent: r.expected_annual_return_percent,
        contribution_nominal_monthly,
        contribution_target_amount,
        notes: r.notes,
        sort_index: r.sort_index,
        owner_user_id: r.owner_user_id,
    }
}

/// Build `asset_id → target_€` from the **first** allocation rule (lowest `priority`) that
/// targets each asset and has a cap. Caps in `months_expense` / `income_multiple` are resolved
/// to absolute € using the scope's **effective** monthly income / expense + debt_service baseline
/// (`assets_projection_context`), es decir los mismos escalares con los que simula el engine.
async fn fetch_asset_resolved_targets(
    pool: &sqlx::PgPool,
    iid: Uuid,
    view: LedgerView,
    session_user_id: Uuid,
    income_monthly: Decimal,
    expense_with_debt: Decimal,
) -> Result<std::collections::HashMap<Uuid, Decimal>, ApiError> {
    let scope = view.scope_where("");
    let sql = format!(
        r#"SELECT DISTINCT ON (target_asset_id)
                  target_asset_id, cap_kind, cap_value
           FROM allocation_rules
           WHERE {scope}
             AND cap_kind IS NOT NULL AND cap_value IS NOT NULL
           ORDER BY target_asset_id, priority ASC, id ASC"#
    );
    let rows: Vec<(Uuid, String, Decimal)> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let mut out = std::collections::HashMap::with_capacity(rows.len());
    for (asset_id, cap_kind, cap_value) in rows {
        let resolved = match cap_kind.as_str() {
            "amount" => cap_value.max(Decimal::ZERO),
            "months_expense" => (cap_value.max(Decimal::ZERO)) * expense_with_debt,
            "income_multiple" => (cap_value.max(Decimal::ZERO)) * income_monthly,
            _ => continue,
        };
        if resolved > Decimal::ZERO {
            out.insert(asset_id, resolved);
        }
    }
    Ok(out)
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
    let out = list_assets_core(&state.pool, iid, user.id.0, q.resolve()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_assets`.
pub(crate) async fn list_assets_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<AssetResponse>, ApiError> {
    let today = installation_naive_today(pool, iid).await?;
    let ctx = assets_projection_context(pool, iid, user_id, view, today).await?;
    let targets = fetch_asset_resolved_targets(
        pool,
        iid,
        view,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let nominals = ctx.nominals;

    let assets_scope = view.scope_where("");
    let assets_sql = format!(
        r#"SELECT id, category_id, name, current_value, purchase_price,
                  is_liquid, expected_annual_return_percent,
                  notes, sort_index, owner_user_id
           FROM assets
           WHERE {assets_scope}
           ORDER BY sort_index ASC, name ASC"#
    );
    let rows: Vec<AssetRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, user_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let n = nominals.get(&r.id).copied().unwrap_or(Decimal::ZERO);
            let t = targets.get(&r.id).copied();
            row_to_response(r, n, t)
        })
        .collect())
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
    let resp = create_asset_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Rentabilidad esperada válida: > −100 %. El engine clampa ≤ −100 a pérdida total, pero la
/// capa API rechaza inputs nuevos absurdos (misma cota que los overrides de simulate_projection).
fn assert_return_percent(pct: Option<Decimal>) -> Result<(), ApiError> {
    if let Some(p) = pct {
        if p <= Decimal::from(-100) {
            return Err(ApiError::BadRequest(
                "expected_annual_return_percent must be greater than -100".into(),
            ));
        }
    }
    Ok(())
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_asset`.
/// Invalidación FULL dentro. Sin owner-check en el ledger de assets (contrato del módulo:
/// cualquier member edita cualquier fila del hogar).
pub(crate) async fn create_asset_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateAssetBody,
) -> Result<AssetResponse, ApiError> {
    assert_asset_category(&state.pool, iid, body.category_id).await?;
    assert_return_percent(body.expected_annual_return_percent)?;

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
               purchase_price, is_liquid,
               expected_annual_return_percent,
               notes, sort_index, owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, expected_annual_return_percent,
                     notes, sort_index, owner_user_id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&name)
    .bind(body.current_value)
    .bind(body.purchase_price)
    .bind(is_liquid)
    .bind(body.expected_annual_return_percent)
    .bind(&notes)
    .bind(sort_index)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    let ctx =
        assets_projection_context(&state.pool, iid, user_id, LedgerView::Household, today).await?;
    let n = ctx.nominals.get(&row.id).copied().unwrap_or(Decimal::ZERO);
    let targets = fetch_asset_resolved_targets(
        &state.pool,
        iid,
        LedgerView::Household,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let t = targets.get(&row.id).copied();

    refresh_projection_after_mutation(state.clone(), iid, user_id);
    Ok(row_to_response(row, n, t))
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
    let resp = patch_asset_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_asset_value` (que expone
/// solo un subset de campos). Invalidación FULL dentro. Sin owner-check (contrato del módulo).
pub(crate) async fn patch_asset_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchAssetBody,
) -> Result<AssetResponse, ApiError> {
    assert_return_percent(body.expected_annual_return_percent)?;
    if body.category_id.is_none()
        && body.name.is_none()
        && body.current_value.is_none()
        && body.purchase_price.is_none()
        && body.is_liquid.is_none()
        && body.expected_annual_return_percent.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<AssetRow> = sqlx::query_as(
        r#"SELECT id, category_id, name, current_value, purchase_price,
                  is_liquid, expected_annual_return_percent,
                  notes, sort_index, owner_user_id
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

    let new_pp = merge_optional_decimal_patch(&body.purchase_price, current.purchase_price, "purchase_price")?;

    let new_liquid = body.is_liquid.unwrap_or(current.is_liquid);

    let new_exp = if body.expected_annual_return_percent.is_some() {
        body.expected_annual_return_percent
    } else {
        current.expected_annual_return_percent
    };

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
               expected_annual_return_percent = $6,
               notes = $7,
               sort_index = $8,
               updated_at = now()
           WHERE id = $9 AND installation_id = $10
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, expected_annual_return_percent,
                     notes, sort_index, owner_user_id"#,
    )
    .bind(new_cat)
    .bind(&new_name)
    .bind(new_val)
    .bind(new_pp)
    .bind(new_liquid)
    .bind(new_exp)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    let ctx =
        assets_projection_context(&state.pool, iid, user_id, LedgerView::Household, today).await?;
    let n = ctx.nominals.get(&updated.id).copied().unwrap_or(Decimal::ZERO);
    let targets = fetch_asset_resolved_targets(
        &state.pool,
        iid,
        LedgerView::Household,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let t = targets.get(&updated.id).copied();

    refresh_projection_after_mutation(state.clone(), iid, user_id);
    Ok(row_to_response(updated, n, t))
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

    refresh_projection_after_mutation(state.clone(), iid, user.id.0);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn assets_router() -> Router {
    Router::new()
        .route("/", get(list_assets).post(create_asset))
        .route("/{id}", patch(patch_asset).delete(delete_asset))
}
