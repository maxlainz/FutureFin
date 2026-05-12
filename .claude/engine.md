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
2. `net_cash = income - expense - debt_service + planning_adj[k]`
3. If `net_cash > 0` (surplus): apply fixed contributions (scaled if pool < sum_fixed), distribute remainder by weight
4. If `net_cash <= 0` (deficit): drain surplus_cash first, then drain liquid assets (lowest-return first)
5. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value
6. Reduce liability principals by payments made
7. If inflation active: deflate net_worth and contributed_capital by `(1 + inf/100)^(k/12)`

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
- Horizon derivation: `projection_target_age` → `(target_age - user_age_years) * 12`; fallback 240 months (20 years)
