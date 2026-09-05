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
| **Refactor de todo un bucle numérico** (5.0.0) | **hash SHA-256 del texto canónico, byte a byte** | Cinco escalares no cubren 10.000 números por caso, y la ESCALA de un `Decimal` se mueve sin que el valor cambie. Ver Recipe 7. |
| **Camino `Decimal` vs camino `f64` del mismo modelo** (5.0.0) | **1 € por mes** en toda la serie; decisiones DISCRETAS exactas; cota relativa 1e-12 solo por encima de `2^53 €`, y el caso marcado | Por encima de `2^53` el espaciado de los `f64` ya supera el euro: exigir ±1 € ahí no es estricto, es imposible, y una cota imposible se acaba desactivando. `crates/engine-stochastic/tests/degeneration.rs`. |

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
decimal strings. **Hay DOS excepciones deliberadas, y la segunda es de 5.0.0** — ver el recuadro tras
los pasos. La primera (v1.4.0): large projection arrays
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

**La segunda excepción sancionada (5.0.0): `crates/engine-stochastic`.** Es el caso que el paso 4
declara imposible —*iterated state* (840 meses de aritmética encadenada) en `f64`— y aun así se
aprobó. Merece la pena entender **por qué no contradice la regla**:

- **Qué se publica**: NADA en euros. Solo magnitudes estadísticas (probabilidad de éxito,
  percentiles de una banda, probabilidad de agotamiento por edad). Todo importe monetario sale del
  camino `Decimal`. La regla del paso 4 sigue intacta: *iterated state* que alimenta un KPI en euros
  es `Decimal` obligatorio, y aquí no alimenta ninguno.
- **La objeción (a) del paso 4 —los umbrales discretos— NO se resuelve con una cota de error, se
  MIDE**: la puerta de degeneración exige que `retirement_month_index`,
  `liquid_crossing_month_index`, `assets_depleted_month_index` y `phase_transitions` salgan
  **exactos** en los dos caminos, sobre toda la batería. Si un umbral se voltea, el test falla; no
  se argumenta que no puede pasar.
- **La objeción (b) —tres cascadas sutilmente distintas— se elimina por construcción**: no hay dos
  implementaciones. Hay **un bucle genérico** (`MoneyOps`) con dos instanciaciones, así que un
  cambio de modelo entra una vez y los dos caminos lo ven a la vez. Duplicar el bucle era la
  alternativa, y es exactamente la familia de fallos que esta casa tiene fichada.
- **La objeción (c) —es un no-negociable de CLAUDE.md— se respeta al pie de la letra**: el freezer
  `crates_engine_src_has_no_f64_outside_comments` **no se tocó ni ganó una excepción**. Lo que lo
  hace posible es la **regla del huérfano**: el trait es público, así que otro crate lo implementa
  sobre su propio newtype sin que `crates/engine` conozca la coma flotante.
- **Las políticas del tipo aproximado van DECLARADAS**, no escondidas: qué hace `total_cmp` con
  `NaN`, cuándo devuelven `None` los `checked_*` (⟺ resultado no finito), cuánta precisión pierde
  `from_decimal`, y la única igualdad con tolerancia del núcleo (`gains_equal`, 1e-12) — aparte
  porque `PartialEq` sigue siendo exacta. **Una tolerancia escondida en un `PartialEq` es lo que
  esta receta existe para evitar.**

Si vas a proponer una tercera excepción, la barra es esta: **de aquí no sale un euro**, hay una
puerta que compara contra el camino exacto sobre casos reales, las decisiones discretas se exigen
idénticas, y no se duplica la implementación.

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
# 1. No clock / RNG / env / IO reads inside the engine.
#    OJO (2026-09-03): el comando de siempre ya NO sale vacío — devuelve `crates/engine/src/lib.rs:
#    use std::fs;`, que es el módulo `#[cfg(test)]` del freezer de f64 leyéndose a sí mismo. No es
#    una violación de pureza (no se compila fuera de tests), pero un grep que imprime cuando el doc
#    dice «nothing» enseña a ignorar el grep. Excluye el fichero del freezer:
grep -rnE "now\(\)|SystemTime|Instant|thread_rng|rand::|env::var|new_v4|std::fs|std::io" crates/engine/src/ \
  | grep -v '^crates/engine/src/lib.rs:'
# → must print nothing.
#    Y el control que 5.0.0 añade: el RNG entra en el crate estocástico, NUNCA aquí.
grep -c "rand" crates/engine/Cargo.toml        # → 0
grep -n "rand_chacha" crates/engine-stochastic/Cargo.toml   # → ahí sí, pineado

# 2. Clock support isn't even compiled in: chrono has default-features=false, features=["alloc"]
#    (no "clock" feature → Utc::now()/Local::now() unavailable at compile time):
grep -n "chrono" crates/engine/Cargo.toml

# 3. Dependency surface stays tiny (chrono, rust_decimal, serde, thiserror, uuid):
sed -n '/\[dependencies\]/,$p' crates/engine/Cargo.toml
# uuid has the "v4" (rand) feature for the type's sake — check no engine code *calls* it:
grep -rn "new_v4" crates/engine/src/    # → nothing; tests use Uuid::from_u128 (deterministic).

# 4. No iteration-order nondeterminism: engine uses Vec everywhere; the one sort with potential
#    ties (drain order) has an explicit deterministic tie-break:
grep -n "then_with" crates/engine/src/sim_core.rs      # → `.then_with(|| i.cmp(&j))` in drain_order_g
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

## Recipe 7 — Refactor bit-idéntico con arnés golden (el motor por fases, 5.0.0)

**Cuándo usarla:** vas a mover, generalizar o reescribir código numérico que **no puede cambiar ni
un dígito**, y el cambio es demasiado grande para la Recipe 3 (un test de endpoint con cinco
escalares no cubre 10.000 números por caso). Casos reales: hoistar un invariante fuera del bucle,
sustituir cuatro escalares por un objeto, hacer el bucle **genérico sobre su tipo numérico**.

La diferencia con la Recipe 3 no es de grado: allí capturas *unos* valores y confías en que sean
representativos. Aquí capturas **todo lo que la función publica**, para **toda** una batería, y
reduces el resultado a un hash por caso. Es lo que hizo posible el tren 5.0.0 —cinco refactores
seguidos sobre el bucle— sin una sola regresión silenciosa.

**Pasos**

1. **Una batería, un sitio.** Extrae los casos a un módulo compartido
   (`crates/engine/tests/common/cases.rs`) y haz que TODOS los consumidores lo usen. Dos baterías
   escritas por separado divergen en cuanto alguien «mejora» una, y entonces el pin deja de pinear
   lo que el volcado vuelca. Añade el test que fija la relación entre ellas
   (`the_audit_battery_is_the_ordered_prefix_of_the_pinned_battery`).
2. **Canonicaliza a TEXTO, no a `f64`.** Cada número por su `Display` completo — en `Decimal` eso
   incluye la **escala**, que es justo lo que un refactor mueve sin querer. Un `assert` sobre el
   valor no habría visto nada.
3. **Hashea por caso y guarda el fixture.** SHA-256 del texto canónico. El hash da la señal binaria
   («esto cambió»); guarda además **cuatro escalares legibles** por caso (patrimonio final, líquido
   final, aportado, mes de agotamiento) para que un hash que se mueve tenga un titular en el diff.
   Sin eso, un pin roto se «arregla» regenerando sin mirar.
4. **Escribe el control negativo.** Muta una salida a propósito y exige que el hash se mueva
   (`the_hash_actually_notices_a_single_moved_decimal`). Un arnés sin él es un test que siempre pasa.
5. **Regenerar es un acto declarado**, con variable de entorno propia (`UPDATE_ENGINE_PINS=1`) y
   **obligación de CHANGELOG**. Un pin regenerado sin entrada es un cambio de números que nadie
   declaró.
6. **Cuando el arnés tenga que CRECER** (una salida nueva entra en la canonicalización), no lo metas
   en el mismo fichero: **fixture aditivo aparte**. El pin viejo demuestra que lo viejo no se movió,
   y dejaría de poder demostrarlo si creciera. Y añade el test de dos etapas que rehashea la capa
   vieja **sola** contra los hashes anteriores
   (`the_5_0_canonicalization_grew_without_moving_the_old_fields`).

**Las cuatro trampas que este refactor encontró, y ninguna la habría cazado un `assert` de valor**

| Trampa | Qué pasa | Cómo se evita |
|---|---|---|
| **`max` inherente vs `Ord::max`** | `rust_decimal` tiene `min`/`max` **inherentes** que devuelven `self` en el empate; `Ord::max` devuelve `other`. Mismo valor, **distinta escala** ⇒ distinto `Display` ⇒ distinto hash: `x.max(ZERO)` con `x = 0.000000000000000000` da `"0"` por `Ord` y `"0.000000000000000000"` por el inherente | Al abstraer a un trait, **delega en el método inherente**, no en el del `Ord`. Y `clamp` no es `max(lo).min(hi)`: `Ord::clamp` devuelve `self` intacto dentro del intervalo, y esa identidad conserva la escala |
| **La escala del cero** | Sumar un cero de escala 0 a un acumulador de escala 18 devuelve **el operando**, no la suma — mismo valor, otro `Display`. Por eso una magnitud que a veces «no aplica» debe ser `Option<M>` y **no acumularse cuando no hubo evento**, en vez de sumar un 0 | Distingue «no ocurrió» de «ocurrió y vale cero» **en el tipo** |
| **Un producto acumulado en vez de la potencia** | `powd` enruta los exponentes ENTEROS por `checked_powu` (potencia exacta); calcular `q(j+1) = q(j)·q(1)` los desvía a `exp`/`ln` y mueve los últimos dígitos | Al precalcular una familia `(1+p)^{k/12}`, haz **la misma llamada** que hacía el bucle, nunca una recurrencia multiplicativa |
| **Re-derivar aguas abajo un hecho que el algoritmo ya sabe** | Un llamante deducía «¿se vendió el techo entero?» comparando `gross >= cap`. Exacto en `Decimal`, **filo de navaja** en aritmética aproximada — y de esa rama colgaba qué es recorte informativo y qué es descubierto que resta patrimonio. Coste medido: 8.138 € en un caso | **Publica el booleano** desde donde se sabe. Dos definiciones del mismo hecho divergen en cuanto cambia el tipo, la escala o el redondeo |

**Un golden de 19 casos no demuestra bit-identidad; el fuzz diferencial contra el motor anterior sí**
(pase de correcciones de la revisión adversarial, 2026-09-03, issue #207). Dos regresiones de
bit-identidad con 4.15.0 pasaron los 19 casos pineados y solo aparecieron al comparar el motor
nuevo contra el de `main` sobre hogares ALEATORIOS (`crates/engine/tests/fuzz_invariants.rs`, 1.500
casos; y una campaña de fuzz diferencial aparte, 3.000 entradas por semilla): (1) `undrained_cumulative`
re-derivado como `need − (need − s)` en vez de acumularse con el operando LITERAL que publica el
paseo — algebraicamente igual, pero cambia la ESCALA (`"0"` vs `"0.00"`), movió 438 de 3.000 hashes
y es la misma trampa de «la escala del cero» de la tabla de arriba, encontrada por un mecanismo
distinto; (2) `debt_service` reagrupado de `acc + ((cash + extra) + fee)` a
`((acc + cash) + extra) + fee` — la asociatividad de `+=` no es libre en `Decimal`: con dos pasivos
redondea distinto en el dígito 28 y la diferencia se propaga mes a mes. La campaña completa bajó las
divergencias de 536/496/496 a 24/21/27 por 3.000 entradas (tres semillas), y las que quedan son
todas «el motor viejo entraba en pánico» (desbordamientos que 4.15.0 no tipaba), no desacuerdos
numéricos. **Por qué el golden no bastaba**: 19 casos fijados por adelantado no pueden cubrir cada
combinación de escala y redondeo que un refactor puede tocar; el fuzz diferencial compara la MISMA
entrada aleatoria por los dos caminos y encuentra la divergencia sin tener que haberla previsto.

**El inverso exacto en vez de la bisección** (variante de la Recipe 1, aplicada aquí). WP2 necesitaba
la operación **inversa** del gross-up mixto: dado un techo BRUTO, qué se vende de cada tramo y qué
netea. La tentación es bisecar sobre la función directa. Pero `F(G) = G − tax(B(G))` es **lineal a
trozos** —pendiente `1 − r·g_j` mientras se vacía el tramo `j` bajo el tipo `r`— y sus quiebros son
conocidos: las fronteras de capacidad (cambia `g`) y los techos de tramo fiscal (cambia `r`).
**Recorrer los quiebros da el resultado EXACTO en ≤ `n + |tramos|` pasos**, sin tolerancias, sin
oscilación en las fronteras y con números reproducibles a mano. La bisección sobre esa misma función
es la familia que la arqueología ya retiró (§2.23). Regla general: **antes de bisecar, pregúntate si
la función es lineal a trozos con quiebros que puedes enumerar.**

**Cuándo la bisección SÍ es la respuesta.** El mismo tren la usa, y a propósito, en `solve.rs`: ahí
la función objetivo es **la simulación entera** (cascada, topes, deuda, fiscalidad, latch de
jubilación), no hay forma cerrada ni la habrá, y una aproximación escalar produciría un número
plausible que ninguna simulación produce. Dos disciplinas para que siga siendo honesta:
(1) **presupuesto fijo de iteraciones** (24 ⇒ el intervalo se divide por ~1,7e7), no un umbral de
convergencia; (2) el invariante clásico —un extremo verificado BUENO y otro verificado MALO— y se
devuelve el BUENO, así que el valor publicado se **ejecutó** y cumplió el criterio. La monotonía
aporta la minimalidad, no la validez, y sus rendijas van declaradas en el doc-comment.

**Los comandos**

```bash
cargo test -p futurefin-engine --test golden_pins            # los dos pines
UPDATE_ENGINE_PINS=1     cargo test -p futurefin-engine --test golden_pins   # regenerar capa 4.15
UPDATE_ENGINE_PINS_5_0=1 cargo test -p futurefin-engine --test golden_pins   # regenerar capa aditiva
cargo test -p futurefin-engine --release --test timing -- --ignored --nocapture  # el coste, en release
cargo test -p futurefin-engine-stochastic                    # la puerta de degeneración
git diff --stat crates/engine/tests/fixtures/                # DEBE salir vacío en un refactor bit-idéntico
```

**Check your work**

- [ ] El fixture del pin **no aparece en el `git diff`** del refactor. Si aparece, el refactor no era
      bit-idéntico: explica el delta antes de seguir, no regeneres.
- [ ] El control negativo existe y falla cuando debe.
- [ ] La medición de coste se hizo en `--release` (en `debug` los `checked_*` sin optimizar dan un
      orden de magnitud de diferencia, y un número de `debug` solo compara con otro de `debug`).
- [ ] Si el cambio **debía** mover números, el delta está en el CHANGELOG con su cifra.

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
(**corrected in the Fase-7 sweep, 2026-08-29**: this used to say `.github/workflows/ci.yml` runs
neither the Postgres integration tests nor Vitest. Both run in CI since 4.0.0 — job `integration`
= `cargo test --workspace --locked` against a `postgres:16.4-alpine` service, and job `web` runs
`npm test --workspace futurefin-web`. Running them locally is the fast loop, not the only place). All line numbers are ~approximate anchors as of v1.4.3.

**Ampliada el 2026-09-03 tras el pase de correcciones de la revisión adversarial** (commit
`0668f37`, issue #207 cerrado): Recipe 7 gana el párrafo sobre por qué un golden de 19 casos no
demuestra bit-identidad y el fuzz diferencial sí, con los dos mecanismos reales que se colaron
(escala del cero, asociatividad de `+=`).

Re-verify before trusting volatile facts:

- Version: `grep -n '^version' apps/api/Cargo.toml`
- Migration count: `ls apps/api/migrations | wc -l`
- Gross-up closed form still in place + tests: `grep -n "gross_up_net_annual_fire\|gross_up_binary_reference" apps/api/src/handlers/projection.rs` — **ojo, `gross_up_binary_reference` ya no vive ahí**: el oráculo de bisección se mudó con la fiscalidad; vive en `crates/engine/src/tax.rs` desde que la fiscalidad se mudó al motor (`grep -n "fn gross_up_binary_reference" crates/engine/src/tax.rs`, 1 hit el 2026-09-03). El oráculo de bisección **sigue existiendo como test**, que es justo lo que la Recipe 1 pide.
- ~~TS side still binary search~~ — **adoptó la forma cerrada en la Ola 2 (#118, 4.6.0) y este grep salía VACÍO**: `grep -n "export function grossUpNetAnnualFire" apps/web/src/lib/fire.ts`
- Single fire-target helper still sole source: `grep -rn "fire_target_at_month_index" crates apps/api/src | grep -v test`
- Chart deflates by month_index: `grep -n "deflator(p.month_index)" apps/web/src/views/ProjectionNetWorthChart.tsx`
- f64 wire boundary unchanged: `grep -n "serialize_decimal_as_f64" apps/api/src/handlers/projection.rs`
- Parity fixture cases + tolerance: `python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"` (**17** el 2026-09-03) y `grep -n "_tolerance_eur" apps/api/tests/fixtures/fire-parity.json`
- **Recipe 7 (arnés golden, 5.0.0)**: `ls crates/engine/tests/fixtures/` (dos fixtures) y los cuatro tests que la sostienen — `grep -n "fn golden_pins_match_4_15_0\|fn the_hash_actually_notices_a_single_moved_decimal\|fn the_5_0_canonicalization_grew_without_moving_the_old_fields\|fn the_audit_battery_is_the_ordered_prefix_of_the_pinned_battery" crates/engine/tests/golden_pins.rs` (4 hits)
- **Recipe 7 — lo que el golden NO cazó y el fuzz diferencial sí (2026-09-03, pase de correcciones)**:
  `grep -n "fn p24_publishes_the_undrained_operand_with_the_scale_of_4_15_0\|fn p25_keeps_the_debt_service_grouping_of_4_15_0" crates/engine/tests/golden_pins.rs` (2 hits) y el arnés de fuzz sobre hogares aleatorios `grep -n "fn random_households_satisfy_the_accounting_identities" crates/engine/tests/fuzz_invariants.rs`
- **La trampa del `max` inherente, declarada en el trait**: `grep -n -B2 -A6 "fn max(self" crates/engine/src/money.rs`
- **El inverso exacto en vez de la bisección** (Recipe 7): `grep -n "fn mixed_drawdown_for_gross_cap" crates/engine/src/tax.rs`; y la bisección legítima, `grep -n "pub const MAX_SOLVE_ITERATIONS" crates/engine/src/solve.rs`
- **La segunda frontera f64 (Recipe 5)**: `grep -c "impl MoneyOps for F64Money" crates/engine-stochastic/src/lib.rs` (1) y el freezer intacto `grep -n "fn crates_engine_src_has_no_f64_outside_comments" crates/engine/src/lib.rs`
- Engine purity greps: the four commands in Recipe 6.
- CI scope (what is / isn't covered): `grep -nE "cargo (test|build)|npm" .github/workflows/ci.yml`
