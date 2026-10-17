//! Cambios de un plan: alta, edición y borrado. La puerta de escritura, la subida de `revision`
//! y la invalidación son las de [`super::crud`]; la forma de cada tipo, la de
//! [`super::validate`].

use super::crud::{bump_revision, lock_plan_for_write, plan_block_not_found, require_writer};
use super::validate::{
    shape_err, validate_shape, ChangeShape, PlanAmountUnit, PlanCadence, PlanChangeKind,
    PlanChangeParams,
};
use crate::error::ApiError;
use crate::handlers::installation::installation_naive_today;
use crate::handlers::max_user_settable_future_date;
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgConnection, PgPool};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub(crate) fn plan_change_not_found() -> ApiError {
    ApiError::NotFoundWith("plan_change_not_found: no change with that id in this plan".into())
}

/// Un cambio del plan. Todos los campos viajan SIEMPRE, también los `null`: un `null` aquí
/// significa «no aplica» o «sin fin», nunca cero.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanChangeResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub plan_id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub block_id: Uuid,
    pub kind: PlanChangeKind,
    #[schema(value_type = String, format = "date")]
    pub start_date: NaiveDate,
    /// Inclusivo. `null` = sin fin (mensual) o no aplica (puntual).
    #[schema(value_type = Option<String>, format = "date")]
    pub end_date: Option<NaiveDate>,
    /// Euros de HOY (o %, con `amount_unit = pct`). Con signo en `income`/`expense` (sube/baja).
    /// `null` solo en `asset_sale` = vender el activo entero.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    pub amount_unit: PlanAmountUnit,
    pub cadence: PlanCadence,
    /// `true` = el importe se indexa con la inflación del plan (mantiene poder adquisitivo);
    /// `false` = importe fijo nominal. Un `loan` es siempre `false`.
    pub keeps_purchasing_power: bool,
    /// `asset_sale` de un activo real. Si ese activo se borra, queda `null` y la venta se queda
    /// sin sujeto (con `target_change_id` también `null`).
    #[schema(value_type = Option<String>, format = "uuid")]
    pub asset_id: Option<Uuid>,
    /// `asset_sale` de un activo que entra en este mismo plan (un `asset_in`).
    #[schema(value_type = Option<String>, format = "uuid")]
    pub target_change_id: Option<Uuid>,
    #[schema(value_type = PlanChangeParams)]
    pub params: Value,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlanChangeBody {
    /// Omitido = el primer bloque del plan (por `sort_index`).
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub block_id: Option<Uuid>,
    /// `income` | `expense` | `asset_in` | `asset_sale` | `loan`.
    pub kind: String,
    #[schema(value_type = String, format = "date")]
    pub start_date: NaiveDate,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub end_date: Option<NaiveDate>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    /// `eur` (default) | `pct` (solo mensual).
    #[serde(default)]
    pub amount_unit: Option<String>,
    /// `monthly` | `one_off`. Default: `monthly` en ingreso/gasto, `one_off` en el resto.
    #[serde(default)]
    pub cadence: Option<String>,
    /// Default `true`, salvo `loan` (siempre `false`).
    #[serde(default)]
    pub keeps_purchasing_power: Option<bool>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub asset_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub target_change_id: Option<Uuid>,
    /// Por tipo: `asset_in` y `loan` lo exigen; el resto, `{}` (default).
    #[serde(default)]
    #[schema(value_type = Option<PlanChangeParams>)]
    pub params: Option<Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// PATCH: se valida el estado RESULTANTE entero. Tri-estado en `end_date`, `amount`,
/// `asset_id` y `target_change_id`: omitido = igual, `null` = borrar, valor = fijar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPlanChangeBody {
    /// Mover el cambio a otro bloque del MISMO plan.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub block_id: Option<Uuid>,
    /// Inmutable: mandarlo distinto del actual es `plan_change_invalid_shape`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub start_date: Option<NaiveDate>,
    #[serde(
        default,
        deserialize_with = "crate::handlers::deserialize_double_option_typed"
    )]
    #[schema(value_type = Option<String>, format = "date")]
    pub end_date: Option<Option<NaiveDate>>,
    /// String decimal o `null`.
    #[serde(
        default,
        deserialize_with = "crate::handlers::deserialize_double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Value>,
    #[serde(default)]
    pub amount_unit: Option<String>,
    #[serde(default)]
    pub cadence: Option<String>,
    #[serde(default)]
    pub keeps_purchasing_power: Option<bool>,
    #[serde(
        default,
        deserialize_with = "crate::handlers::deserialize_double_option_typed"
    )]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub asset_id: Option<Option<Uuid>>,
    #[serde(
        default,
        deserialize_with = "crate::handlers::deserialize_double_option_typed"
    )]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub target_change_id: Option<Option<Uuid>>,
    /// REEMPLAZA los parámetros enteros (validados contra el tipo del cambio).
    #[serde(default)]
    #[schema(value_type = Option<PlanChangeParams>)]
    pub params: Option<Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ChangeRow {
    id: Uuid,
    plan_id: Uuid,
    pub(crate) block_id: Uuid,
    kind: String,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    amount: Option<Decimal>,
    amount_unit: String,
    cadence: String,
    keeps_purchasing_power: bool,
    asset_id: Option<Uuid>,
    target_change_id: Option<Uuid>,
    params: Value,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) const CHANGE_COLS: &str = "id, plan_id, block_id, kind, start_date, end_date, amount, \
     amount_unit, cadence, keeps_purchasing_power, asset_id, target_change_id, params, enabled, \
     created_at, updated_at";

/// TEXT → enum por el MISMO parser que valida la entrada (los `CHECK` garantizan que no falla).
pub(crate) fn change_from_row(r: ChangeRow) -> Result<PlanChangeResponse, ApiError> {
    Ok(PlanChangeResponse {
        id: r.id,
        plan_id: r.plan_id,
        block_id: r.block_id,
        kind: PlanChangeKind::parse(&r.kind)?,
        start_date: r.start_date,
        end_date: r.end_date,
        amount: r.amount,
        amount_unit: PlanAmountUnit::parse(&r.amount_unit)?,
        cadence: PlanCadence::parse(&r.cadence)?,
        keeps_purchasing_power: r.keeps_purchasing_power,
        asset_id: r.asset_id,
        target_change_id: r.target_change_id,
        params: r.params,
        enabled: r.enabled,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

async fn fetch_change(
    pool: &PgPool,
    plan_id: Uuid,
    change_id: Uuid,
) -> Result<PlanChangeResponse, ApiError> {
    let row: Option<ChangeRow> = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLS} FROM plan_changes WHERE id = $1 AND plan_id = $2"
    ))
    .bind(change_id)
    .bind(plan_id)
    .fetch_optional(pool)
    .await?;
    change_from_row(row.ok_or_else(plan_change_not_found)?)
}

fn default_cadence(kind: PlanChangeKind) -> PlanCadence {
    match kind {
        PlanChangeKind::Income | PlanChangeKind::Expense => PlanCadence::Monthly,
        _ => PlanCadence::OneOff,
    }
}

/// Cota de 100 años sobre las fechas que la escritura INTRODUCE — la misma de `due_date` y de
/// las ventanas de los flujos (`max_user_settable_future_date`).
async fn validate_date_range(
    pool: &PgPool,
    iid: Uuid,
    dates: [Option<NaiveDate>; 2],
) -> Result<(), ApiError> {
    if dates.iter().all(Option::is_none) {
        return Ok(());
    }
    let max = max_user_settable_future_date(installation_naive_today(pool, iid).await?);
    if dates.into_iter().flatten().any(|d| d > max) {
        return Err(shape_err(
            "dates must not be more than 100 years in the future",
        ));
    }
    Ok(())
}

/// El activo real que se vende tiene que ser DEL DUEÑO del plan: vender el de otro miembro sería
/// mover su patrimonio desde un plan ajeno (D21).
async fn assert_own_asset(
    conn: &mut PgConnection,
    iid: Uuid,
    user_id: Uuid,
    asset_id: Uuid,
) -> Result<(), ApiError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM assets WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(asset_id)
    .bind(iid)
    .fetch_optional(&mut *conn)
    .await?;
    if owner != Some(user_id) {
        return Err(ApiError::BadRequest(
            "asset_not_in_scope: asset_id is not one of your assets".into(),
        ));
    }
    Ok(())
}

/// `target_change_id` tiene que ser un `asset_in` del mismo plan, y no el propio cambio.
async fn assert_target_is_asset_in(
    conn: &mut PgConnection,
    plan_id: Uuid,
    target: Uuid,
    self_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if Some(target) == self_id {
        return Err(shape_err("a change cannot sell itself"));
    }
    let kind: Option<String> =
        sqlx::query_scalar(r#"SELECT kind FROM plan_changes WHERE id = $1 AND plan_id = $2"#)
            .bind(target)
            .bind(plan_id)
            .fetch_optional(&mut *conn)
            .await?;
    if kind.as_deref() != Some("asset_in") {
        return Err(shape_err(
            "target_change_id must be an asset_in change of this same plan",
        ));
    }
    Ok(())
}

async fn assert_block_in_plan(
    conn: &mut PgConnection,
    plan_id: Uuid,
    block_id: Uuid,
) -> Result<(), ApiError> {
    let found: Option<Uuid> =
        sqlx::query_scalar(r#"SELECT id FROM plan_blocks WHERE id = $1 AND plan_id = $2"#)
            .bind(block_id)
            .bind(plan_id)
            .fetch_optional(&mut *conn)
            .await?;
    found.map(|_| ()).ok_or_else(plan_block_not_found)
}

#[utoipa::path(
    post,
    path = "/v1/plans/{id}/changes",
    tag = "plans",
    request_body = CreatePlanChangeBody,
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 201, description = "Cambio creado", body = PlanChangeResponse),
        (status = 400, description = "plan_change_invalid_shape | asset_not_in_scope | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found | plan_block_not_found"),
    )
)]
pub async fn create_plan_change(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePlanChangeBody>,
) -> Result<(StatusCode, Json<PlanChangeResponse>), ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    let out = create_plan_change_core(&state, iid, user_id, id, body).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

pub(crate) async fn create_plan_change_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    body: CreatePlanChangeBody,
) -> Result<PlanChangeResponse, ApiError> {
    let kind = PlanChangeKind::parse(body.kind.trim())?;
    let amount_unit = match body.amount_unit.as_deref() {
        Some(s) => PlanAmountUnit::parse(s)?,
        None => PlanAmountUnit::Eur,
    };
    let cadence = match body.cadence.as_deref() {
        Some(s) => PlanCadence::parse(s)?,
        None => default_cadence(kind),
    };
    let shape = ChangeShape {
        kind,
        start_date: body.start_date,
        end_date: body.end_date,
        amount: body.amount,
        amount_unit,
        cadence,
        keeps_purchasing_power: body
            .keeps_purchasing_power
            .unwrap_or(kind != PlanChangeKind::Loan),
        asset_id: body.asset_id,
        target_change_id: body.target_change_id,
    };
    validate_shape(&shape)?;
    let params = match body
        .params
        .clone()
        .or_else(|| PlanChangeParams::default_json_for(kind))
    {
        Some(v) => PlanChangeParams::parse(kind, &v)?,
        None => return Err(shape_err("params are required for this kind")),
    };
    validate_date_range(&state.pool, iid, [shape.start_date.into(), shape.end_date]).await?;

    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let block_id = match body.block_id {
        Some(b) => {
            assert_block_in_plan(&mut tx, plan_id, b).await?;
            b
        }
        None => sqlx::query_scalar(
            r#"SELECT id FROM plan_blocks WHERE plan_id = $1
               ORDER BY sort_index ASC, created_at ASC, id ASC LIMIT 1"#,
        )
        .bind(plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(plan_block_not_found)?,
    };
    if let Some(a) = shape.asset_id {
        assert_own_asset(&mut tx, iid, user_id, a).await?;
    }
    if let Some(t) = shape.target_change_id {
        assert_target_is_asset_in(&mut tx, plan_id, t, None).await?;
    }
    let change_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO plan_changes (
               plan_id, block_id, kind, start_date, end_date, amount, amount_unit, cadence,
               keeps_purchasing_power, asset_id, target_change_id, params, enabled
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING id"#,
    )
    .bind(plan_id)
    .bind(block_id)
    .bind(kind.as_str())
    .bind(shape.start_date)
    .bind(shape.end_date)
    .bind(shape.amount)
    .bind(amount_unit.as_str())
    .bind(cadence.as_str())
    .bind(shape.keeps_purchasing_power)
    .bind(shape.asset_id)
    .bind(shape.target_change_id)
    .bind(params.to_stored())
    .bind(body.enabled.unwrap_or(true))
    .fetch_one(&mut *tx)
    .await?;
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_change(&state.pool, plan_id, change_id).await
}

#[utoipa::path(
    patch,
    path = "/v1/plans/{id}/changes/{change_id}",
    tag = "plans",
    request_body = PatchPlanChangeBody,
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("change_id" = Uuid, Path, description = "Change id"),
    ),
    responses(
        (status = 200, description = "Cambio actualizado", body = PlanChangeResponse),
        (status = 400, description = "patch_empty | plan_change_invalid_shape | asset_not_in_scope | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found | plan_block_not_found | plan_change_not_found"),
    )
)]
pub async fn patch_plan_change(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path((id, change_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchPlanChangeBody>,
) -> Result<Json<PlanChangeResponse>, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    Ok(Json(
        patch_plan_change_core(&state, iid, user_id, id, change_id, body).await?,
    ))
}

fn patch_amount_from_json(v: &Value) -> Result<Option<Decimal>, ApiError> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => s
            .trim()
            .parse::<Decimal>()
            .map(Some)
            .map_err(|_| shape_err("amount must be a decimal string or null")),
        _ => Err(shape_err("amount must be a decimal string or null")),
    }
}

pub(crate) async fn patch_plan_change_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    change_id: Uuid,
    body: PatchPlanChangeBody,
) -> Result<PlanChangeResponse, ApiError> {
    if body.block_id.is_none()
        && body.kind.is_none()
        && body.start_date.is_none()
        && body.end_date.is_none()
        && body.amount.is_none()
        && body.amount_unit.is_none()
        && body.cadence.is_none()
        && body.keeps_purchasing_power.is_none()
        && body.asset_id.is_none()
        && body.target_change_id.is_none()
        && body.params.is_none()
        && body.enabled.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one field of the change to update".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let current: Option<ChangeRow> = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLS} FROM plan_changes WHERE id = $1 AND plan_id = $2"
    ))
    .bind(change_id)
    .bind(plan_id)
    .fetch_optional(&mut *tx)
    .await?;
    let current = change_from_row(current.ok_or_else(plan_change_not_found)?)?;

    let kind = current.kind;
    if let Some(k) = body.kind.as_deref() {
        if PlanChangeKind::parse(k.trim())? != kind {
            return Err(shape_err(
                "kind is immutable; delete the change and create a new one",
            ));
        }
    }
    let shape = ChangeShape {
        kind,
        start_date: body.start_date.unwrap_or(current.start_date),
        end_date: body.end_date.unwrap_or(current.end_date),
        amount: match &body.amount {
            Some(v) => patch_amount_from_json(v)?,
            None => current.amount,
        },
        amount_unit: match body.amount_unit.as_deref() {
            Some(s) => PlanAmountUnit::parse(s)?,
            None => current.amount_unit,
        },
        cadence: match body.cadence.as_deref() {
            Some(s) => PlanCadence::parse(s)?,
            None => current.cadence,
        },
        keeps_purchasing_power: body
            .keeps_purchasing_power
            .unwrap_or(current.keeps_purchasing_power),
        asset_id: body.asset_id.unwrap_or(current.asset_id),
        target_change_id: body.target_change_id.unwrap_or(current.target_change_id),
    };
    validate_shape(&shape)?;
    let params = match &body.params {
        Some(v) => PlanChangeParams::parse(kind, v)?.to_stored(),
        None => current.params,
    };
    // Solo se acota lo que el patch INTRODUCE (precedente: `planning.rs`).
    validate_date_range(&state.pool, iid, [body.start_date, body.end_date.flatten()]).await?;
    let block_id = match body.block_id {
        Some(b) => {
            assert_block_in_plan(&mut tx, plan_id, b).await?;
            b
        }
        None => current.block_id,
    };
    if let Some(a) = shape.asset_id.filter(|a| Some(*a) != current.asset_id) {
        assert_own_asset(&mut tx, iid, user_id, a).await?;
    }
    if let Some(t) = shape
        .target_change_id
        .filter(|t| Some(*t) != current.target_change_id)
    {
        assert_target_is_asset_in(&mut tx, plan_id, t, Some(change_id)).await?;
    }

    sqlx::query(
        r#"UPDATE plan_changes
           SET block_id = $3, start_date = $4, end_date = $5, amount = $6, amount_unit = $7,
               cadence = $8, keeps_purchasing_power = $9, asset_id = $10,
               target_change_id = $11, params = $12, enabled = $13, updated_at = now()
           WHERE id = $1 AND plan_id = $2"#,
    )
    .bind(change_id)
    .bind(plan_id)
    .bind(block_id)
    .bind(shape.start_date)
    .bind(shape.end_date)
    .bind(shape.amount)
    .bind(shape.amount_unit.as_str())
    .bind(shape.cadence.as_str())
    .bind(shape.keeps_purchasing_power)
    .bind(shape.asset_id)
    .bind(shape.target_change_id)
    .bind(params)
    .bind(body.enabled.unwrap_or(current.enabled))
    .execute(&mut *tx)
    .await?;
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_change(&state.pool, plan_id, change_id).await
}

#[utoipa::path(
    delete,
    path = "/v1/plans/{id}/changes/{change_id}",
    tag = "plans",
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("change_id" = Uuid, Path, description = "Change id"),
    ),
    responses(
        (status = 204, description = "Borrado. Si era un `asset_in`, las ventas que lo vendían quedan sin sujeto (`target_change_id` null)."),
        (status = 400, description = "household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found | plan_change_not_found"),
    )
)]
pub async fn delete_plan_change(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path((id, change_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    delete_plan_change_core(&state, iid, user_id, id, change_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_plan_change_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    change_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let res = sqlx::query(r#"DELETE FROM plan_changes WHERE id = $1 AND plan_id = $2"#)
        .bind(change_id)
        .bind(plan_id)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(plan_change_not_found());
    }
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;
    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(())
}
