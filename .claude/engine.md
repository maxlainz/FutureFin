# Projection Engine (crates/engine)

Pure Rust crate — no I/O, no DB, no async. Pure financial math (projection + history interpolation).
Only `Decimal` arithmetic. Three modules:
- `projection.rs` — monthly net-worth / FIRE simulation (this doc's main subject).
- `history.rs` — pure interpolation of the **historical** net-worth series from manual snapshots
  (see [History interpolation](#history-interpolation-historyrs) below). Deps unchanged
  (`rust_decimal` feature `maths` already present for `powd`).
- `runway.rs` — liquidity runway with compounded return + inflation (v2.2.0; **SWR threshold for the
  infinite case** since v2.3.0 — `Indefinite` ⟺ the grossed-up annual withdrawal fits inside
  `swr_pct` × liquid balance; see [Runway](#runway-runwayrs) below). Consumed by `GET /v1/summary`.

## Public API

```rust
// Main projection: returns net_worth and contributed_capital series (len = horizon_months + 1, index 0 = today)
pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError>

// Returns nominal contributions routed to each asset in the FIRST simulated month only.
// Thin wrapper over `first_month_allocation` since 3.8.0 — kept because `GET /v1/assets` uses it.
pub fn first_month_per_asset_contribution_nominals(input: &ProjectionInput) -> Result<Vec<Decimal>, EngineError>

// Full resolution of the FIRST month's cascade (3.8.0): what gets distributed, where it comes
// from, what no rule absorbed, and a per-rule trace. Added because the old function returned only
// `per_asset` and threw away both the `leftover` (already computed) and the base — which made it
// impossible to explain why the month-1 contribution does not match the summary's recurring net.
// The gap is `planning_component`, and it is also why that number CHANGES EVERY DAY.
pub struct FirstMonthAllocation {
    pub per_asset: Vec<Decimal>,
    pub base_cash: Decimal,            // what the cascade really distributes (`net_cash_month`)
    pub recurring_net: Decimal,        // income − expense − debt_service (stable)
    pub planning_component: Decimal,   // planning_adjustment[0] − retirement_withdrawal (transient)
    pub debt_service: Decimal,
    pub leftover: Decimal,             // ends up in `surplus_cash`
    pub rules: Vec<RuleOutcome>,
}
pub fn first_month_allocation(input: &ProjectionInput) -> Result<FirstMonthAllocation, EngineError>
// 4.0.0 — resuelve el estado del mes 1 EXACTAMENTE como el bucle de simulación: si el patrimonio
// de partida (Σ activos − Σ principales) ya cruza `fire_target_at_month_index(fire_target, 0)`,
// usa ingreso y gasto DE JUBILACIÓN y el retiro mensual, igual que hace `project_net_worth_series`.
// Antes solo miraba `retirement_start_month` e ignoraba `fire_target`, así que en un hogar ya por
// encima de su número FIRE `GET /v1/assets` y `/v1/allocation-rules/resolution` publicaban una
// aportación CON EL SIGNO CONTRARIO al de la proyección —«aportas 2.000 €/mes» sobre un activo que
// la simulación reduce ese mismo mes— y explicaban regla a regla una cascada que no se ejecuta
// jamás. Sostenido en todo el horizonte, y no es un caso raro: es el estado final del público al
// que sirve la app.

// Per-rule trace. `amount_intent` vs `amount_resolved` separates "trimmed by a cap" (not a skip,
// and the most-asked question) from "skipped". Skip reasons are deliberately NOT collapsed —
// they have different remedies: NoCash = "you have no surplus" (touch income/expense);
// NotReached = "the rules above ate it" (touch priorities/caps); CapFull = "the target asset is
// at its ceiling"; ZeroAmount = "the rule resolves to 0"; InvalidTarget = defensive.
pub struct RuleOutcome {
    pub rule_index: usize,             // the engine knows no UUIDs; the handler maps identity
    pub target_index: usize,
    pub amount_intent: Decimal,
    pub amount_resolved: Decimal,
    pub cap_ceiling: Option<Decimal>,
    pub cap_room: Option<Decimal>,
    pub skipped_reason: Option<AllocationSkipReason>,
}

// Único helper para evaluar el target FIRE inflado en un `month_index` dado (0 = punto de
// partida, 12 = un año después). Lo consumen tanto el motor (para `fire_reached`) como el
// handler (para construir `fire_target_series`). Antes había una fórmula duplicada — el motor
// usaba `years = (k-1)/12` y el handler `years = month_index/12`, lo que generaba un off-by-one
// de un mes entre cuándo se disparaba la jubilación y la serie pintada en el chart.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal>

// Liquidity runway (v2.2.0): months the liquid assets cover the monthly expense, compounding the
// assets' expected return and inflating the expense. See the Runway section below.
// NOT an infinity sentinel (v2.3.0): the finite loop's cap. Surviving it returns `Months(1200)`,
// a FLOOR ("at least 100 years"); only the SWR threshold yields `Indefinite`.
pub const MAX_RUNWAY_MONTHS: u32 = 1200;
pub enum RunwayOutcome { Months(Decimal), Indefinite, NoExpenseBase }
pub fn liquid_runway_months(
    liquid_assets: &[(Decimal, Option<Decimal>)], // (current_value, expected_annual_return_percent)
    monthly_expense: Decimal,
    annual_inflation_percent: Decimal,
    swr_pct: Decimal,              // installation fire_settings.swr_pct (%), v2.3.0
    annual_expense_for_swr: Decimal, // ANNUAL expense already grossed up by the handler, v2.3.0
) -> RunwayOutcome
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
    pub liabilities: Vec<ProjectionLiabilityInput>,   // see per-mode contract note below
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

**Per-mode liability contract (handler-side, engine unchanged — reform 3.4.0):** the engine always
subtracts every input liability's `principal` from net worth each month, and only charges cash /
amortizes when `monthly_payment > 0` and the plan is active. The HANDLER exploits that: in mode A it
passes the real payment plan (debt service charged, principal amortizes, cuota freed at
`payment_end_date`); in the real modes B/C (`savings_source.uses_transactions()`) it zeroes
`monthly_payment` in memory, so the principal becomes a **constant** net-worth subtraction across the
whole horizon — paid cuotas already live inside the raw 12m expense average. The projection input
query also filters expired liabilities (`payment_end_date IS NULL OR >= today`), same predicate as
`/v1/summary`. See `build_installation_projection_input` in `apps/api/src/handlers/projection.rs`.

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
5. If `net_cash > 0` (surplus): **run the allocation cascade** over `allocation_rules` (see [AllocationRule fields](#allocationrule-fields)). Anything no rule absorbed flows into `surplus_cash` (counted in NW). `distribute_contributions` takes an optional trace sink (`Option<&mut Vec<RuleOutcome>>`): the loop passes `None` — it runs up to 840 times per request and nobody reads the trace there — while `first_month_allocation` passes `Some`. **One cascade implementation, not two**: a second one would diverge silently at the first cap change, and an explanation that disagrees with what the engine does is worse than no explanation. The cascade **cannot over-allocate**: `take` is bounded three times (rule intent, cap room, remaining cash) and the loop breaks when cash runs out.
6. If `net_cash <= 0` (deficit): drain `surplus_cash` first, then drain liquid assets (lowest-return first).
7. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value — sin deflactar. `monthly_multiplier` = raíz 12ª del factor anual `1 + p/100`; `None` y `0` → factor 1; **las tasas negativas componen de verdad** (−50 % anual ⇒ ×0,5 en 12 meses); `p ≤ −100` se clampa a factor 0 (la capa API rechaza esos inputs con error tipado).
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
pub fn anchored_cashflow_segment_value(v_a: Decimal, v_b: Decimal, cf: &[CashFlowEntry],
                               seg_start: NaiveDate, seg_end: NaiveDate, eval_date: NaiveDate,
                               days_from_start: i64, days_total: i64) -> Decimal   // v1.6.0, tier-2
pub fn add_months_signed(date: NaiveDate, delta: i32) -> NaiveDate  // month-first, signed (neg = past)
pub fn month_index_of(date: NaiveDate, anchor_month_first: NaiveDate) -> i32  // (y2-y1)*12 + (m2-m1)
// types: HistoryTimeline { dates, items }, HistoryItem { source_item_id, kind, observations, cashflow },
//        HistoryObservation { value, terms }, LoanTerms { apr_percent, monthly_payment },
//        HistoryItemKind { Asset, Liability }, CashFlowEntry { date, delta }
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

### Cash-flow anchoring (tier-2, v1.6.0)
`HistoryItem` gained an optional `cashflow: Vec<CashFlowEntry>` field (`#[serde(default)]`). A
`CashFlowEntry { date, delta }` is a dated cash movement that **shapes** an asset's curve **within**
its segment **without ever contradicting the snapshots** — the anchored curve still passes exactly
through both endpoints. `delta` is already sign-normalized by the caller (**positive raises** the
asset value; account leg = `+amount`, savings-destination leg = `−amount`); the engine never
interprets signs or sources, it only sums `delta`.

`anchored_cashflow_segment_value` computes, for an **asset** segment `[seg_start, seg_end]` observed
at both ends:

```
v(t) = Va + C(a→t) + f(t)·(Vb − Va − C_total)
```

- `C(a→t)` = Σ of `delta` over the **half-open** interval `(seg_start, eval_date]` (a txn dated on
  `seg_start` belongs to the *previous* segment; one dated on `seg_end` **does** count).
- `C_total = C(a→b)` = Σ of `delta` over `(seg_start, seg_end]`.
- `f(t) = days_from_start / days_total`, linear in civil days — the **same** base as
  `interpolate_linear` (same `clamp`, same division).

Properties (unit-tested as P1–P5 in `history.rs`):
- **P1 / P2 — endpoint exactness for arbitrary cash-flow**: `v(seg_start) = Va` (empty `(a→a]`,
  residual term ×0) and `v(seg_end) = Vb` **exactly** (`C(a→b) = C_total` cancels the residual;
  `f = n/n = 1`, no residual division). Holds for deltas that don't sum to zero, a delta dated on
  `seg_end`, etc.
- **P3 — empty ⇒ identical to `interpolate_linear`**: with `cashflow` empty the formula degenerates
  to `Va + f·(Vb − Va)`; moreover the caller (`evaluate_item_at`) only takes the anchored branch
  when some entry falls in `(d_a, d_b]`, otherwise it calls `interpolate_linear` **verbatim** — so a
  timeline with an empty (default) `cashflow` field reproduces the previous history series **bit for
  bit** (P3b).
- Deposit into flat snapshots (`Va = Vb`) jumps just after the deposit date, then decays linearly
  back to `Va` by `seg_end` (the snapshot wins; the inflow is re-absorbed).

**Liabilities and one-sided items ignore cash-flow, deliberately**: only the `(Some, Some)` **Asset**
arm consults `cashflow`. Liabilities already model the principal with residual-corrected French
amortization — injecting the payment as a delta would double-count it — so they stay bit-for-bit
identical to the no-cash-flow curve; items observed at a single endpoint keep their appear/disappear
behavior. Implementation: `O(n)` linear scan over `cf` per evaluation point (no prefix sums, robust
to any input order), sub-ms at this scale, no `f64`.

## Runway (`runway.rs`)

Pure module (v2.2.0) that answers "how many months do the **liquid** assets cover the monthly
expense?" while compounding the assets' expected return and inflating the expense. Sole consumer:
`GET /v1/summary` → `financial_health.runway_months` / `runway_is_indefinite`
(`apps/api/src/handlers/summary.rs`). Public API in the block above; 13 unit tests in-module
(as of 2026-08-15, v2.3.0).

| Input | Meaning |
|---|---|
| `liquid_assets: &[(Decimal, Option<Decimal>)]` | One row per liquid asset: `(current_value, expected_annual_return_percent)`. The handler passes exactly the rows of `assets WHERE is_liquid = true` in the requested scope. |
| `monthly_expense: Decimal` | Total monthly expense to cover — in the handler, `expense_total_monthly_equivalent` (so it follows `savings_source`). |
| `annual_inflation_percent: Decimal` | `installation.annual_inflation_assumption_percent`, clamped to ≥ 0 by the handler. |
| `swr_pct: Decimal` (v2.3.0) | `installation.fire_settings.swr_pct` (in %) — the **same** safe-withdrawal rate the FIRE target uses (Jubilación tab), read via `installation_calendar_inflation_fire`. Only drives the infinite case. |
| `annual_expense_for_swr: Decimal` (v2.3.0) | The **annual** expense already grossed up for taxes by the handler: `gross_up_net_annual_fire(expense_total × 12, fire.tax_brackets, fire.taxes_enabled)` — the *same* gross-up as the FIRE target. With `taxes_enabled = false` it is plainly `12 × monthly_expense`. The engine never recomputes `12 × monthly_expense` itself. |

Model (each rule exists for a reason — do not "simplify" one away):

- **Nominal frame**: assets grow at their *nominal* expected return and the expense is inflated every
  month. The result is a count of months (frame-invariant), but mixing nominal returns with a
  constant expense would overstate the runway.
- **Withdraw-then-grow order**: each month pays the expense first and grows what is left — the same
  order as the simulation loop in `projection.rs` (negative cash flow drains before the multipliers
  apply), so both curves stay coherent.
- **Value-weighted multiplier**: `m = Σ vₐ·monthly_multiplier(rₐ) / Σ vₐ`, i.e. a **prorated drain**
  (every asset funds the expense in proportion to its weight). Slightly **conservative** versus the
  engine's real drain, which empties the lowest-return liquids first and therefore keeps the
  high-return ones longer. Deliberate: the KPI must not promise more than the simulation.
- **Negative rates compound**: inherited from `monthly_multiplier` (shared with the simulation via
  `pub(crate)`, so the runway uses *exactly* the engine's annual→monthly conversion). A negative
  expected return (−100 < r < 0) now decays the balance for real and **shortens** the runway;
  `r ≤ −100` clamps to factor 0. The expense-inflation argument is never negative here (the
  installation validates 0..50).
- **SWR threshold (the infinite case, v2.3.0)**: `Indefinite` ⟺ the grossed-up annual withdrawal does
  not exceed the SWR applied to the starting balance, `annual_expense_for_swr ≤ A·(swr_pct/100)`.
  Compared **without dividing** — `annual_expense_for_swr·100 ≤ A·swr_pct` — so the boundary is
  *exact* in `Decimal`. It is the liquidity "FIRE number": `A ≥ gross_expense / SWR`. `swr_pct ≤ 0`
  can never satisfy it (right side ≤ 0, left side > 0), so no separate guard is needed. Beware: the
  `100` de-percentages `swr_pct` and is unrelated to `MAX_RUNWAY_MONTHS`, even though `12·100 = 1200`.
- **Check order (contract)**: `NoExpenseBase` (expense ≤ 0) → `Months(0)` (balance ≤ 0) → SWR
  threshold → finite loop. `NoExpenseBase` must come **first**: with expense 0 the inequality
  `0 ≤ A·swr` is trivially true and would report an undefined runway as infinite.
- **The trigger is deliberately independent of return and inflation**: it looks only at `A`, the
  grossed expense and the SWR — the definition of SWR already assumes a portfolio whose real return
  sustains that withdrawal long-term. Return and inflation still govern the **finite** case (the loop
  below). Accepted consequence: a balance below the threshold with a huge return is no longer
  "infinite", and one exactly at the threshold with 0 % return is.
- **100-year cap is a floor, not a sentinel**: surviving `MAX_RUNWAY_MONTHS` (1.200) months without
  meeting the SWR threshold returns `Months(1200)` — read as "at least 100 years", not an exact
  measure and **not** `Indefinite` (the UI renders it «+100 años»). Still no epsilon and no closed
  form: `ln`-based closed forms suffer cancellation exactly at the `A·j → g` boundary; the monthly
  loop avoids it and costs microseconds.
- **Exact reduction to `A / g`** (when the SWR threshold is *not* met, i.e. the finite branch): with
  return and inflation 0, `m = m_inf = 1` and the final fractional month reconstructs `A/g` with a
  single division — bit-exact **inside the engine**, which is where the property lives.
  Since 3.8.0 the HTTP surface publishes `runway_months` rounded to **1 decimal**
  (`handlers/summary.rs`, aligned with `sim_kpis` in `handlers/projection.rs`, which already did),
  so the baseline tests assert `(A/g).round_dp(1)`: still no tolerance, just the published
  precision. Anything that needs the full value must call `liquid_runway_months` directly.

  Wire-side consequence worth knowing: a runway below `0,05` months now serializes as `"0.0"`
  instead of a long non-zero decimal. `SummaryView` no longer keys the Runway tile off a
  zero-check for exactly this reason — a runway of zero months is information, not missing data.
- Edge cases: `monthly_expense <= 0` → `NoExpenseBase` (not "infinite"); total balance ≤ 0 →
  `Months(0)`.

Worked values (engine-verified). Finite branch, 12.000 € liquid vs 1.200 €/month, SWR 3,5 % (all four
below the threshold, unchanged since v2.2.0): return 0 % / inflation 0 % → 10; 5 % / 0 % → 10,19;
0 % / 3 % → 9,89; 5 % / 3 % → 10,07 months. Threshold branch (v2.3.0): 240.000 € vs 700 €/month at
SWR 3,5 % with taxes off → `Indefinite` on the **exact** boundary (840.000 = 840.000); 1.000.000 € at
7 % vs 4.000 €/month at SWR 3,5 % → `Months(1200)` floor, since 48.000 > 35.000 (it was `Indefinite`
in v2.2.0, when the cap decided infinity); with the default ES brackets `gross_up(8.400) ≈ 10.481 €`,
raising the threshold to ≈ 299.457 € of liquid balance.

## Notes for the API handler (projection.rs)
- Load `allocation_rules` from DB ordered by `priority ASC`, then map each `target_asset_id` → index in `assets[]` before building the engine input.
- Planning flows with `due_date`: placed in their calendar month. Flows without `due_date`: spread over 90 days from ref_date.
- Horizon derivation (`projection_horizon_months`): se resuelve **una** fecha de nacimiento — `users.birth_date` del usuario de sesión, y si es NULL la primera fila de `persons` con `birth_date` por `is_primary DESC, sort_index ASC`. Horizonte = `clamp(90 − edad, 5, 70)` años × 12. Sin fecha de nacimiento: fallback **360 meses (30 años)**. `?months=N` (12–840) lo sobreescribe. `horizon_basis` reporta la razón: `lifespan_90` | `fallback_no_demographics` | `months_override`. (No existe `projection_target_age` — eliminado en v1.0.6.)
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds, **nominal**), `milestones_real` (same thresholds crossed over the **deflated** net worth = euros de hoy; empty when inflation is 0 — the web reuses `milestones`. The web picks the set from the "Inflation Adjusted" toggle), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`. Both milestone sets are computed over the full monthly series (`points_full`), not the decimated `points`, so `reached_month_index` keeps precision under `density=hybrid`. `deflate_points_to_today` mirrors the chart's visual deflation (`ProjectionNetWorthChart.baseSeries`) but at monthly resolution.
- **Retirement drawdown**: el handler pasa **siempre** `retirement_start_month: None` — la jubilación se dispara únicamente cuando `nw_prev` cruza el target FIRE (`fire_reached`, v1.0.6). A partir de ese mes el ingreso cae a `income_retirement_monthly` (suma de `budget_entries` con `persists_after_retirement = true`) y el gasto pasa a `expense_retirement_monthly` (excluye gastos con `ends_at_retirement`). `retirement_monthly_withdrawal` es siempre 0 — la caída de ingresos por sí sola drena la cartera. El target FIRE lo computa el **servidor** (`compute_fire_target_nw` → `jubilacion_target_net_worth` en el response): `neto = expense_retirement − income_retirement` (modo annual_expense) o `neto = income − income_retirement` (modo current_income); si `neto ≤ 0` **no hay target** (`None`, no `max(0,…)`); si no, `target = gross_up(neto × 12) / (SWR/100)`. El frontend duplica la fórmula solo para el preview en vivo del formulario (paridad garantizada por `apps/api/tests/fixtures/fire-parity.json`).

## Performance notes (handler ↔ engine boundary)
- `project_net_worth_series` is CPU-bound (840 months × N assets × `Decimal::powd`). The handler wraps it in `tokio::task::spawn_blocking` to avoid blocking the reactor.
- `compound_outpaces_true_savings_month` is a **second projection pass** with `planning_adj = 0` and `liability.monthly_payment = 0` so the marker compares `market_growth` against a clean `income − expense` baseline. Eliminating the double pass would change the indicator's semantics; instead the handler runs both projections in parallel with `tokio::join!(spawn_blocking, spawn_blocking)`.
- The gross-up of net-annual FIRE through tax brackets uses a **closed-form per-bracket solver** (no binary search). `gross = (net − r·prev_ceiling + K) / (1 − r)`, advancing one bracket at a time until the candidate fits. Old code used 90 iterations of binary search on `Decimal`. Desde v2.3.0 `gross_up_net_annual_fire` es `pub(crate)` (`apps/api/src/handlers/projection.rs`) y tiene **dos consumidores**: el target FIRE y el umbral SWR del runway en `summary.rs` (`annual_expense_gross`). Cualquier cambio en los tramos o en el solver mueve **ambos** números a la vez — es intencional: comparten definición fiscal por diseño.
- `build_installation_projection_input` returns a `BuiltProjection` struct that carries `input`, `monthly_net_regular`, `asset_id_name` (Vec<(Uuid, String)>) and `planning_rows`. The handler reuses those instead of issuing a second `SELECT id, name FROM assets` and a second `SELECT planning_flows` (deleted with Fase 2.3). Desde v2.2.0 también expone `effective_savings_source` + (desde 3.9.0) `savings_income_basis` / `savings_expense_basis` — que **sustituyen** al escalar `savings_source_months_with_data`: con ventanas configurables por lado no existe *un* número de meses — (fuente **tras** el fallback, serializadas en `ProjectionSeriesResponse`) y `debt_service_monthly` (cuotas de pasivos activos; **no** es input del engine, que amortiza los pasivos aparte), que consume `assets_projection_context` para los caps `months_expense`.
- Initial queries in `get_projection_series` (installation row, user birth_date, household birth_date) run concurrently via `tokio::try_join!`.
