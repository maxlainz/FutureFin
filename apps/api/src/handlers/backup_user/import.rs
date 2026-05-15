use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::crypto::{decrypt_payload, parse_frame};
use super::schema::{
    migrate_to_current, parse_payload, BackupPayloadV1, UiPreferences, CURRENT_SCHEMA_VERSION,
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportRequest {
    /// `.ffbackup` file bytes, base64-encoded.
    pub file_b64: String,
    pub password: String,
    /// Required true on `import_apply`. Ignored on preview.
    #[serde(default)]
    pub confirm_replace: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportPreviewResponse {
    pub schema_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub username_original: String,
    pub counts: ImportCounts,
    pub birth_date_will_change: bool,
    pub ui_preferences_present: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportCounts {
    pub assets: u32,
    pub liabilities: u32,
    pub budget_entries: u32,
    pub planning_flows: u32,
    pub categories_in_backup: u32,
    pub categories_already_present: u32,
    pub categories_to_create: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportApplyResponse {
    pub imported: ImportCounts,
    pub ui_preferences: UiPreferences,
}

#[utoipa::path(
    post,
    path = "/v1/backup/user-import/preview",
    tag = "backup",
    request_body = ImportRequest,
    responses(
        (status = 200, description = "Preview of what import would change", body = ImportPreviewResponse),
        (status = 400, description = "Malformed backup file"),
        (status = 401, description = "Invalid password / session"),
        (status = 403, description = "Not write role"),
        (status = 409, description = "Schema version not supported"),
    )
)]
pub async fn import_user_backup_preview(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ImportRequest>,
) -> Result<Json<ImportPreviewResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let (manifest, payload) = decode_request(&body)?;

    let counts = compute_counts(&state.pool, iid, &payload).await?;
    let birth_date_will_change = compute_birth_date_change(&state.pool, user.id.0, &payload).await?;
    let ui_preferences_present = payload.ui_preferences.person_scope.is_some()
        || payload.ui_preferences.projection_focus.is_some();

    Ok(Json(ImportPreviewResponse {
        schema_version: manifest.schema_version,
        app_version: manifest.app_version,
        exported_at: manifest.exported_at,
        username_original: manifest.username_original,
        counts,
        birth_date_will_change,
        ui_preferences_present,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/backup/user-import",
    tag = "backup",
    request_body = ImportRequest,
    responses(
        (status = 200, description = "Import applied; counts and ui preferences", body = ImportApplyResponse),
        (status = 400, description = "Malformed backup file or missing confirm_replace"),
        (status = 401, description = "Invalid password / session"),
        (status = 403, description = "Not write role"),
        (status = 409, description = "Schema version not supported"),
    )
)]
pub async fn import_user_backup_apply(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ImportRequest>,
) -> Result<Json<ImportApplyResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    if !body.confirm_replace {
        return Err(ApiError::BadRequest(
            "confirm_replace must be true to apply the destructive replace".into(),
        ));
    }

    let (_manifest, payload) = decode_request(&body)?;

    let mut tx = state.pool.begin().await?;

    // 1. Drop existing user-scoped rows in dependency order.
    sqlx::query(
        r#"DELETE FROM assets WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM liabilities WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM budget_entries WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM planning_flows WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;

    // 2. Ensure all referenced categories exist; build (scope, name) -> id map.
    let cat_map = ensure_categories(&mut tx, iid, &payload).await?;

    // 3. Insert rows with fresh UUIDs.
    let counts = insert_payload(&mut tx, iid, user.id.0, &payload, &cat_map).await?;

    // 4. Update user's birth_date if it differs.
    sqlx::query(r#"UPDATE users SET birth_date = $1 WHERE id = $2"#)
        .bind(payload.user.birth_date)
        .bind(user.id.0)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Json(ImportApplyResponse {
        imported: counts,
        ui_preferences: payload.ui_preferences,
    }))
}

fn decode_request(body: &ImportRequest) -> Result<(super::crypto::Manifest, BackupPayloadV1), ApiError> {
    let bytes = B64
        .decode(body.file_b64.as_bytes())
        .map_err(|_| ApiError::BadRequest("file_b64 is not valid base64".into()))?;
    let parsed = parse_frame(&bytes).map_err(map_crypto_to_api)?;
    if parsed.manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(ApiError::BadRequest(format!(
            "backup schema_version {} not supported (server: {CURRENT_SCHEMA_VERSION})",
            parsed.manifest.schema_version,
        )));
    }
    let plain = decrypt_payload(&parsed, &body.password).map_err(map_crypto_to_api)?;
    let any = parse_payload(parsed.manifest.schema_version, &plain)
        .map_err(ApiError::BadRequest)?;
    let v1 = migrate_to_current(any);
    Ok((parsed.manifest, v1))
}

fn map_crypto_to_api(e: super::crypto::CryptoError) -> ApiError {
    match e {
        super::crypto::CryptoError::Bad(m) => ApiError::BadRequest(m),
        super::crypto::CryptoError::Decrypt => ApiError::Unauthorized,
        super::crypto::CryptoError::Internal => ApiError::Unavailable,
    }
}

async fn compute_counts(
    pool: &PgPool,
    iid: Uuid,
    payload: &BackupPayloadV1,
) -> Result<ImportCounts, ApiError> {
    let mut already_present = 0u32;
    for c in &payload.categories_used {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM categories
                 WHERE installation_id = $1 AND scope = $2 AND name = $3
               )"#,
        )
        .bind(iid)
        .bind(&c.scope)
        .bind(&c.name)
        .fetch_one(pool)
        .await?;
        if exists {
            already_present += 1;
        }
    }
    let in_backup = payload.categories_used.len() as u32;
    Ok(ImportCounts {
        assets: payload.assets.len() as u32,
        liabilities: payload.liabilities.len() as u32,
        budget_entries: payload.budget_entries.len() as u32,
        planning_flows: payload.planning_flows.len() as u32,
        categories_in_backup: in_backup,
        categories_already_present: already_present,
        categories_to_create: in_backup.saturating_sub(already_present),
    })
}

async fn compute_birth_date_change(
    pool: &PgPool,
    user_id: Uuid,
    payload: &BackupPayloadV1,
) -> Result<bool, ApiError> {
    let current: Option<chrono::NaiveDate> =
        sqlx::query_scalar(r#"SELECT birth_date FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(current != payload.user.birth_date)
}

async fn ensure_categories(
    tx: &mut Transaction<'_, Postgres>,
    iid: Uuid,
    payload: &BackupPayloadV1,
) -> Result<HashMap<(String, String), Uuid>, ApiError> {
    let mut map = HashMap::new();
    for c in &payload.categories_used {
        sqlx::query(
            r#"INSERT INTO categories (installation_id, scope, name, sort_index)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (installation_id, scope, name) DO NOTHING"#,
        )
        .bind(iid)
        .bind(&c.scope)
        .bind(&c.name)
        .bind(c.sort_index)
        .execute(&mut **tx)
        .await?;

        let id: Uuid = sqlx::query_scalar(
            r#"SELECT id FROM categories
               WHERE installation_id = $1 AND scope = $2 AND name = $3"#,
        )
        .bind(iid)
        .bind(&c.scope)
        .bind(&c.name)
        .fetch_one(&mut **tx)
        .await?;
        map.insert((c.scope.clone(), c.name.clone()), id);
    }
    Ok(map)
}

fn resolve_category<'a>(
    cat_map: &'a HashMap<(String, String), Uuid>,
    scope: &str,
    name: &str,
) -> Result<Uuid, ApiError> {
    cat_map
        .get(&(scope.to_string(), name.to_string()))
        .copied()
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "backup references category ({scope}, {name}) not present in categories_used"
            ))
        })
}

async fn insert_payload(
    tx: &mut Transaction<'_, Postgres>,
    iid: Uuid,
    user_id: Uuid,
    payload: &BackupPayloadV1,
    cat_map: &HashMap<(String, String), Uuid>,
) -> Result<ImportCounts, ApiError> {
    let mut assets = 0u32;
    for a in &payload.assets {
        let cid = resolve_category(cat_map, &a.category_ref.scope, &a.category_ref.name)?;
        sqlx::query(
            r#"INSERT INTO assets (
                   id, installation_id, owner_user_id, category_id, name, current_value,
                   purchase_price, is_liquid, expected_annual_return_percent,
                   monthly_contribution_fixed, contribution_frequency,
                   contribution_remainder_weight, notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(&a.name)
        .bind(a.current_value)
        .bind(a.purchase_price)
        .bind(a.is_liquid)
        .bind(a.expected_annual_return_percent)
        .bind(a.monthly_contribution_fixed)
        .bind(&a.contribution_frequency)
        .bind(a.contribution_remainder_weight)
        .bind(a.notes.as_deref())
        .bind(a.sort_index)
        .execute(&mut **tx)
        .await?;
        assets += 1;
    }

    let mut liabilities = 0u32;
    for l in &payload.liabilities {
        let cid = resolve_category(cat_map, &l.category_ref.scope, &l.category_ref.name)?;
        sqlx::query(
            r#"INSERT INTO liabilities (
                   id, installation_id, owner_user_id, category_id, label, type_tag,
                   principal, principal_derived_from_plan, apr_percent, payment_amount,
                   payment_frequency, payment_end_date, notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(&l.label)
        .bind(l.type_tag.as_deref())
        .bind(l.principal)
        .bind(l.principal_derived_from_plan)
        .bind(l.apr_percent)
        .bind(l.payment_amount)
        .bind(l.payment_frequency.as_deref())
        .bind(l.payment_end_date)
        .bind(l.notes.as_deref())
        .bind(l.sort_index)
        .execute(&mut **tx)
        .await?;
        liabilities += 1;
    }

    let mut budget_entries = 0u32;
    for b in &payload.budget_entries {
        let cid = resolve_category(cat_map, &b.category_ref.scope, &b.category_ref.name)?;
        sqlx::query(
            r#"INSERT INTO budget_entries (
                   id, installation_id, owner_user_id, category_id, amount,
                   persists_after_retirement, ends_at_retirement, expense_end_date,
                   notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(b.amount)
        .bind(b.persists_after_retirement)
        .bind(b.ends_at_retirement)
        .bind(b.expense_end_date)
        .bind(b.notes.as_deref())
        .bind(b.sort_index)
        .execute(&mut **tx)
        .await?;
        budget_entries += 1;
    }

    let mut planning_flows = 0u32;
    for p in &payload.planning_flows {
        let cid = resolve_category(cat_map, &p.category_ref.scope, &p.category_ref.name)?;
        sqlx::query(
            r#"INSERT INTO planning_flows (
                   id, installation_id, owner_user_id, category_id, title, expected_amount,
                   due_date, show_in_chart, notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(&p.title)
        .bind(p.expected_amount)
        .bind(p.due_date)
        .bind(p.show_in_chart)
        .bind(p.notes.as_deref())
        .bind(p.sort_index)
        .execute(&mut **tx)
        .await?;
        planning_flows += 1;
    }

    Ok(ImportCounts {
        assets,
        liabilities,
        budget_entries,
        planning_flows,
        categories_in_backup: payload.categories_used.len() as u32,
        categories_already_present: 0,
        categories_to_create: 0,
    })
}

// Keep Decimal import alive even if its usage is conditional in some branches above.
#[allow(dead_code)]
fn _decimal_marker(_: Decimal) {}
