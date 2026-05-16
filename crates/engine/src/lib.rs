//! Monthly net-worth projection.

mod projection;

pub use projection::{
    first_month_per_asset_contribution_nominals, project_net_worth_series, AllocationCap,
    AllocationKind, AllocationRule, EngineError, ProjectionInput, ProjectionLiabilityInput,
    ProjectionOutput, SimAsset,
};
