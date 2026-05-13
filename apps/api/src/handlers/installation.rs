use crate::error::ApiError;
use crate::handlers::membership::MembershipRole;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use sqlx::{PgPool, Postgres, Transaction};
use sqlx::types::Json as SqlxJson;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Modo de cálculo del FIRE number (presupuesto regular sin cuotas derivadas de pasivos × 12 como base en modos automáticos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FireNumberMode {
    Manual,
    #[default]
    AnnualExpense,
    CurrentIncome,
}

impl<'de> Deserialize<'de> for FireNumberMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "manual" => Self::Manual,
            "annual_expense" => Self::AnnualExpense,
            // Migración: ya no expuesto en API/UI.
            "annual_expense_adjusted" => Self::AnnualExpense,
            "current_income" => Self::CurrentIncome,
            _ => Self::AnnualExpense,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaxBracket {
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub up_to: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub pct: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct FireSettings {
    pub fire_number_mode: FireNumberMode,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub fire_number_manual_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub fire_number_expense_adjustment_pct: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub swr_pct: Decimal,
    pub taxes_enabled: bool,
    pub tax_brackets: Vec<TaxBracket>,
}

impl Default for FireSettings {
    fn default() -> Self {
        default_fire_settings()
    }
}

pub(crate) fn default_fire_settings() -> FireSettings {
    FireSettings {
        fire_number_mode: FireNumberMode::AnnualExpense,
        fire_number_manual_amount: None,
        fire_number_expense_adjustment_pct: None,
        swr_pct: Decimal::new(35, 1),
        taxes_enabled: true,
        tax_brackets: default_es_tax_brackets(),
    }
}

fn default_es_tax_brackets() -> Vec<TaxBracket> {
    vec![
        TaxBracket {
            up_to: Some(Decimal::from(6_000u32)),
            pct: Decimal::from(19u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(50_000u32)),
            pct: Decimal::from(21u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(200_000u32)),
            pct: Decimal::from(23u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(300_000u32)),
            pct: Decimal::from(27u32),
        },
        TaxBracket {
            up_to: None,
            pct: Decimal::from(30u32),
        },
    ]
}

pub(crate) fn resolve_fire_settings(stored: Option<FireSettings>) -> FireSettings {
    match stored {
        None => default_fire_settings(),
        Some(fs) => fs,
    }
}

pub(crate) fn validate_fire_settings(fs: &FireSettings) -> Result<(), ApiError> {
    if fs.swr_pct < Decimal::ZERO || fs.swr_pct > Decimal::from(4u32) {
        return Err(ApiError::BadRequest(
            "swr_pct must be between 0 and 4 (percent)".into(),
        ));
    }
    match fs.fire_number_mode {
        FireNumberMode::Manual => {
            let Some(amt) = fs.fire_number_manual_amount else {
                return Err(ApiError::BadRequest(
                    "fire_number_manual_amount is required when fire_number_mode is manual".into(),
                ));
            };
            if amt <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "fire_number_manual_amount must be > 0".into(),
                ));
            }
        }
        FireNumberMode::AnnualExpense | FireNumberMode::CurrentIncome => {}
    }
    if fs.taxes_enabled {
        validate_tax_brackets(&fs.tax_brackets)?;
    }
    Ok(())
}

fn validate_tax_brackets(brackets: &[TaxBracket]) -> Result<(), ApiError> {
    if brackets.is_empty() {
        return Err(ApiError::BadRequest(
            "tax_brackets must be non-empty when taxes_enabled is true".into(),
        ));
    }
    let last = brackets.len().saturating_sub(1);
    for (i, b) in brackets.iter().enumerate() {
        if b.pct < Decimal::ZERO || b.pct > Decimal::from(99u32) {
            return Err(ApiError::BadRequest(
                "tax bracket pct must be between 0 and 99".into(),
            ));
        }
        let is_last = i == last;
        match (&b.up_to, is_last) {
            (None, true) => {}
            (Some(_), true) => {
                return Err(ApiError::BadRequest(
                    "last tax bracket must have up_to null (open-ended)".into(),
                ));
            }
            (None, false) => {
                return Err(ApiError::BadRequest(
                    "only the last tax bracket may have up_to null".into(),
                ));
            }
            (Some(th), false) => {
                if *th <= Decimal::ZERO {
                    return Err(ApiError::BadRequest(
                        "tax bracket up_to must be > 0 when set".into(),
                    ));
                }
                if i > 0 {
                    let prev = brackets[i - 1].up_to.as_ref().ok_or_else(|| {
                        ApiError::BadRequest("invalid tax_brackets ordering".into())
                    })?;
                    if *th <= *prev {
                        return Err(ApiError::BadRequest(
                            "tax bracket up_to values must be strictly increasing".into(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationSnapshot {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub base_currency: String,
    /// IANA time zone id (e.g. `Europe/Madrid`, `UTC`) for civil calendar operations such as liability derive-principal "today".
    pub calendar_tz: String,
    pub projection_includes_inflation: bool,
    /// Cuando la inflación está activa, supuesto anual % aplicado al modo «dinero de hoy» en la serie.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub annual_inflation_assumption_percent: Option<Decimal>,
    pub projection_target_age: Option<i16>,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationAccess {
    pub installation: InstallationSnapshot,
    pub role: MembershipRole,
}

/// Distinguishes first-time setup (no installation row), pending approval (installation exists but no membership), and active membership.
#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationSessionContext {
    /// True when at least one row exists in `installation`.
    pub installation_initialized: bool,
    pub access: Option<InstallationAccess>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupInstallationBody {
    /// ISO 4217 alphabetic code; MVP allows EUR, USD, GBP.
    pub base_currency: String,
    #[serde(default = "default_calendar_tz")]
    pub calendar_tz: String,
    #[serde(default)]
    pub projection_includes_inflation: bool,
    pub projection_target_age: Option<i16>,
    #[serde(default = "default_show_age_mode")]
    pub show_age_mode: String,
}

fn default_show_age_mode() -> String {
    "dates".into()
}

fn default_calendar_tz() -> String {
    "UTC".into()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchInstallationBody {
    /// When omitted, the time zone is left unchanged.
    #[serde(default)]
    pub calendar_tz: Option<String>,
    /// When omitted, `projection_includes_inflation` is left unchanged.
    #[serde(default)]
    pub projection_includes_inflation: Option<bool>,
    /// When omitted, `projection_target_age` is left unchanged. JSON `null` clears the horizon age.
    #[serde(default)]
    pub projection_target_age: Option<Option<i16>>,
    /// When omitted, `show_age_mode` is left unchanged (`dates` or `ages`).
    #[serde(default)]
    pub show_age_mode: Option<String>,
    /// Omit = unchanged; JSON `null` clears; string percent (e.g. `"2.5"`) when inflation is on.
    #[serde(default)]
    pub annual_inflation_assumption_percent: Option<Option<String>>,
    /// Omit = unchanged; JSON `null` clears stored JSON (defaults apply on read).
    #[serde(default)]
    pub fire_settings: Option<Option<FireSettings>>,
}

#[derive(Debug, sqlx::FromRow)]
struct InstallationMemberRow {
    id: Uuid,
    base_currency: String,
    calendar_tz: String,
    projection_includes_inflation: bool,
    annual_inflation_assumption_percent: Option<Decimal>,
    projection_target_age: Option<i16>,
    show_age_mode: String,
    fire_settings: Option<SqlxJson<FireSettings>>,
    role: String,
}

fn installation_access_from_row(r: InstallationMemberRow) -> Result<InstallationAccess, ApiError> {
    let role = MembershipRole::parse(&r.role)?;
    Ok(InstallationAccess {
        installation: InstallationSnapshot {
            id: r.id,
            base_currency: r.base_currency,
            calendar_tz: r.calendar_tz,
            projection_includes_inflation: r.projection_includes_inflation,
            annual_inflation_assumption_percent: r.annual_inflation_assumption_percent,
            projection_target_age: r.projection_target_age,
            show_age_mode: r.show_age_mode,
            fire_settings: resolve_fire_settings(r.fire_settings.map(|j| j.0)),
        },
        role,
    })
}

fn normalize_currency(code: &str) -> Result<String, ApiError> {
    let trimmed = code.trim();
    if trimmed.len() != 3 || !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(
            "base_currency must be a 3-letter alphabetic code".into(),
        ));
    }
    let upper = trimmed.to_ascii_uppercase();
    if !matches!(upper.as_str(), "EUR" | "USD" | "GBP") {
        return Err(ApiError::BadRequest(
            "unsupported base_currency for MVP (use EUR, USD, or GBP)".into(),
        ));
    }
    Ok(upper)
}

fn validate_show_age_mode(mode: &str) -> Result<(), ApiError> {
    if matches!(mode, "dates" | "ages") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "show_age_mode must be \"dates\" or \"ages\"".into(),
        ))
    }
}

fn validate_target_age(age: Option<i16>) -> Result<(), ApiError> {
    validate_projection_horizon_age(age)
}

fn validate_annual_inflation_assumption(pct: Decimal) -> Result<(), ApiError> {
    if pct.is_sign_negative() || pct > Decimal::from(50) {
        return Err(ApiError::BadRequest(
            "annual_inflation_assumption_percent must be between 0 and 50".into(),
        ));
    }
    Ok(())
}

fn validate_projection_horizon_age(age: Option<i16>) -> Result<(), ApiError> {
    if let Some(a) = age {
        if !(65..=105).contains(&a) {
            return Err(ApiError::BadRequest(
                "projection_target_age must be between 65 and 105 when set".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn normalize_calendar_tz(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if !(3..=64).contains(&t.len()) {
        return Err(ApiError::BadRequest(
            "calendar_tz must be between 3 and 64 characters".into(),
        ));
    }
    let _: Tz = t.parse().map_err(|_| {
        ApiError::BadRequest(
            "calendar_tz must be a valid IANA time zone name (e.g. Europe/Madrid, America/New_York, UTC)"
                .into(),
        )
    })?;
    Ok(t.into())
}

/// Today's date in the installation civil calendar (IANA `calendar_tz`).
pub(crate) async fn installation_naive_today(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<NaiveDate, ApiError> {
    let tz_str: String =
        sqlx::query_scalar(r#"SELECT calendar_tz FROM installation WHERE id = $1"#)
            .bind(installation_id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    naive_date_in_calendar_tz(&tz_str)
}

pub(crate) fn naive_date_in_calendar_tz(tz_name: &str) -> Result<NaiveDate, ApiError> {
    let tz: Tz = tz_name.trim().parse().map_err(|_| {
        ApiError::BadRequest(
            "installation calendar_tz is invalid; update it via PATCH /v1/installation".into(),
        )
    })?;
    Ok(Utc::now().with_timezone(&tz).date_naive())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(ref db) = err {
        return db.code().as_deref() == Some("23505");
    }
    false
}

/// Singleton installation row id, if one exists.
pub async fn singleton_installation_id(pool: &PgPool) -> Result<Option<Uuid>, ApiError> {
    let id: Option<Uuid> =
        sqlx::query_scalar(r#"SELECT id FROM installation ORDER BY created_at ASC LIMIT 1"#)
            .fetch_optional(pool)
            .await?;
    Ok(id)
}

pub async fn require_singleton_installation_id(pool: &PgPool) -> Result<Uuid, ApiError> {
    singleton_installation_id(pool)
        .await?
        .ok_or(ApiError::NotFound)
}

/// First registered user: create singleton installation + owner membership (same transaction).
pub(crate) async fn bootstrap_installation_as_owner_if_empty(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &Uuid,
) -> Result<(), ApiError> {
    let count: i64 = sqlx::query_scalar(r#"SELECT COUNT(*)::bigint FROM installation"#)
        .fetch_one(&mut **tx)
        .await?;
    if count > 0 {
        return Ok(());
    }

    let iid: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        r#"INSERT INTO installation (
               base_currency,
               projection_includes_inflation,
               projection_target_age,
               show_age_mode
           )
           VALUES ('EUR', false, NULL, 'dates')
           RETURNING id"#,
    )
    .fetch_one(&mut **tx)
    .await;

    let iid = match iid {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict);
        }
        Err(e) => return Err(ApiError::Db(e)),
    };

    sqlx::query(
        r#"INSERT INTO installation_memberships (installation_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(MembershipRole::Owner.as_str())
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/installation/session-context",
    tag = "installation",
    responses(
        (status = 200, description = "Whether an installation exists and the caller's membership, if any",
            body = InstallationSessionContext),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn get_installation_session_context(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<InstallationSessionContext>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let installation_initialized: bool =
        sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM installation)"#)
            .fetch_one(&state.pool)
            .await?;

    let row: Option<InstallationMemberRow> = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz, i.projection_includes_inflation,
                  i.annual_inflation_assumption_percent,
                  i.projection_target_age, i.show_age_mode, i.fire_settings, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1
           ORDER BY i.created_at ASC
           LIMIT 1"#,
    )
    .bind(user.id.0)
    .fetch_optional(&state.pool)
    .await?;

    let access = match row {
        Some(r) => Some(installation_access_from_row(r)?),
        None => None,
    };

    Ok(Json(InstallationSessionContext {
        installation_initialized,
        access,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/installation",
    tag = "installation",
    responses(
        (status = 200, description = "Installation context if the user is a member; JSON null if not", body = Option<InstallationAccess>),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn get_my_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Option<InstallationAccess>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let row: Option<InstallationMemberRow> = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz, i.projection_includes_inflation,
                  i.annual_inflation_assumption_percent,
                  i.projection_target_age, i.show_age_mode, i.fire_settings, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1
           ORDER BY i.created_at ASC
           LIMIT 1"#,
    )
    .bind(user.id.0)
    .fetch_optional(&state.pool)
    .await?;

    let Some(r) = row else {
        return Ok(Json(None));
    };

    Ok(Json(Some(installation_access_from_row(r)?)))
}

#[utoipa::path(
    patch,
    path = "/v1/installation",
    tag = "installation",
    request_body = PatchInstallationBody,
    responses(
        (status = 200, description = "Updated", body = InstallationAccess),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not installation owner"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn patch_my_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PatchInstallationBody>,
) -> Result<Json<InstallationAccess>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if role != MembershipRole::Owner {
        return Err(ApiError::Forbidden);
    }

    if body.calendar_tz.is_none()
        && body.projection_includes_inflation.is_none()
        && body.projection_target_age.is_none()
        && body.show_age_mode.is_none()
        && body.annual_inflation_assumption_percent.is_none()
        && body.fire_settings.is_none()
    {
        return Err(ApiError::BadRequest(
            "provide at least one of calendar_tz, projection_includes_inflation, projection_target_age, show_age_mode, annual_inflation_assumption_percent, fire_settings".into(),
        ));
    }

    let row_before: InstallationMemberRow = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz, i.projection_includes_inflation,
                  i.annual_inflation_assumption_percent,
                  i.projection_target_age, i.show_age_mode, i.fire_settings, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1 AND i.id = $2"#,
    )
    .bind(user.id.0)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    let new_tz = if let Some(ref raw) = body.calendar_tz {
        normalize_calendar_tz(raw)?
    } else {
        row_before.calendar_tz.clone()
    };

    let new_inflation = body
        .projection_includes_inflation
        .unwrap_or(row_before.projection_includes_inflation);

    let new_target_age = match body.projection_target_age {
        None => row_before.projection_target_age,
        Some(inner) => {
            validate_projection_horizon_age(inner)?;
            inner
        }
    };

    let new_show_age = if let Some(ref raw) = body.show_age_mode {
        let trimmed = raw.trim();
        validate_show_age_mode(trimmed)?;
        trimmed.to_string()
    } else {
        row_before.show_age_mode.clone()
    };

    let new_ann_inf = match &body.annual_inflation_assumption_percent {
        None => row_before.annual_inflation_assumption_percent,
        Some(None) => None,
        Some(Some(raw)) => {
            let t = raw.trim();
            if t.is_empty() {
                None
            } else {
                let pct = Decimal::from_str(t).map_err(|_| {
                    ApiError::BadRequest(
                        "annual_inflation_assumption_percent must be a decimal number".into(),
                    )
                })?;
                validate_annual_inflation_assumption(pct)?;
                Some(pct)
            }
        }
    };

    let new_fire_settings_json: Option<SqlxJson<FireSettings>> = match &body.fire_settings {
        None => row_before.fire_settings.clone(),
        Some(None) => None,
        Some(Some(fs)) => {
            validate_fire_settings(fs)?;
            Some(SqlxJson(fs.clone()))
        }
    };

    sqlx::query(
        r#"UPDATE installation SET calendar_tz = $1,
               projection_includes_inflation = $2,
               projection_target_age = $3,
               show_age_mode = $4,
               annual_inflation_assumption_percent = $5,
               fire_settings = $6
           WHERE id = $7"#,
    )
    .bind(&new_tz)
    .bind(new_inflation)
    .bind(new_target_age)
    .bind(&new_show_age)
    .bind(new_ann_inf)
    .bind(new_fire_settings_json)
    .bind(iid)
    .execute(&state.pool)
    .await?;

    let row: InstallationMemberRow = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz, i.projection_includes_inflation,
                  i.annual_inflation_assumption_percent,
                  i.projection_target_age, i.show_age_mode, i.fire_settings, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1 AND i.id = $2"#,
    )
    .bind(user.id.0)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(installation_access_from_row(row)?))
}

#[utoipa::path(
    post,
    path = "/v1/installation/setup",
    tag = "installation",
    request_body = SetupInstallationBody,
    responses(
        (status = 201, description = "Installation created; caller becomes owner", body = InstallationAccess),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 409, description = "Installation already exists or user already has access"),
    )
)]
pub async fn setup_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<SetupInstallationBody>,
) -> Result<(axum::http::StatusCode, Json<InstallationAccess>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let currency = normalize_currency(&body.base_currency)?;
    let calendar_tz = normalize_calendar_tz(&body.calendar_tz)?;
    validate_show_age_mode(&body.show_age_mode)?;
    validate_target_age(body.projection_target_age)?;

    let mut tx = state.pool.begin().await?;

    let hc: i64 = sqlx::query_scalar(r#"SELECT COUNT(*)::bigint FROM installation"#)
        .fetch_one(&mut *tx)
        .await?;
    if hc > 0 {
        return Err(ApiError::Conflict);
    }

    let mc: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM installation_memberships WHERE user_id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&mut *tx)
    .await?;
    if mc > 0 {
        return Err(ApiError::Conflict);
    }

    let iid: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        r#"INSERT INTO installation (
               base_currency,
               projection_includes_inflation,
               projection_target_age,
               show_age_mode,
               calendar_tz
           )
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
    )
    .bind(&currency)
    .bind(body.projection_includes_inflation)
    .bind(body.projection_target_age)
    .bind(&body.show_age_mode)
    .bind(&calendar_tz)
    .fetch_one(&mut *tx)
    .await;

    let iid = match iid {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict);
        }
        Err(e) => return Err(ApiError::Db(e)),
    };

    sqlx::query(
        r#"INSERT INTO installation_memberships (installation_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(MembershipRole::Owner.as_str())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(InstallationAccess {
            installation: InstallationSnapshot {
                id: iid,
                base_currency: currency,
                calendar_tz,
                projection_includes_inflation: body.projection_includes_inflation,
                annual_inflation_assumption_percent: None,
                projection_target_age: body.projection_target_age,
                show_age_mode: body.show_age_mode,
                fire_settings: default_fire_settings(),
            },
            role: MembershipRole::Owner,
        }),
    ))
}

pub(crate) async fn user_is_installation_owner(
    pool: &PgPool,
    user_id: Uuid,
    installation_id: Uuid,
) -> Result<bool, ApiError> {
    let role: Option<String> = sqlx::query_scalar(
        r#"SELECT role FROM installation_memberships
           WHERE installation_id = $1 AND user_id = $2"#,
    )
    .bind(installation_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(matches!(role.as_deref(), Some("owner")))
}

/// Resolves the singleton installation and membership role, or `NotFound` / `Forbidden`.
pub async fn require_installation_member(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<(Uuid, MembershipRole), ApiError> {
    let iid = require_singleton_installation_id(pool).await?;
    let Some(role_str) =
        crate::handlers::membership::membership_role(pool, user_id, iid).await?
    else {
        return Err(ApiError::Forbidden);
    };
    let role = MembershipRole::parse(&role_str)?;
    Ok((iid, role))
}
