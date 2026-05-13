# Projection Engine (crates/engine)

Pure Rust crate — no I/O, no DB, no async. Only `Decimal` arithmetic.

## Public API

```rust
// Main projection: returns net_worth and contributed_capital series (len = horizon_months + 1, index 0 = today)
pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError>

// Returns nominal contributions routed to each asset in the FIRST simulated month only
pub fn first_month_per_asset_contribution_nominals(input: &ProjectionInput) -> Result<Vec<Decimal>, EngineError>
```

## ProjectionInput fields
```rust
pub struct ProjectionInput {
    pub ref_date: NaiveDate,           // Civil "today" from installation calendar_tz
    pub horizon_months: u32,           // >= 1
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    pub liabilities: Vec<ProjectionLiabilityInput>,
    pub inflation_annual_percent: Option<Decimal>,  // None = nominal series
    pub planning_monthly_cash_adjustment: Vec<Decimal>, // len == horizon_months; index i = month i+1
    pub retirement_start_month: Option<u32>, // 1-based month when drawdown begins; None = no drawdown
    pub income_retirement_monthly: Decimal,  // income from persisting sources (budget_entries.persists_after_retirement=true); replaces income_regular_monthly from retirement_start_month onward
    pub retirement_monthly_withdrawal: Decimal, // optional extra draw on top of budget (handler always passes 0; income reduction is the primary drain)
}
```

## SimAsset fields
- `monthly_contribution_fixed`: minimum fixed contribution per month
- `contribution_remainder_weight`: proportional weight for splitting surplus after fixed contributions
- `expected_annual_return_percent`: compound growth rate (7 = 7%/year). None = no compound growth.
- `is_liquid`: liquid assets are drained first when cash is negative; sorted by growth rate (lowest first)
- `purchase_price`: optional cost basis; included in `contributed_capital[0]`

## Simulation loop (per month)
1. Compute `debt_service` = sum of active liability payments (capped by remaining principal)
2. Select `income`: if `k >= retirement_start_month` use `income_retirement_monthly`, else `income_regular_monthly`
3. `retirement_withdrawal` = `retirement_monthly_withdrawal` if `k >= retirement_start_month`, else 0 (inflation-adjusted if inflation active)
4. `net_cash = income - expense - debt_service + planning_adj[k] - retirement_withdrawal`
5. If `net_cash > 0` (surplus): apply fixed contributions (scaled if pool < sum_fixed), distribute remainder by weight
6. If `net_cash <= 0` (deficit): drain surplus_cash first, then drain liquid assets (lowest-return first)
7. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value
8. Reduce liability principals by payments made
9. If inflation active: deflate net_worth and contributed_capital by `(1 + inf/100)^(k/12)`

## Output
```rust
pub struct ProjectionOutput {
    pub net_worth: Vec<Decimal>,         // index 0..=horizon_months
    pub contributed_capital: Vec<Decimal>, // cumulative cost basis, deflated if inflation on
}
```

## Errors
- `EngineError::InvalidHorizon` — horizon_months < 1
- `EngineError::InvalidPlanningAdjustments` — planning vec length != horizon_months

## Notes for the API handler (projection.rs)
- `contribution_frequency = "weekly"` assets: multiply fixed amount by `52/12` before building `SimAsset`
- Planning flows with `due_date`: placed in their calendar month. Flows without `due_date`: spread over 90 days from ref_date
- Horizon derivation: `projection_target_age` → `(target_age - user_age_years) * 12`; fallback 240 months (20 years). `horizon_basis` field reports reason: `mac_target_age` | `mac_fallback_no_demographics` | `months_override`
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`
- **Retirement drawdown**: `RetirementInput { target_age, birth, horizon_months }` passed to `build_installation_projection_input`. The handler computes `retirement_start_month` from `projection_target_age` + birth date. Income drops to `income_retirement_monthly` (sum of `budget_entries` where `persists_after_retirement = true`) from that month onward. `retirement_monthly_withdrawal` is always 0 — income reduction alone drives the portfolio drain. FIRE target for the UI is computed by the frontend: `max(0, expense - income_retirement) × 12 / SWR` (annual_expense mode) or `max(0, income - income_retirement) × 12 / SWR` (current_income mode).
