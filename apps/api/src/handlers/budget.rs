use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::{purge_expired_liabilities, PaymentFrequency};
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
    /// Importe mensual (el presupuesto persistido es solo mensual).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
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
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
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
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
pub(crate) struct BudgetEntryJoinRow {
    id: Uuid,
    category_id: Uuid,
    scope: String,
    amount: Decimal,
    notes: Option<String>,
    sort_index: i32,
}

#[derive(Debug, FromRow)]
pub(crate) struct LiabilityDerivedRow {
    id: Uuid,
    category_id: Uuid,
    label: String,
    payment_amount: Decimal,
    payment_frequency: String,
}

pub(crate) fn monthly_equivalent(amount: Decimal, frequency: &str) -> Decimal {
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
    Ok(BudgetEntryResponse {
        id: r.id,
        category_id: r.category_id,
        scope,
        amount: r.amount,
        notes: r.notes,
        sort_index: r.sort_index,
    })
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

pub(crate) async fn assert_budget_category(
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

async fn fetch_budget_rows_and_derived_liabilities(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<(Vec<BudgetEntryJoinRow>, Vec<LiabilityDerivedRow>), ApiError> {
    let rows: Vec<BudgetEntryJoinRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                          b.notes, b.sort_index
                   FROM budget_entries b
                   JOIN categories c ON c.id = b.category_id
                   WHERE b.installation_id = $1
                   ORDER BY b.sort_index ASC, b.id ASC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                          b.notes, b.sort_index
                   FROM budget_entries b
                   JOIN categories c ON c.id = b.category_id
                   WHERE b.installation_id = $1 AND b.owner_user_id = $2
                   ORDER BY b.sort_index ASC, b.id ASC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let derived_raw: Vec<LiabilityDerivedRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
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
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT id, category_id, label, payment_amount, payment_frequency
                   FROM liabilities
                   WHERE
                       installation_id = $1
                       AND owner_user_id = $3
                       AND payment_amount IS NOT NULL
                       AND payment_frequency IS NOT NULL
                       AND payment_end_date IS NOT NULL
                       AND payment_end_date > $2"#,
            )
            .bind(iid)
            .bind(today)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    Ok((rows, derived_raw))
}

pub(crate) fn ledger_budget_totals_from_parts(
    rows: &[BudgetEntryJoinRow],
    derived_raw: &[LiabilityDerivedRow],
) -> Result<BudgetTotalsResponse, ApiError> {
    let mut income_m = Decimal::ZERO;
    let mut expense_reg = Decimal::ZERO;

    for r in rows {
        let me = r.amount;
        match r.scope.as_str() {
            "income" => income_m += me,
            "expense" => expense_reg += me,
            _ => {}
        }
    }

    let mut expense_der = Decimal::ZERO;
    for d in derived_raw {
        PaymentFrequency::parse(&d.payment_frequency)?;
        let me = monthly_equivalent(d.payment_amount, &d.payment_frequency);
        expense_der += me;
    }

    let expense_tot = expense_reg + expense_der;
    let net = income_m - expense_tot;

    Ok(BudgetTotalsResponse {
        income_monthly_equivalent: income_m,
        expense_regular_monthly_equivalent: expense_reg,
        expense_derived_monthly_equivalent: expense_der,
        expense_total_monthly_equivalent: expense_tot,
        net_monthly_equivalent: net,
    })
}

/// Same totals as `GET /v1/budget` (persisted entries + liability-derived lines with plan end after today).
pub(crate) async fn ledger_budget_totals_for_summary(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<BudgetTotalsResponse, ApiError> {
    let (rows, derived_raw) =
        fetch_budget_rows_and_derived_liabilities(pool, iid, session_user_id, view, today).await?;
    ledger_budget_totals_from_parts(&rows, &derived_raw)
}

/// Persisted budget rows only (no liability-derived lines), for projection / FIRE expense bases.
pub(crate) async fn ledger_regular_monthly_income_and_expense(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<(Decimal, Decimal), ApiError> {
    let (rows, _) =
        fetch_budget_rows_and_derived_liabilities(pool, iid, session_user_id, view, today).await?;
    let mut income_m = Decimal::ZERO;
    let mut expense_reg = Decimal::ZERO;
    for r in rows {
        let me = r.amount;
        match r.scope.as_str() {
            "income" => income_m += me,
            "expense" => expense_reg += me,
            _ => {}
        }
    }
    Ok((income_m, expense_reg))
}

#[utoipa::path(
    get,
    path = "/v1/budget",
    tag = "budget",
    params(
        ("view" = Option<String>, Query, description = "`mine` = persisted budget rows attributed to the signed-in user (derived liability lines filtered the same); omit = household."),
    ),
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
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<BudgetSnapshotResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    purge_expired_liabilities(&state.pool, iid).await?;

    let view = q.resolve();
    let (rows, derived_raw) = fetch_budget_rows_and_derived_liabilities(
        &state.pool,
        iid,
        user.id.0,
        view,
        today,
    )
    .await?;

    let totals = ledger_budget_totals_from_parts(&rows, &derived_raw)?;

    let mut entries = Vec::with_capacity(rows.len());
    for r in rows {
        entries.push(row_to_entry_response(r)?);
    }

    let mut derived_from_liabilities = Vec::with_capacity(derived_raw.len());
    for d in derived_raw {
        let pf = PaymentFrequency::parse(&d.payment_frequency)?;
        let me = monthly_equivalent(d.payment_amount, &d.payment_frequency);
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

    Ok(Json(BudgetSnapshotResponse {
        entries,
        derived_from_liabilities,
        totals,
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

    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO budget_entries (
               installation_id, category_id, amount, notes, sort_index,
               owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(body.amount)
    .bind(&notes)
    .bind(sort_index)
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    let row: BudgetEntryJoinRow = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                  b.notes, b.sort_index
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
        && body.amount.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<BudgetEntryJoinRow> = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                  b.notes, b.sort_index
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

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: BudgetEntryJoinRow = sqlx::query_as(
        r#"UPDATE budget_entries
           SET category_id = $1,
               amount = $2,
               notes = $3,
               sort_index = $4,
               updated_at = now()
           WHERE id = $5 AND installation_id = $6
           RETURNING budget_entries.id,
                     budget_entries.category_id,
                     (
                         SELECT c.scope
                         FROM categories c
                         WHERE c.id = budget_entries.category_id
                     ) AS scope,
                     budget_entries.amount,
                     budget_entries.notes,
                     budget_entries.sort_index"#,
    )
    .bind(new_cat)
    .bind(new_amount)
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
