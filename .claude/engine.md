# Projection Engine (crates/engine)

Pure Rust crate — no I/O, no DB, no async. Pure financial math (projection + history interpolation).
Only `Decimal` arithmetic. Two modules:
- `projection.rs` — monthly net-worth / FIRE simulation (this doc's main subject).
- `history.rs` — pure interpolation of the **historical** net-worth series from manual snapshots
  (see [History interpolation](#history-interpolation-historyrs) below). Deps unchanged
  (`rust_decimal` feature `maths` already present for `powd`).

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
- `EngineError::InvalidHistoryTimeline` — `HistoryTimeline::dates` not strictly ascending

## History interpolation (`history.rs`)

Pure module (no I/O, no async, no clock, **no `f64`** — only `Decimal` + `NaiveDate`) that
reconstructs the past net-worth series from **manual snapshots**. The API handler groups snapshots
into per-`(owner_user_id, kind)` timelines and asks the engine to evaluate each item on a grid of
month-first dates; the handler owns aggregation (Σ per user/household), scoping and the projection
join. The engine only interpolates.

Public API (re-exported from `lib.rs`):
```rust
pub fn evaluate_timeline(&HistoryTimeline, grid_dates: &[NaiveDate]) -> Result<Vec<Vec<Decimal>>, EngineError>
pub fn amortized_segment_value(p_a: Decimal, p_b: Decimal, terms: Option<&LoanTerms>,
                               days_from_start: i64, days_total: i64) -> Decimal
pub fn add_months_signed(date: NaiveDate, delta: i32) -> NaiveDate  // month-first, signed (neg = past)
pub fn month_index_of(date: NaiveDate, anchor_month_first: NaiveDate) -> i32  // (y2-y1)*12 + (m2-m1)
// types: HistoryTimeline { dates, items }, HistoryItem { source_item_id, kind, observations },
//        HistoryObservation { value, terms }, LoanTerms { apr_percent, monthly_payment },
//        HistoryItemKind { Asset, Liability }
```

`HistoryTimeline.dates` are **strictly ascending** (non-ascending → `InvalidHistoryTimeline`); the
LAST date may be a "virtual today" observation appended by the caller — the engine neither knows
nor cares which are virtual. `HistoryItem.observations` is parallel to `dates` (`None` = item not
present in that snapshot; a shorter vec is treated as `None` for the missing indices).

Evaluation rules (per item, per grid point `g`):
- Before the first snapshot `s_1`: `0`, **except** the grid point in `s_1`'s own month
  (`month_first(s_1) ≤ g < s_1`) which "clamps" and evaluates at `s_1` (first visible point is the
  observed value, not a false 0).
- Within a segment `[s_a, s_{a+1}]`: observed at **both** ends → interpolate (**Asset** = linear in
  civil days; **Liability** = `amortized_segment_value`); observed at **one** end only → that
  observed value exactly at its own snapshot date, `0` elsewhere in the segment (items appear /
  disappear without inventing ramps); **neither** → `0`.
- Guarantees **endpoint exactness**: the value at every snapshot date equals the observed value.

Liability interpolation is a **residual-corrected French amortization** curve:
`i = apr/1200`, `u = 1+i`, `f = days_from_start/days_total`, `N = days_total / 30.436875`,
`x = f·N`; `theo(y) = P_a·u^y − M·(u^y−1)/i` (via `Decimal::checked_powd`), `theo_c = max(theo, 0)`;
result `= max( theo_c(x) + f·(P_b − theo_c(N)), 0 )`. The residual term makes `f=0 → P_a` and
`f=1 → P_b` **exact** regardless of `powd` approximation. Falls back to **linear** when `terms` is
`None`, `apr ≤ 0`, `M ≤ 0`, `M ≤ P_a·i` (payment doesn't cover interest), or any checked op fails.
Snapshot mutations are **not** projection-engine inputs — they never touch the projection cache.

## Notes for the API handler (projection.rs)
- Load `allocation_rules` from DB ordered by `priority ASC`, then map each `target_asset_id` → index in `assets[]` before building the engine input.
- Planning flows with `due_date`: placed in their calendar month. Flows without `due_date`: spread over 90 days from ref_date.
- Horizon derivation (`projection_horizon_months`): se resuelve **una** fecha de nacimiento — `users.birth_date` del usuario de sesión, y si es NULL la primera fila de `persons` con `birth_date` por `is_primary DESC, sort_index ASC`. Horizonte = `clamp(90 − edad, 5, 70)` años × 12. Sin fecha de nacimiento: fallback **360 meses (30 años)**. `?months=N` (12–840) lo sobreescribe. `horizon_basis` reporta la razón: `lifespan_90` | `fallback_no_demographics` | `months_override`. (No existe `projection_target_age` — eliminado en v1.0.6.)
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds, **nominal**), `milestones_real` (same thresholds crossed over the **deflated** net worth = euros de hoy; empty when inflation is 0 — the web reuses `milestones`. The web picks the set from the "Inflation Adjusted" toggle), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`. Both milestone sets are computed over the full monthly series (`points_full`), not the decimated `points`, so `reached_month_index` keeps precision under `density=hybrid`. `deflate_points_to_today` mirrors the chart's visual deflation (`ProjectionNetWorthChart.baseSeries`) but at monthly resolution.
- **Retirement drawdown**: el handler pasa **siempre** `retirement_start_month: None` — la jubilación se dispara únicamente cuando `nw_prev` cruza el target FIRE (`fire_reached`, v1.0.6). A partir de ese mes el ingreso cae a `income_retirement_monthly` (suma de `budget_entries` con `persists_after_retirement = true`) y el gasto pasa a `expense_retirement_monthly` (excluye gastos con `ends_at_retirement`). `retirement_monthly_withdrawal` es siempre 0 — la caída de ingresos por sí sola drena la cartera. El target FIRE lo computa el **servidor** (`compute_fire_target_nw` → `jubilacion_target_net_worth` en el response): `neto = expense_retirement − income_retirement` (modo annual_expense) o `neto = income − income_retirement` (modo current_income); si `neto ≤ 0` **no hay target** (`None`, no `max(0,…)`); si no, `target = gross_up(neto × 12) / (SWR/100)`. El frontend duplica la fórmula solo para el preview en vivo del formulario (paridad garantizada por `apps/api/tests/fixtures/fire-parity.json`).

## Performance notes (handler ↔ engine boundary)
- `project_net_worth_series` is CPU-bound (840 months × N assets × `Decimal::powd`). The handler wraps it in `tokio::task::spawn_blocking` to avoid blocking the reactor.
- `compound_outpaces_true_savings_month` is a **second projection pass** with `planning_adj = 0` and `liability.monthly_payment = 0` so the marker compares `market_growth` against a clean `income − expense` baseline. Eliminating the double pass would change the indicator's semantics; instead the handler runs both projections in parallel with `tokio::join!(spawn_blocking, spawn_blocking)`.
- The gross-up of net-annual FIRE through tax brackets uses a **closed-form per-bracket solver** (no binary search). `gross = (net − r·prev_ceiling + K) / (1 − r)`, advancing one bracket at a time until the candidate fits. Old code used 90 iterations of binary search on `Decimal`.
- `build_installation_projection_input` returns a `BuiltProjection` struct that carries `input`, `monthly_net_regular`, `asset_id_name` (Vec<(Uuid, String)>) and `planning_rows`. The handler reuses those instead of issuing a second `SELECT id, name FROM assets` and a second `SELECT planning_flows` (deleted with Fase 2.3).
- Initial queries in `get_projection_series` (installation row, user birth_date, household birth_date) run concurrently via `tokio::try_join!`.
