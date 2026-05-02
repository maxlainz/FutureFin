use crate::error::ApiError;
use crate::handlers::membership::MembershipRole;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationSnapshot {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub base_currency: String,
    /// IANA time zone id (e.g. `Europe/Madrid`, `UTC`) for civil calendar operations such as liability derive-principal "today".
    pub calendar_tz: String,
    pub projection_includes_inflation: bool,
    pub projection_target_age: Option<i16>,
    pub show_age_mode: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationAccess {
    pub installation: InstallationSnapshot,
    pub role: MembershipRole,
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
    pub calendar_tz: String,
}

#[derive(Debug, sqlx::FromRow)]
struct InstallationMemberRow {
    id: Uuid,
    base_currency: String,
    calendar_tz: String,
    projection_includes_inflation: bool,
    projection_target_age: Option<i16>,
    show_age_mode: String,
    role: String,
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
                  i.projection_target_age, i.show_age_mode, m.role
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

    let role = MembershipRole::parse(&r.role)?;
    Ok(Json(Some(InstallationAccess {
        installation: InstallationSnapshot {
            id: r.id,
            base_currency: r.base_currency,
            calendar_tz: r.calendar_tz,
            projection_includes_inflation: r.projection_includes_inflation,
            projection_target_age: r.projection_target_age,
            show_age_mode: r.show_age_mode,
        },
        role,
    })))
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

    let tz = normalize_calendar_tz(&body.calendar_tz)?;
    sqlx::query(r#"UPDATE installation SET calendar_tz = $1 WHERE id = $2"#)
        .bind(&tz)
        .bind(iid)
        .execute(&state.pool)
        .await?;

    let row: InstallationMemberRow = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz, i.projection_includes_inflation,
                  i.projection_target_age, i.show_age_mode, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1 AND i.id = $2"#,
    )
    .bind(user.id.0)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    let role_out = MembershipRole::parse(&row.role)?;
    Ok(Json(InstallationAccess {
        installation: InstallationSnapshot {
            id: row.id,
            base_currency: row.base_currency,
            calendar_tz: row.calendar_tz,
            projection_includes_inflation: row.projection_includes_inflation,
            projection_target_age: row.projection_target_age,
            show_age_mode: row.show_age_mode,
        },
        role: role_out,
    }))
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
                projection_target_age: body.projection_target_age,
                show_age_mode: body.show_age_mode,
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
