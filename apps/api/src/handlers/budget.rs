use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::{purge_expired_liabilities, PaymentFrequency};
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
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
#[serde(rename_all = "lowercase")]
pub enum BudgetCategoryScope {
    Income,
    Expense,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetEntryResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub scope: BudgetCategoryScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub frequency: PaymentFrequency,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_equivalent: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DerivedBudgetLineResponse {
    #[schema(value_type = String, format = "uuid")]
    pub liability_id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub label: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub frequency: PaymentFrequency,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_equivalent: Decimal,
    pub notes: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetTotalsResponse {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_regular_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_derived_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_total_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_monthly_equivalent: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetSnapshotResponse {
    pub entries: Vec<BudgetEntryResponse>,
    pub derived_from_liabilities: Vec<DerivedBudgetLineResponse>,
    pub totals: BudgetTotalsResponse,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBudgetEntryBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub frequency: PaymentFrequency,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchBudgetEntryBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub label: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    pub frequency: Option<PaymentFrequency>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct BudgetEntryJoinRow {
    id: Uuid,
    category_id: Uuid,
    scope: String,
    label: Option<String>,
    amount: Decimal,
    frequency: String,
    notes: Option<String>,
    sort_index: i32,
}

#[derive(Debug, FromRow)]
struct LiabilityDerivedRow {
    id: Uuid,
    category_id: Uuid,
    label: String,
    payment_amount: Decimal,
    payment_frequency: String,
}

fn monthly_equivalent(amount: Decimal, frequency: &str) -> Decimal {
    match frequency.trim() {
        "weekly" => (amount * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amount,
    }
}

fn scope_to_budget_enum(scope: &str) -> Result<BudgetCategoryScope, ApiError> {
    match scope {
        "income" => Ok(BudgetCategoryScope::Income),
        "expense" => Ok(BudgetCategoryScope::Expense),
        _ => Err(ApiError::BadRequest(
            "budget entry category must be income or expense scope".into(),
        )),
    }
}

fn row_to_entry_response(r: BudgetEntryJoinRow) -> Result<BudgetEntryResponse, ApiError> {
    let scope = scope_to_budget_enum(&r.scope)?;
    let pf = PaymentFrequency::parse(&r.frequency)?;
    let monthly_equivalent = monthly_equivalent(r.amount, &r.frequency);
    Ok(BudgetEntryResponse {
        id: r.id,
        category_id: r.category_id,
        scope,
        label: r.label,
        amount: r.amount,
        frequency: pf,
        monthly_equivalent,
        notes: r.notes,
        sort_index: r.sort_index,
    })
}

fn normalize_optional_label(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                Ok(None)
            } else if t.len() > 200 {
                Err(ApiError::BadRequest(
                    "label must be at most 200 characters".into(),
                ))
            } else {
                Ok(Some(t.into()))
            }
        }
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

async fn assert_budget_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<String, ApiError> {
    let scope: Option<String> = sqlx::query_scalar(
        r#"SELECT scope FROM categories
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;

    let Some(s) = scope else {
        return Err(ApiError::BadRequest(
            "category_id must reference a category in this installation".into(),
        ));
    };

    if !matches!(s.as_str(), "income" | "expense") {
        return Err(ApiError::BadRequest(
            "budget entries must use a category with scope income or expense".into(),
        ));
    }

    Ok(s)
}

#[utoipa::path(
    get,
    path = "/v1/budget",
    tag = "budget",
    responses(
        (status = 200, description = "Budget snapshot: persisted entries, liability-derived lines (plan end > today), monthly-normalized totals", body = BudgetSnapshotResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_budget_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<BudgetSnapshotResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    purge_expired_liabilities(&state.pool, iid).await?;

    let rows: Vec<BudgetEntryJoinRow> = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.label, b.amount,
                  b.frequency AS frequency, b.notes, b.sort_index
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.installation_id = $1
           ORDER BY b.sort_index ASC, b.label ASC NULLS LAST, b.id ASC"#,
    )
    .bind(iid)
    .fetch_all(&state.pool)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());
    let mut income_m = Decimal::ZERO;
    let mut expense_reg = Decimal::ZERO;

    for r in rows {
        let me = monthly_equivalent(r.amount, &r.frequency);
        match r.scope.as_str() {
            "income" => income_m += me,
            "expense" => expense_reg += me,
            _ => {}
        }
        entries.push(row_to_entry_response(r)?);
    }

    let derived_raw: Vec<LiabilityDerivedRow> = sqlx::query_as(
        r#"SELECT id, category_id, label, payment_amount, payment_frequency
           FROM liabilities
           WHERE
               installation_id = $1
               AND payment_amount IS NOT NULL
               AND payment_frequency IS NOT NULL
               AND payment_end_date IS NOT NULL
               AND payment_end_date > $2"#,
    )
    .bind(iid)
    .bind(today)
    .fetch_all(&state.pool)
    .await?;

    let mut derived_from_liabilities = Vec::with_capacity(derived_raw.len());
    let mut expense_der = Decimal::ZERO;

    for d in derived_raw {
        let pf = PaymentFrequency::parse(&d.payment_frequency)?;
        let me = monthly_equivalent(d.payment_amount, &d.payment_frequency);
        expense_der += me;
        derived_from_liabilities.push(DerivedBudgetLineResponse {
            liability_id: d.id,
            category_id: d.category_id,
            label: d.label,
            amount: d.payment_amount,
            frequency: pf,
            monthly_equivalent: me,
            notes: "Derived from payment plan".into(),
        });
    }

    let expense_tot = expense_reg + expense_der;
    let net = income_m - expense_tot;

    Ok(Json(BudgetSnapshotResponse {
        entries,
        derived_from_liabilities,
        totals: BudgetTotalsResponse {
            income_monthly_equivalent: income_m,
            expense_regular_monthly_equivalent: expense_reg,
            expense_derived_monthly_equivalent: expense_der,
            expense_total_monthly_equivalent: expense_tot,
            net_monthly_equivalent: net,
        },
    }))
}

#[utoipa::path(
    post,
    path = "/v1/budget/entries",
    tag = "budget",
    request_body = CreateBudgetEntryBody,
    responses(
        (status = 201, description = "Created", body = BudgetEntryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateBudgetEntryBody>,
) -> Result<(axum::http::StatusCode, Json<BudgetEntryResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    assert_budget_category(&state.pool, iid, body.category_id).await?;

    if body.amount <= Decimal::ZERO {
        return Err(ApiError::BadRequest(
            "amount must be greater than zero".into(),
        ));
    }

    let label = normalize_optional_label(&body.label)?;
    let notes = normalize_notes(&body.notes)?;
    let freq_str = body.frequency.as_str().to_string();
    let sort_index = body.sort_index.unwrap_or(0);

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO budget_entries (
               installation_id, category_id, label, amount, frequency, notes, sort_index
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&label)
    .bind(body.amount)
    .bind(&freq_str)
    .bind(&notes)
    .bind(sort_index)
    .fetch_one(&state.pool)
    .await?;

    let row: BudgetEntryJoinRow = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.label, b.amount,
                  b.frequency AS frequency, b.notes, b.sort_index
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.id = $1"#,
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(row_to_entry_response(row)?),
    ))
}

#[utoipa::path(
    patch,
    path = "/v1/budget/entries/{id}",
    tag = "budget",
    request_body = PatchBudgetEntryBody,
    params(
        ("id" = Uuid, Path, description = "Budget entry id"),
    ),
    responses(
        (status = 200, description = "Updated", body = BudgetEntryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Entry missing"),
    )
)]
pub async fn patch_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchBudgetEntryBody>,
) -> Result<Json<BudgetEntryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    if body.category_id.is_none()
        && body.label.is_none()
        && body.amount.is_none()
        && body.frequency.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<BudgetEntryJoinRow> = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.label, b.amount,
                  b.frequency AS frequency, b.notes, b.sort_index
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.id = $1 AND b.installation_id = $2"#,
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

    let new_label = match &body.label {
        Some(_) => normalize_optional_label(&body.label)?,
        None => current.label.clone(),
    };

    let new_amount = match body.amount {
        Some(a) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "amount must be greater than zero".into(),
                ));
            }
            a
        }
        None => current.amount,
    };

    let new_freq_str = body
        .frequency
        .map(|f| f.as_str().to_string())
        .unwrap_or_else(|| current.frequency.clone());

    let _ = PaymentFrequency::parse(&new_freq_str)?;

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: BudgetEntryJoinRow = sqlx::query_as(
        r#"UPDATE budget_entries
           SET category_id = $1,
               label = $2,
               amount = $3,
               frequency = $4,
               notes = $5,
               sort_index = $6,
               updated_at = now()
           WHERE id = $7 AND installation_id = $8
           RETURNING budget_entries.id,
                     budget_entries.category_id,
                     (
                         SELECT c.scope
                         FROM categories c
                         WHERE c.id = budget_entries.category_id
                     ) AS scope,
                     budget_entries.label,
                     budget_entries.amount,
                     budget_entries.frequency,
                     budget_entries.notes,
                     budget_entries.sort_index"#,
    )
    .bind(new_cat)
    .bind(&new_label)
    .bind(new_amount)
    .bind(&new_freq_str)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row_to_entry_response(updated)?))
}

#[utoipa::path(
    delete,
    path = "/v1/budget/entries/{id}",
    tag = "budget",
    params(
        ("id" = Uuid, Path, description = "Budget entry id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Entry missing"),
    )
)]
pub async fn delete_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let res = sqlx::query(r#"DELETE FROM budget_entries WHERE id = $1 AND installation_id = $2"#)
        .bind(id)
        .bind(iid)
        .execute(&state.pool)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn budget_router() -> Router {
    Router::new()
        .route("/", get(get_budget_snapshot))
        .route("/entries", axum::routing::post(create_budget_entry))
        .route(
            "/entries/{id}",
            patch(patch_budget_entry).delete(delete_budget_entry),
        )
}
