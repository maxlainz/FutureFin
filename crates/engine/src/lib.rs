//! Monthly net-worth projection aligned to `docs/plan/PRODUCT_DOSSIER_PLAN.md`.

mod projection;

pub use projection::{
    first_month_per_asset_contribution_nominals, project_net_worth_series, EngineError,
    ProjectionInput, ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
