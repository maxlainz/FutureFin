use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PaymentFrequency {
    Monthly,
    Weekly,
}

impl PaymentFrequency {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PaymentFrequency::Monthly => "monthly",
            PaymentFrequency::Weekly => "weekly",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s.trim() {
            "monthly" => Ok(Self::Monthly),
            "weekly" => Ok(Self::Weekly),
            _ => Err(ApiError::BadRequest(
                "payment_frequency must be monthly or weekly".into(),
            )),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LiabilityResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_tag: Option<String>,
    pub principal_derived_from_plan: bool,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub principal: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_frequency: Option<PaymentFrequency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub payment_end_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLiabilityBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub label: String,
    #[serde(default)]
    pub type_tag: Option<String>,
    #[serde(default)]
    pub derive_principal_from_plan: Option<bool>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub principal: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(default)]
    pub payment_frequency: Option<PaymentFrequency>,
    #[serde(default)]
    pub payment_end_date: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchLiabilityBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub label: Option<String>,
    pub type_tag: Option<String>,
    #[serde(default)]
    pub derive_principal_from_plan: Option<bool>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub principal: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    pub payment_frequency: Option<PaymentFrequency>,
    pub payment_end_date: Option<NaiveDate>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct LiabilityRow {
    id: Uuid,
    category_id: Uuid,
    label: String,
    type_tag: Option<String>,
    principal: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
    notes: Option<String>,
    sort_index: i32,
    principal_derived_from_plan: bool,
}

fn normalize_label(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "label must not be empty".into(),
        ));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "label must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn normalize_type_tag(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 120 {
                return Err(ApiError::BadRequest(
                    "type_tag must be at most 120 characters".into(),
                ));
            }
            Ok(Some(t.into()))
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

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("{field} must be >= 0")));
    }
    Ok(())
}

fn validate_payment_pair(
    amount: Option<Decimal>,
    freq: Option<&str>,
) -> Result<(), ApiError> {
    match (amount, freq) {
        (None, None) => Ok(()),
        (Some(a), Some(f)) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "payment_amount must be > 0 when set".into(),
                ));
            }
            PaymentFrequency::parse(f)?;
            Ok(())
        }
        _ => Err(ApiError::BadRequest(
            "payment_amount and payment_frequency must both be set or both omitted"
                .into(),
        )),
    }
}

/// Principal = payment × intervals from **today**
/// (installation `calendar_tz` civil date) through `payment_end_date` inclusive — monthly steps one
/// calendar month at a time; weekly uses ceil(inclusive_days / 7).
fn payment_interval_count(
    frequency: PaymentFrequency,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<u32, ApiError> {
    if end < start {
        return Err(ApiError::BadRequest(
            "payment_end_date must be on or after today when deriving principal".into(),
        ));
    }
    match frequency {
        PaymentFrequency::Monthly => {
            let mut n = 0u32;
            let mut d = start;
            while d <= end {
                n += 1;
                if n > 1200 {
                    return Err(ApiError::BadRequest(
                        "too many monthly payment intervals".into(),
                    ));
                }
                d = d.checked_add_months(Months::new(1)).ok_or_else(|| {
                    ApiError::BadRequest("payment schedule date overflow".into())
                })?;
            }
            Ok(n)
        }
        PaymentFrequency::Weekly => {
            let days = end.signed_duration_since(start).num_days() + 1;
            let days = days.max(1);
            let intervals = ((days + 6) / 7) as u32;
            if intervals > 5200 {
                return Err(ApiError::BadRequest(
                    "too many weekly payment intervals".into(),
                ));
            }
            Ok(intervals)
        }
    }
}

fn derive_principal_from_payment_plan(
    payment_amount: Decimal,
    frequency: PaymentFrequency,
    payment_end_date: NaiveDate,
    today: NaiveDate,
) -> Result<Decimal, ApiError> {
    let n = payment_interval_count(frequency, today, payment_end_date)?;
    if n == 0 {
        return Err(ApiError::BadRequest(
            "derived principal requires at least one payment interval".into(),
        ));
    }
    Ok(payment_amount * Decimal::from(n))
}

async fn assert_liability_category(
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
                AND scope = 'liability'
        )"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;

    if !ok {
        return Err(ApiError::BadRequest(
            "category_id must reference a liability category in this installation".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn purge_expired_liabilities(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"DELETE FROM liabilities
           WHERE
               installation_id = $1
               AND payment_end_date IS NOT NULL
               AND payment_end_date < CURRENT_DATE"#,
    )
    .bind(installation_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_response(r: LiabilityRow) -> Result<LiabilityResponse, ApiError> {
    let payment_frequency = match r.payment_frequency.as_deref() {
        None => None,
        Some(s) => Some(PaymentFrequency::parse(s)?),
    };
    Ok(LiabilityResponse {
        id: r.id,
        category_id: r.category_id,
        label: r.label,
        type_tag: r.type_tag,
        principal_derived_from_plan: r.principal_derived_from_plan,
        principal: r.principal,
        apr_percent: r.apr_percent,
        payment_amount: r.payment_amount,
        payment_frequency,
        payment_end_date: r.payment_end_date,
        notes: r.notes,
        sort_index: r.sort_index,
    })
}

#[utoipa::path(
    get,
    path = "/v1/liabilities",
    tag = "liabilities",
    params(
        ("view" = Option<String>, Query, description = "`mine` = rows attributed to the signed-in user; omit or other value = full household."),
    ),
    responses(
        (status = 200, description = "Liabilities (purges rows whose payment_end_date is past)", body = [LiabilityResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_liabilities(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<LiabilityResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    purge_expired_liabilities(&state.pool, iid).await?;

    let rows: Vec<LiabilityRow> = match q.resolve() {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT id, category_id, label, type_tag, principal, apr_percent,
                          payment_amount, payment_frequency, payment_end_date, notes,
                          sort_index, principal_derived_from_plan
                   FROM liabilities
                   WHERE installation_id = $1
                   ORDER BY sort_index ASC, label ASC"#,
            )
            .bind(iid)
            .fetch_all(&state.pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT id, category_id, label, type_tag, principal, apr_percent,
                          payment_amount, payment_frequency, payment_end_date, notes,
                          sort_index, principal_derived_from_plan
                   FROM liabilities
                   WHERE installation_id = $1 AND owner_user_id = $2
                   ORDER BY sort_index ASC, label ASC"#,
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
    path = "/v1/liabilities",
    tag = "liabilities",
    request_body = CreateLiabilityBody,
    responses(
        (status = 201, description = "Created", body = LiabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_liability(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateLiabilityBody>,
) -> Result<(axum::http::StatusCode, Json<LiabilityResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    assert_liability_category(&state.pool, iid, body.category_id).await?;

    let label = normalize_label(&body.label)?;
    let type_tag = normalize_type_tag(&body.type_tag)?;
    let derive = body.derive_principal_from_plan.unwrap_or(false);
    let freq_str = body.payment_frequency.map(|f| f.as_str().to_string());

    let (principal, principal_derived) = if derive {
        let amt = body.payment_amount.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_amount is required when derive_principal_from_plan is true".into(),
            )
        })?;
        let fs = freq_str.as_deref().ok_or_else(|| {
            ApiError::BadRequest(
                "payment_frequency is required when derive_principal_from_plan is true".into(),
            )
        })?;
        let end = body.payment_end_date.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_end_date is required when derive_principal_from_plan is true".into(),
            )
        })?;
        validate_payment_pair(Some(amt), Some(fs))?;
        let pf = PaymentFrequency::parse(fs)?;
        let today = installation_naive_today(&state.pool, iid).await?;
        (
            derive_principal_from_payment_plan(amt, pf, end, today)?,
            true,
        )
    } else {
        let p = body.principal.ok_or_else(|| {
            ApiError::BadRequest(
                "principal is required unless derive_principal_from_plan is true".into(),
            )
        })?;
        assert_non_negative(p, "principal")?;
        validate_payment_pair(body.payment_amount, freq_str.as_deref())?;
        (p, false)
    };

    if let Some(apr) = body.apr_percent {
        assert_non_negative(apr, "apr_percent")?;
    }

    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let row: LiabilityRow = sqlx::query_as(
        r#"INSERT INTO liabilities (
               installation_id, category_id, label, type_tag, principal,
               apr_percent, payment_amount, payment_frequency,
               payment_end_date, notes, sort_index, principal_derived_from_plan,
               owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING id, category_id, label, type_tag, principal, apr_percent,
                     payment_amount, payment_frequency, payment_end_date, notes,
                     sort_index, principal_derived_from_plan"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&label)
    .bind(&type_tag)
    .bind(principal)
    .bind(body.apr_percent)
    .bind(body.payment_amount)
    .bind(freq_str.as_deref())
    .bind(body.payment_end_date)
    .bind(&notes)
    .bind(sort_index)
    .bind(principal_derived)
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(row_to_response(row)?),
    ))
}

#[utoipa::path(
    patch,
    path = "/v1/liabilities/{id}",
    tag = "liabilities",
    request_body = PatchLiabilityBody,
    params(
        ("id" = Uuid, Path, description = "Liability id"),
    ),
    responses(
        (status = 200, description = "Updated", body = LiabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Liability missing"),
    )
)]
pub async fn patch_liability(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchLiabilityBody>,
) -> Result<Json<LiabilityResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    if body.category_id.is_none()
        && body.label.is_none()
        && body.type_tag.is_none()
        && body.derive_principal_from_plan.is_none()
        && body.principal.is_none()
        && body.apr_percent.is_none()
        && body.payment_amount.is_none()
        && body.payment_frequency.is_none()
        && body.payment_end_date.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one field to update".into(),
        ));
    }

    let row: Option<LiabilityRow> = sqlx::query_as(
        r#"SELECT id, category_id, label, type_tag, principal, apr_percent,
                  payment_amount, payment_frequency, payment_end_date, notes,
                  sort_index, principal_derived_from_plan
           FROM liabilities
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
        assert_liability_category(&state.pool, iid, new_cat).await?;
    }

    let new_label = match &body.label {
        Some(s) => normalize_label(s)?,
        None => current.label.clone(),
    };

    let new_type_tag = match &body.type_tag {
        Some(_) => normalize_type_tag(&body.type_tag)?,
        None => current.type_tag.clone(),
    };

    let derived_flag = body
        .derive_principal_from_plan
        .unwrap_or(current.principal_derived_from_plan);

    let new_apr = match body.apr_percent {
        Some(apr) => {
            assert_non_negative(apr, "apr_percent")?;
            Some(apr)
        }
        None => current.apr_percent,
    };

    let new_pay_amt = body.payment_amount.or(current.payment_amount);
    let new_pay_freq_str = body
        .payment_frequency
        .map(|f| f.as_str().to_string())
        .or(current.payment_frequency.clone());

    validate_payment_pair(new_pay_amt, new_pay_freq_str.as_deref())?;

    let new_pay_end = body.payment_end_date.or(current.payment_end_date);

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let new_principal_derived = derived_flag;

    let new_principal = if derived_flag {
        let amt = new_pay_amt.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_amount is required when principal is derived from plan".into(),
            )
        })?;
        let fs = new_pay_freq_str.as_deref().ok_or_else(|| {
            ApiError::BadRequest(
                "payment_frequency is required when principal is derived from plan".into(),
            )
        })?;
        let end = new_pay_end.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_end_date is required when principal is derived from plan".into(),
            )
        })?;
        let pf = PaymentFrequency::parse(fs)?;
        let today = installation_naive_today(&state.pool, iid).await?;
        derive_principal_from_payment_plan(amt, pf, end, today)?
    } else {
        match body.principal {
            Some(p) => {
                assert_non_negative(p, "principal")?;
                p
            }
            None => current.principal,
        }
    };

    let updated: LiabilityRow = sqlx::query_as(
        r#"UPDATE liabilities
           SET category_id = $1,
               label = $2,
               type_tag = $3,
               principal = $4,
               apr_percent = $5,
               payment_amount = $6,
               payment_frequency = $7,
               payment_end_date = $8,
               notes = $9,
               sort_index = $10,
               principal_derived_from_plan = $11,
               updated_at = now()
           WHERE id = $12 AND installation_id = $13
           RETURNING id, category_id, label, type_tag, principal, apr_percent,
                     payment_amount, payment_frequency, payment_end_date, notes,
                     sort_index, principal_derived_from_plan"#,
    )
    .bind(new_cat)
    .bind(&new_label)
    .bind(&new_type_tag)
    .bind(new_principal)
    .bind(new_apr)
    .bind(new_pay_amt)
    .bind(new_pay_freq_str.as_deref())
    .bind(new_pay_end)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(new_principal_derived)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(row_to_response(updated)?))
}

#[utoipa::path(
    delete,
    path = "/v1/liabilities/{id}",
    tag = "liabilities",
    params(
        ("id" = Uuid, Path, description = "Liability id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Liability missing"),
    )
)]
pub async fn delete_liability(
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
        sqlx::query(r#"DELETE FROM liabilities WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&state.pool)
            .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn liabilities_router() -> Router {
    Router::new()
        .route("/", get(list_liabilities).post(create_liability))
        .route("/{id}", patch(patch_liability).delete(delete_liability))
}
