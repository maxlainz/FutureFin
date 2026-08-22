//! Allocation rules CRUD + reordering.
//!
//! Each rule consumes part of the **monthly surplus** for one target asset, in priority order.
//! See [`crate::handlers::projection`] for how the engine reads these.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationRuleResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub priority: i32,
    /// `fixed` | `percent` | `remainder`
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    /// `amount` | `months_expense` | `income_multiple` | null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_value: Option<Decimal>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAllocationRuleBody {
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub kind: String,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub cap_kind: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_value: Option<Decimal>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchAllocationRuleBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub target_asset_id: Option<Uuid>,
    #[serde(default)]
    pub kind: Option<String>,
    /// `null` JSON clears the amount (only valid for `remainder`).
    #[serde(default)]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub amount: Option<serde_json::Value>,
    /// `null` JSON clears the cap pair.
    #[serde(default)]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub cap: Option<serde_json::Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderBody {
    #[schema(value_type = Vec<String>)]
    pub ids: Vec<Uuid>,
}

#[derive(Debug, FromRow)]
struct RuleRow {
    id: Uuid,
    owner_user_id: Option<Uuid>,
    target_asset_id: Uuid,
    priority: i32,
    kind: String,
    amount: Option<Decimal>,
    cap_kind: Option<String>,
    cap_value: Option<Decimal>,
    enabled: bool,
    notes: Option<String>,
}

fn row_to_response(r: RuleRow) -> AllocationRuleResponse {
    AllocationRuleResponse {
        id: r.id,
        target_asset_id: r.target_asset_id,
        priority: r.priority,
        kind: r.kind,
        amount: r.amount,
        cap_kind: r.cap_kind,
        cap_value: r.cap_value,
        enabled: r.enabled,
        notes: r.notes,
        owner_user_id: r.owner_user_id,
    }
}

fn normalize_kind(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "fixed" => Ok("fixed".into()),
        "percent" => Ok("percent".into()),
        "remainder" => Ok("remainder".into()),
        other => Err(ApiError::BadRequest(format!(
            "rule_kind_invalid: kind must be 'fixed' | 'percent' | 'remainder', got {other:?}"
        ))),
    }
}

fn validate_kind_amount(kind: &str, amount: Option<Decimal>) -> Result<Option<Decimal>, ApiError> {
    match kind {
        "remainder" => Ok(None),
        "fixed" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount_required_for_kind: amount is required for kind=fixed".into())
            })?;
            if v < Decimal::ZERO {
                return Err(ApiError::BadRequest("amount_negative: amount must be >= 0".into()));
            }
            Ok(Some(v))
        }
        "percent" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount_required_for_kind: amount is required for kind=percent".into())
            })?;
            if v < Decimal::ZERO || v > Decimal::from(100) {
                return Err(ApiError::BadRequest(
                    "percent_out_of_range: amount (percent) must be in [0, 100]".into(),
                ));
            }
            Ok(Some(v))
        }
        _ => unreachable!("normalize_kind already validated"),
    }
}

fn normalize_cap_pair(
    kind: Option<&str>,
    value: Option<Decimal>,
) -> Result<(Option<String>, Option<Decimal>), ApiError> {
    match (kind.map(str::trim).filter(|s| !s.is_empty()), value) {
        (None, None) => Ok((None, None)),
        (Some(k @ ("amount" | "months_expense" | "income_multiple")), Some(v)) => {
            if v < Decimal::ZERO {
                return Err(ApiError::BadRequest("cap_value_negative: cap_value must be >= 0".into()));
            }
            Ok((Some(k.into()), Some(v)))
        }
        (Some(other), Some(_)) => Err(ApiError::BadRequest(format!(
            "cap_kind_invalid: cap_kind must be 'amount' | 'months_expense' | 'income_multiple', got {other:?}"
        ))),
        _ => Err(ApiError::BadRequest(
            "cap_pair_incomplete: cap_kind and cap_value must be provided together".into(),
        )),
    }
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
                    "notes_too_long: notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

/// Verifies that the target asset exists in the same scope. For `?view=mine`, the asset must
/// belong to the user (or be a household row visible to them in their scope).
async fn assert_asset_in_scope(
    pool: &sqlx::PgPool,
    iid: Uuid,
    asset_id: Uuid,
    owner_filter: Option<Uuid>,
) -> Result<(), ApiError> {
    let ok: bool = match owner_filter {
        None => {
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM assets
                    WHERE id = $1 AND installation_id = $2
                )"#,
            )
            .bind(asset_id)
            .bind(iid)
            .fetch_one(pool)
            .await?
        }
        Some(uid) => {
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM assets
                    WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3
                )"#,
            )
            .bind(asset_id)
            .bind(iid)
            .bind(uid)
            .fetch_one(pool)
            .await?
        }
    };
    if !ok {
        return Err(ApiError::BadRequest(
            "target_asset_not_found: target_asset_id must reference an asset in your scope".into(),
        ));
    }
    Ok(())
}

/// Returns the count of `remainder` rules with NO cap in the given scope. These act as the
/// "catch-all" sink at the end of the cascade — exactly one is required per scope.
async fn count_uncapped_remainder_rules(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner_filter: Option<Uuid>,
) -> Result<i64, ApiError> {
    let n: i64 = match owner_filter {
        None => {
            sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL
                     AND kind = 'remainder' AND cap_kind IS NULL"#,
            )
            .bind(iid)
            .fetch_one(pool)
            .await?
        }
        Some(uid) => {
            sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2
                     AND kind = 'remainder' AND cap_kind IS NULL"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(n)
}

/// Returns `(id, priority)` of the uncapped-remainder rule in the scope, if any.
async fn find_uncapped_remainder(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner_filter: Option<Uuid>,
) -> Result<Option<(Uuid, i32)>, ApiError> {
    let row: Option<(Uuid, i32)> = match owner_filter {
        None => {
            sqlx::query_as(
                r#"SELECT id, priority FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL
                     AND kind = 'remainder' AND cap_kind IS NULL
                   ORDER BY priority DESC LIMIT 1"#,
            )
            .bind(iid)
            .fetch_optional(pool)
            .await?
        }
        Some(uid) => {
            sqlx::query_as(
                r#"SELECT id, priority FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2
                     AND kind = 'remainder' AND cap_kind IS NULL
                   ORDER BY priority DESC LIMIT 1"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row)
}

#[utoipa::path(
    get,
    path = "/v1/allocation-rules",
    tag = "allocation-rules",
    params(
        ("view" = Option<String>, Query, description = "`mine` = rows attributed to the signed-in user; omit = full household."),
    ),
    responses(
        (status = 200, description = "Rules ordered by priority ascending", body = [AllocationRuleResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_allocation_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<AllocationRuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_allocation_rules_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_allocation_rules`.
pub(crate) async fn list_allocation_rules_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<AllocationRuleResponse>, ApiError> {
    let scope = view.scope_where("");
    let sql = format!(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE {scope}
           ORDER BY priority ASC, id ASC"#
    );
    let rows: Vec<RuleRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().map(row_to_response).collect())
}

#[utoipa::path(
    post,
    path = "/v1/allocation-rules",
    tag = "allocation-rules",
    request_body = CreateAllocationRuleBody,
    responses(
        (status = 201, description = "Created", body = AllocationRuleResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn create_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateAllocationRuleBody>,
) -> Result<(StatusCode, Json<AllocationRuleResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let kind = normalize_kind(&body.kind)?;
    let amount = validate_kind_amount(&kind, body.amount)?;
    let (cap_kind, cap_value) = normalize_cap_pair(body.cap_kind.as_deref(), body.cap_value)?;
    let notes = normalize_notes(&body.notes)?;
    let enabled = body.enabled.unwrap_or(true);

    let owner = Some(user.id.0);
    assert_asset_in_scope(&state.pool, iid, body.target_asset_id, owner).await?;

    // Enforce the invariant "exactly one uncapped-remainder rule per scope, always last".
    let is_uncapped_remainder = kind == "remainder" && cap_kind.is_none();
    let existing_sink = find_uncapped_remainder(&state.pool, iid, owner).await?;
    if is_uncapped_remainder && existing_sink.is_some() {
        return Err(ApiError::BadRequest(
            "uncapped_remainder_exists: only one 'remainder' rule without a cap is allowed per scope".into(),
        ));
    }

    // Decide the new rule's priority + whether to bump the existing sink.
    // - uncapped remainder being created: it becomes the new last priority.
    // - non-sink rule being created when a sink exists: insert it just before the sink
    //   (take the sink's current priority; bump the sink's priority by +1).
    // - no sink yet: append at the end.
    let mut tx = state.pool.begin().await?;
    let new_priority: i32 = if is_uncapped_remainder {
        // Place at MAX+1.
        sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(priority), 0) + 1
               FROM allocation_rules
               WHERE installation_id = $1 AND owner_user_id = $2"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .fetch_one(&mut *tx)
        .await?
    } else if let Some((sink_id, sink_priority)) = existing_sink {
        // Bump the sink's priority by +1 and reuse its old priority for the new rule.
        sqlx::query(
            r#"UPDATE allocation_rules SET priority = priority + 1
               WHERE id = $1 AND installation_id = $2"#,
        )
        .bind(sink_id)
        .bind(iid)
        .execute(&mut *tx)
        .await?;
        sink_priority
    } else {
        sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(priority), 0) + 1
               FROM allocation_rules
               WHERE installation_id = $1 AND owner_user_id = $2"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .fetch_one(&mut *tx)
        .await?
    };

    let row: RuleRow = sqlx::query_as(
        r#"INSERT INTO allocation_rules (
               installation_id, owner_user_id, target_asset_id, priority,
               kind, amount, cap_kind, cap_value, enabled, notes
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, owner_user_id, target_asset_id, priority, kind, amount,
                     cap_kind, cap_value, enabled, notes"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(body.target_asset_id)
    .bind(new_priority)
    .bind(&kind)
    .bind(amount)
    .bind(&cap_kind)
    .bind(cap_value)
    .bind(enabled)
    .bind(&notes)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    refresh_projection_after_mutation(&state, iid, user.id.0).await;
    Ok((StatusCode::CREATED, Json(row_to_response(row))))
}

#[utoipa::path(
    patch,
    path = "/v1/allocation-rules/{id}",
    tag = "allocation-rules",
    request_body = PatchAllocationRuleBody,
    params(
        ("id" = Uuid, Path, description = "Rule id"),
    ),
    responses(
        (status = 200, description = "Updated", body = AllocationRuleResponse),
        (status = 400, description = "Validation error or would orphan remainder"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Rule missing"),
    )
)]
pub async fn patch_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAllocationRuleBody>,
) -> Result<Json<AllocationRuleResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_allocation_rule_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_allocation_rule` (subset
/// amount/cap/enabled — sin create/delete/reorder desde chat). La invariante del sink
/// (`remainder_required` / `uncapped_remainder_exists`) vive AQUÍ dentro: reimplementarla en
/// otro camino es la vía rápida a corromper la cascada. Invalidación FULL dentro.
pub(crate) async fn patch_allocation_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchAllocationRuleBody,
) -> Result<AllocationRuleResponse, ApiError> {
    // Destructuring EXHAUSTIVO y **sin `..`**: añadir un campo al body deja de compilar hasta que
    // alguien decida si cuenta como «algo que actualizar». Ésta era una de las dos únicas cores de
    // PATCH del repo sin `patch_empty` (`assets.rs`, `budget.rs`, `liabilities.rs`, `planning.rs` e
    // `installation.rs` sí lo tienen), y por eso la tool MCP tuvo que escribirse su propia guardia
    // a mano — donde se olvidó `cap_value` y el campo se evaporaba con un 200 (issue #7 §5). Una
    // guardia que enumera campos siempre puede olvidarse uno; ésta la verifica el compilador.
    {
        let PatchAllocationRuleBody {
            target_asset_id,
            kind,
            amount,
            cap,
            enabled,
            notes,
        } = &body;
        if target_asset_id.is_none()
            && kind.is_none()
            && amount.is_none()
            && cap.is_none()
            && enabled.is_none()
            && notes.is_none()
        {
            return Err(ApiError::BadRequest(
                "patch_empty: provide at least one field to update".into(),
            ));
        }
    }

    let current: Option<RuleRow> = sqlx::query_as(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    let new_kind = match &body.kind {
        Some(k) => normalize_kind(k)?,
        None => current.kind.clone(),
    };

    let new_amount = match &body.amount {
        None => {
            // Keep current value, but ensure validity against possibly-changed kind.
            validate_kind_amount(&new_kind, current.amount)?
        }
        Some(v) if v.is_null() => validate_kind_amount(&new_kind, None)?,
        Some(v) => {
            let d: Decimal = if let serde_json::Value::String(s) = v {
                s.trim().parse().map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: amount must be a valid decimal string".into())
                })?
            } else {
                serde_json::from_value(v.clone()).map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: amount must be a valid decimal".into())
                })?
            };
            validate_kind_amount(&new_kind, Some(d))?
        }
    };

    let (new_cap_kind, new_cap_value) = match &body.cap {
        None => (current.cap_kind.clone(), current.cap_value),
        Some(v) if v.is_null() => (None, None),
        Some(v) => {
            let obj = v.as_object().ok_or_else(|| {
                ApiError::BadRequest("cap_type_invalid: cap must be null or an object {kind, value}".into())
            })?;
            let kind = obj.get("kind").and_then(|x| x.as_str());
            let raw_val = obj.get("value");
            let value: Option<Decimal> = match raw_val {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(s)) => Some(s.trim().parse().map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: cap.value must be a valid decimal string".into())
                })?),
                Some(other) => Some(serde_json::from_value(other.clone()).map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: cap.value must be a valid decimal".into())
                })?),
            };
            normalize_cap_pair(kind, value)?
        }
    };

    let new_target = body.target_asset_id.unwrap_or(current.target_asset_id);
    if new_target != current.target_asset_id {
        assert_asset_in_scope(&state.pool, iid, new_target, current.owner_user_id).await?;
    }

    let new_enabled = body.enabled.unwrap_or(current.enabled);
    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    // Enforce invariants around the "sink" rule (uncapped remainder):
    // 1. If the change would leave the scope without an uncapped remainder, reject.
    // 2. If the change would create a second uncapped remainder, reject.
    let was_sink = current.kind == "remainder" && current.cap_kind.is_none();
    let becomes_sink = new_kind == "remainder" && new_cap_kind.is_none();
    if was_sink && !becomes_sink {
        let n = count_uncapped_remainder_rules(&state.pool, iid, current.owner_user_id).await?;
        if n <= 1 {
            return Err(ApiError::BadRequest(
                "remainder_required: scope must keep one uncapped 'remainder' rule (catch-all sink)".into(),
            ));
        }
    }
    if becomes_sink && !was_sink {
        let n = count_uncapped_remainder_rules(&state.pool, iid, current.owner_user_id).await?;
        if n >= 1 {
            return Err(ApiError::BadRequest(
                "uncapped_remainder_exists: only one 'remainder' rule without a cap is allowed per scope".into(),
            ));
        }
    }

    let updated: RuleRow = sqlx::query_as(
        r#"UPDATE allocation_rules
           SET target_asset_id = $1,
               kind = $2,
               amount = $3,
               cap_kind = $4,
               cap_value = $5,
               enabled = $6,
               notes = $7
           WHERE id = $8 AND installation_id = $9
           RETURNING id, owner_user_id, target_asset_id, priority, kind, amount,
                     cap_kind, cap_value, enabled, notes"#,
    )
    .bind(new_target)
    .bind(&new_kind)
    .bind(new_amount)
    .bind(&new_cap_kind)
    .bind(new_cap_value)
    .bind(new_enabled)
    .bind(&new_notes)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(row_to_response(updated))
}

#[utoipa::path(
    delete,
    path = "/v1/allocation-rules/{id}",
    tag = "allocation-rules",
    params(
        ("id" = Uuid, Path, description = "Rule id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Cannot delete the last 'remainder' rule"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Rule missing"),
    )
)]
pub async fn delete_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let current: Option<(String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT kind, cap_kind, owner_user_id FROM allocation_rules
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;
    let Some((kind, cap_kind, owner)) = current else {
        return Err(ApiError::NotFound);
    };

    // Deleting the sink (uncapped remainder) would orphan the scope.
    if kind == "remainder" && cap_kind.is_none() {
        let n = count_uncapped_remainder_rules(&state.pool, iid, owner).await?;
        if n <= 1 {
            return Err(ApiError::BadRequest(
                "remainder_required: scope must keep one uncapped 'remainder' rule (catch-all sink)".into(),
            ));
        }
    }

    sqlx::query(r#"DELETE FROM allocation_rules WHERE id = $1 AND installation_id = $2"#)
        .bind(id)
        .bind(iid)
        .execute(&state.pool)
        .await?;

    refresh_projection_after_mutation(&state, iid, user.id.0).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/v1/allocation-rules/reorder",
    tag = "allocation-rules",
    request_body = ReorderBody,
    params(
        ("view" = Option<String>, Query, description = "`mine` = scope by user; omit = household."),
    ),
    responses(
        (status = 200, description = "Reordered", body = [AllocationRuleResponse]),
        (status = 400, description = "ids does not match the scope or has duplicates"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn reorder_allocation_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
    Json(body): Json<ReorderBody>,
) -> Result<Json<Vec<AllocationRuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    // Validate no duplicates.
    let mut seen = std::collections::HashSet::new();
    for id in &body.ids {
        if !seen.insert(*id) {
            return Err(ApiError::BadRequest("ids_not_unique: ids must be unique".into()));
        }
    }

    // Load all current rules in this scope; the request must list exactly the same set.
    let view = q.resolve()?;
    let scope = view.scope_where("");
    let current_sql = format!("SELECT id FROM allocation_rules WHERE {scope}");
    let current: Vec<Uuid> = view
        .bind_scope_scalar(sqlx::query_scalar(&current_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;
    let current_set: std::collections::HashSet<Uuid> = current.iter().copied().collect();
    if current_set.len() != body.ids.len() || !body.ids.iter().all(|id| current_set.contains(id)) {
        return Err(ApiError::BadRequest(
            "ids_do_not_match_scope: ids must exactly match the rules in this scope".into(),
        ));
    }

    // Invariant: the uncapped-remainder rule (sink) must be the last one in the order.
    let sink_owner = match view {
        LedgerView::Household => None,
        LedgerView::Mine => Some(user.id.0),
    };
    if let Some((sink_id, _)) = find_uncapped_remainder(&state.pool, iid, sink_owner).await? {
        let last_id = body.ids.last().copied();
        if last_id != Some(sink_id) {
            return Err(ApiError::BadRequest(
                "sink_must_be_last: the uncapped 'remainder' rule must remain the last in the cascade".into(),
            ));
        }
    }

    let mut tx = state.pool.begin().await?;
    for (idx, id) in body.ids.iter().enumerate() {
        let new_priority = (idx as i32) + 1;
        sqlx::query(
            r#"UPDATE allocation_rules SET priority = $1
               WHERE id = $2 AND installation_id = $3"#,
        )
        .bind(new_priority)
        .bind(id)
        .bind(iid)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let rows_sql = format!(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE {scope}
           ORDER BY priority ASC, id ASC"#
    );
    let rows: Vec<RuleRow> = view
        .bind_scope_as(sqlx::query_as(&rows_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;

    refresh_projection_after_mutation(&state, iid, user.id.0).await;
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

// ---------------------------------------------------------------------------
// GET /v1/allocation-rules/resolution — la cascada resuelta de este mes
// ---------------------------------------------------------------------------

/// Una regla de la cascada, resuelta para el mes en curso.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolvedRule {
    #[schema(value_type = String, format = "uuid")]
    pub rule_id: Uuid,
    pub priority: i32,
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub target_asset_name: String,
    /// `fixed` | `percent` | `remainder`
    pub kind: String,
    /// Lo que la regla PIDIÓ antes de aplicar cap y caja disponible.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_intent: Decimal,
    /// Lo que la regla se llevó de verdad. Si es menor que `amount_intent` sin `skipped_reason`,
    /// la regla fue **recortada** (normalmente por el cap) — no saltada.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_resolved: Decimal,
    /// Techo absoluto del cap ya resuelto en euros (los caps relativos se evalúan con los
    /// escalares efectivos del mes).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_ceiling: Option<Decimal>,
    /// Espacio que quedaba bajo el techo al evaluar la regla.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_room: Option<Decimal>,
    /// `no_cash` | `not_reached` | `cap_full` | `zero_amount` | `invalid_target`, o ausente si la
    /// regla recibió algo. Las razones **no se colapsan** porque tienen remedios distintos:
    /// `no_cash` es «no te sobra dinero» y `not_reached` es «las reglas de arriba se lo comieron».
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// Aporte resuelto por activo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolvedAssetContribution {
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

/// Resolución completa de la cascada del mes en curso.
#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationResolutionResponse {
    /// Mes al que corresponde la resolución (`YYYY-MM`).
    pub month: String,
    /// La caja que la cascada reparte de verdad. **Incluye el tramo transitorio de planning**, así
    /// que cambia día a día: es la explicación de por qué la aportación del mes 1 no cuadra con el
    /// neto recurrente del summary.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub base_cash: Decimal,
    /// `income − expense − debt_service`: la parte estable.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub recurring_net: Decimal,
    /// El tramo de los planning flows del mes en curso (menos la retirada de jubilación si aplica).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub planning_component: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub debt_service: Decimal,
    /// `true` cuando `planning_component != 0`: avisa de que `base_cash` lleva dentro un término
    /// que se agota en 90 días y que por tanto **no** es un importe mensual estable.
    pub base_includes_transient: bool,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub allocated_total: Decimal,
    /// Lo que ninguna regla absorbió y acaba en `surplus_cash`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub leftover_to_surplus_cash: Decimal,
    pub rules: Vec<ResolvedRule>,
    pub per_asset: Vec<ResolvedAssetContribution>,
}

#[utoipa::path(
    get,
    path = "/v1/allocation-rules/resolution",
    tag = "allocation-rules",
    params(("view" = Option<String>, Query, description = "`mine` | household.")),
    responses(
        (status = 200, description = "Cascada resuelta del mes en curso", body = AllocationResolutionResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_allocation_resolution(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<AllocationResolutionResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out =
        allocation_resolution_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_allocation_resolution`.
///
/// Endpoint **nuevo** en vez de envolver `list_allocation_rules`: convertir aquel array en un
/// objeto habría roto el contrato. Construye su propio `ProjectionInput` con horizonte 1 (mismo
/// coste que `GET /v1/assets`, una tanda de SELECTs) y **no** pasa por la cache de proyección —
/// coherente con `assets_projection_context`.
pub(crate) async fn allocation_resolution_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<AllocationResolutionResponse, ApiError> {
    use crate::handlers::installation::load_fire_settings;
    use crate::handlers::projection::{build_installation_projection_input, map_engine_err};

    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    let fire_settings = load_fire_settings(pool, iid).await?;
    let built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        today,
        1,
        Decimal::ZERO,
        Some(&fire_settings),
        None,
    )
    .await?;
    let alloc =
        futurefin_engine::first_month_allocation(&built.input).map_err(map_engine_err)?;

    // `priority` y `kind` no viven en el engine: se releen de la tabla y se mapean por id.
    let meta = list_allocation_rules_core(pool, iid, user_id, view).await?;

    let rules: Vec<ResolvedRule> = alloc
        .rules
        .iter()
        .filter_map(|r| {
            let rule_id = *built.allocation_rule_ids.get(r.rule_index)?;
            let m = meta.iter().find(|m| m.id == rule_id)?;
            let (target_asset_id, target_asset_name) =
                built.asset_id_name.get(r.target_index).cloned()?;
            Some(ResolvedRule {
                rule_id,
                priority: m.priority,
                target_asset_id,
                target_asset_name,
                kind: m.kind.clone(),
                amount_intent: r.amount_intent.round_dp(4),
                amount_resolved: r.amount_resolved.round_dp(4),
                cap_ceiling: r.cap_ceiling.map(|v: Decimal| v.round_dp(4)),
                cap_room: r.cap_room.map(|v: Decimal| v.round_dp(4)),
                skipped_reason: r.skipped_reason.map(|s| skip_reason_wire(s).to_string()),
            })
        })
        .collect();

    let per_asset: Vec<ResolvedAssetContribution> = built
        .asset_id_name
        .iter()
        .zip(alloc.per_asset.iter())
        .map(|((asset_id, name), amount)| ResolvedAssetContribution {
            asset_id: *asset_id,
            name: name.clone(),
            amount: amount.round_dp(4),
        })
        .collect();

    let allocated_total: Decimal = alloc.per_asset.iter().copied().sum();

    Ok(AllocationResolutionResponse {
        month: today.format("%Y-%m").to_string(),
        base_cash: alloc.base_cash.round_dp(4),
        recurring_net: alloc.recurring_net.round_dp(4),
        planning_component: alloc.planning_component.round_dp(4),
        debt_service: alloc.debt_service.round_dp(4),
        base_includes_transient: !alloc.planning_component.is_zero(),
        allocated_total: allocated_total.round_dp(4),
        leftover_to_surplus_cash: alloc.leftover.round_dp(4),
        rules,
        per_asset,
    })
}

/// Nombre en el wire de cada razón. Se mapea a mano (y no con `Serialize` en el engine) para que
/// el crate del motor no acabe conociendo el formato de la API.
fn skip_reason_wire(r: futurefin_engine::AllocationSkipReason) -> &'static str {
    use futurefin_engine::AllocationSkipReason as R;
    match r {
        R::NoCash => "no_cash",
        R::NotReached => "not_reached",
        R::CapFull => "cap_full",
        R::ZeroAmount => "zero_amount",
        R::InvalidTarget => "invalid_target",
    }
}

pub fn allocation_rules_router() -> Router {
    Router::new()
        .route("/", get(list_allocation_rules).post(create_allocation_rule))
        .route("/resolution", get(get_allocation_resolution))
        .route("/reorder", post(reorder_allocation_rules))
        .route("/{id}", patch(patch_allocation_rule).delete(delete_allocation_rule))
}
