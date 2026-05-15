//! Versioned DTO + migration layer for `.ffbackup` payloads.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::handlers::installation::FireSettings;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
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

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupAsset {
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
    pub projection_includes_inflation: bool,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayloadV1 {
    pub user: BackupUser,
    pub categories_used: Vec<BackupCategory>,
    pub assets: Vec<BackupAsset>,
    pub liabilities: Vec<BackupLiability>,
    pub budget_entries: Vec<BackupBudgetEntry>,
    pub planning_flows: Vec<BackupPlanningFlow>,
    #[serde(default)]
    pub ui_preferences: UiPreferences,
    pub installation_snapshot_informative: InstallationSnapshotInformative,
}

/// Wrapper that lets us decide how to parse based on `schema_version` from the manifest.
/// Add new variants here (`V2`, `V3`, …) and the corresponding `vN_to_current` migrator.
#[derive(Debug)]
pub enum AnyPayload {
    V1(BackupPayloadV1),
}

pub fn parse_payload(schema_version: u32, bytes: &[u8]) -> Result<AnyPayload, String> {
    match schema_version {
        1 => {
            let p: BackupPayloadV1 = serde_json::from_slice(bytes)
                .map_err(|e| format!("payload v1 malformed: {e}"))?;
            Ok(AnyPayload::V1(p))
        }
        v if v > CURRENT_SCHEMA_VERSION => Err(format!(
            "schema_version {v} is newer than this server supports ({CURRENT_SCHEMA_VERSION}); update FutureFin to import this backup",
        )),
        v => Err(format!("schema_version {v} not supported")),
    }
}

pub fn migrate_to_current(any: AnyPayload) -> BackupPayloadV1 {
    match any {
        AnyPayload::V1(p) => p,
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
    fn migrate_v1_passthrough() {
        let raw = serde_json::json!({
            "user": { "username": "alice", "birth_date": null },
            "categories_used": [],
            "assets": [],
            "liabilities": [],
            "budget_entries": [],
            "planning_flows": [],
            "ui_preferences": {},
            "installation_snapshot_informative": {
                "base_currency": "EUR",
                "calendar_tz": "UTC",
                "projection_includes_inflation": false,
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
        let v1 = migrate_to_current(any);
        assert_eq!(v1.user.username, "alice");
        assert_eq!(v1.installation_snapshot_informative.base_currency, "EUR");
    }
}
