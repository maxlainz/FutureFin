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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::crypto::{decrypt_payload, parse_frame};
use super::schema::{
    migrate_to_current, parse_payload, BackupPayload, UiPreferences, CURRENT_SCHEMA_VERSION,
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
    /// History snapshot headers in the backup (schema_version ≥ 4; 0 for older files).
    pub snapshots: u32,
    /// Total history snapshot items across all snapshots in the backup.
    pub snapshot_items: u32,
    /// CSV import batches in the backup (schema_version ≥ 5; 0 for older files).
    pub transaction_imports: u32,
    /// Dated transactions in the backup (schema_version ≥ 5; 0 for older files).
    pub transactions: u32,
    /// Categorization rules in the backup (schema_version ≥ 5; 0 for older files).
    pub categorization_rules: u32,
    /// Recurring-transaction rules in the backup (schema_version ≥ 6; 0 for older files).
    pub recurring_transaction_rules: u32,
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
    //    history_snapshots first: its `source_item_id` is NOT a FK (a snapshot references
    //    nothing and survives ledger deletes), so nothing forces this ordering — but the
    //    snapshots belong to this user and must be cleared before their ledger churns, and
    //    doing it first keeps the whole wipe deterministic. Items cascade via ON DELETE CASCADE.
    sqlx::query(
        r#"DELETE FROM history_snapshots WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    //    transactions + their import batches next (schema_version ≥ 5). Deleting the batch would
    //    cascade to its transactions, but we delete both explicitly before assets/liabilities so
    //    the SET NULL FKs (linked_asset_id/linked_liability_id) never fire mid-wipe and the whole
    //    order stays deterministic. categorization_rules follow (their FK to categories is SET NULL).
    sqlx::query(
        r#"DELETE FROM transactions WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM transaction_imports WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM categorization_rules WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    //    recurring_transaction_rules after transactions/imports/rules (transactions.recurring_rule_id
    //    was already cleared by the delete above) and BEFORE assets/liabilities, so its SET NULL FKs
    //    (linked_asset_id/linked_liability_id) never fire mid-wipe.
    sqlx::query(
        r#"DELETE FROM recurring_transaction_rules WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    //    allocation_rules next because it FKs into assets (ON DELETE CASCADE would also handle
    //    this, but being explicit keeps the order deterministic).
    sqlx::query(
        r#"DELETE FROM allocation_rules WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
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

    // A full replace rewrites assets/liabilities/budget — all projection-engine inputs — so the
    // in-memory projection cache would otherwise stay stale for up to its TTL. Invalidate it now.
    crate::handlers::projection::refresh_projection_after_mutation(state.clone(), iid, user.id.0);

    Ok(Json(ImportApplyResponse {
        imported: counts,
        ui_preferences: payload.ui_preferences,
    }))
}

fn decode_request(body: &ImportRequest) -> Result<(super::crypto::Manifest, BackupPayload), ApiError> {
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
    payload: &BackupPayload,
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
    let snapshot_items: u32 = payload
        .snapshots
        .iter()
        .map(|s| s.items.len() as u32)
        .sum();
    Ok(ImportCounts {
        assets: payload.assets.len() as u32,
        liabilities: payload.liabilities.len() as u32,
        budget_entries: payload.budget_entries.len() as u32,
        planning_flows: payload.planning_flows.len() as u32,
        categories_in_backup: in_backup,
        categories_already_present: already_present,
        categories_to_create: in_backup.saturating_sub(already_present),
        snapshots: payload.snapshots.len() as u32,
        snapshot_items,
        transaction_imports: payload.transaction_imports.len() as u32,
        transactions: payload.transactions.len() as u32,
        categorization_rules: payload.categorization_rules.len() as u32,
        recurring_transaction_rules: payload.recurring_transaction_rules.len() as u32,
    })
}

async fn compute_birth_date_change(
    pool: &PgPool,
    user_id: Uuid,
    payload: &BackupPayload,
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
    payload: &BackupPayload,
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
    payload: &BackupPayload,
    cat_map: &HashMap<(String, String), Uuid>,
) -> Result<ImportCounts, ApiError> {
    // Insert assets and remember each freshly-minted UUID at the same index as the backup,
    // so allocation_rules.target_asset_index can be resolved below.
    let mut new_asset_ids: Vec<Uuid> = Vec::with_capacity(payload.assets.len());
    let mut assets = 0u32;
    for a in &payload.assets {
        let cid = resolve_category(cat_map, &a.category_ref.scope, &a.category_ref.name)?;
        let new_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO assets (
                   id, installation_id, owner_user_id, category_id, name, current_value,
                   purchase_price, is_liquid, expected_annual_return_percent,
                   notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(new_id)
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(&a.name)
        .bind(a.current_value)
        .bind(a.purchase_price)
        .bind(a.is_liquid)
        .bind(a.expected_annual_return_percent)
        .bind(a.notes.as_deref())
        .bind(a.sort_index)
        .execute(&mut **tx)
        .await?;
        new_asset_ids.push(new_id);
        assets += 1;
    }

    // Insert allocation_rules pointing at the freshly-minted asset UUIDs.
    for r in &payload.allocation_rules {
        let Some(&target_id) = new_asset_ids.get(r.target_asset_index) else {
            return Err(ApiError::BadRequest(format!(
                "allocation_rule.target_asset_index {} is out of bounds (assets={})",
                r.target_asset_index,
                new_asset_ids.len(),
            )));
        };
        sqlx::query(
            r#"INSERT INTO allocation_rules (
                   id, installation_id, owner_user_id, target_asset_id, priority,
                   kind, amount, cap_kind, cap_value, enabled, notes
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(target_id)
        .bind(r.priority)
        .bind(&r.kind)
        .bind(r.amount)
        .bind(r.cap_kind.as_deref())
        .bind(r.cap_value)
        .bind(r.enabled)
        .bind(r.notes.as_deref())
        .execute(&mut **tx)
        .await?;
    }

    // Insert liabilities and remember each freshly-minted UUID at the same index as the backup,
    // so snapshot items of kind=liability can be re-linked via their `ledger_index`.
    let mut new_liability_ids: Vec<Uuid> = Vec::with_capacity(payload.liabilities.len());
    let mut liabilities = 0u32;
    for l in &payload.liabilities {
        let cid = resolve_category(cat_map, &l.category_ref.scope, &l.category_ref.name)?;
        // Backups anteriores a 3.4.0 no llevan el campo → NULL (pasivo sin asignar, como los
        // legacy). El INSERT directo bypasea a propósito la obligatoriedad del create.
        let expense_cid = match &l.expense_category_ref {
            Some(r) => Some(resolve_category(cat_map, &r.scope, &r.name)?),
            None => None,
        };
        let new_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO liabilities (
                   id, installation_id, owner_user_id, category_id, expense_category_id, label,
                   type_tag, principal, principal_derived_from_plan, apr_percent, payment_amount,
                   payment_frequency, payment_end_date, notes, sort_index
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
        )
        .bind(new_id)
        .bind(iid)
        .bind(user_id)
        .bind(cid)
        .bind(expense_cid)
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
        new_liability_ids.push(new_id);
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

    // Insert history snapshots LAST: re-linking their items via `ledger_index` needs the
    // freshly-minted asset AND liability UUIDs collected above. See `BackupSnapshotItem`.
    let mut snapshots = 0u32;
    let mut snapshot_items = 0u32;
    for s in &payload.snapshots {
        // Validate the header BEFORE inserting. A hand-edited/corrupted backup could carry a
        // bad `kind` or `source` that trips the `history_snapshots` CHECK constraints
        // (SQLSTATE 23514, unmapped → 500). Reject with 400 so the whole import rolls back.
        let is_liability = match s.kind.as_str() {
            "asset" => false,
            "liability" => true,
            _ => {
                return Err(ApiError::BadRequest(
                    "snapshot_kind_invalid: kind must be 'asset' or 'liability'".into(),
                ))
            }
        };
        if !matches!(s.source.as_str(), "capture" | "backfill") {
            return Err(ApiError::BadRequest(
                "snapshot_source_invalid: source must be 'capture' or 'backfill'".into(),
            ));
        }

        let snapshot_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO history_snapshots (
                   id, installation_id, owner_user_id, kind, snapshot_date, source
               ) VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(snapshot_id)
        .bind(iid)
        .bind(user_id)
        .bind(&s.kind)
        .bind(s.snapshot_date)
        .bind(&s.source)
        .execute(&mut **tx)
        .await?;
        snapshots += 1;

        // Resolved `source_item_id`s already used in THIS snapshot. A repeat would trip the
        // UNIQUE(snapshot_id, source_item_id) constraint (23505) → a misleading 409; reject 400.
        let mut seen_items: HashSet<Uuid> = HashSet::with_capacity(s.items.len());

        for it in &s.items {
            // Validate each item against the `history_snapshot_items` CHECK constraints so a
            // corrupted backup returns a 400 (naming the field) instead of a 500 (23514). Bounds
            // mirror handlers/history.rs and the table CHECKs: label non-empty ≤ 200 chars,
            // value ≥ 0, terms (apr/payment_*) only on liabilities, apr/payment ≥ 0, frequency
            // ∈ {monthly, weekly}.
            let label = it.label.trim();
            if label.is_empty() {
                return Err(ApiError::BadRequest(
                    "snapshot_label_empty: item label must not be empty".into(),
                ));
            }
            if label.chars().count() > 200 {
                return Err(ApiError::BadRequest(
                    "snapshot_label_too_long: item label must be at most 200 characters".into(),
                ));
            }
            if it.value.is_sign_negative() {
                return Err(ApiError::BadRequest(
                    "snapshot_value_negative: item value must be >= 0".into(),
                ));
            }
            let has_terms = it.apr_percent.is_some()
                || it.payment_amount.is_some()
                || it.payment_frequency.is_some();
            if has_terms && !is_liability {
                return Err(ApiError::BadRequest(
                    "snapshot_terms_only_for_liabilities: apr_percent/payment_amount/payment_frequency are only valid for kind 'liability'".into(),
                ));
            }
            if it.apr_percent.map_or(false, |a| a.is_sign_negative()) {
                return Err(ApiError::BadRequest(
                    "snapshot_apr_percent_negative: apr_percent must be >= 0".into(),
                ));
            }
            if it.payment_amount.map_or(false, |p| p.is_sign_negative()) {
                return Err(ApiError::BadRequest(
                    "snapshot_payment_amount_negative: payment_amount must be >= 0".into(),
                ));
            }
            if let Some(freq) = it.payment_frequency.as_deref() {
                if !matches!(freq, "monthly" | "weekly") {
                    return Err(ApiError::BadRequest(
                        "snapshot_payment_frequency_invalid: payment_frequency must be 'monthly' or 'weekly'".into(),
                    ));
                }
            }

            // ledger_index present → point at the fresh UUID of the re-created ledger row
            // (asset or liability, per snapshot kind); absent → keep item_key verbatim so
            // deleted/free-form backfill items stay linked across snapshots.
            let source_item_id = match it.ledger_index {
                Some(i) => {
                    let fresh_ids = if is_liability {
                        &new_liability_ids
                    } else {
                        &new_asset_ids
                    };
                    *fresh_ids.get(i).ok_or_else(|| {
                        ApiError::BadRequest("snapshot_item.ledger_index out of bounds".into())
                    })?
                }
                None => it.item_key,
            };

            if !seen_items.insert(source_item_id) {
                return Err(ApiError::BadRequest(
                    "snapshot_duplicate_item: source_item_id repeated within snapshot".into(),
                ));
            }

            sqlx::query(
                r#"INSERT INTO history_snapshot_items (
                       snapshot_id, source_item_id, label, value,
                       apr_percent, payment_amount, payment_frequency
                   ) VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            )
            .bind(snapshot_id)
            .bind(source_item_id)
            .bind(&it.label)
            .bind(it.value)
            .bind(it.apr_percent)
            .bind(it.payment_amount)
            .bind(it.payment_frequency.as_deref())
            .execute(&mut **tx)
            .await?;
            snapshot_items += 1;
        }
    }

    // Insert CSV import batches (schema_version ≥ 5): `account_asset_index` → fresh asset UUID.
    let mut new_import_ids: Vec<Uuid> = Vec::with_capacity(payload.transaction_imports.len());
    let mut transaction_imports = 0u32;
    for imp in &payload.transaction_imports {
        let account_asset_id = match imp.account_asset_index {
            Some(i) => Some(*new_asset_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("transaction_import.account_asset_index out of bounds".into())
            })?),
            None => None,
        };
        let new_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO transaction_imports
                   (id, installation_id, owner_user_id, source, account_asset_id, original_filename)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(new_id)
        .bind(iid)
        .bind(user_id)
        .bind(&imp.source)
        .bind(account_asset_id)
        .bind(imp.original_filename.as_deref())
        .execute(&mut **tx)
        .await?;
        new_import_ids.push(new_id);
        transaction_imports += 1;
    }

    // Insert recurring-transaction rules (schema_version ≥ 6) BEFORE transactions, so a transaction
    // can resolve its `recurring_rule_index` to a fresh rule UUID. CHECK-backed fields validated in
    // Rust → 400 (not a 500 from the constraint).
    let mut new_recurring_rule_ids: Vec<Uuid> =
        Vec::with_capacity(payload.recurring_transaction_rules.len());
    let mut recurring_transaction_rules = 0u32;
    for r in &payload.recurring_transaction_rules {
        if !matches!(r.kind.as_str(), "expense" | "income" | "savings") {
            return Err(ApiError::BadRequest(
                "recurring_rule_kind_invalid: kind must be expense, income or savings".into(),
            ));
        }
        let concept = r.concept.trim();
        if concept.is_empty() {
            return Err(ApiError::BadRequest(
                "recurring_rule_concept_empty: concept must not be empty".into(),
            ));
        }
        if concept.chars().count() > 500 {
            return Err(ApiError::BadRequest(
                "recurring_rule_concept_too_long: concept must be at most 500 characters".into(),
            ));
        }
        let amount = r.amount.round_dp(4);
        if amount.is_zero() {
            return Err(ApiError::BadRequest(
                "recurring_rule_amount_zero: amount must not be zero".into(),
            ));
        }
        let category_id = match &r.category_ref {
            Some(cr) => Some(resolve_category(cat_map, &cr.scope, &cr.name)?),
            None => None,
        };
        let linked_asset_id = match r.linked_asset_index {
            Some(i) => Some(*new_asset_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("recurring_rule.linked_asset_index out of bounds".into())
            })?),
            None => None,
        };
        let linked_liability_id = match r.linked_liability_index {
            Some(i) => Some(*new_liability_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("recurring_rule.linked_liability_index out of bounds".into())
            })?),
            None => None,
        };
        let new_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO recurring_transaction_rules
                   (id, installation_id, owner_user_id, concept, amount, kind, category_id,
                    linked_asset_id, linked_liability_id, notes,
                    last_materialized_month)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(new_id)
        .bind(iid)
        .bind(user_id)
        .bind(concept)
        .bind(amount)
        .bind(&r.kind)
        .bind(category_id)
        .bind(linked_asset_id)
        .bind(linked_liability_id)
        .bind(r.notes.as_deref())
        .bind(r.last_materialized_month)
        .execute(&mut **tx)
        .await?;
        new_recurring_rule_ids.push(new_id);
        recurring_transaction_rules += 1;
    }

    // Insert transactions: all refs resolved to fresh UUIDs, fingerprint recomputed (never
    // exported), CHECK-backed fields validated in Rust → 400 (not a 500 from the constraint).
    let mut transactions = 0u32;
    for t in &payload.transactions {
        if t.currency != "EUR" {
            return Err(ApiError::BadRequest(
                "transaction_currency_invalid: currency must be 'EUR'".into(),
            ));
        }
        if let Some(k) = t.kind.as_deref() {
            if !matches!(k, "expense" | "income" | "savings") {
                return Err(ApiError::BadRequest(
                    "transaction_kind_invalid: kind must be expense, income or savings".into(),
                ));
            }
        }
        let concept = t.concept.trim();
        if concept.is_empty() {
            return Err(ApiError::BadRequest(
                "transaction_concept_empty: concept must not be empty".into(),
            ));
        }
        if concept.chars().count() > 500 {
            return Err(ApiError::BadRequest(
                "transaction_concept_too_long: concept must be at most 500 characters".into(),
            ));
        }
        let import_id_ref = match t.import_index {
            Some(i) => Some(*new_import_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("transaction.import_index out of bounds".into())
            })?),
            None => None,
        };
        let category_id = match &t.category_ref {
            Some(cr) => Some(resolve_category(cat_map, &cr.scope, &cr.name)?),
            None => None,
        };
        let linked_asset_id = match t.linked_asset_index {
            Some(i) => Some(*new_asset_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("transaction.linked_asset_index out of bounds".into())
            })?),
            None => None,
        };
        let linked_liability_id = match t.linked_liability_index {
            Some(i) => Some(*new_liability_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("transaction.linked_liability_index out of bounds".into())
            })?),
            None => None,
        };
        let recurring_rule_id = match t.recurring_rule_index {
            Some(i) => Some(*new_recurring_rule_ids.get(i).ok_or_else(|| {
                ApiError::BadRequest("transaction.recurring_rule_index out of bounds".into())
            })?),
            None => None,
        };
        let amount = t.amount.round_dp(4);
        let fingerprint =
            crate::handlers::transactions::schema::compute_fingerprint(&t.source, t.op_date, amount, concept);
        sqlx::query(
            r#"INSERT INTO transactions
                   (id, installation_id, owner_user_id, import_id, source, op_date, value_date,
                    concept, amount, currency, kind, category_id, fingerprint, fingerprint_ordinal,
                    linked_asset_id, linked_liability_id, notes, recurring_rule_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(import_id_ref)
        .bind(&t.source)
        .bind(t.op_date)
        .bind(t.value_date)
        .bind(concept)
        .bind(amount)
        .bind(&t.currency)
        .bind(t.kind.as_deref())
        .bind(category_id)
        .bind(&fingerprint)
        .bind(t.fingerprint_ordinal)
        .bind(linked_asset_id)
        .bind(linked_liability_id)
        .bind(t.notes.as_deref())
        .bind(recurring_rule_id)
        .execute(&mut **tx)
        .await?;
        transactions += 1;
    }

    // Insert categorization rules.
    let mut categorization_rules = 0u32;
    for r in &payload.categorization_rules {
        if !matches!(r.match_kind.as_str(), "substring" | "prefix" | "exact") {
            return Err(ApiError::BadRequest(
                "rule_match_kind_invalid: match_kind must be substring, prefix or exact".into(),
            ));
        }
        if let Some(k) = r.assign_kind.as_deref() {
            if !matches!(k, "expense" | "income" | "savings") {
                return Err(ApiError::BadRequest(
                    "rule_assign_kind_invalid: assign_kind must be expense, income or savings".into(),
                ));
            }
        }
        let pattern = r.pattern.trim();
        if pattern.is_empty() {
            return Err(ApiError::BadRequest(
                "rule_pattern_empty: pattern must not be empty".into(),
            ));
        }
        if pattern.chars().count() > 500 {
            return Err(ApiError::BadRequest(
                "rule_pattern_too_long: pattern must be at most 500 characters".into(),
            ));
        }
        let assign_category_id = match &r.assign_category_ref {
            Some(cr) => Some(resolve_category(cat_map, &cr.scope, &cr.name)?),
            None => None,
        };
        let source = r.source.as_deref().map(str::trim).filter(|s| !s.is_empty());
        sqlx::query(
            r#"INSERT INTO categorization_rules
                   (id, installation_id, owner_user_id, match_kind, pattern, source,
                    assign_kind, assign_category_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(iid)
        .bind(user_id)
        .bind(&r.match_kind)
        .bind(pattern)
        .bind(source)
        .bind(r.assign_kind.as_deref())
        .bind(assign_category_id)
        .execute(&mut **tx)
        .await?;
        categorization_rules += 1;
    }

    Ok(ImportCounts {
        assets,
        liabilities,
        budget_entries,
        planning_flows,
        categories_in_backup: payload.categories_used.len() as u32,
        categories_already_present: 0,
        categories_to_create: 0,
        snapshots,
        snapshot_items,
        transaction_imports,
        transactions,
        categorization_rules,
        recurring_transaction_rules,
    })
}

// Keep Decimal import alive even if its usage is conditional in some branches above.
#[allow(dead_code)]
fn _decimal_marker(_: Decimal) {}
