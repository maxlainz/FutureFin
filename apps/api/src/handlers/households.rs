use crate::error::ApiError;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HouseholdRole {
    Owner,
    Member,
    Viewer,
}

impl HouseholdRole {
    fn as_str(self) -> &'static str {
        match self {
            HouseholdRole::Owner => "owner",
            HouseholdRole::Member => "member",
            HouseholdRole::Viewer => "viewer",
        }
    }

    fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            "viewer" => Ok(Self::Viewer),
            _ => Err(ApiError::BadRequest("invalid membership role".into())),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HouseholdSummary {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub name: String,
    pub base_currency: String,
    pub role: HouseholdRole,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateHouseholdBody {
    pub name: String,
    /// ISO 4217 alphabetic code; MVP allows EUR, USD, GBP (`AUTH_MODEL` / product dossier).
    pub base_currency: String,
    #[serde(default)]
    pub projection_includes_inflation: bool,
    pub projection_target_age: Option<i16>,
    #[serde(default = "default_show_age_mode")]
    pub show_age_mode: String,
}

fn default_show_age_mode() -> String {
    "dates".into()
}

#[derive(Debug, FromRow)]
struct HouseholdRow {
    id: Uuid,
    name: String,
    base_currency: String,
    role: String,
}

fn validate_household_name(name: &str) -> Result<(), ApiError> {
    let len = name.chars().count();
    if !(1..=128).contains(&len) {
        return Err(ApiError::BadRequest(
            "household name must be between 1 and 128 characters".into(),
        ));
    }
    Ok(())
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

#[utoipa::path(
    get,
    path = "/v1/households",
    tag = "households",
    responses(
        (status = 200, description = "Households visible to the signed-in user", body = [HouseholdSummary]),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn list_households(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<HouseholdSummary>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let rows: Vec<HouseholdRow> = sqlx::query_as(
        r#"SELECT h.id, h.name, h.base_currency AS base_currency, m.role
           FROM household_memberships m
           JOIN households h ON h.id = m.household_id
           WHERE m.user_id = $1
           ORDER BY h.name"#,
    )
    .bind(user.id.0)
    .fetch_all(&state.pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let role = HouseholdRole::parse(&row.role)?;
        out.push(HouseholdSummary {
            id: row.id,
            name: row.name,
            base_currency: row.base_currency,
            role,
        });
    }
    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/v1/households",
    tag = "households",
    request_body = CreateHouseholdBody,
    responses(
        (status = 201, description = "Household created; caller becomes owner", body = HouseholdSummary),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn create_household(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateHouseholdBody>,
) -> Result<(axum::http::StatusCode, Json<HouseholdSummary>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    validate_household_name(&body.name)?;
    let currency = normalize_currency(&body.base_currency)?;
    validate_show_age_mode(&body.show_age_mode)?;
    validate_target_age(body.projection_target_age)?;

    let summary =
        insert_household_and_membership(&state.pool, &user.id.0, &body, &currency).await?;

    Ok((axum::http::StatusCode::CREATED, Json(summary)))
}

async fn insert_household_and_membership(
    pool: &PgPool,
    user_id: &Uuid,
    body: &CreateHouseholdBody,
    currency: &str,
) -> Result<HouseholdSummary, ApiError> {
    let mut tx = pool.begin().await?;

    let hid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO households (
               name,
               base_currency,
               projection_includes_inflation,
               projection_target_age,
               show_age_mode
           )
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
    )
    .bind(&body.name)
    .bind(currency)
    .bind(body.projection_includes_inflation)
    .bind(body.projection_target_age)
    .bind(&body.show_age_mode)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO household_memberships (household_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(hid)
    .bind(user_id)
    .bind(HouseholdRole::Owner.as_str())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(HouseholdSummary {
        id: hid,
        name: body.name.clone(),
        base_currency: currency.to_string(),
        role: HouseholdRole::Owner,
    })
}
