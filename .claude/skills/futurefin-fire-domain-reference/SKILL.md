---
name: futurefin-fire-domain-reference
description: >
  FIRE/retirement-planning math AS IMPLEMENTED IN FUTUREFIN. Load this skill whenever a task
  touches: the FIRE number / jubilación target, SWR, tax gross-up or Spanish capital-gains
  brackets, the projection engine's nominal-vs-real (inflation) model, fire_target_series,
  jubilacion_month_index, the allocation cascade (fixed/percent/remainder, caps, sobrante),
  retirement drawdown/drain, milestones vs milestones_real, deflation for display, the
  projection horizon rule, fire-parity.json, or any question like "why does the target grow?",
  "why did jubilación move?", "is this number in today-euros?", "how is the surplus split
  across assets?". Also load it BEFORE editing crates/engine/src/projection.rs,
  apps/api/src/handlers/projection.rs or apps/web/src/lib/fire.ts. Do NOT use it as a bug
  triage runbook (futurefin-debugging-playbook), to change the economic model
  (futurefin-change-control + futurefin-projection-realism-campaign), or for env/config axes
  (futurefin-config-and-flags).
---

# FutureFin FIRE Domain Reference

Everything here is the model **as implemented** (verified against code on 2026-07-02,
v1.4.3), not textbook FIRE theory. Line anchors are as of that date; re-verify with the
commands in "Provenance and maintenance" if the files have changed.

Primary sources (ground truth, in this order):
- `crates/engine/src/projection.rs` — the simulation (pure, `Decimal`-only, 22 unit tests).
- `apps/api/src/handlers/projection.rs` — FIRE number, gross-up, horizon, deflation, milestones.
- `apps/api/src/handlers/installation.rs` — `FireSettings` defaults + validation.
- `apps/web/src/lib/fire.ts` — client-side duplicate for the live preview (server stays source of truth).
- `apps/api/tests/fixtures/fire-parity.json` — canonical cases shared by both sides.

Historical note: `projection_target_age` was **removed** in v1.0.6 (migration
`20260516120000_drop_projection_target_age.sql`); FIRE crossover is the sole retirement trigger.
`horizon_basis` strings are `lifespan_90 | fallback_no_demographics | months_override`
(projection.rs:621,626,992). (The docs/comments that still described the old model were fixed on
2026-07-02.)

## 1. Glossary (terms used everywhere in this repo)

| Term | Meaning in FutureFin |
|---|---|
| **FIRE** | "Financial Independence, Retire Early". Reaching a net worth from which withdrawals can fund expenses indefinitely. |
| **SWR** | Safe Withdrawal Rate, `swr_pct`, in **percent** (3.5 = 3.5%). FIRE number = gross annual need / (SWR/100). |
| **FIRE number / target** | The net worth that triggers retirement. Stored per-month as a *moving* target (see §4). API field `jubilacion_target_net_worth` is the base (today-euros) value. |
| **Gross-up** | Converting the *net* annual amount the user needs to spend into the *gross* amount to withdraw, so that after capital-gains tax the net remains. §3. |
| **Nominal euros** | Euros of the moment they occur ("euros del momento"). All engine series are nominal. |
| **Real euros / today-euros** | Nominal divided by `(1+inflation/100)^(years)`. Display-only concept. |
| **Deflation** | The nominal→real division above. Done in the handler (`deflate_points_to_today`) and in the chart, never inside the engine. |
| **Jubilación** | Retirement. UI tab name and API field prefix (`jubilacion_month_index`). Triggered **only** by the FIRE crossover — there is no target-age trigger. |
| **Sobrante / surplus** | Positive monthly net cash (`income − expense − debt_service + planning_adj`). Fed into the allocation cascade. Unrouted surplus accumulates as `surplus_cash` (counted in net worth). |
| **Cascade** | The ordered list of `allocation_rules` that distributes the monthly surplus across assets. §6. |
| **Debt service** | Sum of active liability monthly payments, each capped by remaining principal. |
| **Contributed capital** | Cumulative cost basis: initial `purchase_price` of assets + every euro routed to assets or to `surplus_cash` before retirement. Never includes market growth. |
| **Drawdown / drain** | In deficit months, cash is pulled from `surplus_cash` first, then from assets (liquid, lowest-return first). Retirement is modelled as income dropping, which creates the deficit. |
| **Installation** | The single-household deployment singleton; `fire_settings` and inflation live on its one DB row. |

## 2. The FIRE number: three modes

Server: `compute_fire_target_nw` (projection.rs:137-164). Settings: `installation.fire_settings`
JSONB, deserialized to `FireSettings` (installation.rs:61-73), defaults applied on read by
`resolve_fire_settings` (installation.rs:116-121).

```
need_annual =
  manual         → fire_number_manual_amount                     (must be > 0, else no target)
  annual_expense → (expense_retirement_monthly − income_retirement_monthly) × 12   (≤ 0 → no target)
  current_income → (income_regular_monthly    − income_retirement_monthly) × 12    (≤ 0 → no target)

target_base = gross_up(need_annual, tax_brackets, taxes_enabled) / (swr_pct / 100)
```

Critical input nuances (source of the worst historical bug in this area):
- `annual_expense` uses **`expense_retirement_monthly`** = sum of budget expense entries with
  `ends_at_retirement = false` (budget.rs:355,382) — NOT the full regular expense. The handler
  passes it at projection.rs:657-659. Pre-v1.3.0 the web preview passed
  `expense_regular_monthly_equivalent` instead → 2-3× divergence between preview and server target.
- `income_retirement_monthly` = sum of income entries with `persists_after_retirement = true`
  (budget.rs:354,376) — e.g. rental or pension. It is subtracted because that income keeps
  covering expenses in retirement, so the portfolio only needs to fund the difference.
- No target (`None`) is a **valid outcome** (retirement income covers expenses): the API returns
  `jubilacion_target_net_worth: null`, empty `fire_target_series`, `jubilacion_month_index: null`.

### 2b. Mode B: `savings_source = transactions_avg` — need & net from the real 12-month average

`fire_settings.savings_source` (default `budget` = everything above) can be flipped to
`transactions_avg` (mode B, added Unreleased). It changes **where the pre-retirement income/expense
scalars come from**, and therefore both the FIRE need and the simulation's monthly net. The gross-up,
SWR, moving-target and drain formulas are **unchanged** — only the base numbers differ.

- **Window & average**: weighted mean over `[first-of-month(today) − 12 months, first-of-month(today))`
  — the 12 **complete** calendar months before the current one (the running month is excluded).
  Denominator = `months_with_data` (months in the window with ≥1 transaction of any kind), same
  weighted semantics as the Movimientos comparison → a short history does not dilute the mean.
  Helper `transactions_12m_avg` (`handlers/transactions/summary.rs`).
- **Hybrid liability subtraction**: `expense_eff = max(0, expense_avg − Σ per active liability)`,
  where each active liability (filtered by `payment_end_date`) contributes its **real linked-txn
  average** (`linked_liability_id`) if any exist, otherwise its **nominal monthly payment**
  (`liability_monthly_payment`). Single source of truth `effective_avg_income_expense`, shared by
  `projection.rs` and `summary.rs`. The engine still receives liabilities as `debt_service`, so the
  saving **steps up automatically when a loan ends** (verified by test).
- **Target**: `annual_expense` uses `expense_eff` as the base (mode A used `expense_retirement`);
  `current_income` uses `income_eff` (`income_avg`); `manual` is unchanged. This is a **deliberate,
  semantic change of base** — mode A's `expense_retirement` is a budget line, mode B's `expense_eff`
  is measured spending net of debt service. `end_adj` (budget end-date adjustments) is zeroed in
  mode B; `planning_flows` (`flow_adj`) still apply.
- **Documented mismatch**: mode B only changes the **accumulation** phase. The **retirement** phase
  still draws `income_retirement` / `expense_retirement` from the **budget** in both modes (the drain
  step at §5 is unchanged). So the target is derived from real spending while the drawdown that must
  fund it is budget-based — an accepted, intentional asymmetry.
- **Fallback**: `months_with_data == 0` in mode B → silently reverts to the budget scalars (mode A
  effective). `GET /v1/summary` reports the **effective** source in `financial_health.savings_source`
  (so it can read `"budget"` even when the setting is `transactions_avg`).
- **Preview parity**: the web preview must consume `/v1/summary`'s effective equivalents in mode B
  (`RetirementView`, fetch gated on the mode), never recompute the need from the budget — otherwise
  it re-opens the client/server divergence class of §2 (see §2.5 of failure-archaeology). Tripwire
  case in `fire-parity.json`: `expense_retirement 2137.5 → expected_target_nw 923327.306` (proves both
  sides derive the same need from an avg-style, non-round expense base).

Defaults (Spain, `default_fire_settings`, installation.rs:81-114): mode `annual_expense`,
`swr_pct = 3.5`, `taxes_enabled = true`, 5 IRPF capital-gains brackets:
19% to 6.000, 21% to 50.000, 23% to 200.000, 27% to 300.000, 30% open-ended.

Validation (`validate_fire_settings`, installation.rs:123-190): `swr_pct` bounded **0–4 percent**
(a 5% SWR is rejected with 400); manual mode requires a positive manual amount; when
`taxes_enabled`, brackets must be non-empty, pct in 0–99, `up_to` strictly increasing, and the
**last bracket must be open-ended (`up_to: null`)** — the closed-form solver in §3 relies on this.
`swr_pct = 0` passes validation but yields no target (compute returns `None` for swr ≤ 0).
`fire_number_mode` deserialization is strict since v1.3.0 (unknown value → 422); the legacy alias
`annual_expense_adjusted` maps to `annual_expense` on BOTH sides: the server's strict
`Deserialize` accepts it (installation.rs:41, "Alias preservado para importar backups antiguos";
pinned by the test in `apps/api/tests/installation_patch.rs`) and the web normalizer mirrors it
(fire.ts:54). Do not "tighten" the server deserializer — old-backup imports depend on the alias.

## 3. Tax gross-up through capital-gains brackets

**Why**: when the user withdraws from the portfolio, capital-gains tax is due on the gross
amount (simplified model: the whole withdrawal is taxed through the brackets). To *net*
`need_annual` euros, the target must fund a larger *gross* withdrawal. So the FIRE number is
computed on the grossed-up figure. With `taxes_enabled = false`, gross = net.

**Server (closed form)** — `gross_up_net_annual_fire` (projection.rs:106-135). The after-tax
function is piecewise linear, so no iteration is needed. Walk brackets in order keeping
`prev_ceiling` (lower edge of current bracket) and `K` (cumulative tax of all full brackets
below); in the bracket with rate `r`:

```
gross_candidate = (net + K − r · prev_ceiling) / (1 − r)
if gross_candidate ≤ bracket ceiling → that is the answer
else K += r × bracket_width; advance to next bracket
```

The open last bracket guarantees termination. Degenerate `r ≥ 100%` returns `prev_ceiling`.

**History**: until v1.3.0 the server ran a 90-iteration binary search on `Decimal`. The closed
form replaced it with **identical results ±0.01 €** — proven by the regression test
`closed_form_matches_binary_search_across_es_brackets` (projection.rs:1583-1613), which keeps
the old binary search alive as a reference implementation.

**Client**: `apps/web/src/lib/fire.ts` still uses the 90-iteration **binary search** on `number`
(`grossUpNetAnnualFire`, fire.ts:119-134). That is deliberate (preview-only, float math); do not
"fix" one side without running both parity suites (§8). `taxOnGrossCapitalAnnual`
(fire.ts:86-117) mirrors the server's `tax_on_gross_capital_annual` (projection.rs:71-97,
test-only on the server side).

## 4. The nominal model (v1.2.0, current)

The single most important idea in the codebase. Philosophy: **"haciendo lo que hago ahora,
¿qué tal voy?"** — "if I keep doing exactly what I do today, how does it go?".

- Income, expenses, contributions and planning flows stay **constant in nominal euros** for the
  whole horizon. They are never inflated.
- Asset returns (`expected_annual_return_percent`) are **nominal**, compounded monthly via
  `(1 + p/100)^(1/12)` (projection.rs:154-162). Note: rates ≤ 0 are treated as **no growth**
  (factor 1) — the engine does not model negative expected returns.
- **Only the FIRE target moves with inflation.** `FireTarget { base_amount,
  annual_inflation_percent }` (projection.rs:83-87); the handler clamps inflation to ≥ 0
  (projection.rs:662,983).

The single formula — `fire_target_at_month_index` (projection.rs:171-182), public in the engine
crate and consumed by BOTH the engine and the handler:

```
target(month_index) = base_amount × (1 + inflation/100)^(month_index / 12)
month_index = 0 → base_amount.   inflation = 0 → flat target at every month.
```

**The off-by-one story** (why one helper): before v1.3.0 the engine computed the trigger with
`years = (k−1)/12` while the handler built `fire_target_series` with `years = month_index/12` as
a *separate duplicated formula* — the plotted target curve and the actual retirement trigger
disagreed by one month. Fix: one public helper; the engine passes `k−1` (it compares `nw_prev`,
the close of month k−1, against the target at that same axis point — projection.rs:475-481), the
handler passes the point's own `month_index` (projection.rs:1129,1140). Regression test:
`fire_target_helper_matches_compound_factor_at_year_boundaries` (projection.rs:1092-1113).

**What rewrite #1 got wrong** (v1.0.12, "real pure" model — replaced in v1.2.0): it ran the
whole simulation in today-euros by deflating each asset's return
(`r_real = (1+r_nom)/(1+inf) − 1`) while keeping flows nominal-fixed and the target flat. The
mixture produced incoherent, *silently plausible* output — e.g. assets draining before
retirement when inflation was on. v1.2.0 made everything nominal and moved all inflation into
the target. Guard test: `fire_target_with_inflation_does_not_trigger_early_drain`
(projection.rs:981-1021). If you ever touch inflation semantics, that history says: pick ONE
frame (nominal) and keep every series in it.

**Inflation-invariance of the crossing**: `nominal_nw(k) ≥ base × (1+i)^(k/12)` is algebraically
the same comparison as `deflated_nw(k) ≥ base`. So the jubilación month is identical whether you
look at the nominal chart with the moving target or the deflated chart with a flat target — the
"Inflation Adjusted" display toggle never moves the jubilación marker (CHANGELOG v1.4.2).
(Changing the inflation *value* does move it, of course.) Milestones are NOT invariant: real
milestones are reached later than nominal ones (test at projection.rs:1446-1482), hence the
separate `milestones_real` field.

## 5. The monthly simulation loop, step by step

`project_net_worth_series` (projection.rs:386-575). Series index 0 = today's state; month `k`
(1-based) simulates the calendar month `month_first(ref_date) + (k−1)`. Net worth identity
(projection.rs:437-441): `NW = Σ asset values + surplus_cash − Σ liability principals −
undrained_cumulative`.

For each month `k = 1..=horizon_months`:

1. **Debt service** (458-471): for each liability active this month (`payment_end` is null or
   ≥ month start, payment > 0), pay `min(monthly_payment, remaining principal)`; sum.
   Weekly payments were normalized to monthly (`×52/12`) by the handler (projection.rs:572-580).
2. **Retirement check** (475-481): `fire_reached = nw_prev ≥ target(k−1)` via
   `fire_target_at_month_index`. `in_retirement = fire_reached || k ≥ retirement_start_month`.
   The handler always passes `retirement_start_month = None` and
   `retirement_monthly_withdrawal = 0` (projection.rs:790,793) — FIRE crossover is the sole
   trigger, and the drain is driven purely by the income drop. Once NW dips below the target
   again, `in_retirement` can flip back off next month (the model re-checks every month; there
   is no latch).
3. **Pick the budget** (482-497): in retirement use `income_retirement_monthly` /
   `expense_retirement_monthly`; otherwise the regular pair.
4. **Net cash** (499-503): `income − expense − debt_service + planning_adj[k−1] −
   retirement_withdrawal`. Planning flows with a `due_date` land in their calendar month; undated
   ones are spread over 90 days from `ref_date` (projection.rs:331-384). Budget expenses with an
   `expense_end_date` are cancelled from the following month via positive adjustments
   (projection.rs:386-406).
5. **Surplus** (`net_cash > 0`, not retired) (517-533): run the cascade (§6). Routed euros are
   added to asset values AND to `contributed_capital`; cascade leftover goes to `surplus_cash`
   and ALSO counts as contributed. **In retirement** (514-516) any surplus goes to
   `surplus_cash` only — no contributions, not counted as contributed capital.
6. **Deficit** (`net_cash ≤ 0`) (505-513): drain `surplus_cash` first; remaining need drains
   assets via `drain_from_assets` (184-216) — order: **liquid before illiquid, and within each
   group lowest `expected_annual_return_percent` first** (ties by input order; illiquid assets
   ARE drained if liquids run out). Any shortfall the assets cannot cover accumulates in
   `undrained_cumulative`, which is *subtracted* from NW (implicit debt).
7. **Compound growth** (535-538): each asset value ×= monthly multiplier. Growth applies AFTER
   this month's contribution/drain.
8. **Principal reduction** (540-555): each active liability's principal −= its payment.
9. Push NW, contributed capital, and per-asset values (557-567).

`contributed_capital[0]` = sum of positive asset `purchase_price` (424-432).

## 6. Allocation cascade semantics

`distribute_contributions` (projection.rs:249-305) + `resolve_cap_ceiling` (220-235). Rules come
from the `allocation_rules` table ordered `priority ASC, id ASC`, `enabled = true` only, with
`target_asset_id` resolved to an index (projection.rs:679-770).

Per rule, in order, over the `remaining` surplus:
- Resolve the cap ceiling: `Amount(v)` → `v`; `MonthsExpense(N)` → `N × (expense + debt_service)`;
  `IncomeMultiple(N)` → `N × income`. `cap_room = max(0, ceiling − live value of target asset)`;
  room 0 → skip the rule. Live values update as the cascade runs, so multiple rules into the
  same asset share one ceiling (test projection.rs:814-840).
- Intent: `fixed` → `min(amount, remaining)`; `percent` → `remaining × pct/100` (**percent of
  what is left at this step**, not of the original surplus); `remainder` → all of `remaining`.
- `take = min(intent, cap_room, remaining)`; add to target, subtract from remaining.
- Whatever no rule absorbs is returned as leftover → `surplus_cash`.

**The uncapped-remainder sink invariant** (enforced by the API handler, not the engine —
`apps/api/src/handlers/allocation_rules.rs:387-402,563-581,652-658,722-733`): every scope must
keep **exactly one** `remainder` rule with no cap, and it must be **last** in the cascade. Create
rejects a second sink, update/delete reject orphaning the scope, reorder rejects a non-last sink.
The engine itself tolerates zero rules (surplus → cash, test projection.rs:643-653); the
invariant lives one layer up. (This cascade replaced per-asset contribution config in v1.1.0
with a signed-off, no-data-migration column drop — see futurefin-change-control.)

## 7. Display math: deflation, milestones, horizon

All anchors in this section are in the HANDLER file `apps/api/src/handlers/projection.rs`
(not the engine file of the same name used in §5).

- **Deflate by `month_index`, never by array index.** `deflate_points_to_today`
  (handlers/projection.rs:466-486) divides by `(1+i/100)^(month_index/12)`. v1.4.2 bug: the web chart
  deflated by array index — invisible with `density=monthly` (indices coincide), wrong with
  `density=hybrid` (points 0..12 monthly then annual: non-equidistant). Any code touching
  decimated series must use `month_index`.
- **Milestones** (nominal) and **`milestones_real`** (same 1/2.5/5×10^n thresholds crossed on the
  deflated series) are both computed on the FULL monthly series (`points_full`), not the
  decimated one, to keep `reached_month_index` exact under `hybrid`
  (projection.rs:1062-1118). `milestones_real` is empty when inflation = 0 and the web reuses
  `milestones`. The FIRE crossover (`jubilacion_month_index`) is likewise detected on the full
  series (projection.rs:1126-1134).
- **Horizon rule** — `projection_horizon_months` (projection.rs:598-627): 90-year lifespan.
  `years = clamp(90 − completed_age, 5, 70)` using the session user's `birth_date`, falling back
  to the primary household person's (projection.rs:965-989); no birth date anywhere →
  **30 years**. `?months=N` overrides, clamped 12–840. `horizon_basis` reports `lifespan_90` |
  `fallback_no_demographics` | `months_override`.
- Large arrays (`points[].net_worth`, `fire_target_series`, `asset_series[].values`) serialize as
  f64 for wire size; scalar KPIs (`jubilacion_target_net_worth`, milestones targets,
  `starting_net_worth`) stay Decimal-as-string. This f64 boundary is deliberate (v1.4.0,
  precision < 1 € over 70y) — do not extend it to scalars.

## 8. Worked example (fire-parity.json, case "annual_expense mode, ES taxes, modest expense")

Inputs: mode `annual_expense`, `swr_pct 3.5`, taxes on, default ES brackets;
`expense_retirement = 1500 €/mes`, `income_retirement = 0`.

1. Net annual need = max(0, 1500 − 0) × 12 = **18.000 €**.
2. Gross-up (closed form): bracket 1 (19%, ceiling 6.000): candidate = 18.000/0,81 = 22.222,22
   → exceeds 6.000, advance with K = 0,19×6.000 = 1.140. Bracket 2 (21%, ceiling 50.000):
   candidate = (18.000 + 1.140 − 0,21×6.000)/0,79 = 17.880/0,79 = **22.632,91 €** ≤ 50.000 →
   solution. Check: tax = 1.140 + 0,21×16.632,91 = 4.632,91; 22.632,91 − 4.632,91 = 18.000 ✓.
3. Target = 22.632,91 / 0,035 = **646.654,61 €** — matches the fixture's
   `expected_target_nw: 646654.611` (tolerance ±1 €).
4. That becomes `FireTarget.base_amount`. With 2% inflation the moving target 10 years out is
   646.654,61 × 1,02^10 ≈ 788.267 €; jubilación fires the first month nominal NW ≥ target(month).

**Parity discipline**: the fixture is consumed by BOTH `apps/api/tests/fire_parity.rs`
(full-stack, needs `TEST_DATABASE_URL`; NOT run in CI) and `apps/web/src/lib/fire.test.ts`
(pure functions, runs in `npm test --workspace futurefin-web`). If you change brackets, gross-up
or the need formula on either side, regenerate expected values in the JSON and make **both**
suites pass. The JSON's `_formula` field is the contract.

## 9. What this model deliberately does NOT do (all UNIMPLEMENTED, candidates only)

Deterministic single-path projection; constant nominal returns; no stochastic returns / Monte
Carlo, no sequence-of-returns risk, no variable/guardrail SWR, no tax-aware withdrawal ordering
(the gross-up taxes the whole withdrawal through the brackets — no cost-basis tracking at
withdrawal time), no per-asset inflation, no contribution indexation to wage growth. These are
open research directions — see `.claude/skills/futurefin-research-frontier/SKILL.md`; any move
on them goes through `.claude/skills/futurefin-projection-realism-campaign/SKILL.md`.

## When NOT to use this skill

- **Debugging a wrong number / engine bug symptom** ("chart dips", "target differs from
  preview", "cache serves stale series"): start with
  `.claude/skills/futurefin-debugging-playbook/SKILL.md`; use this file only to know what the
  correct math *should* be.
- **Changing the economic model** (inflation semantics, cascade behavior, SWR bounds, brackets,
  drain order): that is a gated behavior change —
  `.claude/skills/futurefin-change-control/SKILL.md` and, for realism work,
  `.claude/skills/futurefin-projection-realism-campaign/SKILL.md`.
- **Adding/altering config axes** (fire_settings fields, env vars, query params):
  `.claude/skills/futurefin-config-and-flags/SKILL.md`.
- **Writing/running the parity or integration tests themselves**:
  `.claude/skills/futurefin-validation-and-qa/SKILL.md`.
- **Past incidents in full detail** (v1.0.12 vs v1.2.0 forensics, v1.4.2 chart bug):
  `.claude/skills/futurefin-failure-archaeology/SKILL.md`.

## Provenance and maintenance

Facts above verified 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`). Re-verify before trusting:

- Single target formula + signature: `grep -n "pub fn fire_target_at_month_index" crates/engine/src/projection.rs`
- Trigger uses k−1 / nw_prev: `grep -n "fire_target_at_month_index(input.fire_target" crates/engine/src/projection.rs`
- FIRE number modes + inputs: `grep -n "compute_fire_target_nw" apps/api/src/handlers/projection.rs` (mode A passes `expense_retirement`; mode B passes `expense_eff` — see §2b)
- Mode B (`savings_source`) base + hybrid subtraction: `grep -n "savings_source\|transactions_12m_avg\|effective_avg_income_expense\|use_avg" apps/api/src/handlers/projection.rs`
- Closed-form gross-up: `grep -n "gross_up_net_annual_fire" apps/api/src/handlers/projection.rs`
- Defaults (SWR 3.5, ES brackets): `grep -n -A8 "fn default_fire_settings" apps/api/src/handlers/installation.rs`
- SWR bound 0–4: `grep -n "swr_pct must be between" apps/api/src/handlers/installation.rs`
- Horizon constants (90/5/70/30, basis strings): `grep -n "LIFESPAN_AGE\|FALLBACK_YEARS\|lifespan_90" apps/api/src/handlers/projection.rs`
- Deflation by month_index: `grep -n -A6 "fn deflate_points_to_today" apps/api/src/handlers/projection.rs`
- Sink invariant: `grep -n "uncapped_remainder\|sink_must_be_last" apps/api/src/handlers/allocation_rules.rs`
- Client still binary-search (90 iters): `grep -n "for (let i = 0; i < 90" apps/web/src/lib/fire.ts`
- Budget retirement fields: `grep -n "persists_after_retirement\|ends_at_retirement" apps/api/src/handlers/budget.rs | head`
- Fixture case count + tolerance: `grep -c '"name"' apps/api/tests/fixtures/fire-parity.json` (7 as of 2026-07-09, incl. the mode-B avg-style tripwire) and `grep -n "_tolerance_eur" apps/api/tests/fixtures/fire-parity.json`
- If line anchors look stale: `git log --oneline -3 -- crates/engine/src/projection.rs apps/api/src/handlers/projection.rs`

If you change anything this file describes, update this skill in the same change (CLAUDE.md
rule: keep `.claude/` docs of record current) and re-run both parity suites.
