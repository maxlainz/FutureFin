//! Monthly net-worth projection and FIRE milestone helpers aligned to `docs/plan/PRODUCT_DOSSIER_PLAN.md`.
//! Numeric parity vs Swift is validated via `docs/spec/ORACLE_TESTS.md` (golden tests TBD).

mod fire;
mod projection;

pub use fire::{
    fire_milestone_month, gross_withdrawal_for_net_after_cgt,
    net_after_capital_gains_on_withdrawal, portfolio_target_for_net_annual_need,
    portfolio_target_simple, progressive_tax_on_amount, FireMilestoneInput, FireMilestoneOutput,
    TaxBand,
};
pub use projection::{
    project_net_worth_series, EngineError, ProjectionFlowInput, ProjectionInput,
    ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
