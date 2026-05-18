# Projection Engine (crates/engine)

Pure Rust crate — no I/O, no DB, no async. Only `Decimal` arithmetic.

## Public API

```rust
// Main projection: returns net_worth and contributed_capital series (len = horizon_months + 1, index 0 = today)
pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError>

// Returns nominal contributions routed to each asset in the FIRST simulated month only
pub fn first_month_per_asset_contribution_nominals(input: &ProjectionInput) -> Result<Vec<Decimal>, EngineError>

// Único helper para evaluar el target FIRE inflado en un `month_index` dado (0 = punto de
// partida, 12 = un año después). Lo consumen tanto el motor (para `fire_reached`) como el
// handler (para construir `fire_target_series`). Antes había una fórmula duplicada — el motor
// usaba `years = (k-1)/12` y el handler `years = month_index/12`, lo que generaba un off-by-one
// de un mes entre cuándo se disparaba la jubilación y la serie pintada en el chart.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal>
```

## ProjectionInput fields
```rust
pub struct ProjectionInput {
    pub ref_date: NaiveDate,           // Civil "today" from installation calendar_tz
    pub horizon_months: u32,           // >= 1
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    pub allocation_rules: Vec<AllocationRule>,   // cascade, in priority order
    pub liabilities: Vec<ProjectionLiabilityInput>,
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
    pub retirement_start_month: Option<u32>,
    pub income_retirement_monthly: Decimal,
    pub expense_retirement_monthly: Decimal,
    pub retirement_monthly_withdrawal: Decimal,
    pub fire_target: Option<FireTarget>,
}

pub struct FireTarget {
    pub base_amount: Decimal,             // FIRE en euros de hoy (gross-up de impuestos aplicado)
    pub annual_inflation_percent: Decimal, // 0 = target plano; > 0 = target móvil
}
```

## Inflación y target FIRE móvil
- Ingresos, gastos y aportaciones se mantienen **constantes en euros nominales** a lo largo de la simulación (filosofía «haciendo lo que hago ahora, ¿qué tal voy?»). No se inflan.
- El rendimiento de activos (`expected_annual_return_percent`) es **nominal**, sin deflactar.
- El **target FIRE crece con la inflación cada mes**: `target(k) = base_amount × (1 + annual_inflation_percent/100)^((k-1)/12)`. Esto preserva el poder adquisitivo del usuario en el momento de jubilarse.
- `annual_inflation_percent = 0` degenera a un target plano (equivalente a tratar el FIRE como un escalar de euros de hoy).

## SimAsset fields
- `expected_annual_return_percent`: **nominal** compound growth rate (7 = 7%/year). None = no compound growth.
- `is_liquid`: liquid assets are drained first when cash is negative; sorted by growth rate (lowest first).
- `purchase_price`: optional cost basis; included in `contributed_capital[0]`.

## AllocationRule fields
```rust
pub struct AllocationRule {
    pub target_index: usize,            // index into ProjectionInput.assets
    pub kind: AllocationKind,           // Fixed | Percent | Remainder
    pub amount: Option<Decimal>,        // €/mes (Fixed); 0..=100 (Percent); None (Remainder)
    pub cap: Option<AllocationCap>,     // Amount(€) | MonthsExpense(N) | IncomeMultiple(N)
}
```
Rules are evaluated **in vector order** (caller passes them sorted by priority ASC). Per rule:
- Resolve `ceiling` via the cap variant: `MonthsExpense(N)` → `N × (expense + debt_service)`; `IncomeMultiple(N)` → `N × income`; `Amount(v)` → `v`. `None` = no ceiling.
- `cap_room = max(0, ceiling − current_value(target))`. If 0, skip.
- Intent: `Fixed` → `min(amount, remaining)`; `Percent` → `remaining × amount / 100`; `Remainder` → `remaining`.
- `take = min(intent, cap_room?, remaining)` is added to `alloc[target]` and subtracted from `remaining`.

## Simulation loop (per month)
All monetary state is **nominal** throughout (euros del momento). El ajuste por inflación se aplica
únicamente al target FIRE, que crece cada mes para mantener el poder adquisitivo del usuario.

1. Compute `debt_service` = sum of active liability payments (capped by remaining principal).
2. Determine `in_retirement = fire_reached || k >= retirement_start_month`. `fire_reached` compara `nw_prev` contra el target FIRE del mes `k`, que es `base × (1 + inflation/100)^((k-1)/12)`. Si se alcanza, usa `income_retirement_monthly` / `expense_retirement_monthly`; si no, las variantes regulares.
3. `retirement_withdrawal` = `retirement_monthly_withdrawal` if `in_retirement`, else 0.
4. `net_cash = income - expense - debt_service + planning_adj[k] - retirement_withdrawal`.
5. If `net_cash > 0` (surplus): **run the allocation cascade** over `allocation_rules` (see [AllocationRule fields](#allocationrule-fields)). Anything no rule absorbed flows into `surplus_cash` (counted in NW).
6. If `net_cash <= 0` (deficit): drain `surplus_cash` first, then drain liquid assets (lowest-return first).
7. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value — sin deflactar.
8. Reduce liability principals by payments made.

## Output
```rust
pub struct ProjectionOutput {
    pub net_worth: Vec<Decimal>,         // nominal, euros del momento, index 0..=horizon_months
    pub contributed_capital: Vec<Decimal>, // cumulative cost basis (nominal)
    pub per_asset_series: Vec<Vec<Decimal>>, // value per asset per month (nominal)
}
```

## Errors
- `EngineError::InvalidHorizon` — horizon_months < 1
- `EngineError::InvalidPlanningAdjustments` — planning vec length != horizon_months
- `EngineError::InvalidAllocationRuleTarget` — `target_index` out of bounds of `assets[]`

## Notes for the API handler (projection.rs)
- Load `allocation_rules` from DB ordered by `priority ASC`, then map each `target_asset_id` → index in `assets[]` before building the engine input.
- Planning flows with `due_date`: placed in their calendar month. Flows without `due_date`: spread over 90 days from ref_date.
- Horizon derivation: `projection_target_age` → `(target_age - user_age_years) * 12`; fallback 240 months (20 years). `horizon_basis` field reports reason: `mac_target_age` | `mac_fallback_no_demographics` | `months_override`
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`
- **Retirement drawdown**: `RetirementInput { target_age, birth, horizon_months }` passed to `build_installation_projection_input`. The handler computes `retirement_start_month` from `projection_target_age` + birth date. Income drops to `income_retirement_monthly` (sum of `budget_entries` where `persists_after_retirement = true`) from that month onward. `retirement_monthly_withdrawal` is always 0 — income reduction alone drives the portfolio drain. FIRE target for the UI is computed by the frontend: `max(0, expense - income_retirement) × 12 / SWR` (annual_expense mode) or `max(0, income - income_retirement) × 12 / SWR` (current_income mode).

## Performance notes (handler ↔ engine boundary)
- `project_net_worth_series` is CPU-bound (840 months × N assets × `Decimal::powd`). The handler wraps it in `tokio::task::spawn_blocking` to avoid blocking the reactor.
- `compound_outpaces_true_savings_month` is a **second projection pass** with `planning_adj = 0` and `liability.monthly_payment = 0` so the marker compares `market_growth` against a clean `income − expense` baseline. Eliminating the double pass would change the indicator's semantics; instead the handler runs both projections in parallel with `tokio::join!(spawn_blocking, spawn_blocking)`.
- The gross-up of net-annual FIRE through tax brackets uses a **closed-form per-bracket solver** (no binary search). `gross = (net − r·prev_ceiling + K) / (1 − r)`, advancing one bracket at a time until the candidate fits. Old code used 90 iterations of binary search on `Decimal`.
- `build_installation_projection_input` returns a `BuiltProjection` struct that carries `input`, `monthly_net_regular`, `asset_id_name` (Vec<(Uuid, String)>) and `planning_rows`. The handler reuses those instead of issuing a second `SELECT id, name FROM assets` and a second `SELECT planning_flows` (deleted with Fase 2.3).
- Initial queries in `get_projection_series` (installation row, user birth_date, household birth_date) run concurrently via `tokio::try_join!`.
