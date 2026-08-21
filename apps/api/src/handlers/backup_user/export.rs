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

use std::collections::HashMap;

use super::crypto::{encrypt_payload, frame_file};
use super::schema::{
    BackupAllocationRule, BackupAsset, BackupBudgetEntry, BackupCategorizationRule, BackupCategory,
    BackupLiability, BackupPayload, BackupPlanningFlow, BackupRecurringRule, BackupSnapshot,
    BackupSnapshotItem, BackupTransaction, BackupTransactionImport, BackupTransferMatchRejection,
    BackupUser, CategoryRef, InstallationSnapshotInformative, UiPreferences,
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
) -> Result<BackupPayload, ApiError> {
    let (assets, asset_id_to_index) = fetch_assets(pool, iid, user_id).await?;
    let allocation_rules = fetch_allocation_rules(pool, iid, user_id, &asset_id_to_index).await?;
    let (liabilities, liability_id_to_index) = fetch_liabilities(pool, iid, user_id).await?;
    let budget_entries = fetch_budget_entries(pool, iid, user_id).await?;
    let planning_flows = fetch_planning_flows(pool, iid, user_id).await?;
    let categories_used = fetch_categories_used(pool, iid, user_id).await?;
    let snapshot = fetch_installation_snapshot(pool, iid).await?;
    let snapshots = fetch_snapshots(
        pool,
        iid,
        user_id,
        &asset_id_to_index,
        &liability_id_to_index,
    )
    .await?;
    let (transaction_imports, import_id_to_index) =
        fetch_transaction_imports(pool, iid, user_id, &asset_id_to_index).await?;
    // Recurring rules are fetched BEFORE transactions so a transaction can carry a
    // `recurring_rule_index` into the same-ordering vec.
    let (recurring_transaction_rules, recurring_rule_id_to_index) = fetch_recurring_rules(
        pool,
        iid,
        user_id,
        &asset_id_to_index,
        &liability_id_to_index,
    )
    .await?;
    let (transactions, txn_id_to_index) = fetch_transactions(
        pool,
        iid,
        user_id,
        &import_id_to_index,
        &asset_id_to_index,
        &liability_id_to_index,
        &recurring_rule_id_to_index,
    )
    .await?;
    let categorization_rules = fetch_categorization_rules(pool, iid, user_id).await?;
    let transfer_match_rejections =
        fetch_transfer_match_rejections(pool, iid, user_id, &txn_id_to_index).await?;

    Ok(BackupPayload {
        user,
        categories_used,
        assets,
        allocation_rules,
        liabilities,
        budget_entries,
        planning_flows,
        ui_preferences,
        installation_snapshot_informative: snapshot,
        snapshots,
        transaction_imports,
        transactions,
        categorization_rules,
        recurring_transaction_rules,
        transfer_match_rejections,
    })
}

/// Recurring-transaction rules (schema_version ≥ 6). Returns the rules plus a `rule_id → index`
/// map so transactions can carry a `recurring_rule_index`. Category is denormalized to
/// `(scope, name)`; asset/liability links are resolved to indices in this payload's vecs.
async fn fetch_recurring_rules(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    asset_id_to_index: &HashMap<Uuid, usize>,
    liability_id_to_index: &HashMap<Uuid, usize>,
) -> Result<(Vec<BackupRecurringRule>, HashMap<Uuid, usize>), ApiError> {
    type Row = (
        Uuid,           // id
        String,         // concept
        Decimal,        // amount
        String,         // kind
        Option<String>, // cat_scope
        Option<String>, // cat_name
        Option<Uuid>,   // linked_asset_id
        Option<Uuid>,   // linked_liability_id
        Option<String>, // notes
        NaiveDate,      // origin_month
    );
    let rows: Vec<Row> = sqlx::query_as(
        r#"SELECT r.id, r.concept, r.amount, r.kind, c.scope AS cat_scope, c.name AS cat_name,
                  r.linked_asset_id, r.linked_liability_id, r.notes,
                  r.origin_month
           FROM recurring_transaction_rules r
           LEFT JOIN categories c ON c.id = r.category_id
           WHERE r.installation_id = $1 AND r.owner_user_id = $2
           ORDER BY r.created_at ASC, r.id ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut id_to_index = HashMap::with_capacity(rows.len());
    let rules = rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            id_to_index.insert(r.0, i);
            let category_ref = match (r.4, r.5) {
                (Some(scope), Some(name)) => Some(CategoryRef { scope, name }),
                _ => None,
            };
            BackupRecurringRule {
                concept: r.1,
                amount: r.2,
                kind: r.3,
                category_ref,
                linked_asset_index: r.6.and_then(|a| asset_id_to_index.get(&a).copied()),
                linked_liability_index: r.7.and_then(|l| liability_id_to_index.get(&l).copied()),
                notes: r.8,
                origin_month: r.9,
            }
        })
        .collect();
    Ok((rules, id_to_index))
}

/// CSV import batches (schema_version ≥ 5). Returns the batches plus an `import_id → index` map so
/// transactions can carry an `import_index`.
async fn fetch_transaction_imports(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    asset_id_to_index: &HashMap<Uuid, usize>,
) -> Result<(Vec<BackupTransactionImport>, HashMap<Uuid, usize>), ApiError> {
    let rows: Vec<(Uuid, String, Option<Uuid>, Option<String>)> = sqlx::query_as(
        r#"SELECT id, source, account_asset_id, original_filename
           FROM transaction_imports
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut id_to_index = HashMap::with_capacity(rows.len());
    let imports = rows
        .into_iter()
        .enumerate()
        .map(|(i, (id, source, account_asset_id, original_filename))| {
            id_to_index.insert(id, i);
            BackupTransactionImport {
                source,
                account_asset_index: account_asset_id.and_then(|a| asset_id_to_index.get(&a).copied()),
                original_filename,
            }
        })
        .collect();
    Ok((imports, id_to_index))
}

/// Dated transactions (schema_version ≥ 5). The fingerprint is NOT exported (recomputed on
/// import); `fingerprint_ordinal` is. Category is denormalized to `(scope, name)`.
///
/// Two passes since v8: (1) build the `transaction_id → index` map from the stable ordering,
/// (2) resolve `transfer_counterpart_id` into `transfer_counterpart_index` against that map.
/// The map is also returned so `transfer_match_rejections` can serialize against it.
#[allow(clippy::too_many_arguments)]
async fn fetch_transactions(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    import_id_to_index: &HashMap<Uuid, usize>,
    asset_id_to_index: &HashMap<Uuid, usize>,
    liability_id_to_index: &HashMap<Uuid, usize>,
    recurring_rule_id_to_index: &HashMap<Uuid, usize>,
) -> Result<(Vec<BackupTransaction>, HashMap<Uuid, usize>), ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        import_id: Option<Uuid>,
        source: String,
        op_date: NaiveDate,
        value_date: Option<NaiveDate>,
        concept: String,
        amount: Decimal,
        currency: String,
        kind: Option<String>,
        cat_scope: Option<String>,
        cat_name: Option<String>,
        fingerprint_ordinal: i32,
        linked_asset_id: Option<Uuid>,
        linked_liability_id: Option<Uuid>,
        notes: Option<String>,
        recurring_rule_id: Option<Uuid>,
        transfer_counterpart_id: Option<Uuid>,
        transfer_reconciled_at: Option<chrono::DateTime<chrono::Utc>>,
        transfer_reconciled_source: Option<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        r#"SELECT t.id, t.import_id, t.source, t.op_date, t.value_date, t.concept, t.amount,
                  t.currency, t.kind, c.scope AS cat_scope, c.name AS cat_name,
                  t.fingerprint_ordinal, t.linked_asset_id, t.linked_liability_id, t.notes,
                  t.recurring_rule_id, t.transfer_counterpart_id, t.transfer_reconciled_at,
                  t.transfer_reconciled_source
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
           WHERE t.installation_id = $1 AND t.owner_user_id = $2
           ORDER BY t.op_date ASC, t.created_at ASC, t.id ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    // Pasada 1: mapa id → índice (mismo orden estable que el vec resultante).
    let txn_id_to_index: HashMap<Uuid, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id, i))
        .collect();

    // Pasada 2: DTOs con el counterpart resuelto por índice.
    let transactions = rows
        .into_iter()
        .map(|r| {
            let category_ref = match (r.cat_scope, r.cat_name) {
                (Some(scope), Some(name)) => Some(CategoryRef { scope, name }),
                _ => None,
            };
            // Solo se exporta un par si la contrapartida está en el propio export (siempre, salvo
            // estados asimétricos imposibles por construcción — defensivo).
            let transfer_counterpart_index = r
                .transfer_counterpart_id
                .and_then(|id| txn_id_to_index.get(&id).copied());
            BackupTransaction {
                import_index: r.import_id.and_then(|id| import_id_to_index.get(&id).copied()),
                source: r.source,
                op_date: r.op_date,
                value_date: r.value_date,
                concept: r.concept,
                amount: r.amount,
                currency: r.currency,
                kind: r.kind,
                category_ref,
                fingerprint_ordinal: r.fingerprint_ordinal,
                linked_asset_index: r
                    .linked_asset_id
                    .and_then(|a| asset_id_to_index.get(&a).copied()),
                linked_liability_index: r
                    .linked_liability_id
                    .and_then(|l| liability_id_to_index.get(&l).copied()),
                notes: r.notes,
                recurring_rule_index: r
                    .recurring_rule_id
                    .and_then(|id| recurring_rule_id_to_index.get(&id).copied()),
                transfer_reconciled_at: transfer_counterpart_index
                    .and(r.transfer_reconciled_at),
                transfer_reconciled_source: transfer_counterpart_index
                    .is_some()
                    .then_some(r.transfer_reconciled_source)
                    .flatten(),
                transfer_counterpart_index,
            }
        })
        .collect();
    Ok((transactions, txn_id_to_index))
}

/// Pares rechazados por el usuario al desconciliar (schema_version ≥ 8), por índices del vec
/// `transactions`. Un rechazo cuya pata no esté en el mapa (imposible por FK, defensivo) se omite.
async fn fetch_transfer_match_rejections(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    txn_id_to_index: &HashMap<Uuid, usize>,
) -> Result<Vec<BackupTransferMatchRejection>, ApiError> {
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT transaction_a_id, transaction_b_id
           FROM transfer_match_rejections
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(a, b)| {
            match (txn_id_to_index.get(&a), txn_id_to_index.get(&b)) {
                (Some(&ai), Some(&bi)) => Some(BackupTransferMatchRejection {
                    transaction_a_index: ai,
                    transaction_b_index: bi,
                }),
                _ => None,
            }
        })
        .collect())
}

/// Categorization rules (schema_version ≥ 5). `assign_category_id` is denormalized to `(scope, name)`.
async fn fetch_categorization_rules(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<BackupCategorizationRule>, ApiError> {
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            r#"SELECT r.match_kind, r.pattern, r.source, r.assign_kind,
                      c.scope AS cat_scope, c.name AS cat_name
               FROM categorization_rules r
               LEFT JOIN categories c ON c.id = r.assign_category_id
               WHERE r.installation_id = $1 AND r.owner_user_id = $2
               ORDER BY r.created_at ASC, r.id ASC"#,
        )
        .bind(iid)
        .bind(user_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(match_kind, pattern, source, assign_kind, cat_scope, cat_name)| {
            let assign_category_ref = match (cat_scope, cat_name) {
                (Some(scope), Some(name)) => Some(CategoryRef { scope, name }),
                _ => None,
            };
            BackupCategorizationRule {
                match_kind,
                pattern,
                source,
                assign_kind,
                assign_category_ref,
            }
        })
        .collect())
}

/// Returns the exportable assets plus a map `(asset_id) → index in the vec` so we can
/// serialize `allocation_rules.target_asset_index` against the same ordering.
async fn fetch_assets(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<(Vec<BackupAsset>, std::collections::HashMap<Uuid, usize>), ApiError> {
    let rows: Vec<(
        Uuid,
        String,
        String,
        String,
        Decimal,
        Option<Decimal>,
        bool,
        Option<Decimal>,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        r#"SELECT a.id, c.scope, c.name AS cat_name, a.name, a.current_value, a.purchase_price,
                  a.is_liquid, a.expected_annual_return_percent,
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

    let mut id_to_index = std::collections::HashMap::with_capacity(rows.len());
    let assets: Vec<BackupAsset> = rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            id_to_index.insert(r.0, i);
            BackupAsset {
                category_ref: CategoryRef { scope: r.1, name: r.2 },
                name: r.3,
                current_value: r.4,
                purchase_price: r.5,
                is_liquid: r.6,
                expected_annual_return_percent: r.7,
                notes: r.8,
                sort_index: r.9,
            }
        })
        .collect();
    Ok((assets, id_to_index))
}

async fn fetch_allocation_rules(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    asset_id_to_index: &std::collections::HashMap<Uuid, usize>,
) -> Result<Vec<BackupAllocationRule>, ApiError> {
    let rows: Vec<(
        Uuid,
        i32,
        String,
        Option<Decimal>,
        Option<String>,
        Option<Decimal>,
        bool,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT target_asset_id, priority, kind, amount, cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY priority ASC, id ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let idx = *asset_id_to_index.get(&r.0)?;
            Some(BackupAllocationRule {
                target_asset_index: idx,
                priority: r.1,
                kind: r.2,
                amount: r.3,
                cap_kind: r.4,
                cap_value: r.5,
                enabled: r.6,
                notes: r.7,
            })
        })
        .collect())
}

/// Returns the exportable liabilities plus a map `(liability_id) → index in the vec`, mirroring
/// [`fetch_assets`], so snapshot items of `kind == "liability"` can carry a `ledger_index`.
async fn fetch_liabilities(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<(Vec<BackupLiability>, HashMap<Uuid, usize>), ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        Uuid,
        String,
        String,
        Option<String>,
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
        r#"SELECT l.id, c.scope, c.name AS cat_name, ec.name AS expense_cat_name, l.label,
                  l.type_tag, l.principal,
                  l.principal_derived_from_plan, l.apr_percent, l.payment_amount,
                  l.payment_frequency, l.payment_end_date, l.notes, l.sort_index
           FROM liabilities l
           JOIN categories c ON c.id = l.category_id
           LEFT JOIN categories ec ON ec.id = l.expense_category_id
           WHERE l.installation_id = $1 AND l.owner_user_id = $2
           ORDER BY l.sort_index ASC, l.label ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut id_to_index = HashMap::with_capacity(rows.len());
    let liabilities: Vec<BackupLiability> = rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            id_to_index.insert(r.0, i);
            BackupLiability {
                category_ref: CategoryRef { scope: r.1, name: r.2 },
                // La categoría de gasto siempre tiene scope 'expense' (validación de la API).
                expense_category_ref: r.3.map(|name| CategoryRef {
                    scope: "expense".into(),
                    name,
                }),
                label: r.4,
                type_tag: r.5,
                principal: r.6,
                principal_derived_from_plan: r.7,
                apr_percent: r.8,
                payment_amount: r.9,
                payment_frequency: r.10,
                payment_end_date: r.11,
                notes: r.12,
                sort_index: r.13,
            }
        })
        .collect();
    Ok((liabilities, id_to_index))
}

/// Serializes this user's history snapshots for the backup. For each item, `ledger_index` is
/// the position of its `source_item_id` in the payload's `assets` (kind=asset) or `liabilities`
/// (kind=liability) vec — `None` when the source row no longer exists (deleted / free-form
/// backfill) — and `item_key` is the original `source_item_id`. See [`BackupSnapshotItem`].
async fn fetch_snapshots(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    asset_id_to_index: &HashMap<Uuid, usize>,
    liability_id_to_index: &HashMap<Uuid, usize>,
) -> Result<Vec<BackupSnapshot>, ApiError> {
    let headers: Vec<(Uuid, String, NaiveDate, String)> = sqlx::query_as(
        r#"SELECT id, kind, snapshot_date, source
           FROM history_snapshots
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY kind ASC, snapshot_date ASC"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    if headers.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<Uuid> = headers.iter().map(|h| h.0).collect();
    let item_rows: Vec<(
        Uuid,
        Uuid,
        String,
        Decimal,
        Option<Decimal>,
        Option<Decimal>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT snapshot_id, source_item_id, label, value,
                  apr_percent, payment_amount, payment_frequency
           FROM history_snapshot_items
           WHERE snapshot_id = ANY($1)
           ORDER BY label ASC"#,
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    // Group items by parent, preserving the label-ASC order within each snapshot.
    let mut by_parent: HashMap<
        Uuid,
        Vec<(Uuid, String, Decimal, Option<Decimal>, Option<Decimal>, Option<String>)>,
    > = HashMap::new();
    for r in item_rows {
        by_parent
            .entry(r.0)
            .or_default()
            .push((r.1, r.2, r.3, r.4, r.5, r.6));
    }

    let mut out = Vec::with_capacity(headers.len());
    for (sid, kind, snapshot_date, source) in headers {
        let rows = by_parent.remove(&sid).unwrap_or_default();
        let idx_map = if kind == "asset" {
            asset_id_to_index
        } else {
            liability_id_to_index
        };
        let items = rows
            .into_iter()
            .map(|(source_item_id, label, value, apr, pay, freq)| BackupSnapshotItem {
                ledger_index: idx_map.get(&source_item_id).copied(),
                item_key: source_item_id,
                label,
                value,
                apr_percent: apr,
                payment_amount: pay,
                payment_frequency: freq,
            })
            .collect();
        out.push(BackupSnapshot { kind, snapshot_date, source, items });
    }
    Ok(out)
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
                OR EXISTS (SELECT 1 FROM liabilities le WHERE le.expense_category_id = c.id AND le.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM budget_entries b WHERE b.category_id = c.id AND b.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM planning_flows p WHERE p.category_id = c.id AND p.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM transactions t WHERE t.category_id = c.id AND t.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM categorization_rules r WHERE r.assign_category_id = c.id AND r.owner_user_id = $2)
                OR EXISTS (SELECT 1 FROM recurring_transaction_rules rr WHERE rr.category_id = c.id AND rr.owner_user_id = $2)
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
        Decimal,
        String,
        Option<SqlxJson<FireSettings>>,
    ) = sqlx::query_as(
        r#"SELECT base_currency, calendar_tz,
                  annual_inflation_assumption_percent, show_age_mode, fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool)
    .await?;

    Ok(InstallationSnapshotInformative {
        base_currency: row.0,
        calendar_tz: row.1,
        annual_inflation_assumption_percent: Some(row.2),
        show_age_mode: row.3,
        fire_settings: resolve_fire_settings(row.4.map(|j| j.0)),
    })
}
