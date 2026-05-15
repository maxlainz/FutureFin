use crate::auth::password::verify_password;
use crate::error::ApiError;
use crate::handlers::installation::{resolve_fire_settings, require_installation_member, FireSettings};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{NaiveDate, Utc};
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::crypto::{encrypt_payload, frame_file};
use super::schema::{
    BackupAsset, BackupBudgetEntry, BackupCategory, BackupLiability, BackupPayloadV1,
    BackupPlanningFlow, BackupUser, CategoryRef, InstallationSnapshotInformative, UiPreferences,
    CURRENT_SCHEMA_VERSION,
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportRequest {
    pub password: String,
    #[serde(default)]
    pub ui_preferences: Option<UiPreferences>,
}

#[utoipa::path(
    post,
    path = "/v1/backup/user-export",
    tag = "backup",
    request_body = ExportRequest,
    responses(
        (status = 200, description = "Binary .ffbackup file", content_type = "application/octet-stream"),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Session or password invalid"),
        (status = 403, description = "Not an installation member"),
        (status = 503, description = "Internal error"),
    )
)]
pub async fn export_user_backup(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ExportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;

    let (username, birth_date, password_hash) = fetch_user_for_export(&state.pool, user.id.0).await?;
    verify_password(&body.password, &password_hash)?;

    let payload = build_payload(
        &state.pool,
        iid,
        user.id.0,
        BackupUser { username: username.clone(), birth_date },
        body.ui_preferences.unwrap_or_default(),
    )
    .await?;

    let plaintext = serde_json::to_vec(&payload).map_err(|_| ApiError::Unavailable)?;

    let now = Utc::now();
    let exported_at = now.to_rfc3339();
    let app_version = env!("CARGO_PKG_VERSION");

    let enc = encrypt_payload(
        &plaintext,
        &body.password,
        app_version,
        CURRENT_SCHEMA_VERSION,
        &user.id.0.to_string(),
        &username,
        &exported_at,
    )
    .map_err(|e| {
        tracing::error!(?e, "ffbackup encryption");
        ApiError::Unavailable
    })?;
    let framed = frame_file(&enc.manifest, &enc.ciphertext).map_err(|e| {
        tracing::error!(?e, "ffbackup framing");
        ApiError::Unavailable
    })?;

    let safe_user = sanitize_filename(&username);
    let filename = format!(
        "futurefin-{}-{}.ffbackup",
        safe_user,
        now.format("%Y%m%d")
    );

    let mut res = axum::response::Response::new(axum::body::Body::from(framed));
    res.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/octet-stream"),
    );
    res.headers_mut().insert(
        CONTENT_DISPOSITION,
        http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| {
                http::HeaderValue::from_static("attachment; filename=\"futurefin.ffbackup\"")
            }),
    );
    Ok(res)
}

fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    if cleaned.is_empty() {
        "user".into()
    } else {
        cleaned
    }
}

async fn fetch_user_for_export(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<(String, Option<NaiveDate>, String), ApiError> {
    let row: Option<(String, Option<NaiveDate>, String)> = sqlx::query_as(
        r#"SELECT username, birth_date, password_hash FROM users WHERE id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or(ApiError::Unauthorized)
}

async fn build_payload(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    user: BackupUser,
    ui_preferences: UiPreferences,
) -> Result<BackupPayloadV1, ApiError> {
    let assets = fetch_assets(pool, iid, user_id).await?;
    let liabilities = fetch_liabilities(pool, iid, user_id).await?;
    let budget_entries = fetch_budget_entries(pool, iid, user_id).await?;
    let planning_flows = fetch_planning_flows(pool, iid, user_id).await?;
    let categories_used = fetch_categories_used(pool, iid, user_id).await?;
    let snapshot = fetch_installation_snapshot(pool, iid).await?;

    Ok(BackupPayloadV1 {
        user,
        categories_used,
        assets,
        liabilities,
        budget_entries,
        planning_flows,
        ui_preferences,
        installation_snapshot_informative: snapshot,
    })
}

async fn fetch_assets(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupAsset>, ApiError> {
    let rows: Vec<(
        String,
        String,
        String,
        Decimal,
        Option<Decimal>,
        bool,
        Option<Decimal>,
        Decimal,
        String,
        Decimal,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        r#"SELECT c.scope, c.name AS cat_name, a.name, a.current_value, a.purchase_price,
                  a.is_liquid, a.expected_annual_return_percent, a.monthly_contribution_fixed,
                  a.contribution_frequency, a.contribution_remainder_weight,
                  a.notes, a.sort_index
           FROM assets a
           JOIN categories c ON c.id = a.category_id
           WHERE a.installation_id = $1 AND a.owner_user_id = $2
           ORDER BY a.sort_index ASC, a.name ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BackupAsset {
            category_ref: CategoryRef { scope: r.0, name: r.1 },
            name: r.2,
            current_value: r.3,
            purchase_price: r.4,
            is_liquid: r.5,
            expected_annual_return_percent: r.6,
            monthly_contribution_fixed: r.7,
            contribution_frequency: r.8,
            contribution_remainder_weight: r.9,
            notes: r.10,
            sort_index: r.11,
        })
        .collect())
}

async fn fetch_liabilities(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupLiability>, ApiError> {
    let rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Decimal,
        bool,
        Option<Decimal>,
        Option<Decimal>,
        Option<String>,
        Option<NaiveDate>,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        r#"SELECT c.scope, c.name AS cat_name, l.label, l.type_tag, l.principal,
                  l.principal_derived_from_plan, l.apr_percent, l.payment_amount,
                  l.payment_frequency, l.payment_end_date, l.notes, l.sort_index
           FROM liabilities l
           JOIN categories c ON c.id = l.category_id
           WHERE l.installation_id = $1 AND l.owner_user_id = $2
           ORDER BY l.sort_index ASC, l.label ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BackupLiability {
            category_ref: CategoryRef { scope: r.0, name: r.1 },
            label: r.2,
            type_tag: r.3,
            principal: r.4,
            principal_derived_from_plan: r.5,
            apr_percent: r.6,
            payment_amount: r.7,
            payment_frequency: r.8,
            payment_end_date: r.9,
            notes: r.10,
            sort_index: r.11,
        })
        .collect())
}

async fn fetch_budget_entries(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupBudgetEntry>, ApiError> {
    let rows: Vec<(
        String,
        String,
        Decimal,
        bool,
        bool,
        Option<NaiveDate>,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        r#"SELECT c.scope, c.name AS cat_name, b.amount,
                  b.persists_after_retirement, b.ends_at_retirement, b.expense_end_date,
                  b.notes, b.sort_index
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.installation_id = $1 AND b.owner_user_id = $2
           ORDER BY b.sort_index ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BackupBudgetEntry {
            category_ref: CategoryRef { scope: r.0, name: r.1 },
            amount: r.2,
            persists_after_retirement: r.3,
            ends_at_retirement: r.4,
            expense_end_date: r.5,
            notes: r.6,
            sort_index: r.7,
        })
        .collect())
}

async fn fetch_planning_flows(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupPlanningFlow>, ApiError> {
    let rows: Vec<(
        String,
        String,
        String,
        Decimal,
        Option<NaiveDate>,
        bool,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        r#"SELECT c.scope, c.name AS cat_name, p.title, p.expected_amount, p.due_date,
                  p.show_in_chart, p.notes, p.sort_index
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE p.installation_id = $1 AND p.owner_user_id = $2
           ORDER BY p.sort_index ASC, p.title ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BackupPlanningFlow {
            category_ref: CategoryRef { scope: r.0, name: r.1 },
            title: r.2,
            expected_amount: r.3,
            due_date: r.4,
            show_in_chart: r.5,
            notes: r.6,
            sort_index: r.7,
        })
        .collect())
}

async fn fetch_categories_used(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupCategory>, ApiError> {
    let rows: Vec<(String, String, i32)> = sqlx::query_as(
        r#"SELECT DISTINCT c.scope, c.name, c.sort_index FROM categories c
           WHERE c.installation_id = $1
             AND (
                EXISTS (SELECT 1 FROM assets a WHERE a.category_id = c.id AND a.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM liabilities l WHERE l.category_id = c.id AND l.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM budget_entries b WHERE b.category_id = c.id AND b.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM planning_flows p WHERE p.category_id = c.id AND p.owner_user_id = $2)
             )
           ORDER BY c.scope ASC, c.sort_index ASC, c.name ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(scope, name, sort_index)| BackupCategory { scope, name, sort_index })
        .collect())
}

async fn fetch_installation_snapshot(
    pool: &PgPool,
    iid: Uuid,
) -> Result<InstallationSnapshotInformative, ApiError> {
    let row: (
        String,
        String,
        bool,
        Option<Decimal>,
        String,
        Option<SqlxJson<FireSettings>>,
    ) = sqlx::query_as(
        r#"SELECT base_currency, calendar_tz, projection_includes_inflation,
                  annual_inflation_assumption_percent, show_age_mode, fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool)
    .await?;

    Ok(InstallationSnapshotInformative {
        base_currency: row.0,
        calendar_tz: row.1,
        projection_includes_inflation: row.2,
        annual_inflation_assumption_percent: row.3,
        show_age_mode: row.4,
        fire_settings: resolve_fire_settings(row.5.map(|j| j.0)),
    })
}
