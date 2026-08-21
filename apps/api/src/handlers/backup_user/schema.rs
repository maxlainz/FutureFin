//! Versioned DTO + migration layer for `.ffbackup` payloads.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::handlers::installation::FireSettings;

pub const CURRENT_SCHEMA_VERSION: u32 = 9;
pub const SUPPORTED_FORMAT_VERSION: u8 = 1;
pub const MAGIC: &[u8; 4] = b"FFBK";

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupUser {
    pub username: String,
    pub birth_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupCategory {
    pub scope: String,
    pub name: String,
    pub sort_index: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CategoryRef {
    pub scope: String,
    pub name: String,
}

/// v1 asset record (legacy backup files). Kept verbatim for parsing old `.ffbackup`s.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupAssetV1 {
    pub category_ref: CategoryRef,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub current_value: Decimal,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub expected_annual_return_percent: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub monthly_contribution_fixed: Decimal,
    pub contribution_frequency: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub contribution_remainder_weight: Decimal,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

/// v2 asset record: v1 fields + optional per-asset contribution cap.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupAssetV2 {
    pub category_ref: CategoryRef,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub current_value: Decimal,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub expected_annual_return_percent: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub monthly_contribution_fixed: Decimal,
    pub contribution_frequency: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub contribution_remainder_weight: Decimal,
    #[serde(default)]
    pub contribution_cap_kind: Option<String>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub contribution_cap_value: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

/// v3 asset record: per-asset contribution fields moved out to `allocation_rules`.
/// The asset itself now only carries identity, valuation, and metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupAssetV3 {
    pub category_ref: CategoryRef,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub current_value: Decimal,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub expected_annual_return_percent: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

/// v3 allocation rule: cascade entry that routes part of the monthly surplus into one asset.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupAllocationRule {
    /// Position of the target asset in the `assets` vec (0-based). The importer resolves this
    /// to the freshly-minted asset UUID after inserting assets in order.
    pub target_asset_index: usize,
    pub priority: i32,
    /// `fixed` | `percent` | `remainder`
    pub kind: String,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub amount: Option<Decimal>,
    /// `amount` | `months_expense` | `income_multiple` | None
    #[serde(default)]
    pub cap_kind: Option<String>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub cap_value: Option<Decimal>,
    pub enabled: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Alias for the current-version asset DTO. Always points to the latest variant.
pub type BackupAsset = BackupAssetV3;

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupLiability {
    pub category_ref: CategoryRef,
    /// Categoría de GASTO de la cuota (3.4.0). Aditivo con `default`: los backups anteriores no
    /// llevan el campo → `None` → el pasivo importa sin asignar (SIN bump de schema_version,
    /// mismo patrón que `savings_source`).
    #[serde(default)]
    pub expense_category_ref: Option<CategoryRef>,
    pub label: String,
    #[serde(default)]
    pub type_tag: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub principal: Decimal,
    pub principal_derived_from_plan: bool,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub apr_percent: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub payment_amount: Option<Decimal>,
    #[serde(default)]
    pub payment_frequency: Option<String>,
    #[serde(default)]
    pub payment_end_date: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupBudgetEntry {
    pub category_ref: CategoryRef,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub persists_after_retirement: bool,
    pub ends_at_retirement: bool,
    #[serde(default)]
    pub expense_end_date: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPlanningFlow {
    pub category_ref: CategoryRef,
    pub title: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub expected_amount: Decimal,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    pub show_in_chart: bool,
    #[serde(default)]
    pub notes: Option<String>,
    pub sort_index: i32,
}

/// A history snapshot exported inside a `.ffbackup` (schema_version ≥ 4).
///
/// `kind` = `asset` | `liability`; `source` = `capture` | `backfill`. One header per
/// (user, kind, civil day); items carry a per-row value (and, for liabilities, loan terms).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupSnapshot {
    /// `asset` | `liability`.
    pub kind: String,
    pub snapshot_date: NaiveDate,
    /// `capture` | `backfill`.
    pub source: String,
    pub items: Vec<BackupSnapshotItem>,
}

/// A single item inside an exported snapshot.
///
/// ## Re-link mechanism (reconciled — supersedes any per-agent detail)
/// - `ledger_index` = position of the referenced row in **this payload's** `assets` vec
///   (when `kind == "asset"`) or `liabilities` vec (when `kind == "liability"`), and is
///   present **only** when the source ledger row still existed at export time. `None`
///   otherwise (the row was deleted before export, or the item is a free-form backfill).
/// - `item_key` = the **original** `source_item_id`, **always** present.
///
/// ## On import
/// - `ledger_index: Some(i)` → the stored `source_item_id` becomes the **fresh UUID** of
///   the row re-created at index `i` (this preserves both the cross-snapshot linkage and the
///   join-to-today at month 0). Out-of-bounds `i` → `400 BadRequest` and the whole import
///   rolls back.
/// - `ledger_index: None` → `item_key` is kept **verbatim** (deleted rows and free-form
///   backfill items stay linked to each other across snapshots).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupSnapshotItem {
    #[serde(default)]
    pub ledger_index: Option<usize>,
    pub item_key: Uuid,
    pub label: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub apr_percent: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub payment_amount: Option<Decimal>,
    #[serde(default)]
    pub payment_frequency: Option<String>,
}

/// A CSV import batch exported inside a `.ffbackup` (schema_version ≥ 5).
///
/// `account_asset_index` = position of the origin account asset in this payload's `assets` vec,
/// when it still existed at export (`None` otherwise — the FK is `ON DELETE SET NULL`).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupTransactionImport {
    pub source: String,
    #[serde(default)]
    pub account_asset_index: Option<usize>,
    #[serde(default)]
    pub original_filename: Option<String>,
}

/// A dated transaction exported inside a `.ffbackup` (schema_version ≥ 5).
///
/// Refs are by index into this payload's vecs: `import_index` → `transaction_imports`
/// (`None` = manual/cash), `linked_asset_index` → `assets`, `linked_liability_index` →
/// `liabilities`. The **fingerprint is NOT exported** (recomputed on import from
/// source·op_date·amount·concept); only `fingerprint_ordinal` is carried to preserve the
/// dedup ordinal of repeated occurrences.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupTransaction {
    #[serde(default)]
    pub import_index: Option<usize>,
    pub source: String,
    pub op_date: NaiveDate,
    #[serde(default)]
    pub value_date: Option<NaiveDate>,
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub currency: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category_ref: Option<CategoryRef>,
    pub fingerprint_ordinal: i32,
    #[serde(default)]
    pub linked_asset_index: Option<usize>,
    #[serde(default)]
    pub linked_liability_index: Option<usize>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Regla recurrente de la que procede (índice en `recurring_transaction_rules`); `None` para
    /// movimientos sueltos (schema_version ≥ 6).
    #[serde(default)]
    pub recurring_rule_index: Option<usize>,
    /// Índice (en `transactions` de este payload) de la contrapartida de la conciliación de
    /// transferencia; `None` = movimiento sin conciliar. SIMÉTRICO: ambas patas se apuntan
    /// (schema_version ≥ 8). Los payloads anteriores deserializan con `None`.
    #[serde(default)]
    pub transfer_counterpart_index: Option<usize>,
    #[serde(default)]
    pub transfer_reconciled_at: Option<DateTime<Utc>>,
    /// `auto` | `manual` (schema_version ≥ 8).
    #[serde(default)]
    pub transfer_reconciled_source: Option<String>,
}

/// Un par RECHAZADO por el usuario al desconciliar a mano (schema_version ≥ 8), por índices en
/// `transactions` de este payload. Sin exportarlos, un restore resucitaría todos los rechazos en
/// el primer pase de auto-conciliación post-import.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupTransferMatchRejection {
    pub transaction_a_index: usize,
    pub transaction_b_index: usize,
}

/// A learned/user categorization rule exported inside a `.ffbackup` (schema_version ≥ 5).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupCategorizationRule {
    pub match_kind: String,
    pub pattern: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub assign_kind: Option<String>,
    #[serde(default)]
    pub assign_category_ref: Option<CategoryRef>,
}

/// A recurring-transaction rule as exported in schema_version 6 — rules still carried a
/// configurable `day_of_month` (dropped in v7: rules became month-resolution only).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupRecurringRuleV6 {
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub kind: String,
    #[serde(default)]
    pub category_ref: Option<CategoryRef>,
    pub day_of_month: i32,
    #[serde(default)]
    pub linked_asset_index: Option<usize>,
    #[serde(default)]
    pub linked_liability_index: Option<usize>,
    #[serde(default)]
    pub notes: Option<String>,
    pub last_materialized_month: NaiveDate,
}

/// A recurring-transaction rule as exported in schema_version 7 and 8 — rules still carried the
/// monotonic idempotency cursor `last_materialized_month`, retired in v9 (see 3.10.0: instances
/// converge on the months that actually have data, so the anchor is the ORIGIN month instead).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupRecurringRuleV8 {
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub kind: String,
    #[serde(default)]
    pub category_ref: Option<CategoryRef>,
    #[serde(default)]
    pub linked_asset_index: Option<usize>,
    #[serde(default)]
    pub linked_liability_index: Option<usize>,
    #[serde(default)]
    pub notes: Option<String>,
    pub last_materialized_month: NaiveDate,
}

/// A recurring-transaction rule exported inside a `.ffbackup` (schema_version ≥ 9).
///
/// Category is denormalized to `(scope, name)` (like transactions); `linked_asset_index` /
/// `linked_liability_index` are positions into this payload's `assets` / `liabilities` vecs
/// (`None` when the FK was already SET NULL at export).
///
/// `origin_month` (first day of a month) is the rule's ANCHOR, not a cursor: convergence
/// materializes from that month onward into every ACTIVE month, and its prune never removes the
/// instance living in it. It replaces v8's `last_materialized_month`, which was monotonic — the
/// opposite of what convergence needs (a CSV for an old month imported today must still
/// materialize it).
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupRecurringRule {
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub kind: String,
    #[serde(default)]
    pub category_ref: Option<CategoryRef>,
    #[serde(default)]
    pub linked_asset_index: Option<usize>,
    #[serde(default)]
    pub linked_liability_index: Option<usize>,
    #[serde(default)]
    pub notes: Option<String>,
    pub origin_month: NaiveDate,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, ToSchema)]
pub struct UiPreferences {
    #[serde(default)]
    pub person_scope: Option<String>,
    #[serde(default)]
    pub projection_focus: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationSnapshotInformative {
    pub base_currency: String,
    pub calendar_tz: String,
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub annual_inflation_assumption_percent: Option<Decimal>,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV1 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV1>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV2 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV2>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV3 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    /// Empty vec when migrating from older versions (the legacy contribution fields
    /// are dropped on migration — see [`payload_v2_to_v3`]).
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV4 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    /// History snapshots (schema_version ≥ 4). Empty when migrating from an older backup.
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV5 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
    /// CSV import batches (schema_version ≥ 5). Empty when migrating from an older backup.
    #[serde(default)]
    pub transaction_imports: Vec<BackupTransactionImport>,
    /// Dated transactions (schema_version ≥ 5). Empty when migrating from an older backup.
    #[serde(default)]
    pub transactions: Vec<BackupTransaction>,
    /// Categorization rules (schema_version ≥ 5). Empty when migrating from an older backup.
    #[serde(default)]
    pub categorization_rules: Vec<BackupCategorizationRule>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV6 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
    #[serde(default)]
    pub transaction_imports: Vec<BackupTransactionImport>,
    #[serde(default)]
    pub transactions: Vec<BackupTransaction>,
    #[serde(default)]
    pub categorization_rules: Vec<BackupCategorizationRule>,
    /// Recurring-transaction rules (schema_version ≥ 6). Empty when migrating from an older backup.
    #[serde(default)]
    pub recurring_transaction_rules: Vec<BackupRecurringRuleV6>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV7 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
    #[serde(default)]
    pub transaction_imports: Vec<BackupTransactionImport>,
    #[serde(default)]
    pub transactions: Vec<BackupTransaction>,
    #[serde(default)]
    pub categorization_rules: Vec<BackupCategorizationRule>,
    /// Recurring-transaction rules (month-resolution since v7: `day_of_month` was dropped).
    #[serde(default)]
    pub recurring_transaction_rules: Vec<BackupRecurringRuleV8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV8 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
    #[serde(default)]
    pub transaction_imports: Vec<BackupTransactionImport>,
    #[serde(default)]
    pub transactions: Vec<BackupTransaction>,
    #[serde(default)]
    pub categorization_rules: Vec<BackupCategorizationRule>,
    #[serde(default)]
    pub recurring_transaction_rules: Vec<BackupRecurringRuleV8>,
    /// Pares desconciliados A MANO (schema_version ≥ 8): la memoria anti-resurrección del
    /// auto-matcher. Empty when migrating from an older backup.
    #[serde(default)]
    pub transfer_match_rejections: Vec<BackupTransferMatchRejection>,
}

/// v9 (3.10.0): las reglas recurrentes cambian el cursor `last_materialized_month` por el ancla
/// `origin_month`. Es un cambio NO aditivo, como el v6→v7 que quitó `day_of_month`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV9 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAssetV3>,
    #[serde(default)]
    pub allocation_rules: Vec<BackupAllocationRule>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
    #[serde(default)]
    pub snapshots: Vec<BackupSnapshot>,
    #[serde(default)]
    pub transaction_imports: Vec<BackupTransactionImport>,
    #[serde(default)]
    pub transactions: Vec<BackupTransaction>,
    #[serde(default)]
    pub categorization_rules: Vec<BackupCategorizationRule>,
    #[serde(default)]
    pub recurring_transaction_rules: Vec<BackupRecurringRule>,
    #[serde(default)]
    pub transfer_match_rejections: Vec<BackupTransferMatchRejection>,
}

/// Alias for the current-version payload. Export and import code work against this type.
pub type BackupPayload = BackupPayloadV9;

#[derive(Debug)]
pub enum AnyPayload {
    V1(BackupPayloadV1),
    V2(BackupPayloadV2),
    V3(BackupPayloadV3),
    V4(BackupPayloadV4),
    V5(BackupPayloadV5),
    V6(BackupPayloadV6),
    V7(BackupPayloadV7),
    V8(BackupPayloadV8),
    V9(BackupPayloadV9),
}

pub fn parse_payload(schema_version: u32, bytes: &[u8]) -> Result<AnyPayload, String> {
    match schema_version {
        1 => {
            let p: BackupPayloadV1 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v1 malformed: {e}"))?;
            Ok(AnyPayload::V1(p))
        }
        2 => {
            let p: BackupPayloadV2 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v2 malformed: {e}"))?;
            Ok(AnyPayload::V2(p))
        }
        3 => {
            let p: BackupPayloadV3 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v3 malformed: {e}"))?;
            Ok(AnyPayload::V3(p))
        }
        4 => {
            let p: BackupPayloadV4 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v4 malformed: {e}"))?;
            Ok(AnyPayload::V4(p))
        }
        5 => {
            let p: BackupPayloadV5 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v5 malformed: {e}"))?;
            Ok(AnyPayload::V5(p))
        }
        6 => {
            let p: BackupPayloadV6 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v6 malformed: {e}"))?;
            Ok(AnyPayload::V6(p))
        }
        7 => {
            let p: BackupPayloadV7 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v7 malformed: {e}"))?;
            Ok(AnyPayload::V7(p))
        }
        8 => {
            let p: BackupPayloadV8 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v8 malformed: {e}"))?;
            Ok(AnyPayload::V8(p))
        }
        9 => {
            let p: BackupPayloadV9 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v9 malformed: {e}"))?;
            Ok(AnyPayload::V9(p))
        }
        v if v > CURRENT_SCHEMA_VERSION => Err(format!(
            "schema_version {v} is newer than this server supports ({CURRENT_SCHEMA_VERSION}); update FutureFin to import this backup",
        )),
        v => Err(format!("schema_version {v} not supported")),
    }
}

fn asset_v1_to_v2(a: BackupAssetV1) -> BackupAssetV2 {
    BackupAssetV2 {
        category_ref: a.category_ref,
        name: a.name,
        current_value: a.current_value,
        purchase_price: a.purchase_price,
        is_liquid: a.is_liquid,
        expected_annual_return_percent: a.expected_annual_return_percent,
        monthly_contribution_fixed: a.monthly_contribution_fixed,
        contribution_frequency: a.contribution_frequency,
        contribution_remainder_weight: a.contribution_remainder_weight,
        contribution_cap_kind: None,
        contribution_cap_value: None,
        notes: a.notes,
        sort_index: a.sort_index,
    }
}

fn payload_v1_to_v2(p: BackupPayloadV1) -> BackupPayloadV2 {
    BackupPayloadV2 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets.into_iter().map(asset_v1_to_v2).collect(),
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
    }
}

fn asset_v2_to_v3(a: BackupAssetV2) -> BackupAssetV3 {
    // Legacy per-asset contribution fields are dropped on migration. User reconfigures
    // their allocation_rules from scratch after importing an older backup.
    BackupAssetV3 {
        category_ref: a.category_ref,
        name: a.name,
        current_value: a.current_value,
        purchase_price: a.purchase_price,
        is_liquid: a.is_liquid,
        expected_annual_return_percent: a.expected_annual_return_percent,
        notes: a.notes,
        sort_index: a.sort_index,
    }
}

fn payload_v2_to_v3(p: BackupPayloadV2) -> BackupPayloadV3 {
    BackupPayloadV3 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets.into_iter().map(asset_v2_to_v3).collect(),
        allocation_rules: Vec::new(),
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
    }
}

fn payload_v3_to_v4(p: BackupPayloadV3) -> BackupPayloadV4 {
    // History snapshots did not exist before v4, so they start empty when importing an
    // older backup. Everything else is carried over unchanged.
    BackupPayloadV4 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: Vec::new(),
    }
}

fn payload_v4_to_v5(p: BackupPayloadV4) -> BackupPayloadV5 {
    // Transactions, imports and rules did not exist before v5 → start empty when importing an
    // older backup. Everything else is carried over unchanged.
    BackupPayloadV5 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: p.snapshots,
        transaction_imports: Vec::new(),
        transactions: Vec::new(),
        categorization_rules: Vec::new(),
    }
}

fn payload_v5_to_v6(p: BackupPayloadV5) -> BackupPayloadV6 {
    // Recurring-transaction rules did not exist before v6 → start empty when importing an older
    // backup. Everything else is carried over unchanged.
    BackupPayloadV6 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: p.snapshots,
        transaction_imports: p.transaction_imports,
        transactions: p.transactions,
        categorization_rules: p.categorization_rules,
        recurring_transaction_rules: Vec::new(),
    }
}

fn payload_v6_to_v7(p: BackupPayloadV6) -> BackupPayloadV7 {
    // v7 dropped the per-rule `day_of_month` (rules are month-resolution only; instances are dated
    // at end-of-month by the materializer). The field is discarded; everything else carries over.
    BackupPayloadV7 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: p.snapshots,
        transaction_imports: p.transaction_imports,
        transactions: p.transactions,
        categorization_rules: p.categorization_rules,
        recurring_transaction_rules: p
            .recurring_transaction_rules
            .into_iter()
            .map(|r| BackupRecurringRuleV8 {
                concept: r.concept,
                amount: r.amount,
                kind: r.kind,
                category_ref: r.category_ref,
                linked_asset_index: r.linked_asset_index,
                linked_liability_index: r.linked_liability_index,
                notes: r.notes,
                last_materialized_month: r.last_materialized_month,
            })
            .collect(),
    }
}

fn payload_v7_to_v8(p: BackupPayloadV7) -> BackupPayloadV8 {
    // La conciliación de transferencias no existía antes de v8 → las transacciones llegan sin
    // counterpart (los `#[serde(default)]` ya lo garantizan) y los rechazos empiezan vacíos.
    // Tras importar un backup ≤v7, el pase post-import re-concilia lo que corresponda.
    BackupPayloadV8 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: p.snapshots,
        transaction_imports: p.transaction_imports,
        transactions: p.transactions,
        categorization_rules: p.categorization_rules,
        recurring_transaction_rules: p.recurring_transaction_rules,
        transfer_match_rejections: Vec::new(),
    }
}

/// v8 → v9: el cursor `last_materialized_month` se sustituye por el ancla `origin_month`.
///
/// El cursor NO es el origen — es el mes más reciente ya materializado, así que usarlo tal cual
/// impediría materializar todo lo anterior. La reconstrucción correcta es el mes de la instancia
/// MÁS ANTIGUA de la regla dentro del propio payload (`recurring_rule_index` apunta a su posición
/// en `recurring_transaction_rules`); si la regla no tiene ninguna instancia en el fichero, el
/// cursor es la única cota disponible. Es la misma regla que aplica la migración de base de datos
/// `20260821120000_recurring_converge_on_real_movement`, para que importar un backup y actualizar
/// una instalación produzcan el mismo ancla.
fn payload_v8_to_v9(p: BackupPayloadV8) -> BackupPayloadV9 {
    use chrono::Datelike;
    use std::collections::HashMap;
    let mut earliest: HashMap<usize, NaiveDate> = HashMap::new();
    for t in &p.transactions {
        if let Some(ix) = t.recurring_rule_index {
            let month = NaiveDate::from_ymd_opt(t.op_date.year(), t.op_date.month(), 1)
                .expect("valid first-of-month");
            earliest
                .entry(ix)
                .and_modify(|m| {
                    if month < *m {
                        *m = month;
                    }
                })
                .or_insert(month);
        }
    }
    let recurring_transaction_rules = p
        .recurring_transaction_rules
        .into_iter()
        .enumerate()
        .map(|(ix, r)| BackupRecurringRule {
            concept: r.concept,
            amount: r.amount,
            kind: r.kind,
            category_ref: r.category_ref,
            linked_asset_index: r.linked_asset_index,
            linked_liability_index: r.linked_liability_index,
            notes: r.notes,
            origin_month: earliest
                .get(&ix)
                .copied()
                .map(|m| m.min(r.last_materialized_month))
                .unwrap_or(r.last_materialized_month),
        })
        .collect();
    BackupPayloadV9 {
        user: p.user,
        categories_used: p.categories_used,
        assets: p.assets,
        allocation_rules: p.allocation_rules,
        liabilities: p.liabilities,
        budget_entries: p.budget_entries,
        planning_flows: p.planning_flows,
        ui_preferences: p.ui_preferences,
        installation_snapshot_informative: p.installation_snapshot_informative,
        snapshots: p.snapshots,
        transaction_imports: p.transaction_imports,
        transactions: p.transactions,
        categorization_rules: p.categorization_rules,
        recurring_transaction_rules,
        transfer_match_rejections: p.transfer_match_rejections,
    }
}

pub fn migrate_to_current(any: AnyPayload) -> BackupPayload {
    // Cadena completa v1..v9: TODOS los backups antiguos siguen importando (regla de
    // change-control §5 — un backup es la única vía de recuperación de un usuario).
    match any {
        AnyPayload::V1(p) => payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(
            payload_v5_to_v6(payload_v4_to_v5(payload_v3_to_v4(payload_v2_to_v3(
                payload_v1_to_v2(p),
            )))),
        ))),
        AnyPayload::V2(p) => payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(
            payload_v5_to_v6(payload_v4_to_v5(payload_v3_to_v4(payload_v2_to_v3(p)))),
        ))),
        AnyPayload::V3(p) => payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(
            payload_v5_to_v6(payload_v4_to_v5(payload_v3_to_v4(p))),
        ))),
        AnyPayload::V4(p) => payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(
            payload_v5_to_v6(payload_v4_to_v5(p)),
        ))),
        AnyPayload::V5(p) => {
            payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(payload_v5_to_v6(p))))
        }
        AnyPayload::V6(p) => payload_v8_to_v9(payload_v7_to_v8(payload_v6_to_v7(p))),
        AnyPayload::V7(p) => payload_v8_to_v9(payload_v7_to_v8(p)),
        AnyPayload::V8(p) => payload_v8_to_v9(p),
        AnyPayload::V9(p) => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_future_schema_version() {
        let err = parse_payload(999, b"{}").unwrap_err();
        assert!(err.contains("newer than this server supports"), "{err}");
    }

    #[test]
    fn migrate_v1_drops_legacy_contribution_fields() {
        let raw = serde_json::json!({
            "user": { "username": "alice", "birth_date": null },
            "categories_used": [{ "scope": "asset", "name": "Equity", "sort_index": 0 }],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Equity" },
                "name": "Fondo",
                "current_value": "1000.00",
                "is_liquid": true,
                "monthly_contribution_fixed": "100.00",
                "contribution_frequency": "monthly",
                "contribution_remainder_weight": "0",
                "sort_index": 0
            }],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": {
                "base_currency": "EUR",
                "calendar_tz": "UTC",
                "show_age_mode": "dates",
                "fire_settings": {
                    "fire_number_mode": "annual_expense",
                    "fire_number_manual_amount": null,
                    "fire_number_expense_adjustment_pct": null,
                    "swr_pct": "3.5",
                    "taxes_enabled": true,
                    "tax_brackets": [
                        { "up_to": null, "pct": "19" }
                    ]
                }
            }
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(1, &bytes).unwrap();
        let v3 = migrate_to_current(any);
        assert_eq!(v3.user.username, "alice");
        assert_eq!(v3.assets.len(), 1);
        assert_eq!(v3.assets[0].name, "Fondo");
        assert!(v3.allocation_rules.is_empty());
    }

    #[test]
    fn v3_with_rules_round_trip() {
        let raw = serde_json::json!({
            "user": { "username": "bob", "birth_date": null },
            "categories_used": [],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Cash" },
                "name": "Cuenta",
                "current_value": "500.00",
                "is_liquid": true,
                "sort_index": 0
            }],
            "allocation_rules": [{
                "target_asset_index": 0,
                "priority": 1,
                "kind": "remainder",
                "enabled": true
            }],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": {
                "base_currency": "EUR",
                "calendar_tz": "UTC",
                "show_age_mode": "dates",
                "fire_settings": {
                    "fire_number_mode": "annual_expense",
                    "fire_number_manual_amount": null,
                    "fire_number_expense_adjustment_pct": null,
                    "swr_pct": "3.5",
                    "taxes_enabled": true,
                    "tax_brackets": [
                        { "up_to": null, "pct": "19" }
                    ]
                }
            }
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(3, &bytes).unwrap();
        let v3 = migrate_to_current(any);
        assert_eq!(v3.allocation_rules.len(), 1);
        assert_eq!(v3.allocation_rules[0].kind, "remainder");
        // A v3 payload has no snapshots; migration must default them to empty.
        assert!(v3.snapshots.is_empty());
    }

    /// Minimal but complete installation snapshot block reused across the payload tests.
    fn installation_snapshot_json() -> serde_json::Value {
        serde_json::json!({
            "base_currency": "EUR",
            "calendar_tz": "UTC",
            "show_age_mode": "dates",
            "fire_settings": {
                "fire_number_mode": "annual_expense",
                "fire_number_manual_amount": null,
                "fire_number_expense_adjustment_pct": null,
                "swr_pct": "3.5",
                "taxes_enabled": true,
                "tax_brackets": [ { "up_to": null, "pct": "19" } ]
            }
        })
    }

    #[test]
    fn migrate_v3_fills_empty_snapshots() {
        // A v3 file (no `snapshots` key) parses and migrates to a v4 with an empty vec.
        let raw = serde_json::json!({
            "user": { "username": "carol", "birth_date": null },
            "categories_used": [],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Cash" },
                "name": "Cuenta",
                "current_value": "500.00",
                "is_liquid": true,
                "sort_index": 0
            }],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json()
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(3, &bytes).unwrap();
        let v4 = migrate_to_current(any);
        assert!(
            v4.snapshots.is_empty(),
            "v3→v4 must default snapshots to empty"
        );
        assert_eq!(v4.assets.len(), 1);
    }

    #[test]
    fn v4_snapshot_items_round_trip() {
        let k_asset = "11111111-1111-1111-1111-111111111111";
        let k_deleted = "22222222-2222-2222-2222-222222222222";
        let k_liab = "33333333-3333-3333-3333-333333333333";
        let raw = serde_json::json!({
            "user": { "username": "dave", "birth_date": null },
            "categories_used": [],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Cash" },
                "name": "Fondo",
                "current_value": "1000.00",
                "is_liquid": true,
                "sort_index": 0
            }],
            "allocation_rules": [],
            "liabilities": [{
                "category_ref": { "scope": "liability", "name": "Loan" },
                "label": "Hipoteca",
                "principal": "80000.00",
                "principal_derived_from_plan": false,
                "sort_index": 0
            }],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [
                {
                    "kind": "asset",
                    "snapshot_date": "2025-01-15",
                    "source": "backfill",
                    "items": [
                        { "ledger_index": 0, "item_key": k_asset, "label": "Fondo", "value": "1234.5600" },
                        { "ledger_index": null, "item_key": k_deleted, "label": "Borrado", "value": "50.0000" }
                    ]
                },
                {
                    "kind": "liability",
                    "snapshot_date": "2025-02-20",
                    "source": "capture",
                    "items": [
                        { "ledger_index": 0, "item_key": k_liab, "label": "Hipoteca", "value": "80000.0000",
                          "apr_percent": "3.2500", "payment_amount": "500.0000", "payment_frequency": "monthly" }
                    ]
                }
            ]
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(4, &bytes).unwrap();
        let v4 = migrate_to_current(any);

        assert_eq!(v4.snapshots.len(), 2);

        let asnap = &v4.snapshots[0];
        assert_eq!(asnap.kind, "asset");
        assert_eq!(
            asnap.snapshot_date,
            NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
        );
        assert_eq!(asnap.source, "backfill");
        assert_eq!(asnap.items.len(), 2);
        assert_eq!(asnap.items[0].ledger_index, Some(0));
        assert_eq!(asnap.items[0].item_key, Uuid::parse_str(k_asset).unwrap());
        assert_eq!(asnap.items[0].label, "Fondo");
        assert_eq!(
            asnap.items[0].value,
            Decimal::from_str_exact("1234.5600").unwrap()
        );
        assert!(asnap.items[0].apr_percent.is_none());
        assert_eq!(asnap.items[1].ledger_index, None);
        assert_eq!(asnap.items[1].item_key, Uuid::parse_str(k_deleted).unwrap());

        let lsnap = &v4.snapshots[1];
        assert_eq!(lsnap.kind, "liability");
        assert_eq!(lsnap.source, "capture");
        assert_eq!(
            lsnap.items[0].value,
            Decimal::from_str_exact("80000.0000").unwrap()
        );
        assert_eq!(
            lsnap.items[0].apr_percent,
            Some(Decimal::from_str_exact("3.2500").unwrap())
        );
        assert_eq!(
            lsnap.items[0].payment_amount,
            Some(Decimal::from_str_exact("500.0000").unwrap())
        );
        assert_eq!(lsnap.items[0].payment_frequency.as_deref(), Some("monthly"));

        // Decimal-string round-trip: re-serialize and confirm the scale-preserving strings.
        let reser = serde_json::to_value(&v4.snapshots).unwrap();
        assert_eq!(reser[0]["items"][0]["value"], "1234.5600");
        assert_eq!(reser[1]["items"][0]["apr_percent"], "3.2500");
        assert_eq!(reser[1]["items"][0]["payment_amount"], "500.0000");
        // ledger_index null must serialize back as null (present, not dropped).
        assert!(reser[0]["items"][1]["ledger_index"].is_null());
    }

    #[test]
    fn migrate_v4_fills_empty_transactions() {
        // A v4 file (no transaction keys) parses and migrates to v5 with empty vecs.
        let raw = serde_json::json!({
            "user": { "username": "erin", "birth_date": null },
            "categories_used": [],
            "assets": [],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": []
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(4, &bytes).unwrap();
        let v5 = migrate_to_current(any);
        assert!(
            v5.transaction_imports.is_empty(),
            "v4→v5 must default transaction_imports to empty"
        );
        assert!(
            v5.transactions.is_empty(),
            "v4→v5 must default transactions to empty"
        );
        assert!(
            v5.categorization_rules.is_empty(),
            "v4→v5 must default categorization_rules to empty"
        );
        assert!(v5.snapshots.is_empty());
    }

    #[test]
    fn v5_transactions_round_trip() {
        let raw = serde_json::json!({
            "user": { "username": "frank", "birth_date": null },
            "categories_used": [
                { "scope": "expense", "name": "Supermercado", "sort_index": 0 },
                { "scope": "income", "name": "Nómina", "sort_index": 1 }
            ],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Cash" },
                "name": "Cuenta", "current_value": "1000.00", "is_liquid": true, "sort_index": 0
            }],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [],
            "transaction_imports": [
                { "source": "n26", "account_asset_index": 0, "original_filename": "junio.csv" }
            ],
            "transactions": [
                {
                    "import_index": 0, "source": "n26", "op_date": "2026-06-02",
                    "value_date": "2026-06-02", "concept": "CONSUM BARNA", "amount": "-4.9800",
                    "currency": "EUR", "kind": "expense",
                    "category_ref": { "scope": "expense", "name": "Supermercado" },
                    "fingerprint_ordinal": 0, "linked_asset_index": null,
                    "linked_liability_index": null, "notes": null
                },
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-05",
                    "concept": "Efectivo", "amount": "-20.0000", "currency": "EUR",
                    "kind": "expense", "fingerprint_ordinal": 0
                }
            ],
            "categorization_rules": [
                {
                    "match_kind": "substring", "pattern": "CONSUM", "source": "n26",
                    "assign_kind": "expense",
                    "assign_category_ref": { "scope": "expense", "name": "Supermercado" }
                }
            ]
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(5, &bytes).unwrap();
        let v5 = migrate_to_current(any);

        assert_eq!(v5.transaction_imports.len(), 1);
        assert_eq!(v5.transaction_imports[0].source, "n26");
        assert_eq!(v5.transaction_imports[0].account_asset_index, Some(0));

        assert_eq!(v5.transactions.len(), 2);
        assert_eq!(v5.transactions[0].import_index, Some(0));
        assert_eq!(
            v5.transactions[0].amount,
            Decimal::from_str_exact("-4.9800").unwrap()
        );
        assert_eq!(v5.transactions[0].kind.as_deref(), Some("expense"));
        assert_eq!(v5.transactions[1].import_index, None);
        assert_eq!(v5.transactions[1].source, "manual");

        assert_eq!(v5.categorization_rules.len(), 1);
        assert_eq!(v5.categorization_rules[0].pattern, "CONSUM");
        assert_eq!(v5.categorization_rules[0].source.as_deref(), Some("n26"));

        // Decimal-string scale round-trip.
        let reser = serde_json::to_value(&v5.transactions).unwrap();
        assert_eq!(reser[0]["amount"], "-4.9800");
        // fingerprint is never present in the payload (recomputed on import).
        assert!(reser[0].get("fingerprint").is_none());
    }

    #[test]
    fn migrate_v5_fills_empty_recurring() {
        // A v5 file (no recurring key) parses and migrates to current with an empty vec.
        let raw = serde_json::json!({
            "user": { "username": "gwen", "birth_date": null },
            "categories_used": [],
            "assets": [],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [],
            "transaction_imports": [],
            "transactions": [],
            "categorization_rules": []
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(5, &bytes).unwrap();
        let cur = migrate_to_current(any);
        assert!(
            cur.recurring_transaction_rules.is_empty(),
            "v5→current must default recurring_transaction_rules to empty"
        );
    }

    #[test]
    fn v6_recurring_rules_round_trip() {
        let raw = serde_json::json!({
            "user": { "username": "hank", "birth_date": null },
            "categories_used": [
                { "scope": "income", "name": "Nómina", "sort_index": 0 }
            ],
            "assets": [],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [],
            "transaction_imports": [],
            "transactions": [
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-01",
                    "concept": "Nomina", "amount": "2000.0000", "currency": "EUR",
                    "kind": "income",
                    "category_ref": { "scope": "income", "name": "Nómina" },
                    "fingerprint_ordinal": 0, "recurring_rule_index": 0
                }
            ],
            "categorization_rules": [],
            "recurring_transaction_rules": [
                {
                    "concept": "Nomina", "amount": "2000.0000", "kind": "income",
                    "category_ref": { "scope": "income", "name": "Nómina" },
                    "day_of_month": 1, "linked_asset_index": null,
                    "linked_liability_index": null, "notes": null,
                    "last_materialized_month": "2026-06-01"
                }
            ]
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(6, &bytes).unwrap();
        let v7 = migrate_to_current(any);

        assert_eq!(v7.recurring_transaction_rules.len(), 1);
        let rule = &v7.recurring_transaction_rules[0];
        assert_eq!(rule.concept, "Nomina");
        assert_eq!(rule.amount, Decimal::from_str_exact("2000.0000").unwrap());
        assert_eq!(rule.kind, "income");
        assert_eq!(
            rule.category_ref.as_ref().map(|c| c.name.as_str()),
            Some("Nómina")
        );
        // v9: el cursor se convirtió en ancla. Aquí ambos coinciden (la única instancia del
        // payload vive en el mes del cursor), así que este caso NO discrimina entre coger el
        // origen y coger el cursor — de eso se encarga `v8_to_v9_anchors_on_earliest_instance`.
        assert_eq!(
            rule.origin_month,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        // The transaction points back at the rule by index.
        assert_eq!(v7.transactions[0].recurring_rule_index, Some(0));

        // Decimal-string scale round-trip; v6→v7 drops the legacy `day_of_month`.
        let reser = serde_json::to_value(&v7.recurring_transaction_rules).unwrap();
        assert_eq!(reser[0]["amount"], "2000.0000");
        assert!(
            reser[0].get("day_of_month").is_none(),
            "v7 rules must not carry day_of_month"
        );
    }

    #[test]
    fn migrate_v7_fills_empty_transfer_rejections() {
        // Un v7 (sin claves de conciliación) parsea y migra a v8 con rechazos vacíos y
        // transacciones sin counterpart.
        let raw = serde_json::json!({
            "user": { "username": "iris", "birth_date": null },
            "categories_used": [],
            "assets": [],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [],
            "transaction_imports": [],
            "transactions": [
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-05",
                    "concept": "Efectivo", "amount": "-20.0000", "currency": "EUR",
                    "kind": "expense", "fingerprint_ordinal": 0
                }
            ],
            "categorization_rules": [],
            "recurring_transaction_rules": []
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(7, &bytes).unwrap();
        let v8 = migrate_to_current(any);
        assert!(
            v8.transfer_match_rejections.is_empty(),
            "v7→v8 must default transfer_match_rejections to empty"
        );
        assert!(v8.transactions[0].transfer_counterpart_index.is_none());
        assert!(v8.transactions[0].transfer_reconciled_source.is_none());
    }

    #[test]
    fn v8_transfer_pairing_round_trip() {
        let raw = serde_json::json!({
            "user": { "username": "juan", "birth_date": null },
            "categories_used": [],
            "assets": [],
            "allocation_rules": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json(),
            "snapshots": [],
            "transaction_imports": [],
            "transactions": [
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-10",
                    "concept": "Salida", "amount": "-100.0000", "currency": "EUR",
                    "kind": "expense", "fingerprint_ordinal": 0,
                    "transfer_counterpart_index": 1,
                    "transfer_reconciled_at": "2026-06-12T10:00:00Z",
                    "transfer_reconciled_source": "auto"
                },
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-11",
                    "concept": "Entrada", "amount": "100.0000", "currency": "EUR",
                    "kind": "income", "fingerprint_ordinal": 0,
                    "transfer_counterpart_index": 0,
                    "transfer_reconciled_at": "2026-06-12T10:00:00Z",
                    "transfer_reconciled_source": "auto"
                },
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-20",
                    "concept": "Gasto suelto", "amount": "-50.0000", "currency": "EUR",
                    "kind": "expense", "fingerprint_ordinal": 0
                },
                {
                    "import_index": null, "source": "manual", "op_date": "2026-06-21",
                    "concept": "Reembolso rechazado", "amount": "50.0000", "currency": "EUR",
                    "kind": "income", "fingerprint_ordinal": 0
                }
            ],
            "categorization_rules": [],
            "recurring_transaction_rules": [],
            "transfer_match_rejections": [
                { "transaction_a_index": 2, "transaction_b_index": 3 }
            ]
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(8, &bytes).unwrap();
        let v8 = migrate_to_current(any);

        // Par simétrico por índices.
        assert_eq!(v8.transactions[0].transfer_counterpart_index, Some(1));
        assert_eq!(v8.transactions[1].transfer_counterpart_index, Some(0));
        assert_eq!(
            v8.transactions[0].transfer_reconciled_source.as_deref(),
            Some("auto")
        );
        // El par rechazado viaja por índices.
        assert_eq!(v8.transfer_match_rejections.len(), 1);
        assert_eq!(v8.transfer_match_rejections[0].transaction_a_index, 2);
        assert_eq!(v8.transfer_match_rejections[0].transaction_b_index, 3);

        // Round-trip de serialización: los campos de conciliación se conservan.
        let reser = serde_json::to_value(&v8.transactions).unwrap();
        assert_eq!(reser[0]["transfer_counterpart_index"], 1);
        assert_eq!(reser[1]["transfer_reconciled_source"], "auto");
    }

    #[test]
    fn migrate_v1_chain_reaches_v4() {
        // The full v1 → v2 → v3 → v4 chain must succeed and leave snapshots empty.
        let raw = serde_json::json!({
            "user": { "username": "alice", "birth_date": null },
            "categories_used": [{ "scope": "asset", "name": "Equity", "sort_index": 0 }],
            "assets": [{
                "category_ref": { "scope": "asset", "name": "Equity" },
                "name": "Fondo",
                "current_value": "1000.00",
                "is_liquid": true,
                "monthly_contribution_fixed": "100.00",
                "contribution_frequency": "monthly",
                "contribution_remainder_weight": "0",
                "sort_index": 0
            }],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": installation_snapshot_json()
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let any = parse_payload(1, &bytes).unwrap();
        let v4 = migrate_to_current(any);
        assert_eq!(v4.user.username, "alice");
        assert_eq!(v4.assets.len(), 1);
        assert_eq!(v4.assets[0].name, "Fondo");
        assert!(v4.allocation_rules.is_empty());
        assert!(v4.snapshots.is_empty());
    }

    /// v8 → v9 ancla en la instancia MÁS ANTIGUA, no en el cursor. Es el caso discriminante: el
    /// cursor (mes más reciente materializado) va por delante del origen, así que copiarlo tal
    /// cual dejaría la regla sin poder materializar los meses intermedios tras el import.
    #[test]
    fn v8_to_v9_anchors_on_earliest_instance_not_on_the_cursor() {
        let payload = serde_json::json!({
            "user": {"username": "u", "birth_date": null},
            "categories_used": [],
            "assets": [], "liabilities": [], "budget_entries": [], "planning_flows": [],
            "installation_snapshot_informative": installation_snapshot_json(),
            "transactions": [
                {"import_index": null, "source": "manual", "op_date": "2026-03-31",
                 "value_date": null, "concept": "Nomina", "amount": "2000.0000",
                 "currency": "EUR", "kind": "income", "category_ref": null,
                 "linked_asset_index": null, "linked_liability_index": null, "notes": null,
                 "fingerprint": "fp-mar", "fingerprint_ordinal": 0, "recurring_rule_index": 0},
                {"import_index": null, "source": "manual", "op_date": "2026-06-30",
                 "value_date": null, "concept": "Nomina", "amount": "2000.0000",
                 "currency": "EUR", "kind": "income", "category_ref": null,
                 "linked_asset_index": null, "linked_liability_index": null, "notes": null,
                 "fingerprint": "fp-jun", "fingerprint_ordinal": 0, "recurring_rule_index": 0}
            ],
            "recurring_transaction_rules": [
                {"concept": "Nomina", "amount": "2000.0000", "kind": "income",
                 "category_ref": null, "linked_asset_index": null,
                 "linked_liability_index": null, "notes": null,
                 "last_materialized_month": "2026-06-01"}
            ]
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let v9 = migrate_to_current(parse_payload(8, &bytes).unwrap());

        // Cursor = junio; instancia más antigua = marzo. El ancla debe ser MARZO.
        assert_eq!(
            v9.recurring_transaction_rules[0].origin_month,
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            "el ancla debe salir de la instancia más antigua, no del cursor"
        );
    }

    /// Sin instancias en el payload (el usuario las borró a mano), el cursor es la única cota.
    #[test]
    fn v8_to_v9_falls_back_to_the_cursor_when_the_rule_has_no_instances() {
        let payload = serde_json::json!({
            "user": {"username": "u", "birth_date": null},
            "categories_used": [],
            "assets": [], "liabilities": [], "budget_entries": [], "planning_flows": [],
            "installation_snapshot_informative": installation_snapshot_json(),
            "transactions": [],
            "recurring_transaction_rules": [
                {"concept": "Nomina", "amount": "2000.0000", "kind": "income",
                 "category_ref": null, "linked_asset_index": null,
                 "linked_liability_index": null, "notes": null,
                 "last_materialized_month": "2026-06-01"}
            ]
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let v9 = migrate_to_current(parse_payload(8, &bytes).unwrap());
        assert_eq!(
            v9.recurring_transaction_rules[0].origin_month,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
    }
}
