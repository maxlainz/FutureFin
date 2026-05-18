//! Monthly net-worth projection.

mod projection;

pub use projection::{
    fire_target_at_month_index, first_month_per_asset_contribution_nominals,
    project_net_worth_series, AllocationCap, AllocationKind, AllocationRule, EngineError,
    FireTarget, ProjectionInput, ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
