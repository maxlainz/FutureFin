//! Monthly net-worth projection.

mod history;
mod net_return;
mod projection;
mod runway;

pub use history::{
    add_months_signed, amortized_segment_value, anchored_cashflow_segment_value, evaluate_timeline,
    month_index_of, CashFlowEntry, HistoryItem, HistoryItemKind, HistoryObservation,
    HistoryTimeline, LoanTerms,
};
pub use net_return::{net_return_percentages, NetReturn};
pub use projection::{
    fire_target_at_month_index, first_month_allocation,
    first_month_per_asset_contribution_nominals, present_value_of_payments,
    project_net_worth_series, AllocationCap, AllocationKind, AllocationRule, AllocationSkipReason,
    EngineError, FireTarget, FirstMonthAllocation, ProjectionInput, ProjectionLiabilityInput,
    ProjectionOutput, RepaymentModel, RuleOutcome, SimAsset,
};
pub use runway::{liquid_runway_months, RunwayOutcome, MAX_RUNWAY_MONTHS};
