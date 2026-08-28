---
name: futurefin-projection-realism-campaign
description: >-
  Executable, decision-gated campaign for improving the CORRECTNESS and REALISM of FutureFin's
  projection/FIRE economic model — the owner-confirmed hardest live problem, whose errors are
  SILENT (numbers look plausible but are wrong). Load this skill when the task is to audit,
  stress, extend or redesign the projection model: "make the projection more realistic",
  "add Monte Carlo / stochastic returns / volatility / sequence-of-returns risk", "model taxes
  in drawdown", "variable SWR", "add property-based / invariant tests to the engine", "is the
  inflation model right?", "audit the allocation cascade / retirement drain", "why does the
  engine ignore negative returns / loan interest?", or any planned change to
  crates/engine/src/projection.rs semantics. Do NOT load it for: triaging a live wrong-number
  bug (futurefin-debugging-playbook first), learning the FIRE math as implemented
  (futurefin-fire-domain-reference), analysis technique recipes
  (futurefin-proof-and-analysis-toolkit), or generic merge/release gates
  (futurefin-change-control — this campaign routes through it, never around it).
---

# Projection Realism Campaign

An executable campaign, not an essay. Facts verified against the repo as of **2026-07-02, v1.4.3**.

Jargon used below (one-line definitions — full math in `.claude/skills/futurefin-fire-domain-reference/SKILL.md`):
- **FIRE target**: net worth needed to retire = `gross_up(annual_net_need) / (SWR/100)`.
- **SWR**: safe withdrawal rate, % of portfolio withdrawn per year (`fire_settings.swr_pct`, default 3.5).
- **Gross-up**: inflating an annual net need through Spanish capital-gains tax brackets so the after-tax withdrawal equals the need.
- **Nominal vs real**: nominal = euros of the moment; real = today-euros (deflated). The engine is **all-nominal**; only the FIRE target grows with inflation (v1.2.0 model).
- **Cascade**: ordered allocation rules (`fixed`/`percent`/`remainder`, optional caps) that split each month's surplus across assets.
- **Jubilación crossing**: first month where net worth ≥ the inflated FIRE target; the sole retirement trigger.

## Campaign charter

**Goal**: increase the fidelity of the economic model (what the engine simulates vs what would actually happen to a household) and catch silent wrongness — without breaking determinism, `Decimal` money discipline, or the client↔server FIRE parity.

**Done means**: (1) every model simplification is inventoried, code-anchored, and classified (Phase 1 table kept current); (2) every classification `realism gap` either has a measured effect size and an accepted-as-is note, or a Phase 2 item with a pre-registered acceptance test; (3) all baseline suites green.

**Standing rule — success is MEASURED, never judged by eye.** A projection that "looks right" is exactly the failure mode this campaign exists for (v1.0.12's model looked right and was incoherent). Evidence is: engine unit tests, the 6-case fire-parity fixture (±1 €), integration regression tests, and predicted-vs-observed numbers written down **before** running (discipline: `.claude/skills/futurefin-research-methodology/SKILL.md`).

## When NOT to use this skill

| Situation | Use instead |
|---|---|
| A live symptom (wrong KPI, chart diverges, 4xx, stale cache) | `futurefin-debugging-playbook` |
| Understanding SWR/gross-up/cascade/nominal-model as implemented | `futurefin-fire-domain-reference` |
| A derivation/refactor-equivalence technique (index math, closed forms, f64 audit) | `futurefin-proof-and-analysis-toolkit` |
| Whether/how a change may merge, migrations, releases | `futurefin-change-control` |
| Running/writing tests mechanics (TestApp, schemas, Vitest) | `futurefin-validation-and-qa` |
| Was this idea already tried and killed? | `futurefin-failure-archaeology` |
| Other improvement directions (non-projection) | `futurefin-research-frontier` |

## Phase 0 — Establish the baseline

Do this before touching anything. All commands from repo root.

```bash
# 0a. Engine unit tests (pure, no DB, no env)
cargo test -p futurefin-engine

# 0b. Backend integration tests incl. fire-parity server side (needs a test Postgres; NOT run
#     in CI). Test DB not running? One-time ff-test-db setup: futurefin-validation-and-qa §2.
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace

# 0c. Frontend fire-parity (client side of the same fixture) + other Vitest suites
npm install
npm test --workspace futurefin-web
```

Expected observations (verify counts, don't trust memory):
- 0a: **22 engine tests pass** (`grep -c '#\[test\]' crates/engine/src/projection.rs` → 22 as of 2026-07-02). The api crate additionally has 15 unit tests inside `apps/api/src/handlers/projection.rs` (horizon, planning spread, milestones, gross-up) that run in 0b.
- 0b: all integration tests pass, including `fire_parity.rs`: **6 fixture cases** (5 numeric within ±1 €, 1 expecting `null` target) from `apps/api/tests/fixtures/fire-parity.json`, plus `projection_marker.rs` (marker = month 1, 25 points at `?months=24`) and `projection_cache.rs`.
- 0c: `lib/fire.test.ts` passes the **same 6 cases** ±1 € (7 Vitest `it`s: 1 count check + 6 cases).

CI note: `.github/workflows/ci.yml` runs `cargo test -p futurefin-engine` + build + Docker smoke, but the Postgres integration tests (0b) are **not** in CI — they only run when you run them locally. Never claim "CI covers it" for anything in `apps/api/tests/`.

**Gate P0**: if ANY baseline check fails → you are in a regression hunt, not a realism campaign. Pause the campaign, branch to `futurefin-debugging-playbook` (and `futurefin-failure-archaeology` if the failure smells historical). If exactly one side of fire-parity fails → the duplicated FIRE math drifted; fix drift first (see Promotion protocol).

## Phase 1 — Characterize current model fidelity

The inventory below was verified line-by-line on 2026-07-02. Re-verify anchors before relying on them (this is the most-rewritten area of the codebase). Classification: **[CBD]** correct-by-design (documented philosophy, do not "fix"), **[AS]** acceptable simplification (known, cheap, small effect), **[GAP]** realism gap (candidate for Phase 2).

| # | Simplification | Code anchor | Class |
|---|---|---|---|
| 1 | Deterministic single-path returns: no volatility, no sequence-of-returns risk, one expected-value trajectory | whole loop `crates/engine/src/projection.rs:454-568`; no RNG anywhere in the crate | [GAP] |
| 2 | Incomes/expenses/contributions flat in **nominal** euros forever; only the FIRE target moves with inflation | constants consumed at `projection.rs:482-491`; target at `projection.rs:171-182` | [CBD] — v1.2.0 philosophy «haciendo lo que hago ahora, ¿qué tal voy?»; the un-modeled part (salary growth ≈ inflation) is a documented bias |
| 3 | Monthly compounding of the annual rate: multiplier `(1 + p/100)^(1/12)` per month | `monthly_multiplier`, `projection.rs:154-162` | [CBD] — geometric root, so 12 months compose to exactly the stated annual rate |
| 4 | **Negative expected returns silently clamped to 0%**: `p <= 0` returns multiplier 1 | `projection.rs:158-160` | [GAP] — an asset marked −5%/yr does not decay |
| 5 | Tax modeled **only** in the FIRE gross-up (target side). The drawdown path drains net expenses with **no tax on realized gains** | gross-up: `apps/api/src/handlers/projection.rs:106-135`; untaxed drain: `crates/engine/src/projection.rs:184-216,505-513` | [GAP] — target assumes gross withdrawals, simulation drains net: the two halves of the model disagree after the crossing |
| 6 | No rebalancing: money never moves between assets; each grows at its own rate forever | no such code path exists in the crate | [AS] |
| 7 | Deficit drain order: liquid assets first, then illiquid, each group **lowest-return-first** (ties by input index). Illiquid assets ARE drained once liquids are exhausted | `drain_from_assets`, `projection.rs:194-214` | [AS] — tax/interest-agnostic heuristic; note `.claude/engine.md` only says "liquid first" and omits the illiquid fallback |
| 8 | Planning flows («Próximos») without `due_date`: total spread evenly over **90 civil days** from ref_date; dated flows land in their calendar month, past-dated ones dropped. **Since Fase 5/issue #86 (4.4.0)** this same dated/undated split is surfaced, not just simulated: `GET /v1/projection/series` publishes `events`/`events_truncated` (cap `PROJECTION_EVENTS_MAX = 100`, `ProjectionEvent`) listing only the **dated** Próximos that land inside the horizon — because only those produce a discrete step in the curve; undated ones spread over the 90 days and produce a ramp, so including them as an "event" would misdescribe them (rejected design, see `futurefin-failure-archaeology` §2.18: a finer `density` was considered and rejected as answering WHERE, not WHY) | `PLANNING_UNDATED_SPREAD_DAYS=90`, `apps/api/src/handlers/projection.rs:298,331-384`; `events`/`events_truncated`: `PROJECTION_EVENTS_MAX`, `struct ProjectionEvent`, `apps/api/src/handlers/projection.rs` | [AS] |
| 9 | `retirement_monthly_withdrawal` is **always 0** from the production handler, and `retirement_start_month` is **always None** — FIRE crossing is the sole retirement trigger; drawdown = income drops to `income_retirement_monthly`, expenses to `expense_retirement_monthly` | `apps/api/src/handlers/projection.rs:790-793`; engine fields exist only for tests/future use (`crates/engine/src/projection.rs:105-118`) | [CBD] — v1.0.6 removed age triggers |
| 10 | `surplus_cash` (money no rule absorbs, and post-retirement surpluses) earns **0%** and is counted in NW | only asset values get the multiplier (`projection.rs:535-538`); `surplus_cash` never compounds (`projection.rs:435,514-516,529-532`) | [AS] |
| 11 | ~~Liabilities carry **no interest**~~ — **RESUELTO en 4.2.0**. Cada pasivo lleva `repayment_model` (`fixed_payments` \| `french` \| `interest_only` \| `revolving`) y `apr_percent`; `french`/`revolving` devengan `P' = P·(1+i) − M` con `i = apr/1200`, interés sobre el saldo de apertura y cuota a fin de mes. `fixed_payments` (el default de la columna) conserva la recurrencia 1:1 de siempre, bit a bit, así que **la simplificación sigue viva para quien no elija modelo** — es opt-in, no una corrección global | recurrencia: `liability_month`, `crates/engine/src/projection.rs:152`; predicado de actividad: `liability_active`, `:121`; enum: `:81`; cierre aplicado sin recomputar: `closing_principals`, `:820,833,906` | [GAP] cerrado para los modelos que devengan; [AS] residual en `fixed_payments` |
| 14 | **Un pasivo sin plan de pago activo no devenga interés** — ni en modos B/C ni en la segunda pasada neutra del marcador `compound_outpaces_true_savings_month`. El devengo comparte predicado con el cobro de la cuota (`liability_active`: `monthly_payment > 0` y `payment_end` vivo), así que poner `monthly_payment = 0` congela el principal por completo. El handler además anula el TIN en B/C, redundante a propósito | `liability_active` `crates/engine/src/projection.rs:121`; anulación en B/C `apps/api/src/handlers/projection.rs:1099` | [AS] — deliberado: en B/C las cuotas ya viven dentro del promedio de gasto real, devengar encima las contaría dos veces; el coste es que un hogar en modo real ve su deuda plana |
| 15 | **`revolving` comparte recurrencia con `french` en 4.2.0**: misma fórmula, dos etiquetas. Lo propio de una revolving —cuota mínima como % del saldo vivo, disposiciones nuevas— queda **diferido**. La equivalencia está pineada por un test que se cambiará A PROPÓSITO el día que diverjan | `RepaymentModel::Revolving` en `liability_month`, `crates/engine/src/projection.rs:152`; pin `revolving_matches_french_recurrence` | [AS] — decisión de producto (el usuario nombra su deuda como es) con la matemática todavía prestada; la cuota fija sobre una revolving real subestima el coste cuando el saldo baja |
| 12 | `fire_reached` compares NW at close of month k−1 against the target at index k−1 (one shared helper, no off-by-one) | `projection.rs:475-481`; helper `fire_target_at_month_index` `projection.rs:171-182` | [CBD] — post-incident invariant, see fenced paths |
| 13 | In retirement, contributions stop entirely; surpluses pile into 0% cash | `projection.rs:514-516` | [AS] |

Horizon ground truth: 90-year lifespan from the resolved birth date, clamped 5–70 years, 30-year fallback (`projection_horizon_months`, `apps/api/src/handlers/projection.rs:599-627`); `horizon_basis` values are `lifespan_90 | fallback_no_demographics | months_override`. (`projection_target_age` was removed in v1.0.6; the docs that still described it were fixed on 2026-07-02.)

### Discriminating experiments (write the predicted number FIRST, then run)

Each experiment is an engine unit test you add temporarily (or a scratch `#[test]`) — pure, no DB. Pattern: build a minimal `ProjectionInput` like the existing tests (`crates/engine/src/projection.rs:619-641`).

| Targets # | Experiment | Predicted magnitude |
|---|---|---|
| 4 | One asset 10.000 €, `expected_annual_return_percent = -5`, 12 months, no flows | Engine: NW[12] = 10.000 (clamp). Faithful model: 9.500. Effect: **500 € / 5% per year** hidden |
| 11 | Liability 100.000 € principal, 500 €/mes payment, TIN 3 % | `fixed_payments`: debt gone in exactly **200** months (100.000/500). `french` at 3 %: extinguished in month **278**, last instalment partial. Effect: **78 months (6,5 años) of debt service** that the old model skipped, ≈ **38.800 €** of interest never charged (277 × 500 + ~303 − 100.000). **CORRECCIÓN (4.2.0)**: esta fila dijo «≈ 430 months» desde 2026-07-02 y era **falso al 3 %** — 430 meses corresponden a un TIN de ≈ 5 %, y el «~115.000 € de interés» que lo acompañaba salía de la misma cuenta equivocada. El número verificado por el engine (`french_extinction_at_month_278`) es 278. Cifra estimada de memoria en una tabla que nadie recontó: exactamente el modo de fallo que esta campaña persigue |
| 5 | Portfolio 1.000.000 € (purchase_price 400.000 €), retirement drain 2.000 €/mes net expense | Engine drains exactly 2.000 €/mes. Tax-aware: withdrawing enough to net 2.000 € with 60% embedded gain taxed at 19–21% ⇒ gross ≈ 2.245–2.270 €/mes (~11–13% faster depletion). Derive exactly with the bracket math before running |
| 2 | Same input, inflation 0% vs 3%: only `jubilacion_month_index` and `fire_target_series` may change; `net_worth` series must be **bit-identical** | Zero series delta. If the NW series moves with inflation → someone re-broke the v1.2.0 model; stop and treat as regression |
| 10 | Income 3.000/expense 1.000, no allocation rules, 36 months | NW grows exactly +2.000/mes linearly (test `no_rules_routes_surplus_to_cash` proves months 0–3). Any compounding on that cash = bug |
| 7 | Two liquid assets (2% and 7%), force a deficit month | The 2% asset depletes first; the 7% asset untouched until 2% hits zero |
| 1 | No in-repo experiment possible (no stochastic machinery). Effect size is the literature's, not ours — label any number you quote as external | n/a — motivates Phase 2(b)/(c) |

**Gate P1**: every row you rely on must be re-verified at its anchor (the line numbers WILL drift). If an anchor no longer matches the described behavior → the model changed since 2026-07-02; re-derive the row, update this table (this skill file is the doc of record for the inventory), and check `CHANGELOG.md` for the change before proceeding. If you find a behavior not in this table → add it, classified, before designing anything.

## Phase 2 — Solution menu (ranked by value/effort for a solo self-hosted app)

**All items below are CANDIDATES — unimplemented as of 2026-07-02.** Nothing here is promised API. Each item lists: theory/derivation obligations (what must be written down and reviewed before code) and a measurable acceptance test **defined before implementation**. Every item goes through `futurefin-change-control` (engine semantics changes are output-changing: version bump + CHANGELOG).

### (a) Property-based invariant testing — HIGHEST value/effort. Do this first.
Adds `proptest` as a dev-dependency of `crates/engine` only. Catches silent wrongness in the code we already have; changes zero behavior.
- **Obligations**: for each invariant, a one-paragraph proof sketch of WHY it must hold, including its domain of validity.
- **Candidate invariants** (validity caveats are the hard part — encode them):
  1. *Cascade conservation*: for any pool ≥ 0, `sum(alloc) + leftover == pool` and each `alloc[i] ≥ 0` (`distribute_contributions`). Also: no allocation exceeds cap room.
  2. *Per-asset decomposition*: with **no liabilities and no deficit months**, `net_worth[k] == sum_i per_asset_series[i][k] + surplus_cash_k`; testable from outputs alone when a `remainder` rule exists and pool is fully absorbed (then implied surplus_cash increments are exactly the leftovers = 0).
  3. *Monotonicity under +income* — **scoped**: with `fire_target = None` and `retirement_start_month = None`, raising `income_regular_monthly` never decreases any `net_worth[k]`. Do NOT assert it globally: more income ⇒ earlier FIRE crossing ⇒ income drops to `income_retirement_monthly` sooner ⇒ later NW can legitimately be LOWER. This non-obvious falsifier is exactly what proptest should also document.
  4. *NW continuity*: `|net_worth[k] − net_worth[k−1]|` bounded by `|net_cash_month| + growth + debt payments` for that month.
  5. *Determinism*: same input twice ⇒ identical output (guards future stochastic work).
- **Acceptance test**: proptest suite green over ≥ 10^4 generated cases per invariant; each invariant's domain caveat written into the test's doc comment; `cargo test -p futurefin-engine` still passes.

### (b) Monte Carlo / stochastic returns — high value, high effort. Design before code.
- **Obligations (all BEFORE implementation)**: (1) justified return distribution (e.g. annual lognormal with user-set μ = current `expected_annual_return_percent`, σ per asset class; cite the justification in the design note); (2) **SEEDED determinism preserved** — seed derived deterministically from input (e.g. hash of installation id + parameters), same request ⇒ same bands; the projection cache and the `r1.body == r2.body` assertion in `apps/api/tests/projection_cache.rs` must keep holding; (3) percentile-band API design: extend `ProjectionSeriesResponse` additively (e.g. `net_worth_p10/p50/p90` alongside the existing deterministic series — never replace it); (4) wire-size plan following the v1.4.0 precedent: bands as f64 arrays, decimated under `?density=hybrid`, gzip; budget the payload (each extra band ≈ the size of `points` — measure, don't guess).
- **Acceptance test (pre-registered)**: with σ = 0 the p10/p50/p90 bands equal the deterministic series exactly; with σ > 0, p50 within a stated tolerance of the deterministic path over 1.000 seeded runs; fire-parity untouched; cache tests green; response size at `density=hybrid` under a stated KB budget.

### (c) Sequence-of-returns risk surfacing near the jubilación crossing — depends on (b) or a cheaper deterministic stress.
Cheap deterministic variant that needs no RNG: re-run the projection with a fixed stress path (e.g. −30% shock applied in the crossing year) and report how many months the crossing slips. **Obligation**: define the shock convention (when, to which assets) in writing first. **Acceptance test**: a fixture input with known crossing month k where the stressed crossing is a hand-derived k+Δ.

### (d) Tax-aware drawdown — closes gap #5.
- **Obligations**: derive the gross-withdrawal formula from embedded-gain fraction `g = (value − basis)/value` and the existing bracket math (reuse `gross_up_net_annual_fire`'s closed-form approach — see `futurefin-proof-and-analysis-toolkit` for the closed-form-vs-iteration recipe); decide basis tracking per asset (`purchase_price` exists on `SimAsset`, `projection.rs:32`); state the interaction with drain ordering (#7) — tax-aware ordering is a separate, later decision.
- **Acceptance test**: hand-computed case (single asset, one bracket, known g) matches engine drain to < 0,01 €; with `taxes_enabled = false` output is bit-identical to today's; fire-parity fixture regenerated ONLY if the target-side formula changed (it should not).

### (e) Variable / dynamic SWR — lowest priority.
Guyton-Klinger-style guardrails or CAPE-based SWR. **Obligations**: pick ONE published rule, cite it, define its inputs from data we actually have; UI copy in Spanish. **Acceptance test**: fixture cases where the dynamic rule degenerates to the fixed SWR (guardrails never triggered) produce today's numbers exactly. Requires (b) to be honest — a dynamic SWR on a deterministic path is theater; if (b) is not done, defer (e).

**Gate P2**: an item may move from candidate → in-progress only when its obligations doc + pre-registered acceptance test exist in the PR/branch. If while implementing you discover the acceptance test was wrong, STOP, rewrite the prediction, and note the miss (research-methodology lifecycle) — do not quietly fit the test to the code.

## Fenced-off wrong paths (do NOT re-litigate; evidence cited)

1. **Do NOT re-deflate simulation internals** (converting the loop to today-euros / real returns). Tried in v1.0.12 ("modelo real puro", CHANGELOG 2026-05-16) — produced incoherent behavior (asset drain before retirement with inflation on) and was replaced in v1.2.0 by the current all-nominal + moving-target model. Settled. Deflation exists ONLY at display/handler edges (`deflate_points_to_today`, `apps/api/src/handlers/projection.rs:466-486`).
2. **Do NOT reintroduce age-based retirement triggers / target age.** `projection_target_age` was removed in v1.0.6 (migration `20260516120000_drop_projection_target_age.sql`); it caused the contributed-capital-stops-early visual bug. FIRE crossing is the sole trigger. (`futurefin-failure-archaeology` has the full chronicle.)
3. **Do NOT compute anything from array index on decimated series.** v1.4.2: the chart deflated by array index instead of `month_index` — invisible at monthly density, wrong at `hybrid` (non-equidistant points). Always use `p.month_index`. Corollary: **milestones and the jubilación crossover stay computed on the full monthly series** (`points_full`, `apps/api/src/handlers/projection.rs:1062-1075,1124-1134`), never on the serialized `points`.
4. **Do NOT switch engine internals to f64.** Money is `rust_decimal::Decimal` end-to-end (non-negotiable, `futurefin-architecture-contract`). The only sanctioned f64 is the serialization boundary for large arrays (v1.4.0, `serialize_decimal_as_f64`, `apps/api/src/handlers/projection.rs:177-179`) — precision audited < 1 € over 70y. New percentile bands may use that same boundary; the simulation may not.
5. **Do NOT duplicate the moving-target formula outside `fire_target_at_month_index`** (`crates/engine/src/projection.rs:171-182`). Historical incident: engine used `(k−1)/12`, handler used `month_index/12` — a one-month off-by-one between the drawn series and the actual crossing. One helper, both consumers. Same rule for the FIRE-number formula: the sanctioned duplication is exactly Rust handler ↔ `apps/web/src/lib/fire.ts`, guarded by the shared fixture — no third copy, ever.
6. **Do NOT let a "small model tweak" skip the parity fixture.** If tax brackets, gross-up, or `compute_fire_target_nw`'s contract change on either side, regenerate `expected_target_nw` in `apps/api/tests/fixtures/fire-parity.json` and BOTH suites must pass. One-sided green = drift, not success.

## Validation-and-promotion protocol (routed through futurefin-change-control)

Any Phase 2 implementation, and any Phase 1 "gap fix", follows this sequence:

1. **Classify** the change with `futurefin-change-control` (engine-output-changing ⇒ version bump + CHANGELOG "breaking"/"changed" entry; additive API fields ⇒ still CHANGELOG + `.claude/api-routes.md`).
2. **Predict before running** (`futurefin-research-methodology`): write the exact expected numbers (series values, crossing month, target €) into the PR description or test comments BEFORE executing the new code.
3. **Capture a baseline regression test**: before changing the engine, add a unit test pinning current outputs for a representative input (assets + rules + liability + planning + fire_target). Run it green on the OLD code, commit it, then change the engine. If the change is meant to alter outputs, the test's diff is your measured effect size — update expectations explicitly, with the delta stated in the CHANGELOG.
4. **Run the full Phase 0 baseline** (all three commands). fire-parity discipline per fenced path #6. Tolerance culture: parity ±1 €; engine assertions typically < 0,01 € — pick and state a tolerance, never `assert_eq!` on long Decimal chains involving `powd`.
5. **Docs of record** (`futurefin-docs-and-writing`): update `.claude/engine.md` (fixing the known drift if you're in that section), and write a **forensic CHANGELOG entry** — what changed, why, the old behavior, and the measured deltas, in the house style (see v1.2.0's entry as the model).
6. **Update this skill's Phase 1 table** if any row's classification or anchor changed.

**Gate PROMOTE**: a realism improvement is "adopted" only when: pre-registered acceptance test passes, baseline suites green on both sides, effect size measured and documented, CHANGELOG written. Anything less stays labeled candidate/in-progress. If two consecutive implementation attempts of the same item fail their acceptance test, retire the hypothesis into `futurefin-failure-archaeology` territory with the evidence, rather than trying a third blind variation.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`). Re-verify before trusting:

- Engine test count: `grep -c '#\[test\]' crates/engine/src/projection.rs` (22) and handler unit tests: `grep -c '#\[test\]' apps/api/src/handlers/projection.rs` (15).
- Parity case count: `grep -c '"name"' apps/api/tests/fixtures/fire-parity.json` (6; tolerance `_tolerance_eur: 1.0`).
- Negative-return clamp still present: `grep -n "p <= Decimal::ZERO" crates/engine/src/projection.rs` (monthly_multiplier).
- Handler still forces no explicit withdrawal/age: `grep -n "retirement_monthly_withdrawal: Decimal::ZERO\|retirement_start_month: None" apps/api/src/handlers/projection.rs`.
- Liability interest (rows 11/14/15) — **el grep viejo (`principals\[i\] -= pay`) ya no existe**: 4.2.0 sustituyó el bloque de resta por el helper único. Vigente: `grep -n "fn liability_month\|fn liability_active\|enum RepaymentModel" crates/engine/src/projection.rs` (tres hits) y `grep -c '#\[test\]' crates/engine/src/projection.rs` (**44** a 2026-08-25). Que `fixed_payments` siga siendo la recurrencia 1:1 de antes lo prueba el pin: `grep -n "pre_4_2_0\|liability_pin_input" crates/engine/src/projection.rs`.
- Horizon basis strings: `grep -n "lifespan_90\|fallback_no_demographics\|months_override" apps/api/src/handlers/projection.rs`.
- Undated planning spread: `grep -n "PLANNING_UNDATED_SPREAD_DAYS" apps/api/src/handlers/projection.rs` (90).
- Dated-flow `events`/`events_truncated` (row 8, added Fase 5/issue #86, 4.4.0): `grep -n "PROJECTION_EVENTS_MAX\|struct ProjectionEvent" apps/api/src/handlers/projection.rs`; the rejected `density`-parameter alternative is `futurefin-failure-archaeology` §2.18.
- Single FIRE-target helper still sole source: `grep -rn "fire_target_at_month_index" crates/ apps/api/src/ | wc -l` (definition + engine + handler call sites only).
- CI still excludes integration tests: `grep -n "cargo test" .github/workflows/ci.yml` (only `-p futurefin-engine`).
- Stochastic work still unimplemented: `grep -rn "proptest\|rand\|monte" crates/engine/ apps/api/src/handlers/projection.rs` (expect no hits) — if this hits, Phase 2 items (a)/(b) have started; reconcile this file.
- Doc drift record (all previously stale docs on this area were fixed 2026-07-02): standing-errata table in futurefin-docs-and-writing §7; migration count via `ls apps/api/migrations | wc -l`.
