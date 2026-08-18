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
            "kind must be 'fixed' | 'percent' | 'remainder', got {other:?}"
        ))),
    }
}

fn validate_kind_amount(kind: &str, amount: Option<Decimal>) -> Result<Option<Decimal>, ApiError> {
    match kind {
        "remainder" => Ok(None),
        "fixed" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount is required for kind=fixed".into())
            })?;
            if v < Decimal::ZERO {
                return Err(ApiError::BadRequest("amount must be >= 0".into()));
            }
            Ok(Some(v))
        }
        "percent" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount is required for kind=percent".into())
            })?;
            if v < Decimal::ZERO || v > Decimal::from(100) {
                return Err(ApiError::BadRequest(
                    "amount (percent) must be in [0, 100]".into(),
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
                return Err(ApiError::BadRequest("cap_value must be >= 0".into()));
            }
            Ok((Some(k.into()), Some(v)))
        }
        (Some(other), Some(_)) => Err(ApiError::BadRequest(format!(
            "cap_kind must be 'amount' | 'months_expense' | 'income_multiple', got {other:?}"
        ))),
        _ => Err(ApiError::BadRequest(
            "cap_kind and cap_value must be provided together".into(),
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
                    "notes must be at most 4000 characters".into(),
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
            "target_asset_id must reference an asset in your scope".into(),
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
    let out = list_allocation_rules_core(&state.pool, iid, user.id.0, q.resolve()).await?;
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

    refresh_projection_after_mutation(state.clone(), iid, user.id.0);
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
                    ApiError::BadRequest("amount must be a valid decimal string".into())
                })?
            } else {
                serde_json::from_value(v.clone()).map_err(|_| {
                    ApiError::BadRequest("amount must be a valid decimal".into())
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
                ApiError::BadRequest("cap must be null or an object {kind, value}".into())
            })?;
            let kind = obj.get("kind").and_then(|x| x.as_str());
            let raw_val = obj.get("value");
            let value: Option<Decimal> = match raw_val {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(s)) => Some(s.trim().parse().map_err(|_| {
                    ApiError::BadRequest("cap.value must be a valid decimal string".into())
                })?),
                Some(other) => Some(serde_json::from_value(other.clone()).map_err(|_| {
                    ApiError::BadRequest("cap.value must be a valid decimal".into())
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

    refresh_projection_after_mutation(state.clone(), iid, user_id);
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

    refresh_projection_after_mutation(state.clone(), iid, user.id.0);
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
            return Err(ApiError::BadRequest("ids must be unique".into()));
        }
    }

    // Load all current rules in this scope; the request must list exactly the same set.
    let view = q.resolve();
    let scope = view.scope_where("");
    let current_sql = format!("SELECT id FROM allocation_rules WHERE {scope}");
    let current: Vec<Uuid> = view
        .bind_scope_scalar(sqlx::query_scalar(&current_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;
    let current_set: std::collections::HashSet<Uuid> = current.iter().copied().collect();
    if current_set.len() != body.ids.len() || !body.ids.iter().all(|id| current_set.contains(id)) {
        return Err(ApiError::BadRequest(
            "ids must exactly match the rules in this scope".into(),
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

    refresh_projection_after_mutation(state.clone(), iid, user.id.0);
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

pub fn allocation_rules_router() -> Router {
    Router::new()
        .route("/", get(list_allocation_rules).post(create_allocation_rule))
        .route("/reorder", post(reorder_allocation_rules))
        .route("/{id}", patch(patch_allocation_rule).delete(delete_allocation_rule))
}
