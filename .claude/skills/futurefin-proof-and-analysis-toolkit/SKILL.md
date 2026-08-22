---
name: futurefin-proof-and-analysis-toolkit
description: >
  First-principles analysis recipes for FutureFin numeric/engine work, each with a worked example
  from this repo's real history. Load this skill BEFORE: replacing an iterative numeric method with
  a closed form (gross-up, solvers), touching any month_index / array-index math (projection series,
  deflation, decimated densities), refactoring code that must not change output (cache, spawn_blocking,
  query consolidation), changing FIRE math that is duplicated Rust↔TypeScript (fire-parity.json),
  deciding Decimal vs f64 for a new field, or auditing engine determinism/purity. Symptom triggers
  while doing planned numeric work (refactoring, deriving, reviewing): "refactor changed a value",
  "is f64 safe here?", "my index math might be off by one month", "does this closed form really
  equal the old iteration?". NOT for live-symptom triage — if you start from a bug report
  ("numbers look wrong", "chart diverges from KPI"), load futurefin-debugging-playbook FIRST and
  come here once you know which computation to prove. Also NOT for process/lifecycle discipline
  (use futurefin-research-methodology), the projection realism campaign itself
  (futurefin-projection-realism-campaign), FIRE formula reference (futurefin-fire-domain-reference),
  or test-harness mechanics (futurefin-validation-and-qa).
---

# FutureFin proof-and-analysis toolkit

Facts date-stamped 2026-07-02, app version 1.4.3 (`apps/api/Cargo.toml`), 31 files in
`apps/api/migrations/`.

**Core principle — "prove it, don't just install it".** In this codebase, errors in projection
math are *silent*: the output is a plausible-looking curve either way. Every incident below shipped
compiling, type-checked code that produced wrong numbers. So a change to numeric code is not done
when it compiles or when one spot-check looks right; it is done when you have an *argument* (a
derivation, an index-domain proof, an error bound, a parity fixture) plus *evidence* (a comparison
against an independent reference). Each recipe here is: when to use → steps → the real historical
case → a checklist you can execute.

## Vocabulary (used throughout)

| Term | Meaning here |
|---|---|
| FIRE | Financial Independence / Retire Early. FutureFin's sole retirement trigger is the FIRE crossover (net worth ≥ moving FIRE target). |
| SWR | Safe Withdrawal Rate (`swr_pct`, e.g. 3.5). `target_nw = gross_annual_need / (swr_pct/100)`. |
| Gross-up | Inverting a progressive tax: find `gross` such that `gross − tax(gross) = net`. Needed because SWR withdrawals are taxed. |
| Nominal vs real | Nominal = euros of the moment; real = deflated to today's purchasing power. Since v1.2.0 the whole series is nominal; only the FIRE target grows with inflation. |
| Deflation | Multiplying a nominal value at month `m` by `1/(1+infl/100)^(m/12)` to express it in today's euros. Display-only. |
| Cascade | Ordered `AllocationRule` list distributing monthly surplus to assets (`crates/engine/src/projection.rs`). |
| month_index | Position on the simulated time axis: 0 = today (series start), k = end of simulated month k. NOT the same as array index once density decimation exists. |
| Density `hybrid` | `/v1/projection/series?density=hybrid` returns months 0..12 monthly then 24, 36, … annual (~82 of ~841 points). Points are non-equidistant. |

---

## Recipe 1 — Closed-form derivation before code (the tax gross-up, v1.3.0)

**When to use:** you are replacing an iterative/numeric method (binary search, fixed-point loop)
with a direct formula — or introducing any new formula whose correctness is not obvious from types.

**Steps**
1. Write the function you are inverting/solving as explicit math, per regime. For piecewise-linear
   functions, write each piece with its domain.
2. Solve symbolically per piece. State the *domain condition* under which each piece's solution is
   valid (the candidate must land inside its own piece).
3. Walk the real production constants through the derivation by hand (or `python3` + `decimal`)
   and record intermediate values.
4. Identify degenerate regimes (division by ~0, empty input, disabled feature) and decide behavior
   explicitly.
5. Keep the old method as a *reference implementation inside the test module* and assert
   new ≈ old across cases spanning every piece, plus the round-trip property (`f(solution) = input`).

**Worked example.** Before v1.3.0, `gross_up_net_annual_fire` ran a 90-iteration binary search.
The after-tax function is piecewise-linear in gross: within bracket *i* (rate `r_i`, lower bound
`prev_i`, cumulative tax of the lower brackets `K_i`):

```
after(g) = g − tax(g) = g·(1 − r_i) + (r_i·prev_i − K_i)
⇒  g = (net + K_i − r_i·prev_i) / (1 − r_i)      valid iff g ≤ ceiling_i
```

Walk `net = 30 000 €` through the Spanish defaults (0–6 000 @19 %, –50 000 @21 %, –200 000 @23 %,
–300 000 @27 %, ∞ @30 %):

- Bracket 1: `g = 30000/0.81 = 37 037.04` > 6 000 → not valid. Advance: `K = 0.19×6000 = 1140`, `prev = 6000`.
- Bracket 2: `g = (30000 + 1140 − 0.21×6000)/0.79 = 29 880/0.79 = 37 822.78` ≤ 50 000 → **solution**.
- Check: `tax(37 822.78) = 1140 + 0.21×31 822.78 = 7 822.78`; after-tax = 30 000 exactly. ✓
- This is the fixture value: `_calc_note: "gross_up(30000, ES brackets) ≈ 37822.78 → /0.035 = 1080650.99"`.

The shipped code is `gross_up_net_annual_fire` in `apps/api/src/handlers/projection.rs` (~line 106),
with the derivation in its doc comment. Degenerate case handled explicitly: `r ≥ 100 %` →
`denom ≤ 0` → returns `prev_ceiling` instead of dividing. Acceptance evidence was **equivalence to
the old method**: the test module `gross_up_tests` (same file, ~line 1555) keeps
`gross_up_binary_reference` (the literal old 90-iteration search) and asserts closed-form == binary
within **±0.01 €** over 9 nets from 1 000 to 1 000 000 (spanning every bracket), plus the
round-trip `after(g_closed) ≈ net`. CHANGELOG v1.3.0: "Resultado idéntico ±0.01 €."

**Check your work**
- [ ] Derivation written down (doc comment or CHANGELOG), not just code.
- [ ] Per-piece validity condition stated and enforced in code.
- [ ] Old method preserved as in-test reference; comparison cases cover *every* piece/regime,
      including boundaries (a net that lands exactly on a bracket ceiling).
- [ ] Round-trip property asserted (`f(solution) − input| ≤ tol`), not only new-vs-old.
- [ ] Degenerate inputs (0, negative, disabled flag, 100 % rate) return an explicitly chosen value.
- [ ] Tolerance justified: ±0.01 € here because the binary reference itself only converges to
      ~(hi−lo)/2^90; the closed form is the *more* exact of the two.

---

## Recipe 2 — Off-by-one / indexing proofs (fire_target off-by-one; v1.4.2 deflation bug)

**When to use:** any code mapping between loop counters, array indices, and `month_index` —
projection series, deflators, milestone crossings, decimated arrays, anything with `k`, `k-1`,
`i`, `/12`.

**Steps**
1. **Write the index domain explicitly** before touching code. For the projection series:
   `net_worth` has length `horizon_months + 1`; index 0 = today (state before any simulated month);
   index k = state at *end* of simulated month k. The engine loop runs `for k in 1..=horizon_months`.
2. **Prove the formula at boundaries by hand**: evaluate at k = 0, 1, 12, 13 and check each against
   the English sentence of what it should mean ("month_index 12 = one full year of inflation" →
   factor `(1+r)^1`, not `(1+r)^(11/12)`).
3. **If two consumers need the same formula, extract ONE function and make both call it.** Duplicated
   formulas *will* drift by one.
4. **Never index a decimated array by position.** After `density=hybrid`, array index ≠ month_index.
   Any per-point computation must read `p.month_index` from the point itself.

**Worked example A — the engine/handler fire-target off-by-one (fixed v1.3.0).** The engine's
crossover check used `years = (k−1)/12` (correct for *its* frame: at the top of month k it compares
`nw_prev` = net worth at end of month k−1 against the target *at that same axis point*, see
`projection.rs` line ~478: `fire_target_at_month_index(…, k − 1)`). The handler independently built
`fire_target_series` with `years = month_index/12`. Both formulas are individually defensible — but
they were **two implementations of one axis**, one month apart: the plotted target curve disagreed
with the crossing that set `fire_reached`. Fix: single public helper
`fire_target_at_month_index(ft, month_index)` in `crates/engine/src/projection.rs` (~line 171),
consumed by both engine and handler; its doc comment declares it "la única fuente de verdad".
Regression test `fire_target_helper_matches_compound_factor_at_year_boundaries` (same file, ~line
1093) pins the boundary semantics: `month_index = 12` → exactly `base × (1+r)`, `60` → `(1+r)^5`.

**Worked example B — deflating by array index (fixed v1.4.2).** `ProjectionNetWorthChart` deflated
each point as `value / (1+infl)^(arrayIndex/12)`. With `monthly` density arrayIndex == month_index,
so the bug was **invisible in every monthly-density test**. With `hybrid` density, point 13 is
month 24: the chart deflated it by 13/12 years instead of 24/12 — systematically under-deflating
everything after month 12, and disagreeing with the backend's `milestones_real` (which are computed
on the full monthly series via `deflate_points_to_today`, `apps/api/src/handlers/projection.rs`
~line 466, precisely so hybrid decimation cannot corrupt crossing months). Fix:
`deflator(p.month_index)` — see `apps/web/src/views/ProjectionNetWorthChart.tsx` ~lines 195–201.
Lesson stated in CHANGELOG v1.4.2: *decimated series break index math*.

**Check your work**
- [ ] Index domain written as a sentence: what does index 0 mean, what does index k mean, what is len?
- [ ] Formula evaluated by hand at k = 0, 1, 12, 13 and each matches its English meaning.
- [ ] Any formula needed in two places lives in ONE function both call
      (grep for the exponent pattern: `grep -rn "powd\|Math.pow" apps crates | grep -i "12"`).
- [ ] No per-point math uses array position where `month_index` exists on the point.
- [ ] Tested under BOTH densities (`monthly` and `hybrid`) — a bug invisible under monthly is the
      *expected* failure mode, not a corner case.
- [ ] A boundary regression test pins the year-boundary values (like
      `fire_target_helper_matches_compound_factor_at_year_boundaries`).

---

## Recipe 3 — Bit-exact regression capture (refactors that must not change output)

**When to use:** performance/structure refactors of numeric paths where the contract is "same
numbers out": `spawn_blocking`, parallelizing simulations, query consolidation, caching,
serialization reshaping, extracting helpers.

**Steps**
1. **Before** refactoring, write a test that drives the real endpoint/function with a deterministic
   setup and asserts the *current* values. Capture them by running the test once with a wrong
   expected value, reading the failure output, then committing the real value (rule of thumb from
   `.claude/tests.md`: "capture the value first (test will fail), then commit the expected value
   once green").
2. Commit the green test on the pre-refactor code.
3. Refactor.
4. Test must stay green with the *same* expected values. Any diff = the refactor changed behavior;
   stop and explain the diff before proceeding.

**Worked example.** `apps/api/tests/projection_marker.rs` was written for exactly this: v1.3.0
wrapped the engine in `spawn_blocking` and ran the second simulation (for the
`compound_outpaces_true_savings_month_index` marker) under `tokio::join!`. The test seeds a
deterministic installation (100 k asset at 15 % annual, +100 €/month net savings, one remainder
rule), calls `GET /v1/projection/series?months=24`, and pins: `starting_net_worth = 100 000 ±0.01`,
`monthly_delta_assumption = 100 ±0.01`, marker `= Some(1)` exactly, `points.len() == 25`
(horizon 24 → indices 0..=24), and NW at month 12 inside a stated reasoned range. CHANGELOG v1.3.0
claims the refactor kept output "bit-exact" — the underlying Decimal pipeline is unchanged — while
the *test* asserts through `f64` parsing of the decimal strings, hence ±0.01 tolerances.

**Tolerance policy** (match what the repo already does):

| Comparison | Tolerance | Why |
|---|---|---|
| Pure-Decimal refactor, asserted on Decimal directly | bit-exact (`assert_eq!`) | Nothing lossy in the path. |
| Decimal asserted via string→f64 parse (integration tests) | ±0.01 € | API serializes `"1000.0000"`; f64 parse is the only lossy step. |
| Closed form vs iterative reference | ±0.01 € | Iterative side only converges, doesn't terminate exactly. |
| Cross-language Rust↔TS parity | ±1 € | TS side computes in f64 end-to-end (see Recipe 4/5). |
| Discrete outputs (month indices, marker months, point counts) | exact, always | An off-by-one here is a bug, never noise. |

**Check your work**
- [ ] Test existed and was green BEFORE the refactor commit (check `git log` order).
- [ ] Setup is deterministic: fixed amounts, no reliance on wall-clock beyond the injected ref date
      (the engine takes `ref_date` as input — see Recipe 6).
- [ ] Discrete outputs asserted exactly; continuous ones with the table's tolerance and a comment
      saying which lossy step justifies it.
- [ ] Run them locally anyway: `TEST_DATABASE_URL=… cargo test --workspace` and
      `npm test --workspace futurefin-web`. Since 4.0.0 CI does run both (jobs `integration` and
      `web`), but waiting for CI to find what a local run finds in two minutes is a bad trade.

---

## Recipe 4 — Cross-implementation parity proof (the fire-parity.json pattern)

**When to use:** the same math intentionally exists in two implementations. Live case: FIRE target
math is duplicated in Rust (`compute_fire_target_nw` + `gross_up_net_annual_fire`,
`apps/api/src/handlers/projection.rs` — source of truth) and TypeScript
(`computeFireAnnualNeedNetEur` + `grossUpNetAnnualFire`, `apps/web/src/lib/fire.ts` — live form
preview without a round-trip). Note the asymmetry: as of v1.4.3 the TS side still uses the
90-iteration binary search while Rust uses the closed form — parity within ±1 € is exactly what the
fixture proves despite different algorithms.

**The pattern:** ONE canonical fixture, `apps/api/tests/fixtures/fire-parity.json` (6 cases:
modes manual/annual_expense/current_income × taxes on/off × null-target case), consumed by BOTH
suites:
- `apps/api/tests/fire_parity.rs` — seeds a real installation per case via the HTTP API and asserts
  `jubilacion_target_net_worth` ≈ `expected_target_nw` ±1 €.
- `apps/web/src/lib/fire.test.ts` — loads the same file via `readFileSync("../../../api/tests/fixtures/fire-parity.json")`
  and runs the TS helpers to the same ±1 €.

If someone edits brackets or the formula on one side only, exactly one suite goes red — the drift
is localized automatically.

**How to derive an expected value INDEPENDENTLY** (never copy one implementation's output into the
fixture — that would enshrine its bugs). Use a `python3` decimal walk-through as the third opinion:

```bash
python3 - <<'EOF'
from decimal import Decimal as D
brackets = [(D(6000), D("0.19")), (D(50000), D("0.21")),
            (D(200000), D("0.23")), (D(300000), D("0.27")), (None, D("0.30"))]
def gross_up(net):
    prev, K = D(0), D(0)
    for up_to, r in brackets:
        g = (net + K - r*prev) / (1 - r)
        if up_to is None or g <= up_to:
            return g
        K += r * (up_to - prev); prev = up_to
net = D(30000)                       # annual net need for the case
g = gross_up(net)
print("gross:", g, " target_nw:", g / D("0.035"))   # / (swr_pct/100)
EOF
# → gross: 37822.78…  target_nw: 1080650.99…  (matches fixture case 2)
```

**Regeneration discipline** (from `.claude/tests.md` "Shared fixtures"): if you change
`tax_brackets`, the gross-up formula, or the `compute_fire_target_nw` contract on either side —
regenerate every affected `expected_target_nw` from the independent derivation, update `_calc_note`
with how the number was derived, and run BOTH suites. Both must pass before the change is real.
Adding a case = append to `cases[]` with `name`, `fire_settings`, `monthly`, `expected_target_nw`,
`_calc_note`; re-run both suites.

**Check your work**
- [ ] Expected value derived by hand/python3, not pasted from either implementation's output.
- [ ] `_calc_note` records the derivation so the next session can re-check it.
- [ ] Both suites run locally and green: `TEST_DATABASE_URL=… cargo test -p futurefin-api --test fire_parity`
      and `npm test --workspace futurefin-web` (neither runs in CI — see Recipe 3).
- [ ] New cases cover the regime you changed (which bracket, which mode, null-target path).
- [ ] Tolerance stays ±1 € (`_tolerance_eur` in the fixture); do not silently widen it to make a
      diff pass — a needed widening is itself a finding to explain.

---

## Recipe 5 — Decimal precision analysis (when f64 is provably safe)

The non-negotiable: money is `rust_decimal::Decimal` in domain/engine/DB; amounts cross the API as
decimal strings. The *one deliberate exception* (v1.4.0): large projection arrays
(`points[].net_worth`, `points[].contributed_capital`, `fire_target_series`,
`asset_series[].values`) serialize as f64 via `serialize_decimal_as_f64`
(`apps/api/src/handlers/projection.rs` ~line 177), cutting ~30 KB JSON and ~5 000 client-side
parses; scalar KPIs (`starting_net_worth`, `jubilacion_target_net_worth`, milestones) stay
Decimal-as-string.

**When to use:** you are adding a field/path and must decide Decimal vs f64, or reviewing whether
an existing f64 shortcut is safe.

**Steps — bound the error, don't assert it**
1. **Magnitude bound M**: largest value the field can carry. Projections: assume M ≤ 10⁹ € (a
   billion-euro net worth saturates any household case).
2. **Count lossy operations n** on the path: one Decimal→f64 conversion = 1; each subsequent f64
   multiply/add ≈ 1 more ulp each.
3. **Worst-case absolute error ≈ M × n × 2⁻⁵²** (f64 has 52 fraction bits; relative error per op
   ≤ 2⁻⁵³, use 2⁻⁵² to be lazy-safe). Wire case: M = 10⁹, n ≈ 3 (convert + deflate multiply +
   scale) → error ≈ 10⁹ × 3 × 2.2×10⁻¹⁶ ≈ **7×10⁻⁷ €** — seven orders of magnitude below the
   display quantum (UI rounds to whole euros, so anything ≪ 0.5 € is invisible). Even over an
   840-month series this bounds *each point independently*; wire values are never fed back into
   compounding. That is the proof behind the CHANGELOG claim "precision <1 € over 70 y" — the real
   bound is far tighter.
4. **Classify the consumer**:
   - *Terminal display* → f64 OK if step-3 bound ≪ 0.5 €.
   - *Iterated state* (compounding, cascade balances, drain) → **Decimal mandatory**. Not mainly
     because accumulated f64 error is large (840 sequential rounds is still ~10⁻¹³ relative), but
     because (a) engine state feeds **discrete threshold comparisons** (`nw_prev >= target` sets
     `fire_reached`): an error of any size can flip a comparison at the boundary and move a
     *retirement month* — errors amplify from continuous to discrete; (b) decimal fractions like
     0.1 are not representable in binary, so Rust-f64 vs TS-f64 vs Decimal would give three subtly
     different cascades, destroying Recipe-4 parity; (c) it is a repo non-negotiable (CLAUDE.md).
   - *Anything compared against a tolerance in a test* → keep Decimal until the final assert.
5. Write the bound (M, n, result, consumer class) in the PR/CHANGELOG. "It's fine" is not an argument;
   "≤ 7×10⁻⁷ € on a display-only value" is.

**Check your work**
- [ ] M, n, and the computed bound written down where the decision is recorded.
- [ ] The f64 value is terminal: grep the frontend to confirm nothing arithmetic-critical consumes
      it (deflation/scaling for display is fine; feeding it into a target computation is not).
- [ ] Threshold decisions (crossings, month indices) computed server-side on Decimal at full
      monthly resolution — the `milestones_real` design (Recipe 2B) exists precisely so the client
      never derives discrete decisions from decimated f64 arrays.
- [ ] Scalars/KPIs stay Decimal-as-string; only bulk arrays get the f64 treatment.

---

## Recipe 6 — Determinism audit (engine purity contract)

**Contract:** `crates/engine` is pure — same `ProjectionInput`, same `ProjectionOutput`, bit-for-bit,
on any machine, at any time of day. "Today" is *injected* as `ProjectionInput.ref_date` by the
handler; the engine never asks the system for it. (Status of the claim: argued from the purity
audit below — no test currently pins it; a replay regression test asserting two runs are
`assert_eq!`-identical is still an unimplemented candidate, see futurefin-research-frontier item 1.)

**Why it matters**
- **Cache correctness**: the v1.4.0 projection cache serves a stored response for
  (installation, view, owner, density). If output depended on clock/RNG/env, a cache hit would be
  a lie.
- **Parity and regression tests** (Recipes 3–4) assume replaying inputs replays outputs.
- **Future stochastic work** (Monte Carlo, sequence-of-returns risk — candidate directions, all
  UNIMPLEMENTED as of 2026-07-02): randomness must enter as a **seeded RNG passed in the input**,
  never `thread_rng()` inside the crate, or the whole proof toolkit above dies.

**Audit recipe (verified clean 2026-07-02)**

```bash
# 1. No clock / RNG / env / IO reads inside the engine:
grep -rnE "now\(\)|SystemTime|Instant|thread_rng|rand::|env::var|new_v4|std::fs|std::io" crates/engine/src/
# → must print nothing.

# 2. Clock support isn't even compiled in: chrono has default-features=false, features=["alloc"]
#    (no "clock" feature → Utc::now()/Local::now() unavailable at compile time):
grep -n "chrono" crates/engine/Cargo.toml

# 3. Dependency surface stays tiny (chrono, rust_decimal, serde, thiserror, uuid):
sed -n '/\[dependencies\]/,$p' crates/engine/Cargo.toml
# uuid has the "v4" (rand) feature for the type's sake — check no engine code *calls* it:
grep -rn "new_v4" crates/engine/src/    # → nothing; tests use Uuid::from_u128 (deterministic).

# 4. No iteration-order nondeterminism: engine uses Vec everywhere; the one sort with potential
#    ties (drain order) has an explicit deterministic tie-break:
grep -n "then_with" crates/engine/src/projection.rs   # → `.then_with(|| i.cmp(&j))` in drain_from_assets
```

Point 4 is a subtle one worth internalizing: `drain_from_assets` sorts assets (liquid first, then
by rate); without the `.then_with(|| i.cmp(&j))` tie-break, two assets with equal
liquidity+rate could drain in unspecified order. Values would still sum the same, but
`per_asset_series` would be nondeterministic. Equal-total ≠ deterministic.

**Check your work** (run after ANY change to `crates/engine`)
- [ ] All four greps above still clean.
- [ ] No new dependency added to `crates/engine/Cargo.toml` without checking it for clock/RNG/IO.
- [ ] Every new sort/dedup has a total, deterministic ordering (tie-break on index or id).
- [ ] Any new "current date/time" need is added as a `ProjectionInput` field, resolved by the handler.
- [ ] If you introduce randomness: RNG seed/state is an input field; identical seed ⇒ identical
      output is asserted by a test.
- [ ] Double-run check: `cargo test -p futurefin-engine` twice; any flaky test is a determinism
      bug by definition.

---

## When NOT to use this skill

- **Triaging a live symptom from scratch** ("numbers look wrong", "chart diverges from KPI",
  HTTP errors, stale data): `.claude/skills/futurefin-debugging-playbook/SKILL.md` first; return
  here when triage has identified which computation needs a proof.
- **Process/lifecycle questions** — how a hunch becomes an accepted result, evidence bar,
  predict-then-run, when to abandon a line: `.claude/skills/futurefin-research-methodology/SKILL.md`.
- **The projection-realism campaign itself** (which realism gaps to attack, decision gates, current
  campaign state): `.claude/skills/futurefin-projection-realism-campaign/SKILL.md`. This toolkit
  supplies the *proof techniques* that campaign work must apply.
- **FIRE math as a reference** (what the modes mean, SWR semantics, the nominal model, cascade
  behavior): `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- **Test harness mechanics** (TestApp, schema isolation, how to add a suite):
  `.claude/skills/futurefin-validation-and-qa/SKILL.md`.
- **What already broke and why** (full incident chronicle beyond the worked examples here):
  `.claude/skills/futurefin-failure-archaeology/SKILL.md`.
- **Whether a change is allowed at all / release gates**:
  `.claude/skills/futurefin-change-control/SKILL.md` — this toolkit never authorizes skipping those gates.

## Provenance and maintenance

Sources: `crates/engine/src/projection.rs`; `apps/api/src/handlers/projection.rs` (gross-up ~106,
`serialize_decimal_as_f64` ~177, `deflate_points_to_today` ~466, `gross_up_tests` ~1555);
`apps/web/src/lib/fire.ts`; `apps/web/src/views/ProjectionNetWorthChart.tsx` (~190–210);
`apps/api/tests/{projection_marker.rs,fire_parity.rs,fixtures/fire-parity.json}`;
`apps/web/src/lib/fire.test.ts`; `CHANGELOG.md` v1.2.0/v1.3.0/v1.4.0/v1.4.2; `.claude/tests.md`
(reminder: `.github/workflows/ci.yml` runs neither the Postgres integration tests nor Vitest —
run them locally). All line numbers are ~approximate anchors as of v1.4.3.

Re-verify before trusting volatile facts:

- Version: `grep -n '^version' apps/api/Cargo.toml`
- Migration count: `ls apps/api/migrations | wc -l`
- Gross-up closed form still in place + tests: `grep -n "gross_up_net_annual_fire\|gross_up_binary_reference" apps/api/src/handlers/projection.rs`
- TS side still binary search (or has adopted the closed form): `grep -n "for (let i = 0; i < 90" apps/web/src/lib/fire.ts`
- Single fire-target helper still sole source: `grep -rn "fire_target_at_month_index" crates apps/api/src | grep -v test`
- Chart deflates by month_index: `grep -n "deflator(p.month_index)" apps/web/src/views/ProjectionNetWorthChart.tsx`
- f64 wire boundary unchanged: `grep -n "serialize_decimal_as_f64" apps/api/src/handlers/projection.rs`
- Parity fixture cases + tolerance: `grep -n "_tolerance_eur\|\"name\"" apps/api/tests/fixtures/fire-parity.json`
- Engine purity greps: the four commands in Recipe 6.
- CI scope (what is / isn't covered): `grep -nE "cargo (test|build)|npm" .github/workflows/ci.yml`
