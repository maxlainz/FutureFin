//! Versioned DTO + migration layer for `.ffbackup` payloads.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::handlers::installation::FireSettings;

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
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

/// Alias for the current-version payload. Export and import code work against this type.
pub type BackupPayload = BackupPayloadV3;

#[derive(Debug)]
pub enum AnyPayload {
    V1(BackupPayloadV1),
    V2(BackupPayloadV2),
    V3(BackupPayloadV3),
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

pub fn migrate_to_current(any: AnyPayload) -> BackupPayload {
    match any {
        AnyPayload::V1(p) => payload_v2_to_v3(payload_v1_to_v2(p)),
        AnyPayload::V2(p) => payload_v2_to_v3(p),
        AnyPayload::V3(p) => p,
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
    }
}
