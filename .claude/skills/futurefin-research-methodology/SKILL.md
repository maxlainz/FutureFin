---
name: futurefin-research-methodology
description: >
  Load this skill whenever you are about to turn a hunch into a change in FutureFin: forming a
  hypothesis about a bug's cause, proposing an improvement, deciding whether an investigation is
  "done", or deciding whether an idea should be adopted or dropped. Triggers: "I think the cause
  is", "this should fix it", "let me try changing X", "root cause", "hypothesis", "the numbers
  look right now", "is this fixed?", "should we implement X", "experiment", "validate my theory",
  repeated failed fixes for the same symptom, or any session where you are your own reviewer.
  It defines the evidence bar (one mechanism must explain ALL observations, including the
  negatives), predict-numbers-before-running discipline, and the hunch→adopted/retired lifecycle.
  Do NOT use it for specific analysis techniques or math derivations
  (futurefin-proof-and-analysis-toolkit), for WHAT to research next
  (futurefin-research-frontier), for symptom→cause triage tables
  (futurefin-debugging-playbook), or for merge/release gates (futurefin-change-control) —
  this skill is the discipline that wraps those tools, not a replacement for them.
---

# FutureFin Research Methodology

The discipline that turns a hunch into an accepted change in this repo. Written for a
Sonnet-class session with zero memory and **no human reviewer in the loop**: you must impose
this rigor on yourself, because nobody else will, and because this codebase's hardest errors
are **silent** — a wrong projection produces plausible-looking euro amounts, not a crash.

As of 2026-07-02: version **v1.4.3** (`apps/api/Cargo.toml`), all history claims below verified
against `CHANGELOG.md` and git. Paths are from the repo root.

Vocabulary (defined once): **engine** = `crates/engine`, pure `Decimal` projection math, no I/O.
**FIRE target** = the retirement net-worth goal; reaching it is the sole retirement trigger.
**nominal vs real** = the engine simulates in nominal euros; only the FIRE target grows with
inflation (`base × (1+inf/100)^(month_index/12)`, single source of truth
`fire_target_at_month_index` in `crates/engine/src/projection.rs`). **density=hybrid** =
`GET /v1/projection/series?density=hybrid` returns a decimated, non-equidistant subset of the
monthly series (~82 of ~841 points). **mechanism** = the specific causal story ("X does Y
because Z"), as opposed to a correlation ("changing X made the symptom go away").

## When NOT to use this skill

- You need a concrete analysis recipe (dimensional check, invariant derivation, worked math) →
  `.claude/skills/futurefin-proof-and-analysis-toolkit/SKILL.md`
- You want candidate research directions and their falsifiable milestones →
  `.claude/skills/futurefin-research-frontier/SKILL.md`
- You have a symptom and want the known-trap triage table →
  `.claude/skills/futurefin-debugging-playbook/SKILL.md`
- You are ready to merge/release and need the gates → `.claude/skills/futurefin-change-control/SKILL.md`
- You want the full incident chronicle → `.claude/skills/futurefin-failure-archaeology/SKILL.md`
- You need to write/run the tests your experiment requires → `.claude/skills/futurefin-validation-and-qa/SKILL.md`

---

## 1. The evidence bar

A hypothesis is accepted only when **one mechanism explains ALL observations — including the
negative ones** — and survives an adversarial-refutation pass you assign yourself.

### The negative check (mandatory)

Before declaring a root cause, answer in writing: **"Does my mechanism explain why the OTHER
cases still work?"** If your explanation predicts the bug everywhere but the bug appears only
somewhere, your mechanism is wrong or incomplete — even if your fix makes the visible symptom
disappear.

**Cautionary exemplar — the table-CSS saga (v1.0.18 → v1.0.20, CHANGELOG).** Symptom: action
buttons overlapped the previous column's content in some tables. Two plausible-mechanism fixes
shipped and failed:

| Version | Claimed mechanism | Fix shipped | Why it was wrong |
|---|---|---|---|
| v1.0.18 | `display: flex` on the `<td>` misbehaves "in some browsers" | switch to `inline-flex` + padding + background | Didn't explain why only SOME tables/columns broke; symptom persisted |
| v1.0.19 | actions column not anchored | `position: sticky; right: 0` + shadow hacks | Same — patched the visual effect, not the cause |
| v1.0.20 | **`.budget-row-actions { display: inline-flex }` applied directly to the `<td>` removed the cell from the table display model**, so the browser rendered it outside its column | wrap buttons in an inner `<div>`, leave the `<td>` as default `table-cell`; revert all v1.0.18–19 hacks | Explains everything: only cells carrying that class broke, and only when the neighbor column had long `white-space: nowrap` content to be covered |

Two releases were burned because the first two mechanisms could not answer "why only the
Ingresos table's Importe mensual column?". The third investigation started from that question.

**Positive exemplar — the v1.4.2 hybrid deflation bug (commit 669307d).** The mechanism (chart
deflated by **array index**, not `month_index`) made an asymmetric prediction matching every
observation: invisible at `monthly` density (index == month), wrong under `hybrid` only past
month 12, converging when the full monthly series arrives. That asymmetry — "explains precisely
why A is fine and B is wrong, and where the boundary is" — is what a good mechanism looks like.
(Full index-math analysis: futurefin-proof-and-analysis-toolkit Recipe 2; chronicle:
futurefin-failure-archaeology §2.8.)

### The adversarial-refutation pass (mandatory before writing the fix)

You have no reviewer, so play one. Write down, literally: **"The strongest argument that my
explanation is wrong is: …"** — then go test that argument. Concretely:

1. State the observation your mechanism explains *least* well. Test that one first.
2. Ask what ELSE your mechanism predicts that you have not yet observed. Go observe it
   (e.g. v1.4.2's mechanism predicts monthly density is bit-identical — check it, don't assume).
3. Try to construct a counterexample input. For engine work, write it as a unit test in
   `crates/engine/src/projection.rs` (22+ tests already there as patterns) and run
   `cd apps/api && cargo test -p futurefin-engine`.
4. Only if the refutation attempt fails may you promote the hypothesis to "root cause".

If you cannot refute it but also cannot make it explain a known observation, the verdict is
**"partially confirmed — mechanism incomplete"**, not "fixed". Say so in the CHANGELOG/notes.

## 2. Predict numbers before running

Write the predicted values down **before** executing the experiment. A prediction made after
seeing the output is a rationalization; a numeric prediction made before is a discriminating
test. This matters doubly here because projection output always *looks* plausible.

Real examples of the form your predictions must take:

- Cache (v1.4.0): "if the projection cache works, the first `GET /v1/projection/series` after
  login is a warm-up **hit** (fast), a GET after any asset mutation recomputes **once** and
  subsequent GETs are sub-ms hits." `scripts/smoke-projection-cache.sh` encodes exactly this
  sequence against a running stack.
- Off-by-one (engine/handler FIRE divergence, fixed in v1.3.0): "if the off-by-one exists, the
  chart's `fire_target_series` and the engine's `fire_reached` trigger differ by **exactly one
  month** at the crossover." The fix made `fire_target_at_month_index` the single public helper
  both sides consume (`crates/engine/src/projection.rs:171`).
- Deflation (v1.4.2): "if the array-index bug exists, `monthly` output is **bit-identical**
  before and after the fix, and `hybrid` diverges only for `month_index > 12`."

### Worksheet (copy this into your working notes for every experiment)

```
Hypothesis:          <one sentence: X causes Y>
Mechanism:           <the causal story — why X produces Y and NOT Z>
Predicts:            <concrete numbers/relations, written BEFORE running:
                      "second GET < 10ms", "values differ by exactly 1 month",
                      "monthly density bit-identical", "422 not 200">
Discriminating test: <the single observation that separates this hypothesis
                      from the next-best alternative — name the alternative>
Result:              <what actually happened, numbers included>
Verdict:             confirmed | refuted | partially confirmed (mechanism incomplete)
```

Rules: fill `Predicts` with numbers or exact relations, never "should improve" / "should look
right". If `Result` matches but via a different route than `Mechanism` claimed, the verdict is
NOT "confirmed" — investigate the discrepancy. A refuted hypothesis is a result: record it
(Section 3) so the next session doesn't re-test it.

## 3. The idea lifecycle in this repo

Every idea ends **adopted** or **retired with a written reason** — never silently dropped.
"Retired" is a first-class outcome: the CHANGELOG's Removed/Fixed entries are the graveyard,
and it has repeatedly saved later sessions from re-proposing dead ideas.

```
hunch
  → issue-shaped note            (symptom, suspected mechanism, what would confirm/refute it —
                                  in your working notes or a GitHub issue; enough that a
                                  zero-context session could pick it up)
  → discriminating experiment    (on a branch off `dev`; worksheet from Section 2 filled in
                                  BEFORE running; smallest change that tests the mechanism)
  → test capturing the numbers   (engine unit test in crates/engine/src/projection.rs and/or
                                  integration test in apps/api/tests/ asserting the predicted
                                  values — the prediction becomes a permanent regression guard;
                                  see futurefin-validation-and-qa for the TestApp harness)
  → change-control gates         (classification, migration/release discipline, doc updates —
                                  .claude/skills/futurefin-change-control/SKILL.md; this skill
                                  never routes around those gates)
  → forensic CHANGELOG entry     (symptom + mechanism + why the fix is the fix, house style in
                                  futurefin-docs-and-writing; v1.0.20 and v1.4.2 entries are
                                  the gold standard)
  → ADOPTED

or, at any stage:
  → RETIRED, with the reason written where the next session will find it
    (CHANGELOG Removed/Fixed entry, or the issue note closed with the refutation)
```

The graveyard in action — retired ideas whose written reasons still do work (all in CHANGELOG):

| Retired idea | Where buried | Written reason |
|---|---|---|
| Cache warm-up after every mutation | v1.4.0 | Rejected for a race: two concurrent warm-ups could leave the cache stale. Warm-up runs after login only; mutations just invalidate. |
| Migration auto-repair loop | v1.3.0 | Masked real drift for 12 rounds of checksum-repair; retired so drift fails loud, manual fix only. |
| GETs purging expired liabilities | v1.3.0 | Reads were mutating (HTTP-semantics violation, blocks caching); replaced by a `WHERE` filter. |
| Inflation model v1 ("real pure", deflate returns) | v1.0.12 → superseded v1.2.0 | Produced incoherent behavior (asset drain before retirement with inflation on); replaced by all-nominal + moving FIRE target. |
| `projection_target_age` | v1.0.6 | Age-triggered retirement caused a visual gap; FIRE crossover became the sole trigger. |
| Per-asset contribution config | v1.1.0 / v1.0.13 | Fixed sums could exceed surplus, weights confusing >100 %, no explicit order; replaced by the allocation cascade. |
| 90-iteration binary-search gross-up | v1.3.0 | The after-tax function is piecewise-linear, so a closed form exists; replaced, result identical ±0.01 €. |

If your investigation refutes a hunch, write it down the same way. A dead hypothesis with a
recorded refutation is worth more than an untested idea — this is failure-archaeology's raw
material (`.claude/skills/futurefin-failure-archaeology/SKILL.md`).

## 4. Where good ideas historically came from

Observable in this repo's history — use these as prompts when looking for the next improvement:

- **Refactors surface latent bugs.** The v1.3.0 `App.tsx` split (10,384 → ~3,079 LOC) itself
  found `RetirementView` passing `expense_regular_monthly_equivalent` where the server used
  `expense_retirement_monthly_equivalent` — a 2–3× FIRE-preview divergence nobody had reported.
  When restructuring, treat every "these two things don't line up" moment as a potential bug,
  not friction.
- **Parity tests freeze correctness across duplication.** The FIRE math exists in both Rust and
  TypeScript; `apps/api/tests/fixtures/fire-parity.json` is consumed by BOTH
  `apps/api/tests/fire_parity.rs` and `apps/web/src/lib/fire.test.ts`. Written for parity, it
  now converts any one-sided edit (e.g. tax brackets) into a test failure. When you find
  duplicated logic you can't merge, write a shared fixture.
- **Performance work forces semantic clarity.** `density=hybrid` (v1.4.0) made points
  non-equidistant, which forced the chart onto real `month_index` coordinates — and that is
  exactly what exposed and then fixed the array-index deflation bug (v1.4.2). Optimizations
  that change data shape are semantic audits in disguise; run them as such.
- **User-visible incoherence exposes model flaws.** The inflation toggle barely moving the
  retirement age was the observation that condemned the flat FIRE target and produced the
  v1.2.0 moving-target model. When a UI control has implausibly small effect, suspect the
  model, not the UI.
- **Una puerta de paridad nueva ILUMINA lo viejo, no solo protege lo nuevo** (5.0.0). La puerta que
  compara el camino `Decimal` con el `f64` del motor
  (`crates/engine-stochastic/tests/degeneration.rs`) se escribió para validar el camino estocástico,
  y lo primero que encontró fue un **filo de navaja preexistente** que ninguna suite `Decimal` podía
  ver: un llamante deducía «¿se vendió el techo entero?» comparando dos números en vez de leer el
  booleano que el algoritmo ya conocía, y de esa rama colgaba qué era recorte informativo y qué era
  descubierto que resta patrimonio. Coste medido: 8.138 € en un caso. Crónica:
  `futurefin-failure-archaeology` §2.26. **Cuando montes un gate que compara dos implementaciones
  del mismo modelo, espera que lo primero que falle sea un bug antiguo** — y no lo trates como ruido
  del gate.
- **Escribir el número ANTES convierte un test en una comprobación, no en una foto.**
  `crates/engine/tests/phases_wp3.rs` lleva la disciplina de §2 al extremo: cada assert va precedido
  del comentario con su aritmética a mano, y casi todos los casos corren con rentabilidad 0 %,
  inflación 0 % y sin impuestos **a propósito** — no por realismo, sino para que cada euro de la
  serie sea una suma que cabe en una línea y una discrepancia señale el mes exacto. Un test que
  compara el motor consigo mismo solo pinea lo que el motor hace hoy; uno con el número escrito
  antes comprueba que hace lo que se pidió.

## 5. Anti-patterns for AI sessions specifically

These are the failure modes of a competent model working alone in this repo. Check yourself
against each one before declaring any investigation finished.

1. **Declaring success from plausible-looking output.** Projection errors are SILENT: the
   v1.0.12 model produced smooth, reasonable-looking curves while draining assets before
   retirement. "The chart looks right" is not evidence. Evidence = a predicted number matched
   (Section 2) or an assertion in a test.
2. **Fixing symptoms serially without a mechanism.** The table-CSS saga shipped two fixes in
   two days before anyone asked "why only some tables?". If your second fix for the same
   symptom is being written, STOP — you skipped the negative check. Go find the mechanism.
3. **Trusting stale docs over code.** The `.claude/*.md` reference docs have drifted before —
   eight verified errata (a false "there is no CI yet", the removed `projection_target_age`
   still documented, a dead README endpoint…) accumulated until they were fixed on 2026-07-02.
   The standing-errata record: futurefin-docs-and-writing §7.
   Rule: docs give you the hypothesis; the code and a running test give you the fact.
4. **Skipping the negative check.** "Does my fix explain why the OTHER cases still worked?" —
   the single question that separates v1.0.20 from v1.0.18/19, and the reason v1.4.2's fix
   could promise "no regression for monthly density" before anyone ran it.
5. **Modifying golden fixtures to make tests pass.** If `fire-parity.json` expectations fail,
   the default assumption is that YOUR change broke parity. Regenerating expected values is
   legitimate only when the domain math intentionally changed (e.g. tax brackets), and then
   BOTH suites (`fire_parity.rs` and `fire.test.ts`) must pass against the regenerated file,
   with the change explained in the CHANGELOG. Editing a fixture to silence a failing test
   without a mechanism is falsifying your own lab notebook.
6. **Letting an idea die undocumented.** You have zero memory; the next session re-derives —
   and possibly re-ships — anything you didn't write down. Retire ideas per Section 3, always
   with the refutation attached.

## Provenance and maintenance

All historical claims verified 2026-07-02 against `CHANGELOG.md` (v1.0.6, v1.0.12, v1.0.18–20,
v1.1.0, v1.2.0, v1.3.0, v1.4.0, v1.4.2) and the working tree. Re-verify before trusting:

- Current version: `grep '^version' apps/api/Cargo.toml`
- Single FIRE-target source of truth still exists: `grep -n "pub fn fire_target_at_month_index" crates/engine/src/projection.rs` (y desde 5.0.0 su gemelo consciente del plan **la llama** en vez de reimplementarla: `grep -n "fire_target_at_index_g(Some(ft), month_index)" crates/engine/src/target.rs`, 2 hits)
- Los dos ejemplos de §4 añadidos el 2026-09-03 (rama `release/5.0.0`, issue #207):
  `grep -n "fn every_case_degenerates_from_decimal_to_floating_point" crates/engine-stochastic/tests/degeneration.rs`,
  `grep -n "pub cap_exhausted" crates/engine/src/tax.rs` (el booleano que sustituyó a la comparación
  re-derivada) y `ls crates/engine/tests/phases_wp3.rs`
- Parity fixture + both consumers: `ls apps/api/tests/fixtures/fire-parity.json apps/api/tests/fire_parity.rs apps/web/src/lib/fire.test.ts`
- Cache smoke script: `ls scripts/smoke-projection-cache.sh`
- Migration count (do not quote a stale number): `ls apps/api/migrations | wc -l`
- ~~CI still excludes Postgres integration tests~~ — **falso desde 4.0.0** (job `integration`, `cargo test --workspace --locked` contra un Postgres de servicio). Corregido en la Fase 7: `grep -n "cargo test" .github/workflows/ci.yml`
- Case citations: `grep -n "1.0.20\|inline-flex" CHANGELOG.md`, `grep -n "month_index" CHANGELOG.md` (v1.4.2 entry), `grep -n "warm-up\|Sin warm-up" CHANGELOG.md`
- Sibling skills referenced here: `ls .claude/skills/` (some may still be in authoring as of 2026-07-02)
