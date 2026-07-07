//! Monthly net-worth projection.

mod history;
mod projection;

pub use history::{
    add_months_signed, amortized_segment_value, anchored_cashflow_segment_value, evaluate_timeline,
    month_index_of, CashFlowEntry, HistoryItem, HistoryItemKind, HistoryObservation,
    HistoryTimeline, LoanTerms,
};
pub use projection::{
    fire_target_at_month_index, first_month_per_asset_contribution_nominals,
    project_net_worth_series, AllocationCap, AllocationKind, AllocationRule, EngineError,
    FireTarget, ProjectionInput, ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
