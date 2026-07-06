//! Versioned DTO + migration layer for `.ffbackup` payloads.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::handlers::installation::FireSettings;

pub const CURRENT_SCHEMA_VERSION: u32 = 4;
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

/// Alias for the current-version payload. Export and import code work against this type.
pub type BackupPayload = BackupPayloadV4;

#[derive(Debug)]
pub enum AnyPayload {
    V1(BackupPayloadV1),
    V2(BackupPayloadV2),
    V3(BackupPayloadV3),
    V4(BackupPayloadV4),
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

pub fn migrate_to_current(any: AnyPayload) -> BackupPayload {
    match any {
        AnyPayload::V1(p) => payload_v3_to_v4(payload_v2_to_v3(payload_v1_to_v2(p))),
        AnyPayload::V2(p) => payload_v3_to_v4(payload_v2_to_v3(p)),
        AnyPayload::V3(p) => payload_v3_to_v4(p),
        AnyPayload::V4(p) => p,
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
        assert!(v4.snapshots.is_empty(), "v3→v4 must default snapshots to empty");
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
        assert_eq!(asnap.snapshot_date, NaiveDate::from_ymd_opt(2025, 1, 15).unwrap());
        assert_eq!(asnap.source, "backfill");
        assert_eq!(asnap.items.len(), 2);
        assert_eq!(asnap.items[0].ledger_index, Some(0));
        assert_eq!(asnap.items[0].item_key, Uuid::parse_str(k_asset).unwrap());
        assert_eq!(asnap.items[0].label, "Fondo");
        assert_eq!(asnap.items[0].value, Decimal::from_str_exact("1234.5600").unwrap());
        assert!(asnap.items[0].apr_percent.is_none());
        assert_eq!(asnap.items[1].ledger_index, None);
        assert_eq!(asnap.items[1].item_key, Uuid::parse_str(k_deleted).unwrap());

        let lsnap = &v4.snapshots[1];
        assert_eq!(lsnap.kind, "liability");
        assert_eq!(lsnap.source, "capture");
        assert_eq!(lsnap.items[0].value, Decimal::from_str_exact("80000.0000").unwrap());
        assert_eq!(lsnap.items[0].apr_percent, Some(Decimal::from_str_exact("3.2500").unwrap()));
        assert_eq!(lsnap.items[0].payment_amount, Some(Decimal::from_str_exact("500.0000").unwrap()));
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
}
