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

An executable campaign, not an essay. Facts verified against the repo as of **2026-07-02, v1.4.3**;
**Fase 0, Fase 2 y los caminos vallados re-verificados el 2026-09-03 contra `release/5.0.0`**
(issue #207), que entregó tres de los cinco ítems de la Fase 2.

Jargon used below (one-line definitions — full math in `.claude/skills/futurefin-fire-domain-reference/SKILL.md`):
- **FIRE target**: net worth needed to retire = `gross_up(annual_net_need) / (SWR/100)`.
- **SWR**: safe withdrawal rate, % of portfolio withdrawn per year (`fire_settings.swr_pct`, default 3.5).
- **Gross-up**: inflating an annual net need through Spanish capital-gains tax brackets so the after-tax withdrawal equals the need.
- **Nominal vs real**: nominal = euros of the moment; real = today-euros (deflated). The engine is **all-nominal**; only the FIRE target grows with inflation (v1.2.0 model).
- **Cascade**: ordered allocation rules (`fixed`/`percent`/`remainder`, optional caps) that split each month's surplus across assets.
- **Jubilación crossing**: first month where the LIQUID worth of month k−1 ≥ the FIRE target at that same index. **Dejó de ser el único disparador el 2026-09-03**: desde 5.0.0 lo elige la estrategia del usuario (cruce o EDAD), con un solo trigger por simulación — ver el camino vallado #2, reescrito.
- **Estrategia / fase / regla de retirada** (5.0.0): los tres ejes nuevos del motor. Semántica completa en `futurefin-fire-domain-reference` §4b; contrato en `financial-contracts.md` §2.5.

## Campaign charter

**Goal**: increase the fidelity of the economic model (what the engine simulates vs what would actually happen to a household) and catch silent wrongness — without breaking determinism, `Decimal` money discipline, or the client↔server FIRE parity.

**Done means**: (1) every model simplification is inventoried, code-anchored, and classified (Phase 1 table kept current); (2) every classification `realism gap` either has a measured effect size and an accepted-as-is note, or a Phase 2 item with a pre-registered acceptance test; (3) all baseline suites green.

**Standing rule — success is MEASURED, never judged by eye.** A projection that "looks right" is exactly the failure mode this campaign exists for (v1.0.12's model looked right and was incoherent). Evidence is: engine unit tests, **los dos pines dorados byte a byte** (5.0.0), the fire-parity fixture (±1 €, recuéntalo — no son 6), integration regression tests, and predicted-vs-observed numbers written down **before** running (discipline: `.claude/skills/futurefin-research-methodology/SKILL.md`).

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

```bash
# 0d. El crate estocástico (5.0.0): la puerta de degeneración NO necesita Postgres y corre aparte
cargo test -p futurefin-engine-stochastic
```

**Observaciones esperadas — pide el número al comando, nunca a esta página.** Los tres contadores
que aquí vivieron congelados (56 tests de motor, 24 del handler, 6 casos de paridad) estaban los
tres equivocados el 2026-09-03; se sustituyen por sus comandos:

```bash
cargo test -p futurefin-engine 2>&1 | grep "test result"   # total autoritativo del motor
grep -c '#\[test\]' crates/engine/src/*.rs | awk -F: '{s+=$2} END{print s}'
                    # 199 el 2026-09-03 (eran 195 unas horas antes, en la misma rama: pídeselo al
                    # runner). OJO: el glob, no la lista de cuatro ficheros que esta
                    # página usaba — se dejaba fuera money/phases/withdrawal/target/solve/tax
grep -c '#\[test\]' apps/api/src/handlers/projection.rs   # 26 el 2026-09-03 (decía 24)
python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"
                    # 17 el 2026-09-03. Esta página decía 6 y otras tres fichas decían 7
```

- 0a: **engine tests pass**, y desde 5.0.0 eso incluye cuatro binarios de test además del `mod
  tests` inline (`golden_pins`, `phases_wp3`, `audit_dump`, `timing` — este último todo `#[ignore]`
  a propósito: mide, no afirma). Detalle en [`.claude/tests.md`](../../tests.md) §Arneses del motor.
- 0b: all integration tests pass, including `fire_parity.rs` (casos numéricos ±1 € más los que
  esperan `null` target), plus `projection_marker.rs` (marker = month 1, 25 points at `?months=24`)
  and `projection_cache.rs`.
- 0c: `lib/fire.test.ts` passes **los mismos casos del fixture** ±1 € (los `it`s son 1 comprobación
  de recuento + un caso).
- 0d: la puerta de degeneración verde ⇒ el camino `f64` y el `Decimal` son la misma simulación
  dentro de 1 €/mes. Si falla, **no toques la cota**: es la salvaguarda que sostiene la excepción
  `f64` entera (`futurefin-failure-archaeology` §2.9 scope note).

CI note (**corregida 2026-09-03: este párrafo llevaba desde 4.0.0 diciendo lo contrario de lo que
hace el workflow, exactamente el modo de fallo que §3.1 de `futurefin-docs-and-writing` describe
para las afirmaciones NEGATIVAS**): `.github/workflows/ci.yml` corre en el job `rust`
`cargo test -p futurefin-engine --locked` **y** `cargo test -p futurefin-engine-stochastic --locked`,
y en el job `integration` `cargo test --workspace --locked` **contra un servicio Postgres** — o sea,
la integración (0b) **sí** está en CI, y Vitest y ESLint también (job `web`). Compruébalo antes de
citarlo: `grep -n "cargo test\|npm test\|lint:web" .github/workflows/ci.yml` (debe imprimir varias
líneas). Correrlo en local sigue siendo el bucle rápido, no la única red.

**Gate P0 (5.0.0 añade una condición)**: además de las suites, **los dos pines dorados deben estar
verdes y `pins-4.15.json` byte-idéntico**. Es la red que hace que «este refactor no cambia números»
sea una afirmación comprobada y no una esperanza:
`cargo test -p futurefin-engine --test golden_pins`.

**Gate P0**: if ANY baseline check fails → you are in a regression hunt, not a realism campaign. Pause the campaign, branch to `futurefin-debugging-playbook` (and `futurefin-failure-archaeology` if the failure smells historical). If exactly one side of fire-parity fails → the duplicated FIRE math drifted; fix drift first (see Promotion protocol).

## Phase 1 — Characterize current model fidelity

**La tabla fila a fila que vivía aquí se movió el 2026-08-30 a la única fuente de verdad:
[`.claude/financial-contracts.md`](../../financial-contracts.md)** — §4 (divergencias conocidas,
cada una con coste en escenario sintético, estado y su issue) y §3 (lo correcto-por-diseño que no
hay que «arreglar»). Mantener dos copias de este inventario fue exactamente lo que dejó la antigua
fila #4 (el falso «clamp de retornos negativos») siendo mentira aquí durante meses después de
corregirse en `futurefin-fire-domain-reference`. Las clases [CBD]/[AS]/[GAP] sobreviven como
estados de esa tabla; ten en cuenta que la auditoría del modelo financiero (2026-08) re-examinó
también lo [CBD]/[AS] contra la realidad española y varias filas pasaron a divergencias con
dirección decidida por el owner (indexación de gastos al IPC, latch de jubilación, base del
gross-up en plusvalía, default de amortización francés, deuda vencida visible…) — no reintroduzcas
aquí un resumen: enlaza.

Horizon ground truth: **edad límite CONFIGURABLE** desde 4.9.0/#149 (`horizon_lifespan_age`, 85..=105, default 90 — y desde 5.0.0 vive en `users.retirement_profile`, no en `installation.fire_settings`), clamped 5–70 years, 30-year fallback (`projection_horizon_months`, `apps/api/src/handlers/projection.rs` — localiza por nombre); `horizon_basis` values are `lifespan_age | fallback_no_demographics | months_override` (**`lifespan_90` dejó de ser un literal vivo en 4.9.0**; hoy solo sobrevive dentro de un comentario histórico). Sobre `projection_target_age`: ver el camino vallado #2, reescrito para 5.0.0.

### Discriminating experiments (write the predicted number FIRST, then run)

Each experiment is an engine unit test you add temporarily (or a scratch `#[test]`) — pure, no DB. Pattern: build a minimal `ProjectionInput` like the existing tests (`crates/engine/src/projection.rs:619-641`).

| Targets # | Experiment | Predicted magnitude |
|---|---|---|
| 4 | ~~One asset 10.000 €, `expected_annual_return_percent = -5`~~ | **Predicción retirada con la fila #4**: el motor NO clampa, así que NW[12] ≈ 9.500 y el «efecto oculto» de 500 € no existe. Si vuelves a necesitar un caso de rentabilidad negativa, el que sí muerde es `p <= −100` (pérdida total, factor 0) |
| 11 | Liability 100.000 € principal, 500 €/mes payment, TIN 3 % | `fixed_payments`: debt gone in exactly **200** months (100.000/500). `french` at 3 %: extinguished in month **278**, last instalment partial. Effect: **78 months (6,5 años) of debt service** that the old model skipped, ≈ **38.800 €** of interest never charged (277 × 500 + ~303 − 100.000). **CORRECCIÓN (4.2.0)**: esta fila dijo «≈ 430 months» desde 2026-07-02 y era **falso al 3 %** — 430 meses corresponden a un TIN de ≈ 5 %, y el «~115.000 € de interés» que lo acompañaba salía de la misma cuenta equivocada. El número verificado por el engine (`french_extinction_at_month_278`) es 278. Cifra estimada de memoria en una tabla que nadie recontó: exactamente el modo de fallo que esta campaña persigue |
| 5 | Portfolio 1.000.000 € (purchase_price 400.000 €), retirement drain 2.000 €/mes net expense | Engine drains exactly 2.000 €/mes. Tax-aware: withdrawing enough to net 2.000 € with 60% embedded gain taxed at 19–21% ⇒ gross ≈ 2.245–2.270 €/mes (~11–13% faster depletion). Derive exactly with the bracket math before running |
| 2 | Same input, inflation 0% vs 3%: only `jubilacion_month_index` and `fire_target_series` may change; `net_worth` series must be **bit-identical** | Zero series delta. If the NW series moves with inflation → someone re-broke the v1.2.0 model; stop and treat as regression |
| 10 | Income 3.000/expense 1.000, no allocation rules, 36 months | NW grows exactly +2.000/mes linearly (test `no_rules_routes_surplus_to_cash` proves months 0–3). Any compounding on that cash = bug |
| 7 | Two liquid assets (2% and 7%), force a deficit month | The 2% asset depletes first; the 7% asset untouched until 2% hits zero |
| 1 | No in-repo experiment possible (no stochastic machinery). Effect size is the literature's, not ours — label any number you quote as external | n/a — motivates Phase 2(b)/(c) |

**Gate P1**: every row you rely on must be re-verified at its anchor (the line numbers WILL drift). If an anchor no longer matches the described behavior → the model changed since 2026-07-02; re-derive the row, update this table (this skill file is the doc of record for the inventory), and check `CHANGELOG.md` for the change before proceeding. If you find a behavior not in this table → add it, classified, before designing anything.

## Phase 2 — Solution menu (ranked by value/effort for a solo self-hosted app)

**Estado 2026-09-03: de los cinco ítems, (d) está ENTREGADO desde 4.10.0/4.12.0, (e) está
ENTREGADO en 5.0.0 y (b) está a medias con su capa base entregada.** Solo (a) y (c) siguen
enteramente abiertos. Cada ítem lleva su estado abajo; **no vuelvas a leer esta sección como una
lista de candidatos**. Lo que no cambia es la disciplina: nada aquí es API prometida hasta que su
test de aceptación pre-registrado existe y pasa. Each item lists: theory/derivation obligations (what must be written down and reviewed before code) and a measurable acceptance test **defined before implementation**. Every item goes through `futurefin-change-control` (engine semantics changes are output-changing: version bump + CHANGELOG).

### (a) Property-based invariant testing — HIGHEST value/effort. Do this first.
Adds `proptest` as a dev-dependency of `crates/engine` only. Catches silent wrongness in the code we already have; changes zero behavior.
- **Obligations**: for each invariant, a one-paragraph proof sketch of WHY it must hold, including its domain of validity.
- **Candidate invariants** (validity caveats are the hard part — encode them):
  1. *Cascade conservation*: for any pool ≥ 0, `sum(alloc) + leftover == pool` and each `alloc[i] ≥ 0` (`distribute_contributions`). Also: no allocation exceeds cap room.
  2. *Per-asset decomposition*: `surplus_cash` retired (4.12.1, #175) simplifies this identity — with **no liabilities and no deficit months (no undrained shortfall)**, `net_worth[k] == sum_i per_asset_series[i][k]` holds directly, no cash term to add back; testable from outputs alone.
  3. *Monotonicity under +income* — **scoped**: with `fire_target = None` and `retirement_start_month = None`, raising `income_regular_monthly` never decreases any `net_worth[k]`. Do NOT assert it globally: more income ⇒ earlier FIRE crossing ⇒ income drops to `income_retirement_monthly` sooner ⇒ later NW can legitimately be LOWER. This non-obvious falsifier is exactly what proptest should also document.
  4. *NW continuity*: `|net_worth[k] − net_worth[k−1]|` bounded by `|net_cash_month| + growth + debt payments` for that month.
  5. *Determinism*: same input twice ⇒ identical output (guards future stochastic work).
- **Acceptance test**: proptest suite green over ≥ 10^4 generated cases per invariant; each invariant's domain caveat written into the test's doc comment; `cargo test -p futurefin-engine` still passes.

### (b) Monte Carlo / stochastic returns — **ENTREGADA en 5.0.0, suite verde**

**Estado (2026-09-03, tras el pase de correcciones de la revisión adversarial — commit `0668f37`,
issue #207 cerrado)**: lo que la obligación (1)–(4) exigía «antes de implementar» está escrito en
el plan de la issue #207 (§B.4/§B.5) y firmado por el owner (D11, D12, D22, D23, D25). Y la mitad
difícil ya está en `main`: **el bucle es genérico sobre su tipo numérico**
(`MoneyOps`) y `crates/engine-stochastic` lo instancia en `f64` con la **puerta de degeneración**
verde — o sea, el coste que esta sección llamaba «a real design problem» está resuelto sin duplicar
el bucle. WP6a (`ba6bdfe`) está commiteado —
`crates/engine-stochastic/src/mc.rs` + `tests/monte_carlo.rs`, con `rand_chacha` pineado y
`crates/engine` todavía sin RNG— y **su suite está VERDE entera: 29 tests, 0 fallos** (13
unitarios + 3 de `degeneration.rs` + 13 de `monte_carlo.rs`). El `mc_cash_buffer_…` que fallaba
—`mc_cash_buffer_changes_the_band_under_sequence_risk`, predicción «el colchón mejora la
probabilidad de éxito» **falsada** por la medición (0,713 frente a 0,775 sin colchón)— fue un
*predict-then-run miss* de manual, y se resolvió como manda la regla de la casa: parando y
revisando el MODELO, no relajando el assert. El modelo del colchón tenía dos bugs (relleno
anticipativo con el shock del propio mes, colchón elegido sin filtrar por liquidez); corregidos,
el test se rehízo como `mc_cash_buffer_protects_and_the_drag_is_what_costs`, que separa el lastre
de tener el colchón fuera del mercado (−7,9 pp) de la protección real de gastar de una reserva sin
riesgo (+3,9 pp): con la cuenta a su rentabilidad real (0 %) sigue costando neto (−3,5 pp).
Compruébalo tú: `cargo test -p futurefin-engine-stochastic 2>&1 | grep "test result"`.

**El modelo, decidido** (D11/D12): **un shock de mercado común por mes** escalado por la sd de cada
activo —factor `m_i·exp(σ_i z − σ_i²/2)` con `σ_i = annual_volatility_percent/100/√12`, de modo que
`E[factor] = m_i`—, no una matriz de correlaciones: la instalación no tiene covarianzas y el sesgo
de la correlación perfecta es **conservador** (bandas más anchas). Declarado como divergencia
aceptada en `financial-contracts.md` §4.

**Éxito YA NO es «la cartera no se agota nunca» (D22 original)** — corregido por la segunda
revisión adversarial (D20): esa definición premiaba al hogar que **no se jubila jamás**, porque
quien nunca drena nunca se agota (medido: 0,960 publicado frente a 0,940 entre los que sí llegan a
jubilarse, en un hogar que cruza en el mes 655 de 840, con el 33,1 % de los caminos sin jubilarse
contando como éxito — hasta +6,8 pp de sesgo con SWR 6 %). Hoy `success_probability` exige
jubilarse dentro del horizonte (o un trigger por edad) **y** no agotar la cartera;
`never_retired_probability` y `success_given_retired` se publican al lado. **El recorte de una
regla de retirada sigue sin ser fracaso** y viaja aparte (`withdrawal_shortfall`): es la separación
de magnitudes de `financial-contracts.md` §2.5, y confundirlas fue un hallazgo de la revisión
adversarial, no una sutileza.

**La puerta de aceptación pre-registrada, y sigue siendo la de esta sección**: (1) misma entrada +
misma semilla ⇒ mismo resultado bit a bit; (2) con σ = 0 las bandas **son** la serie determinista;
(3) `p10 ≤ p50 ≤ p90` en todos los meses; (4) la media del terminal coincide con el terminal
determinista dentro de una tolerancia **derivada** de la varianza log-normal, no elegida a ojo;
(5) el presupuesto de tiempo y de payload medidos contra el arnés de tiempos de WP0. Semilla estable
por usuario (`hash(installation_id, user_id)`), override solo en `simulate`.

**Diseño original, conservado como referencia:**
- **Obligations (all BEFORE implementation)**: (1) justified return distribution (e.g. annual lognormal with user-set μ = current `expected_annual_return_percent`, σ per asset class; cite the justification in the design note); (2) **SEEDED determinism preserved** — seed derived deterministically from input (e.g. hash of installation id + parameters), same request ⇒ same bands; the projection cache and the `r1.body == r2.body` assertion in `apps/api/tests/projection_cache.rs` must keep holding; (3) percentile-band API design: extend `ProjectionSeriesResponse` additively (e.g. `net_worth_p10/p50/p90` alongside the existing deterministic series — never replace it); (4) wire-size plan following the v1.4.0 precedent: bands as f64 arrays, decimated under `?density=hybrid`, gzip; budget the payload (each extra band ≈ the size of `points` — measure, don't guess).
- **Acceptance test (pre-registered)**: with σ = 0 the p10/p50/p90 bands equal the deterministic series exactly; with σ > 0, p50 within a stated tolerance of the deterministic path over 1.000 seeded runs; fire-parity untouched; cache tests green; response size at `density=hybrid` under a stated KB budget.

### (c) Sequence-of-returns risk surfacing near the jubilación crossing — depends on (b) or a cheaper deterministic stress.
Cheap deterministic variant that needs no RNG: re-run the projection with a fixed stress path (e.g. −30% shock applied in the crossing year) and report how many months the crossing slips. **Obligation**: define the shock convention (when, to which assets) in writing first. **Acceptance test**: a fixture input with known crossing month k where the stressed crossing is a hand-derived k+Δ.

### (d) Tax-aware drawdown — **✅ ENTREGADO (4.10.0/#140 + 4.12.0/#178)**

**Esta sección describía como candidato algo que ya existe, y llevaba así desde 4.10.0.** Hoy: todo
drenaje vende **BRUTO** por la escala de tramos (`gross_up_monthly`, dentro del bucle), la base de
coste es **por activo** y baja al vender (`b' = b·v_post/v_pre`), y la fracción de plusvalía gravable
`g_i = 1 − b_i/v_i` se **deriva** de esa base viva cuando el coste está declarado; con `g`
heterogénea el bruto lo resuelve la forma cerrada por tramos (`gross_up_mixed_monthly`), sin
iteración. El ancla medida del issue: agotamiento **mes 403 → 561**. Contrato en
`financial-contracts.md` §2.4 («una sola fiscalidad, dos regímenes declarados»).
**Lo que sigue abierto** es la parte que esta sección ya marcaba como decisión aparte: la
**ORDENACIÓN** tax-aware del drenaje (hoy: líquidos primero, menor rentabilidad primero, desempate
por índice). Eso sí es un ítem vivo.

**Diseño original, conservado como referencia:**
- **Obligations**: derive the gross-withdrawal formula from embedded-gain fraction `g = (value − basis)/value` and the existing bracket math (reuse `gross_up_net_annual_fire`'s closed-form approach — see `futurefin-proof-and-analysis-toolkit` for the closed-form-vs-iteration recipe); decide basis tracking per asset (`purchase_price` exists on `SimAsset`, `projection.rs:32`); state the interaction with drain ordering (#7) — tax-aware ordering is a separate, later decision.
- **Acceptance test**: hand-computed case (single asset, one bracket, known g) matches engine drain to < 0,01 €; with `taxes_enabled = false` output is bit-identical to today's; fire-parity fixture regenerated ONLY if the target-side formula changed (it should not).

### (e) Variable / dynamic SWR — **✅ ENTREGADO en 5.0.0 (WP2)**

Entregado como **reglas de retirada**, no como «SWR variable»: el SWR sigue dimensionando SOLO el
objetivo, y lo que se retira cada mes jubilado lo decide `withdrawal_rule` ×`spend_mode`
(`crates/engine/src/withdrawal.rs`; semántica en `futurefin-fire-domain-reference` §4b). Cuatro
reglas —`fixed_real`, `percent_of_balance`, `hybrid`, `guardrails`— × dos modos —`ceiling`,
`rule_is_spend`—.

- **La regla publicada elegida es Guyton-Klinger (2006)**, citada, y con sus omisiones **declaradas**:
  la *portfolio management rule* (ventana de 15 años) y la *inflation rule* NO están implementadas,
  y omitirlas deja el modelo **más reactivo** — la dirección prudente
  (`financial-contracts.md` §4).
- **El test de aceptación pre-registrado se cumplió tal cual**: con `fixed_real` —la regla por
  defecto, que es el drenaje de 4.15.0— los números de hoy salen **exactos**, y quien lo demuestra
  es el pin dorado `pins-4.15.json` **byte-idéntico**. Los dos `spend_mode` coinciden bajo
  `fixed_real` por construcción, con test propio
  (`under_fixed_real_both_spend_modes_are_the_same_simulation`).
- **La advertencia de esta sección era correcta y sigue en pie**: sobre un camino determinista con
  rentabilidad > SWR, la regla de prosperity de los guardarraíles dispara **todos los años**
  (ratchet). No es un bug — es lo que la regla dice sobre un camino sin volatilidad — pero significa
  que los guardarraíles **solo tienen sentido pleno con (b)**. Va dicho en el `helpTexts` de la
  regla, no solo aquí.

**Gate P2**: an item may move from candidate → in-progress only when its obligations doc + pre-registered acceptance test exist in the PR/branch. If while implementing you discover the acceptance test was wrong, STOP, rewrite the prediction, and note the miss (research-methodology lifecycle) — do not quietly fit the test to the code.

## Fenced-off wrong paths (do NOT re-litigate; evidence cited)

1. **Do NOT re-deflate simulation internals** (converting the loop to today-euros / real returns). Tried in v1.0.12 ("modelo real puro", CHANGELOG 2026-05-16) — produced incoherent behavior (asset drain before retirement with inflation on) and was replaced in v1.2.0 by the current all-nominal + moving-target model. Settled. Deflation exists ONLY at display/handler edges (`deflate_points_to_today`, `apps/api/src/handlers/projection.rs:466-486`).
2. **Do NOT add a retirement trigger by age OUTSIDE a strategy** (reescrito 2026-09-03 — la
   redacción anterior, «FIRE crossing is the sole trigger», dejó de ser cierta con 5.0.0).
   `projection_target_age` sigue muerto y su migración también
   (`20260516120000_drop_projection_target_age.sql`): lo que causó el hueco visual de v1.0.6 fue
   **la coexistencia ambigua de dos disparadores** — el motor paraba las aportaciones en la edad
   mientras el marcador enseñaba el cruce. **Lo que 5.0.0 readmite** es la edad como trigger de dos
   ESTRATEGIAS (`retire_at_age`, `coast`), con tres condiciones que son lo vallado de verdad:
   (a) **un solo trigger por simulación** — con una estrategia por edad, `crossing_is_reading_only`
   deja el cruce como pura lectura (`liquid_crossing_month_index`) y no jubila;
   (b) el marcador del chart **es** `retirement_month_index`, el mes efectivo, así que el hueco de
   v1.0.6 es imposible por construcción, y el invariante se testea **sobre la serie**
   (`golden_pins.rs::the_phase_readings_agree_with_the_series_they_describe`);
   (c) sin `users.birth_date` la estrategia **degrada a `asap`** con `birth_date_missing`.
   Un ajuste global de edad que conviva con el cruce sigue siendo lo que v1.0.6 mató.
   (`futurefin-failure-archaeology` §2.2 + su scope note tiene la crónica completa.)
3. **Do NOT compute anything from array index on decimated series.** v1.4.2: the chart deflated by array index instead of `month_index` — invisible at monthly density, wrong at `hybrid` (non-equidistant points). Always use `p.month_index`. Corollary: **milestones and the jubilación crossover stay computed on the full monthly series** (`points_full`, `apps/api/src/handlers/projection.rs:1062-1075,1124-1134`), never on the serialized `points`.
4. **Do NOT switch `crates/engine` internals to f64** (reescrito 2026-09-03 — la frase «the
   simulation may not» necesita matizarse, no borrarse). Money is `rust_decimal::Decimal`
   end-to-end (non-negotiable, `futurefin-architecture-contract`), y el freezer
   `crates_engine_src_has_no_f64_outside_comments` **sigue sin una sola excepción**. Hay ahora
   **dos** fronteras sancionadas, y ninguna toca ese freezer:
   (a) la serialización de arrays grandes (v1.4.0, `serialize_decimal_as_f64`,
   `apps/api/src/handlers/projection.rs`) — precisión auditada < 1 € a 70 años;
   (b) **`crates/engine-stochastic`** (5.0.0), que implementa `MoneyOps` sobre `F64Money` y con él
   **instancia el MISMO bucle** — no lo duplica. De ese crate **no sale un euro**: solo magnitudes
   estadísticas. Lo sostiene la puerta de degeneración (1 €/mes sobre toda la batería, decisiones
   discretas exactas) y lo hace posible la regla del huérfano, no una excepción al freezer.
   **Sigue vallado**: `f64` dentro de `crates/engine` o `crates/domain`, un **segundo bucle**
   duplicado en coma flotante, y publicar un KPI en euros que venga del camino aproximado.
   Las bandas de percentiles usan la frontera (a) para el wire y el camino (b) para calcularse.
5. **Do NOT duplicate the moving-target formula outside `fire_target_at_month_index`** (localiza por
   nombre: los números de línea que aquí vivían llevaban dos releases podridos). Corolario de 5.0.0:
   el objetivo consciente del plan (`target.rs::PlanFireTarget`) **no reimplementa** la fórmula —
   sin pensión con fecha **llama** a la del núcleo, y por eso el pin dorado no puede moverse. Historical incident: engine used `(k−1)/12`, handler used `month_index/12` — a one-month off-by-one between the drawn series and the actual crossing. One helper, both consumers. Same rule for the FIRE-number formula: the sanctioned duplication is exactly Rust handler ↔ `apps/web/src/lib/fire.ts`, guarded by the shared fixture — no third copy, ever.
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

Verified 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`); **Fase 0, Fase 2 y los caminos vallados
re-verificados el 2026-09-03 contra `release/5.0.0`** (issue #207) ejecutando cada comando de esta
lista. Re-verify before trusting:

**Re-sincronizado el 2026-09-03 tras el pase de correcciones de la revisión adversarial** (commit
`0668f37`, issue #207 cerrado): §(b) pasa de «capa MC en curso, suite en rojo» a **entregada, suite
verde** (29 tests, 0 fallos — `cargo test -p futurefin-engine-stochastic 2>&1 | grep "test result"`)
y la definición de éxito se corrige (jubilarse dentro del horizonte **y** no agotar, no solo «no
agotar nunca»). La misma afirmación de «suite en rojo» estaba repetida en otros cinco documentos
(`futurefin-research-frontier`, `futurefin-fire-domain-reference`, `.claude/tests.md`,
`.claude/financial-contracts.md`, `futurefin-validation-and-qa`), todos corregidos en la misma
pasada.

- Engine test count — **usa el GLOB**: `grep -c '#\[test\]' crates/engine/src/*.rs | awk -F: '{s+=$2} END{print s}'` (**199** el 2026-09-03, y 195 unas horas antes en la misma rama). La lista de cuatro ficheros que aquí vivía (`{projection,history,runway,net_return}`) daba **139** y se dejaba fuera los seis módulos que 5.0.0 creó. Handler: `grep -c '#\[test\]' apps/api/src/handlers/projection.rs` (**26** el 2026-09-03; decía 24). Total autoritativo, siempre del runner: `cargo test -p futurefin-engine 2>&1 | grep "test result"`.
- Parity case count: `python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"` (**17** el 2026-09-03; tolerance `_tolerance_eur: 1.0`). Esta línea decía **7** y antes **6** — tercera vez que el mismo contador se queda corto en esta ficha.
- ~~Negative-return clamp still present~~: `grep -n "p <= Decimal::ZERO" crates/engine/src/projection.rs` daba **vacío** y nadie lo notó — el patrón no existe en el motor. Lo que hay que comprobar hoy es lo contrario: `grep -n 'annual_factor <= M::zero()' crates/engine/src/sim_core.rs` (el único clamp, a pérdida total) y `grep -n 'fn negative_return_composes_downward' crates/engine/src/projection.rs` (el test que fija que compone). **Un grep de provenance que sale vacío es la señal, no el ruido.**
- ~~Handler still forces no explicit withdrawal/age~~ — **VACÍO desde 5.0.0 WP1b, y el hecho que
  comprobaba ya no es cierto**: los cuatro escalares de jubilación (`retirement_start_month`,
  `retirement_monthly_withdrawal`, `income/expense_retirement_monthly`) los absorbió `PhasePlan`, y
  el handler **sí** fuerza un mes con las estrategias por edad. Lo vigente:
  `grep -n "crossing_is_reading_only = \|retirement_trigger = " apps/api/src/handlers/projection.rs`
  (**3 hits** el 2026-09-03: el flag del plan y las dos ramas que resuelven el literal que la
  respuesta publica).
- Liability interest (rows 11/14/15) — **el grep viejo (`principals\[i\] -= pay`) ya no existe**: 4.2.0 sustituyó el bloque de resta por el helper único. Vigente: `grep -n "fn liability_month\|fn liability_active\|enum RepaymentModel" crates/engine/src/{projection,sim_core}.rs` (tres hits; desde 5.0.0 WP5.5 la recurrencia vive en el núcleo genérico y el enum en la superficie pública) y `grep -c '#\[test\]' crates/engine/src/projection.rs` (**44** a 2026-08-25). Que `fixed_payments` siga siendo la recurrencia 1:1 de antes lo prueba el pin: `grep -n "pre_4_2_0\|liability_pin_input" crates/engine/src/projection.rs`.
- Horizon basis strings: `grep -n "lifespan_90\|fallback_no_demographics\|months_override" apps/api/src/handlers/projection.rs`.
- Undated planning spread: `grep -n "PLANNING_UNDATED_SPREAD_DAYS" apps/api/src/handlers/projection.rs` (90).
- Dated-flow `events`/`events_truncated` (row 8, added Fase 5/issue #86, 4.4.0): `grep -n "PROJECTION_EVENTS_MAX\|struct ProjectionEvent" apps/api/src/handlers/projection.rs`; the rejected `density`-parameter alternative is `futurefin-failure-archaeology` §2.18.
- Single FIRE-target helper still sole source: `grep -rn "fire_target_at_month_index" crates/ apps/api/src/ | wc -l` (definition + engine + handler call sites only).
- ~~CI still excludes integration tests~~ — **falso desde 4.0.0**: `grep -n "cargo test" .github/workflows/ci.yml` muestra `cargo test --workspace --locked` en el job `integration`, contra un `postgres:16.4-alpine` de servicio. Corregido en la Fase 7 (2026-08-29).
- ~~Stochastic work still unimplemented~~ — **el disparador de esta línea SE ACTIVÓ el 2026-09-03
  y este fichero está reconciliado en consecuencia** (Fase 2 (b) y (e)). El comando
  `grep -rniw "proptest\|rand\|monte" crates/engine/ apps/api/src/handlers/projection.rs` devuelve
  hoy hits legítimos: menciones a Monte Carlo en los doc-comments del núcleo genérico. Lo que sigue
  valiendo como control:
  - **proptest sigue sin existir** (ítem (a) abierto): `grep -rn proptest crates/ apps/ --include=Cargo.toml` (debe salir **vacío**).
  - **`crates/engine` sigue sin RNG**: `grep -c "rand" crates/engine/Cargo.toml` (**0**). El RNG pineado vive en `crates/engine-stochastic`.
  - **`-w` sigue siendo obligatorio** en el grep de arriba: sin él, `rand` casa dentro de la palabra española «g**rand**e» y devuelve falsos positivos.
- Doc drift record (all previously stale docs on this area were fixed 2026-07-02): standing-errata table in futurefin-docs-and-writing §7; migration count via `ls apps/api/migrations | wc -l`.
