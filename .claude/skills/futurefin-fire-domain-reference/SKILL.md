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

> **Contratos canónicos y divergencias con la realidad**: la ficha
> [`.claude/financial-contracts.md`](../../financial-contracts.md) (auditoría 2026-08) es la fuente
> de verdad de qué convención rige cada magnitud y de la deuda de modelo pendiente (§4, con issues).
> Este skill explica la matemática TAL COMO ESTÁ IMPLEMENTADA; aquella dice qué es defecto conocido.

Everything here is the model **as implemented** (verified against code on 2026-07-02,
v1.4.3; §2b re-verified 2026-08-14 against v2.2.0; §2c, §4, §5, §7 y §9 revisados 2026-08-28 con la
Fase 6 del tren 4.4.0), not textbook FIRE theory.

> **⚠️ Los números de línea de este fichero son masivamente obsoletos y NO se han corregido uno a
> uno.** `crates/engine/src/projection.rs` creció ~880 líneas con la Fase 6 (issue #87) y
> `apps/api/src/handlers/projection.rs` otras ~515, así que casi todo `projection.rs:NNN` de aquí
> apunta a otra cosa — y **varios ya estaban podridos antes**: en `main`,
> `project_net_worth_series` ya vivía en la línea 739, no en la 386 que decía el §5, así que sus
> anclajes llevaban releases señalando `distribute_contributions`. Los del §5 se **retiraron** en favor de nombres de función; el resto se
> conservan solo como pista de en qué fichero mirar. **Localiza por nombre de símbolo, nunca por
> número**, y usa los `grep` de «Provenance and maintenance». Un anclaje numérico en un fichero vivo
> caduca en silencio y miente con más credibilidad que la ausencia.

Primary sources (ground truth, in this order):
- `crates/engine/src/projection.rs` — the simulation (pure, `Decimal`-only, **56** unit tests tras
  la Fase 6; esta línea decía 22 desde v1.4.3 — cuéntalos con
  `grep -c '#\[test\]' crates/engine/src/projection.rs`).
- `apps/api/src/handlers/projection.rs` — FIRE number, gross-up, horizon, deflation, milestones.
- `apps/api/src/handlers/installation.rs` — `FireSettings` defaults + validation.
- `apps/web/src/lib/fire.ts` — client-side duplicate for the live preview (server stays source of truth).
- `apps/api/tests/fixtures/fire-parity.json` — canonical cases shared by both sides.

Historical note: `projection_target_age` was **removed** in v1.0.6 (migration
`20260516120000_drop_projection_target_age.sql`); FIRE crossover is the sole retirement trigger.
`horizon_basis` strings are `lifespan_age | fallback_no_demographics | months_override` since
4.9.0/#149 (`lifespan_90` hasta 4.8.0 — el 90 dejó de ser constante), con `horizon_lifespan_age`
ecoado al lado. (The docs/comments that still described the old model were fixed on 2026-07-02.)

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
| **Sobrante / surplus** | Positive monthly net cash (`income − expense − debt_service + planning_adj`). Fed into the allocation cascade — **also in retirement, the SAME cascade, no exception** (4.12.1, #175). Unrouted surplus does NOT enter net worth: it accumulates in `unallocated_savings_total`, unreachable in production with live assets (indestructible sink, #176). |
| **Cascade** | The ordered list of `allocation_rules` that distributes the monthly surplus across assets. §6. |
| **Debt service** | Sum of active liability monthly payments, each capped by remaining principal. |
| **Contributed capital** | `Σ basis por activo` (4.10.0/#120; `surplus_cash` retirado del término en 4.12.1/#175): arranca en los `purchase_price`, sube con cada euro que la cascada asigna a un activo — también en jubilación, donde reinvertir SIGUE subiendo la base y abarata las ventas futuras (#178) — y **BAJA al vender** (`b' = b·v_post/v_pre`) — ya NO es monótono. El sobrante que ninguna regla absorbe NO cuenta como aportado: vive fuera del balance, en `unallocated_savings_total`. Never includes market growth. |
| **Drawdown / drain** | In deficit months **the whole deficit is sold, gross, from assets** — no cash-first step since 4.12.1 (#175, `surplus_cash` retired). Order: liquid first, lowest-return first. `gross_up_monthly`/`gross_up_mixed_monthly` con la misma escala y `g` que el objetivo — **`g` per-asset since 4.12.0/#178**: an asset whose basis was fed by the cascade derives its `g` from that basis even without a declared `purchase_price` (`basis_declared`, extension B of #178 in 4.12.1 — the contributed euro IS the data); `undrained` se acumula NETO. Retirement is modelled as income dropping, which creates the deficit. |
| **Installation** | The single-household deployment singleton; `fire_settings` and inflation live on its one DB row. |
| **Runway** | Months the **liquid** assets cover the total monthly expense, compounding their expected return and inflating the expense (§2c). A KPI of `/v1/summary`. It shares two inputs with the FIRE target since v2.3.0 — the same `swr_pct` and the same tax gross-up — because its *infinite* case is exactly a liquidity FIRE number: `Indefinite` ⟺ the grossed-up annual withdrawal ≤ SWR × liquid balance. The finite case remains its own simulation. |

## 2. The FIRE number: three modes

Server: `compute_fire_need` (handler) construye el `FireNeed` por modo y el ENGINE evalúa el
objetivo POR MES (`fire_target_base_at_month_index`, 4.10.0/#170). Settings:
`installation.fire_settings` JSONB, defaults applied on read by `resolve_fire_settings`.

```
need(k) =
  manual         → fire_number_manual_amount · f(k)                        (hoy > 0, else no target)
  annual_expense → max(0, expense_retirement_monthly·f(k) − income_retirement_monthly) × 12
                                                   (la pensión PLANA se resta DESPUÉS de inflar;
                                                    need(0) ≤ 0 → no target para TODA la serie)
  current_income → (income_regular_monthly − income_retirement_monthly) × 12 · f(k)  (≤ 0 hoy → no target)

target(k) = gross_up(need(k), tax_brackets, taxes_enabled, taxable_gain_ratio) / (swr_pct/100)
            + debt_term(k)
```

`f(k)` es el factor de inflación (`inflation_factor_at_month_index`). En `k = 0` todo degenera a
la fórmula histórica — la vista previa y `fire-parity.json` no se mueven. El gross-up de la
necesidad inflada NO es el gross-up inflado (fiscal drag: la escala es afín y los tramos
nominales), por eso la evaluación por mes aplica a los TRES modos.

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

### 2b. Modes B & C: `savings_source` — need & net from the real 12-month average

`fire_settings.savings_source` (default `budget` = everything above) has **two transactions-derived
modes** (gate `SavingsSource::uses_transactions()`):

- **Mode B `transactions_avg`** (shipped v2.0.0): income **and** expense come from the real 12m average.
- **Mode C `budget_income_real_expense`** (shipped v2.1.0): income from the **budget**, expense from the real
  12m average — same raw `expense_avg` as B. For a stable salary whose spending you want measured for real.

They change **where the pre-retirement income/expense scalars come from**, and therefore both the FIRE
need and the simulation's monthly net. The gross-up, SWR, moving-target and drain formulas are
**unchanged** — only the base numbers differ.

- **Window & average**: weighted mean over `[first-of-month(today) − 12 months, first-of-month(today))`
  — the 12 **complete** calendar months before the current one (the running month is excluded).
  Denominator = `months_with_data`, but counting **only real months** — a month in the window with ≥1
  transaction `recurring_rule_id IS NULL` **with a classified `kind`** (4.8.0, #125: a month whose
  only content is unclassified imports added 0 € to the numerator and 1 to the denominator — six
  such months halved the reported average and with it the mode-B FIRE target). A pseudo-empty month
  (only recurring instances, e.g. after backfilling recurring rules) is excluded **entirely** —
  neither numerator nor denominator; a real month counts **whole**, including its recurring
  transactions. Since auditoría MCP the Movimientos comparison (`GET /v1/transactions/summary`)
  applies the **same real-month predicate**, and since 4.8.0 (#125) also the **same window anchor**
  (today): the app's two "N-month averages" finally describe the same tranche (until 4.7.x the
  summary anchored on the selected month, one month off under the same label). They are still not
  identical: `transactions_avg` has per-side configurable windows (`AvgWindowSpec`,
  `data`/`calendar` modes) while the summary is always calendar-windowed, and the summary publishes
  its denominator as `avg_months` alongside the unchanged `months_with_data`. Amounts average in
  **nominal euros of their date** — no CPI adjustment (declared in the help text; #125's third leg,
  accepted as declared-only debt). Worked example (test `pseudo_empty_month_excluded_from_avg`):
  real month M−2 with manual income 2000 + recurring-only month M−1 with recurring salary 3000 →
  `months_with_data = 1`, `income_avg = 2000` (before the fix: months=2, avg=2500). Helper
  `transactions_avg` (`handlers/transactions/summary.rs`); the real-month predicate is the inline
  `COUNT(*) FILTER (WHERE t.recurring_rule_id IS NULL AND t.kind IS NOT NULL) AS real_txns` of its
  single query (there is no `real_months` CTE), mirrored by `transactions_summary_core`.
- **Liabilities in real modes (4.8.0, #142 — the 3.4.0 annulment is REVERTED; owner's option 3)**:
  the engine's expense in B/C is `max(0, expense_avg − Σ cuotas declaradas activas)` — the declared
  monthly-equivalent quotas of live-plan liabilities are **subtracted from the raw average**, and
  the liabilities keep their real `monthly_payment` in the engine input: **debt amortizes in all
  three modes**, with the step at `payment_end_date`, one accounting rule everywhere. Rationale:
  the paid cuota lives inside the measured average, so charging the engine's debt service on top
  double-counted it (that was 3.4.0's motive for zeroing payments); subtracting the declared quota
  removes the duplicate WITHOUT freezing the debt. The residual difference between the real paid
  cuota inside the average and the declared quota subtracted stays in the expense base (honest:
  it is spending the declaration does not cover). The summary panel (`/v1/summary`) still shows
  the **raw** average — the subtraction is an engine-input concern, not a display one. The 3.4.0
  trade-off prose ("the projection is conservative… no step at the loan's end") no longer holds:
  `mode_b_no_step_up_at_liability_end` was INVERTED (the curve now steps up when the plan ends,
  by more than 300 k€ in its scenario), `mode_b_raw_avg_ignores_liability_links` re-pinned
  (delta 4.000 → 4.800 = 6.000 − (2.000 − 800)), and `mode_b_liability_static_nw_subtraction`
  survives only via the 0 %-loan identity plus the clamp.
- **Target**: `annual_expense` uses the raw `expense_avg` as the base (mode A used `expense_retirement`);
  `current_income` uses `income_eff` (`income_avg`) in mode B, and the **budget income** (`income_reg`)
  in mode C; `manual` is unchanged. This is a **deliberate, semantic change of base** — mode A's
  `expense_retirement` is a budget line, B/C's `expense_avg` is measured spending (cuotas included).
  `end_adj` (budget end-date adjustments) is zeroed in both B and C; `planning_flows` (`flow_adj`) still
  apply. In `projection.rs` this lives in `EffectiveInputs`: B|C share the avg/liability branch and only
  differ in the income scalar (mode C → `income_reg`, mode B → `income_eff`); flag `expense_from_avg`.
- **Documented mismatch**: B/C only change the **accumulation** phase. The **retirement** phase
  still draws `income_retirement` / `expense_retirement` from the **budget** in all modes (the drain
  step at §5 is unchanged). So the target is derived from real spending while the drawdown that must
  fund it is budget-based — an accepted, intentional asymmetry.
- **Everything downstream follows the mode too (v2.2.0)** — B/C are no longer "only the projection
  scalars":
  - `GET /v1/summary` `financial_health`: `expense_derived_monthly_equivalent` = **0 in all three
    modes** since 3.7.0 — in B/C because the cuota is ordinary spending inside the average
    (reform 3.4.0), in A because it is now an ordinary budget entry inside
    `expense_regular_monthly_equivalent` (the derived block was merged into `entries`). In B/C
    `expense_total_monthly_equivalent` = `expense_avg`. The v2.2.0 identities
    `expense_total = expense_reg + expense_der` and `net = income − expense_total` keep holding in
    all three modes (the first one degenerately).
  - `runway_months` follows `expense_total`, hence the real base in B/C (see §2c).
  - `GET/POST/PATCH /v1/assets`: allocation caps `months_expense` / `income_multiple` resolve with the
    **effective** scalars (`assets_projection_context`), so the € target matches the month-1
    contribution shown in the same response and the simulation.
  - `GET /v1/projection/series` echoes the effective mode in `savings_source` +
    `savings_income_basis`/`savings_expense_basis` (same naming as summary), so the chart can label the Δ base.
- **Fallback**: `months_with_data == 0` in mode B/C → silently reverts to the budget scalars (mode A
  effective). `GET /v1/summary` reports the **effective** source in `financial_health.savings_source`
  (so it can read `"budget"` even when the setting is `transactions_avg` / `budget_income_real_expense`),
  and the whole `financial_health` block is then byte-identical to mode A.
- **Preview parity**: the web preview must consume `/v1/summary`'s effective equivalents in mode B/C
  (`RetirementView`, fetch gated on `savingsSourceUsesTransactions`; in mode C the summary income is
  already the budget income so the preview matches the server), never recompute the need from the budget
  — otherwise
  it re-opens the client/server divergence class of §2 (see §2.5 of failure-archaeology). Tripwire
  case in `fire-parity.json`: `expense_retirement 2137.5 → expected_target_nw 923327.306` (proves both
  sides derive the same need from an avg-style, non-round expense base).
- **Read the mode from `summary.financial_health`, never from the root of `SummaryResponse`** (v2.2.0
  fix): the server nests `savings_source` and `savings_income_basis`/`savings_expense_basis` inside
  `FinancialHealthMetrics`, but `types.ts` declared them at the root until v2.2.0. `SummaryView` and
  `RetirementView` therefore read `undefined`, `savingsSourceUsesTransactions(undefined)` returned
  `false`, and the Jubilación tab ("Gasto actual", "Ingresos actuales", "Patrimonio objetivo", "Primer
  cruce") silently used budget figures in B/C, diverging from the server's
  `jubilacion_target_net_worth`. Nothing failed loudly — a missing optional field is legitimate
  `undefined` for TypeScript. The parenthetical now goes through one shared pure helper,
  `savingsAvgParenthetical(source, months)` in `lib/fire.ts`.

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

### 2c. Runway (v2.2.0; SWR threshold since v2.3.0): liquid assets vs an inflating expense

Not the FIRE target itself, but the same economic frame — and, since v2.3.0, literally the same SWR
and the same tax gross-up for its infinite case. It is the one KPI users read as "how long
could I stop earning". Pure function `liquid_runway_months` in `crates/engine/src/runway.rs`,
consumed only by `GET /v1/summary` (`financial_health.runway_months` + `runway_is_indefinite`);
full contract in `.claude/engine.md` §Runway.

- **Was** `liquid_assets_total / expense_total`. **Is** a month-by-month `Decimal` loop where the
  expense is funded by **sequential drain** (4.8.0, #128: lowest-expected-return liquid first,
  `None` = 0, ties by index — the SAME order as `drain_from_assets` in the simulation), each
  remaining balance compounds its OWN monthly multiplier, and the expense grows with
  `annual_inflation_assumption_percent` (clamped ≥ 0 by the handler).
- **Same frame as the simulation**: nominal euros, withdraw-then-grow order, and *literally* the
  engine's `monthly_multiplier` (made `pub(crate)` for this) — including how it treats non-positive
  rates (see §4: only `None` and exactly `0` mean factor 1; **negative rates compound**). A KPI that
  used a different annual→monthly conversion would drift from the chart.
- **Sequential drain replaced the value-weighted multiplier in 4.8.0 (#128)**: until 4.7.x the
  balance grew at `m = Σ vₐ·monthly_multiplier(rₐ) / Σ vₐ` (a prorated drain), systematically
  ~2 % shorter on mixed portfolios because it consumed the high-return assets from month 1. The
  card now matches what the simulation actually does (mixed 10k@0 % + 10k@10 % vs 1.000 €/mes:
  20,80 → 21,27 months). Single-asset portfolios are bit-identical under both models.
- **The infinite case is an SWR threshold, not the cap** (v2.3.0): `RunwayOutcome::Indefinite` ⟺
  `annual_expense_for_swr ≤ A·(swr_pct/100)`, compared without dividing
  (`annual_expense_for_swr·100 ≤ A·swr_pct`) so the boundary is exact in `Decimal`. The handler
  passes `swr_pct` from `fire_settings` (the **same** rate as the target, Jubilación tab) and
  `annual_expense_for_swr = gross_up_net_annual_fire(expense_total·12, tax_brackets, taxes_enabled)`
  — the **same** gross-up as §3. So the liquidity question is the FIRE question restricted to liquid
  assets: `A ≥ gross_expense / SWR`. `swr_pct ≤ 0` never triggers it. **Since 4.8.0 (#128) the
  threshold alone is not enough**: `Indefinite` also requires the liquid portfolio's value-weighted
  expected return to be strictly positive (`Σ vₐ·rₐ > 0`) — the Trinity/Bengen rule presumes an
  invested portfolio, never cash parked at 0 % (300.000 € at 0 % vs 875 €/mes meets the threshold
  by exact equality and now publishes 342,9 months instead of «indefinida»). Inflation still never
  touches the trigger (it governs only the finite loop). Below the threshold, an enormous expected
  return still does not buy "infinite".
- **Cap 1.200 months (100 years) is a floor, not infinity**: surviving it without meeting the SWR
  threshold returns `Months(1200)`, meaning "at least 100 years" (UI: «+100 años»). Compared to v2.2.0 this
  is the behavioral change that flips scenarios like 1M @ 7 % vs 4.000 €/month from `Indefinite` to
  the floor.
- **Three outcomes, do not conflate them**: `Indefinite` → `runway_months` omitted from the JSON
  (`skip_serializing_if`) + `runway_is_indefinite: true` (UI value «Infinito», parenthetical
  «dentro del SWR 3,5 %» via `runwaySwrParenthetical` in `apps/web/src/lib/fire.ts`).
  `monthly_expense <= 0` → `NoExpenseBase` → also omitted, but the flag stays `false` (UI hides the
  tile). `Months(_)` → the value travels. Check order is part of the contract: `NoExpenseBase` is
  decided **before** the threshold, or a zero expense would satisfy `0 ≤ A·swr` and report an
  undefined runway as infinite.
- **Follows `savings_source`**: the expense base is `expense_total_monthly_equivalent`, so in B/C with
  data it is the raw real spending average (§2b, cuotas included), not budget. (The #142 quota
  subtraction is an ENGINE-input concern; the summary's expense base — and hence the runway — stays
  raw.)
- **Reduces exactly to `A/g`** with return and inflation 0 **and the SWR threshold unmet** (single
  final division, no tolerances) — that is how the pre-change regression test stayed valid across
  both changes (`runway_pre_change_baseline_liquid_over_expense` is still exact). Since 3.8.0 the
  exactness is an **engine** property: `handlers/summary.rs` publishes the value rounded to 1
  decimal (matching `sim_kpis`, which already did), so the two threshold tests assert
  `(A/g).round_dp(1)` — same rigour, published precision. Call `liquid_runway_months` directly if
  you need every digit.

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
gross_candidate = (net + K − r · prev_ceiling) / (1 − r·g)
if g · gross_candidate ≤ bracket ceiling → that is the answer
else K += r × bracket_width; advance to next bracket
```

`g` = `taxable_gain_ratio` (fracción [0,1] de cada euro bruto que es plusvalía gravable —
4.10.0/#140 fase 2; `g = 1` colapsa BIT-idéntico a la forma histórica, `g = 0` ≡ sin impuestos).
OJO: con `g` el test de validez cambia de FORMA, no solo el denominador — comparar `G ≤ techo`
en vez de `g·G ≤ techo` es el bug silencioso de la fase (caso N-4 del fixture). The open last
bracket guarantees termination. Degenerate effective `r·g ≥ 100%` returns `prev_ceiling`.

**Location**: `crates/engine/src/tax.rs` desde la Ola 6 (#140) — el objetivo (por mes, #170), el
drenaje bruto del bucle y los dos umbrales del runway consumen LA MISMA función.

**History**: until v1.3.0 the server ran a 90-iteration binary search on `Decimal`. The closed
form replaced it with **identical results ±0.01 €** — proven by the regression test
`closed_form_matches_binary_search_across_es_brackets` (moved to `crates/engine/src/tax.rs` with
the function), which keeps the old binary search alive as a reference implementation.

**Client**: `apps/web/src/lib/fire.ts` uses the SAME closed form as the server since Ola 2/#118
(`grossUpNetAnnualFire`, fire.ts:236-272; its own comment records that the old 90-iteration
binary search SATURATED and published a target ~20 % low — the claim that the bisection was
"deliberate" died there). Do not change one side without running both parity suites (§8).
`taxOnGrossCapitalAnnual` mirrors the server's `tax_on_gross_capital_annual` (until the Ola 6
move to `crates/engine/src/tax.rs`, test-only on the server side).

## 4. The nominal model (v1.2.0, current)

The single most important idea in the codebase. Philosophy: **"haciendo lo que hago ahora,
¿qué tal voy?"** — "if I keep doing exactly what I do today, how does it go?".

- Income, expenses, contributions and planning flows stay **constant in nominal euros** for the
  whole horizon. They are never inflated.
- Asset returns (`expected_annual_return_percent`) are **nominal**, compounded monthly via
  `(1 + p/100)^(1/12)` (`monthly_multiplier`). **Corrección 2026-08-28 — este bullet decía lo
  contrario de lo que hace el código, y llevaba tiempo diciéndolo**: afirmaba que «rates ≤ 0 are
  treated as no growth (factor 1) — the engine does not model negative expected returns». Falso.
  Solo `None` y **exactamente `0`** dan factor 1; **una tasa negativa compone de verdad** (−50 %
  anual ⇒ ×0,5 en doce meses), y `p ≤ −100` se clampa a factor 0 (la capa API rechaza esos inputs
  con error tipado). Lo confirman tres tests del engine —`monthly_multiplier_none_and_zero_are_flat`,
  `negative_return_composes_downward` y `minus_100_or_less_clamps_to_zero_factor`— y
  [`engine.md`](../../engine.md) §Simulation loop paso 7, que siempre lo tuvo bien. Deriva
  preexistente, no causada por la Fase 6; se arregla aquí porque un lector que la creyera
  concluiría que un activo con rentabilidad esperada negativa se queda plano.
- **Inflation moves the FIRE target AND the loop's expense — never incomes** (4.9.0, #139/#146).
  `FireTarget { base_amount, annual_inflation_percent, debt_payments_remaining }` for the target;
  `ProjectionInput.annual_inflation_percent` indexes the expense with the same
  `inflation_factor_at_month_index` on the `(k−1)/12` axis. The old handler clamps to ≥ 0 are
  GONE (#146: range [−2, 50]; negative inflation composes — target and expense DECREASE);
  re-verify with `grep -n "inflation" apps/api/src/handlers/projection.rs | grep -c "max(Decimal::ZERO)"` (must print 0).

The single formula — `fire_target_at_month_index` (projection.rs:171-182), public in the engine
crate and consumed by BOTH the engine and the handler:

```
target(month_index) = base_amount × (1 + inflation/100)^(month_index / 12) + debt_term(month_index)
month_index = 0 → base_amount + debt_term(0).   inflation = 0 → flat BASE at every month.
debt_term(m) (4.8.0, #142) = Σ cuota cash-outs remaining AFTER month m + residual tail
                             (from FireTarget.debt_payments_remaining; 0 with no liabilities).
```

**The target is no longer monotonic (4.8.0)**: growing base + decaying debt term. No optimization
may assume monotonicity (binary-search crossing, early exit); the crossing scan is linear. With
inflation 0 and live debt the full target is *strictly decreasing*.

**The off-by-one story** (why one helper): before v1.3.0 the engine computed the trigger with
`years = (k−1)/12` while the handler built `fire_target_series` with `years = month_index/12` as
a *separate duplicated formula* — the plotted target curve and the actual retirement trigger
disagreed by one month. Fix: one public helper; the engine passes `k−1` (since 4.8.0 it compares
`liquid_prev`, the close of month k−1, against the target at that same axis point), the
handler passes the point's own `month_index`. Regression test:
`fire_target_helper_matches_compound_factor_at_year_boundaries`.

**What rewrite #1 got wrong** (v1.0.12, "real pure" model — replaced in v1.2.0): it ran the
whole simulation in today-euros by deflating each asset's return
(`r_real = (1+r_nom)/(1+inf) − 1`) while keeping flows nominal-fixed and the target flat. The
mixture produced incoherent, *silently plausible* output — e.g. assets draining before
retirement when inflation was on. v1.2.0 made everything nominal and moved all inflation into
the target. Guard test: `fire_target_with_inflation_does_not_trigger_early_drain`
(projection.rs:981-1021). If you ever touch inflation semantics, that history says: pick ONE
frame (nominal) and keep every series in it.

**Inflation-invariance of the crossing**: `nominal_liquid(k) ≥ base × (1+i)^(k/12)` is algebraically
the same comparison as `deflated_liquid(k) ≥ base` — with the 4.8.0 debt term the pure-base
equivalence picks up a deflated-vs-nominal term mismatch, but the guarantee that matters survives
intact for a stronger reason: since 4.6.0 the client reads `jubilacion_month_index` from the
server and never re-derives the crossing, so the "Inflation Adjusted" display toggle cannot move
the jubilación marker by construction (CHANGELOG v1.4.2 established it; 4.6.0 made it structural).
(Changing the inflation *value* does move it, of course.) Milestones are NOT invariant: real
milestones are reached later than nominal ones (test at projection.rs:1446-1482), hence the
separate `milestones_real` field.

**`simulate_projection`'s `model_note` (MCP-only what-if, Fase 5/issue #86, 4.4.0)**: the tool
restates this section's warning at the point of use, because the two axes a caller varies most —
`annual_inflation_percent` and `swr_pct` — are exactly the two that interact this way.
`SIMULATE_MODEL_NOTE` (`handlers/projection.rs`): lowering `annual_inflation_percent` raises every
asset's REAL return (nominal returns are unchanged — only the frame moved, same nominal-model fact
as above) **and** freezes the FIRE target at the same time, in the same move — jubilación can land
years earlier with nothing about the plan actually improved. Read a change to either as a change of
*assumption*, never as an improvement. `swr_pct` the same way in miniature: raising it lowers the
target by division, not by saving more. Neither is a tool bug — it is this file's nominal model,
surfaced to a caller (often an agent) that has no access to this document. **Three** more `model_note`
facts worth knowing here: the cash axes (`extra_monthly_savings`, `extra_monthly_cash_adjustment`,
`one_off_expense`) do NOT touch income or expense — they move `net_cash_monthly`, never
`net_recurring_monthly`/`savings_rate` (§2b); `final_net_worth` is nominal euros of the
horizon's last month, comparable across scenarios only via `final_net_worth_real`, and only when
`deltas.real_delta_absent_reason` is `null`; and — **desde 4.4.0 (Fase 6, issue #87)** — el eje
`liability_overrides` (amortización anticipada: `extra_monthly_principal`, `lump_sum_*`,
`apr_percent`, `repayment_model` por pasivo) es el bloque más extenso del `model_note` —que es una
sola cadena sin saltos de línea, así que «párrafo» aquí es una forma de hablar—, y la
identidad de caja cambia con él: `net_cash_monthly = net_recurring_monthly +
monthly_cash_adjustment − liability_extra_principal_monthly` (el último es un campo publicado
nuevo, pineado en `simulate_liability_kpis.rs::net_cash_monthly_stays_verifiable_by_subtraction`).
**No está disponible en los modos B/C** (`liability_overrides_unavailable_in_real_expense_mode`):
ahí las cuotas ya viven dentro del promedio de gasto, así que amortizar dos veces el mismo euro
sería doble conteo.

## 5. The monthly simulation loop, step by step

`project_net_worth_series` en `crates/engine/src/projection.rs`. Series index 0 = today's state;
month `k` (1-based) simulates the calendar month `month_first(ref_date) + (k−1)`. Net worth
identity (4.12.1 — `surplus_cash` retired, #175): `NW = Σ asset values − Σ liability principals −
undrained_cumulative`. Companion identities: `liquid_worth = Σ liquid asset values` and
`contributed_capital = Σ basis per asset` — the three live together in
[`engine.md`](../../engine.md#output).

> **Los números de línea de esta sección estaban obsoletos y se han retirado** (2026-08-28): el
> fichero creció ~880 líneas con la Fase 6 y cada anclaje apuntaba a otra cosa —
> `project_net_worth_series` pasó de la línea 386 a ~1000. Localiza cada paso por nombre de
> función, no por número; un anclaje numérico en un fichero vivo caduca sin avisar y **miente con
> más credibilidad que la ausencia**.

For each month `k = 1..=horizon_months`:

1. **Debt service** (`liability_month` + `liability_extra_principal`): for each liability active
   this month (`payment_end` is null or ≥ month start, payment > 0), charge the model's cash leg
   **plus any what-if extra principal**; sum. Weekly payments were normalized to monthly (`×52/12`)
   by the handler. **Dos correcciones sobre lo que decía este paso**: (a) desde **4.2.0** el tope de
   la cuota en los modelos que devengan es el **payoff** (`P·(1+i)`), no el principal — «pay
   `min(monthly_payment, remaining principal)`» solo describe `fixed_payments`; (b) desde **4.4.0**
   `debt_service` incluye la amortización extra, y el principal de cierre que se asienta en el paso
   8 es `closing − extra`. **Las dos mitades o ninguna**: cobrar el extra sin bajar el principal
   drenaría caja sin reducir deuda, y bajarlo sin cobrarlo *imprimiría dinero*. Efecto instantáneo
   sobre el patrimonio: cero exacto **en el balance** (con un matiz de coste de oportunidad que
   explica [`engine.md`](../../engine.md) §Calendario de amortización). Efecto de segundo orden a
   conocer: el techo de un cap `months_expense` es `N × (expense + debt_service)`, así que **se mueve**
   en un what-if que amortiza — alcanzable solo desde `simulate_projection` y **sin test que lo
   cubra**.
2. **Retirement check (4.8.0)**: `fire_reached = liquid_prev ≥ target(k−1)` via
   `fire_target_at_month_index` — the basis is the **LIQUID** worth of the previous month
   (Σ `is_liquid` assets, GROSS, no principal subtracted — #143, `surplus_cash` retired from this
   term in 4.12.1/#175 — theorem: the crossing could only move LATER with that change, never
   earlier, and in production it is invariant; paired with the
   target's full-quota debt term of #142), not total NW. The state is an **absorbing latch**
   (#141): `retired = retired || fire_reached || k ≥ retirement_start_month` — once retired,
   always retired, even if worth later dips below the target (until 4.7.x the model re-checked
   every month and flickered between regular and retirement budgets). The handler always passes
   `retirement_start_month = None` and `retirement_monthly_withdrawal = 0` — FIRE crossover is
   the sole trigger, and the drain is driven purely by the income drop.
3. **Pick the budget**: in retirement use `income_retirement_monthly` /
   `expense_retirement_monthly`; otherwise the regular pair.
4. **Net cash**: `income − expense − debt_service + planning_adj[k−1] −
   retirement_withdrawal`. Planning flows with a `due_date` land in their calendar month; undated
   ones are spread over 90 days from `ref_date`. Budget expenses with an
   `expense_end_date` are cancelled from the following month via positive adjustments.
5. **Surplus** (`net_cash > 0`): run the cascade (§6) — **also in retirement, the SAME cascade,
   no special-case branch** (4.12.1, #175: `AllocationSkipReason::InRetirement` and the
   `in_retirement` wire literal died with the branch that produced them; the caps of the #171
   phase now govern euros in retirement too). Routed euros are added to asset values AND to
   `contributed_capital` (reinvesting raises the cost basis even in retirement — cheaper future
   sales, #178). Whatever no rule absorbs does **NOT** enter net worth: it accumulates separately
   in `unallocated_savings_total`, unreachable in production with live assets (indestructible
   sink, #176).
6. **Deficit** (`net_cash ≤ 0`): **no cash-first step** — the whole deficit is sold, gross, from
   assets (4.12.1, #175: the `surplus_cash`-first step died) via `drain_from_assets` — order:
   **liquid before illiquid, and within each group lowest `expected_annual_return_percent` first**
   (ties by input order; illiquid assets ARE drained if liquids run out). The tax exemption cash
   used to carry is inherited by the drain itself: the 0%-return account drains first and, if its
   basis was fed by the cascade, derives `g = 0` (`b = v`). Any shortfall the assets cannot cover
   accumulates in `undrained_cumulative`, which is *subtracted* from NW (implicit debt).
7. **Compound growth**: each asset value ×= monthly multiplier. Growth applies AFTER
   this month's contribution/drain.
8. **Principal reduction**: each liability is **assigned the closing balance computed in step 1**
   (`principals[i] = closing`), never a fresh `P − payment`. It only equals «principal −= payment»
   in `fixed_payments` with no extra repayment: with an accruing model the closing is
   `P(1+i) − cash`, and since 4.4.0 the extra principal is already subtracted from it. Recomputing
   it here would recompute the accrual, and the two copies would drift.
9. Push NW, contributed capital, and per-asset values.

`contributed_capital[0]` = sum of positive asset `purchase_price`.

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
- Whatever no rule absorbs is returned as leftover → `unallocated_savings_total` (4.12.1, #175) —
  it does NOT enter net worth; `unallocated_savings_reason` (`null` | `"no_assets"` | `"no_sink"`)
  explains why (unreachable in production with live assets, indestructible sink #176).

**The uncapped-remainder sink invariant** (enforced by the API handler, not the engine —
`apps/api/src/handlers/allocation_rules.rs:387-402,563-581,652-658,722-733`): every scope must
keep **exactly one** `remainder` rule with no cap, and it must be **last** in the cascade. Create
rejects a second sink, update/delete reject orphaning the scope, reorder rejects a non-last sink.
The engine itself tolerates zero rules (surplus → `unallocated_savings_total`, not net worth,
4.12.1/#175); the invariant lives one layer up — it is what makes that case unreachable by API
with live assets (#176). (This cascade replaced per-asset contribution config in v1.1.0
with a signed-off, no-data-migration column drop — see futurefin-change-control.)

## 7. Display math: deflation, milestones, horizon

All anchors in this section are in the HANDLER file `apps/api/src/handlers/projection.rs`
(not the engine file of the same name used in §5).

- **Deflate by `month_index`, never by array index.** El núcleo es `deflator_at_month_index`
  (`handlers/projection.rs`), que devuelve `1/(1+i/100)^(month_index/12)`. v1.4.2 bug: the web chart
  deflated by array index — invisible with `density=monthly` (indices coincide), wrong with
  `density=hybrid` (points 0..12 monthly then annual: non-equidistant). Any code touching
  decimated series must use `month_index`.
- **Desde 4.4.0 (Fase 6) el deflactado también se SIRVE** —la SPA **sigue rehaciéndolo**, y tiene que:
  solo `net_worth` viaja deflactado y el chart necesita además `contributed_capital`, la serie del
  objetivo FIRE y cada `asset_series[].values` (`deflationFactorAt`,
  `apps/web/src/lib/projection-chart.ts`)—. Y hay **un solo deflactor, con cuatro consumidores**: `points[].net_worth_real` de cada punto servido,
  `milestones_real`, `final_net_worth_real` (y su delta) de `simulate_projection`, y el endpoint
  dedicado `GET /v1/projection/deflate` (que además publica las dos
  direcciones y un `deflator` a 10 decimales para que multiplicar a mano reproduzca la cifra).
  `deflate_points_to_today` sigue existiendo pero opera sobre un tipo interno (`NwPoint`), no sobre
  el `ProjectionPoint` serializado. **Esto NO reabre el motor «real puro»** rechazado en v1.2.0
  (`futurefin-failure-archaeology` §1 fila 3), y la afirmación es **testable**: `net_worth_real` es
  exactamente `net_worth / (1+i)^(month_index/12)`, o sea no lleva información que el motor no haya
  producido ya. El motor sigue simulando 100 % en nominal. **Se publica siempre, también con
  inflación 0** (deflactor exactamente `1`), para que «no hay inflación» no se confunda con «esta
  versión no publica el campo» — contraste deliberado con `milestones_real`, que sí queda vacío con
  inflación 0 porque su contrato es anterior. Regresión: `apps/api/tests/projection_deflation.rs`.
- **Milestones** (nominal) and **`milestones_real`** (same 1/2.5/5×10^n thresholds crossed on the
  deflated series) are both computed on the FULL monthly series (`points_full`), not the
  decimated one, to keep `reached_month_index` exact under `hybrid`
  (projection.rs:1062-1118). `milestones_real` is empty when inflation = 0 and the web reuses
  `milestones`. The FIRE crossover (`jubilacion_month_index`) is likewise detected on the full
  series (projection.rs:1126-1134).
- **`jubilacion_month_index` does NOT index any served series (issue #82, 4.4.0) — it never did.**
  It is a MONTH number; `points`/`fire_target_series`/`asset_series[].values` are **position**-
  indexed and, under `density=hybrid` (the density the MCP tool `get_projection` forces), carry
  far fewer positions than months. Indexing an array with the raw month either falls off the end
  or — worse — silently lands on `[0]`, presenting today's FIRE target as if it were the target
  decades out. Two fields close the hole, both `null` iff there is no crossing:
  - **`jubilacion_series_position`** — the array position to use instead. Convention: the LAST
    served position `p` with `points[p].month_index <= jubilacion_month_index` (the crossing
    falls in the segment `[p, p+1)`; chosen over "next" because reading `points[p]` is
    conservative — it underweights net worth instead of overstating it).
  - **`jubilacion_target_net_worth_nominal`** — the FIRE target AT the crossing month, in NOMINAL
    euros of that month. `jubilacion_target_net_worth` (the older field) is in TODAY's euros
    (`FireTarget.base_amount`, undeflated); with inflation the two diverge by a growing factor
    over decades. Evaluated EXACTLY via `fire_target_at_month_index(ft, jubilacion_month_index)`
    — never interpolated between two points of `fire_target_series`, which under `hybrid` may not
    even contain the crossing month as a served point.
  Pin: `apps/api/tests/projection_number_semantics.rs`.
- **Horizon rule** — `projection_horizon_months`: configurable lifespan since 4.9.0 (#149).
  `years = clamp(horizon_lifespan_age − completed_age, 5, 70)` con
  `fire_settings.horizon_lifespan_age` (85..=105, default 90), using the session user's
  `birth_date`, falling back to the primary household person's; no birth date anywhere →
  **30 years**. El clamp [5, 70] no se tocó ⇒ el eje solo muerde si `edad ≥ edad_límite − 70`.
  `?months=N` overrides; fuera de 12–840 **rechaza** con 400 `months_out_of_range` (hasta 4.3.1
  clampaba en silencio). `horizon_basis` reports `lifespan_age` | `fallback_no_demographics` |
  `months_override`, con `horizon_lifespan_age` al lado; el margen al final es
  `points[último].net_worth` + `final_net_worth_real` (euros de hoy, paridad con simulate).
- Large arrays (`points[].net_worth`, **`points[].net_worth_real`** desde 4.4.0,
  `fire_target_series`, `asset_series[].values`) serialize as f64 for wire size; scalar KPIs (`jubilacion_target_net_worth`, milestones targets,
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

**Lo que sí hace desde 4.4.0 y conviene no confundir con la lista de arriba**: amortización
anticipada (mensual y puntual) como **eje what-if** de `simulate_projection`, con tres límites
reales — solo con plan de pago activo (el gate `liability_active`), tope al saldo del mes, y un
coste de oportunidad implícito por el orden cascada → crecimiento → asiento del principal. Y publica
el calendario de amortización (`GET /v1/liabilities/{id}/schedule`), que **no es modelo nuevo** sino
el `closing_principal` que el motor ya derivaba y tiraba.

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
- Trigger uses k−1 / liquid_prev + absorbing latch (4.8.0): `grep -n "fire_target_at_month_index(input.fire_target\|liquid_prev\|retired = retired" crates/engine/src/projection.rs`
- FIRE number modes + inputs: `grep -n "compute_fire_target_nw" apps/api/src/handlers/projection.rs` (mode A passes `expense_retirement`; mode B passes the raw `expense_avg` — see §2b)
- Mode B (`savings_source`) base + quota subtraction (4.8.0, #142 — the 3.4.0 `payment_amount = None` zeroing is GONE): `grep -n "savings_source\|transactions_avg\|expense_from_avg\|active_quotas" apps/api/src/handlers/projection.rs`
- Mode B/C reach into summary/assets/series: `grep -n "expense_reg\|expense_tot" apps/api/src/handlers/summary.rs` (el patrón anterior —`expense_der = Decimal::ZERO\|expense_tot = avg.expense_avg`— daba **vacío** desde un refactor: `expense_der` solo sobrevive dentro de un comentario y `expense_tot` se deriva de la resolución compartida, no de `avg.expense_avg`); `grep -n "assets_projection_context" apps/api/src/handlers/{projection,assets}.rs`
- Runway model + cap: `grep -n "pub fn liquid_runway_months\|MAX_RUNWAY_MONTHS\|RunwayOutcome" crates/engine/src/runway.rs` and `grep -n "liquid_runway_months\|runway_is_indefinite" apps/api/src/handlers/summary.rs`
- Runway infinite case is the SWR threshold, not the cap (v2.3.0): `grep -n "swr_pct" crates/engine/src/runway.rs` (the `Indefinite` branch must compare `annual_expense_for_swr * 100 <= balance_0 * swr_pct`) and `grep -n "annual_expense_gross\|gross_up_net_annual_fire" apps/api/src/handlers/summary.rs` (the handler must reuse the target's gross-up)
- Frontend reads the mode from `financial_health` (not the root): `grep -n "savings_source" apps/web/src/api/types.ts apps/web/src/views/RetirementView.tsx` (RetirementView is the only view reading the field directly; SummaryView consumes `financial_health` as a whole without touching `savings_source`)
- Closed-form gross-up: `grep -n "gross_up_net_annual_fire" apps/api/src/handlers/projection.rs`
- Defaults (SWR 3.5, ES brackets): `grep -n -A8 "fn default_fire_settings" apps/api/src/handlers/installation.rs`
- SWR bound 0–4: `grep -n "swr_pct must be between" apps/api/src/handlers/installation.rs`
- Horizon constants (5/70/30, basis strings, configurable age): `grep -n "MIN_HORIZON_LIFESPAN_AGE\|FALLBACK_YEARS\|lifespan_age" apps/api/src/handlers/{projection,installation}.rs`
- Deflation by month_index (**el núcleo desde 4.4.0 es `deflator_at_month_index`, con tres consumidores**):
  `grep -n -A10 "fn deflator_at_month_index" apps/api/src/handlers/projection.rs` y
  `grep -n "net_worth_real\|deflation_annual_inflation_percent\|deflate_amount_core" apps/api/src/handlers/projection.rs`
- **Negative returns DO compound** (§4, corrección 2026-08-28):
  `grep -n -A14 "fn monthly_multiplier" crates/engine/src/projection.rs` — solo `None` y `0` dan
  factor 1; `p ≤ −100` se clampa a 0. Si vuelves a leer «rates ≤ 0 → no growth» en algún doc, es drift.
- Calendario de amortización y amortización extra (4.4.0, Fase 6):
  `grep -n "pub fn liability_amortization_schedule\|MAX_LIABILITY_SCHEDULE_MONTHS\|enum LiabilityPayoffAbsence\|extra_principal_monthly\|fn liability_extra_principal" crates/engine/src/projection.rs`;
  la identidad contable la pinea `schedule_payment_identity_holds_in_every_model` y el contraste
  entre superficies, `apps/api/tests/simulate_liability_kpis.rs::the_what_if_debt_kpis_agree_with_the_liability_schedule`.
- Sink invariant: `grep -n "uncapped_remainder\|sink_must_be_last" apps/api/src/handlers/allocation_rules.rs`
- Client still binary-search (90 iters): `grep -n "for (let i = 0; i < 90" apps/web/src/lib/fire.ts`
- Budget retirement fields: `grep -n "persists_after_retirement\|ends_at_retirement" apps/api/src/handlers/budget.rs | head`
- Fixture case count + tolerance: `grep -c '"name"' apps/api/tests/fixtures/fire-parity.json` (7 as of 2026-07-09, incl. the mode-B avg-style tripwire) and `grep -n "_tolerance_eur" apps/api/tests/fixtures/fire-parity.json`
- If line anchors look stale: `git log --oneline -3 -- crates/engine/src/projection.rs apps/api/src/handlers/projection.rs`.
  **INCIDENTE (2026-08-28)**: TODOS los anclajes numéricos del §5 estaban obsoletos —
  `crates/engine/src/projection.rs` creció ~880 líneas con la Fase 6 y `project_net_worth_series`
  pasó de la línea 386 a ~1000, así que cada «(458-471)» apuntaba a otra función. Se retiraron en
  favor de nombres de función. **No vuelvas a poner números de línea en esta skill**: un anclaje
  numérico en un fichero vivo caduca en silencio y miente con más credibilidad que la ausencia.
- `jubilacion_series_position`/`jubilacion_target_net_worth_nominal` (issue #82, 4.4.0):
  `grep -n "jubilacion_series_position\|jubilacion_target_net_worth_nominal" apps/api/src/handlers/projection.rs` and the pin `grep -n "jubilacion_series_position_indexes_the_arrays" apps/api/tests/projection_number_semantics.rs`
- `simulate_projection`'s `model_note` (Fase 5, issue #86, 4.4.0): `grep -n "const SIMULATE_MODEL_NOTE" -A3 apps/api/src/handlers/projection.rs` (full text of the warning); `apps/api/tests/mcp_simulate.rs` pins the cache-neutral behavior this note sits next to.

If you change anything this file describes, update this skill in the same change (CLAUDE.md
rule: keep `.claude/` docs of record current) and re-run both parity suites.
