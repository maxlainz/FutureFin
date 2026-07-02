---
name: futurefin-research-frontier
description: >
  Open improvement directions where FutureFin could genuinely lead, each with first concrete
  steps in this repo and a falsifiable milestone. Load this skill when the task is
  forward-looking: "what should we build next", "roadmap", "improvement ideas", "Monte Carlo",
  "stochastic projection", "percentile bands", "sequence-of-returns risk", "property-based
  testing", "proptest", "fuzzing the engine", "tax-aware drawdown / withdrawal", "variable SWR /
  guardrails", "multi-currency", "pre-migration backup hook", "downgrade guard", "projection
  snapshot / auditable artifact / recomputable export" — or when writing README/CHANGELOG/blog
  text that makes a public CLAIM about capabilities (this skill owns the claims-evidence rule).
  Do NOT load it for: executing deep projection-model changes (that work runs through
  futurefin-projection-realism-campaign), hypothesis/evidence discipline while investigating
  (futurefin-research-methodology), merge/release gates (futurefin-change-control), past
  rejected ideas — check futurefin-failure-archaeology BEFORE proposing anything here that
  smells like a reintroduction — or current FIRE math semantics (futurefin-fire-domain-reference).
---

# FutureFin Research Frontier

Candidate directions where FutureFin — a self-hosted household finance app — can be genuinely
better than consumer tools. As of 2026-07-02: **v1.4.3** (`apps/api/Cargo.toml`), 31 migrations,
engine at `crates/engine/src/projection.rs` (~1114 lines incl. unit tests), 8 integration-test
files in `apps/api/tests/`. Every item below is a **CANDIDATE — none is implemented**. Do not
describe any of them as existing features anywhere.

**Calibration (owner-confirmed):** this is NOT a research program. "Beyond SOTA" means practical
excellence on exactly three axes:

1. **Best-in-class FIRE modeling** — realism of the projection (returns, risk, taxes, drawdown).
2. **Deterministic, auditable engine** — same input → same output, provable, replayable.
3. **Self-hosted product excellence** — upgrades, backups, migrations that never lose data.

Anything that does not serve one of these axes for a solo self-hosted household is out of scope.

Vocabulary (once): **SWR** = Safe Withdrawal Rate, the % of net worth withdrawn annually in
retirement (validated 0–4 in `apps/api/src/handlers/installation.rs`). **gross-up** = solving for
the gross withdrawal whose after-tax value equals the net annual need, through Spanish
capital-gains brackets (closed form: `gross_up_net_annual_fire`,
`apps/api/src/handlers/projection.rs:106`). **nominal vs real** = the engine simulates in nominal
euros; only the FIRE target grows with inflation (v1.2.0 model). **cascade / sobrante** =
ordered allocation rules distributing the monthly surplus across assets. **jubilación crossing**
= first month where net worth ≥ the inflated FIRE target (`jubilacion_month_index`).
**parity fixture** = `apps/api/tests/fixtures/fire-parity.json`, one canonical case set consumed
by both `apps/api/tests/fire_parity.rs` and `apps/web/src/lib/fire.test.ts`.

## When NOT to use this skill

- Executing a projection-model change end-to-end → `.claude/skills/futurefin-projection-realism-campaign/SKILL.md` (items 4, 5, 6, 7 below EXECUTE through it)
- Evidence bar, predict-then-run, idea lifecycle while investigating → `.claude/skills/futurefin-research-methodology/SKILL.md`
- Whether/how a change may merge, migration + release gates → `.claude/skills/futurefin-change-control/SKILL.md`
- "Has this been tried and rejected?" → `.claude/skills/futurefin-failure-archaeology/SKILL.md` — **check it before pitching anything from this file**; e.g. deflated-engine simulation, age-based retirement trigger and per-asset contribution config are settled rejections, not open frontier
- Current FIRE math as implemented → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`
- Writing/running the tests a frontier item needs → `.claude/skills/futurefin-validation-and-qa/SKILL.md`
- Numeric proof techniques (closed forms, index math, refactor-equivalence) → `.claude/skills/futurefin-proof-and-analysis-toolkit/SKILL.md`

## The assets that make frontier work tractable HERE

These four repo properties are the competitive advantage. Every item below leans on at least one;
if a proposal leans on none, it probably belongs in a different product.

| Asset | Where it lives | Why it matters for frontier work |
|---|---|---|
| Pure deterministic Decimal engine | `crates/engine/src/projection.rs` — no I/O, no clock, no RNG (deps: chrono, rust_decimal, serde, thiserror, uuid only) | Any input can be replayed bit-exactly; properties and stress scenarios are cheap to test |
| Parity-fixture discipline | `apps/api/tests/fixtures/fire-parity.json` + dual consumers (Rust + TS) | Duplicated math (server vs UI preview) cannot silently diverge; new math extends the fixture |
| Forensic changelog | `CHANGELOG.md` (root causes documented per release) | Model changes are explainable years later; claims can be traced to the release that earned them |
| Schema-isolated integration harness | `apps/api/tests/common/mod.rs` (per-test Postgres schema) | End-to-end behavior of a new endpoint/flag is testable against a real DB without cross-test pollution |

## Claims discipline (external positioning)

Nothing may be claimed in `README.md`, `CHANGELOG.md`, release notes, or any public text unless
it traces to **in-repo, runnable evidence**. As of 2026-07-02 `README.md` makes no claims about
Monte Carlo, stochastic modeling or bit-exactness — keep it that way until the evidence exists.

| Claim you might be tempted to write | Minimum evidence required first |
|---|---|
| "Monte Carlo / percentile bands" | Engine module + tests proving seeded reproducibility AND zero-volatility degeneration to the deterministic series (item 6) |
| "Deterministic / bit-exact projections" | A replay test asserting two runs of `project_net_worth_series` on the same input are `assert_eq!`-identical, running in CI (`cargo test -p futurefin-engine --locked` — CI runs engine tests; the Postgres integration tests do NOT run in CI, they need local `TEST_DATABASE_URL`) |
| "Property-tested engine invariants" | proptest suite merged and green (item 1), named properties listed in the CHANGELOG entry |
| "Tax-aware withdrawals" | Engine drawdown tax model + regenerated parity fixture with both suites green (item 7) |
| "Safe upgrades / never lose data" | Pre-migration dump hook + a restore actually exercised in a test or documented drill (item 2) |

Rule of thumb: the CHANGELOG entry that introduces a capability must name the test(s) that prove
it. If you cannot name the test, the claim is not ready.

## Ranked frontier items

Ranking = value for a solo self-hosted household app ÷ effort, discounted by risk. Items 4–7
change the economic model and therefore execute through
`futurefin-projection-realism-campaign`; ALL items gate through `futurefin-change-control`.

| # | Item | Axis | Value | Effort | Risk | Verdict |
|---|---|---|---|---|---|---|
| 1 | Property-based engine invariants (proptest) | 2 | High | Low | Low | Do first |
| 2 | Migration-safety tooling (pre-migration dump, downgrade guard) | 3 | High | Low–Med | Low | Do |
| 3 | Projection-as-auditable-artifact (recomputable snapshot) | 2, 3 | Med–High | Med | Low | Do |
| 4 | Sequence-of-returns risk surfacing (deterministic stress) | 1 | High | Med | Med | Via campaign |
| 5 | Liability interest accrual (discovered gap) | 1 | Med–High | Med | Med | Via campaign |
| 6 | Monte Carlo percentile bands (seeded) | 1 | Med | High | High | Candidate, gate behind 4 |
| 7 | Tax-aware drawdown path | 1 | Med | Med–High | Med | Candidate, via campaign |
| 8 | Variable/dynamic SWR strategies | 1 | Low–Med | Med | Med | Deferred until 6 exists |
| 9 | Multi-currency correctness | — | Low | High | Med | Demoted — see inventory |

---

### 1. Property-based testing of engine invariants — DO FIRST

**Why consumer tools fall short:** closed-source planners cannot show their engines hold ANY
invariant; users take numbers on faith. FutureFin's history (v1.4.2 index-vs-month_index bug,
the engine/handler `(k-1)/12` off-by-one) shows example-based tests miss whole input classes.

**FutureFin's asset:** the engine is a pure function of `ProjectionInput` — proptest can generate
thousands of households with zero mocking. 22+ example tests already encode expected shapes.

**Invariants grounded in the actual simulation loop** (`project_net_worth_series`,
`crates/engine/src/projection.rs:386`):

- **I1 — cascade conserves the pool.** In `distribute_contributions` (line 249),
  `sum(alloc) + leftover == pool` and every `alloc[i] >= 0`. Observable via
  `first_month_per_asset_contribution_nominals`: sum of nominals ≤ first-month net cash, with
  equality when an uncapped `Remainder` rule exists.
- **I2 — caps never exceeded.** After distribution, no asset's live value exceeds its resolved
  ceiling (`resolve_cap_ceiling`, line 220) when it started at or below it — including multiple
  rules targeting the same asset (the `live_values` mechanism, line 264).
- **I3 — series alignment.** `net_worth.len() == contributed_capital.len() ==
  horizon_months + 1`, and every inner `per_asset_series[i]` has the same length.
- **I4 — contributed capital is monotone non-decreasing.** `contributed_cumulative` only ever
  grows (lines 527–532); retirement drain must never reduce it.
- **I5 — zero-rate accounting identity.** With all `expected_annual_return_percent = None`, no
  FIRE target and no retirement: `net_worth[k] − net_worth[k−1] == income − expense +
  planning_adj[k−1]` for every k — debt service is net-worth-neutral while principal covers the
  payment (cash −pay, liability −pay), and the drain path stays exact because `nw_fn` subtracts
  `undrained_cumulative` (line 437).
- **I6 — replay determinism.** Two calls with a cloned input are `assert_eq!`-identical
  (guards against anyone ever introducing RNG, HashMap iteration order or clock reads).
- Bonus targeted property — **drain priority**: `drain_from_assets` (line 184) empties liquid
  assets before illiquid, and lower-return before higher-return within each class.

**First three steps in this repo:**
1. Add `proptest = "1"` under a new `[dev-dependencies]` section in `crates/engine/Cargo.toml`
   (it currently has none).
2. Create `crates/engine/tests/proptest_invariants.rs` with a bounded `ProjectionInput`
   generator: amounts as `Decimal` with 2 decimal places in `0..=10_000_000`, horizon
   `1..=240` (keep runtime sane; 840 in a few `#[test]` spot checks only), 0–4 assets,
   0–4 rules with valid `target_index`, rates `0..=15`.
3. Implement properties named `prop_cascade_conserves_pool`, `prop_caps_never_exceeded`,
   `prop_series_lengths_align`, `prop_contributed_capital_monotone`,
   `prop_zero_rate_accounting_identity`, `prop_replay_bit_identical`; run with
   `cargo test -p futurefin-engine` (CI already runs this target, `.github/workflows/ci.yml`).

**You have a result when:** all six properties are green in CI AND a deliberate mutation (e.g.
change `k - 1` to `k` in the `fire_reached` check at projection.rs:478, or drop the
`live_values[target] += take` line) is caught by at least one property. If no property catches
either mutation, the suite is decorative — fix the generators before merging.

---

### 2. Migration-safety tooling for self-hosted installs

**Why consumer tools fall short:** SaaS handles upgrades invisibly; most self-hosted apps just
say "back up first". FutureFin auto-migrates on startup (`apps/api/src/db.rs:20`,
`sqlx::migrate!`) — an upgrade IS a migration, and today nothing snapshots the DB before it runs.

**FutureFin's asset:** `scripts/backup-postgres.sh` already does a correct compose-exec
`pg_dump | gzip -9` with retention (`ENV_FILE=.env.prod`, `BACKUP_DIR=./backups`,
`KEEP_BACKUPS=30` defaults — read it before extending). sqlx already fails loud on checksum
mismatch, and an older binary refuses to run when `_sqlx_migrations` contains versions it does
not know (a de-facto partial downgrade guard — verify the exact error text before documenting it
as such; unverified beyond sqlx's documented behavior).

**First three steps in this repo:**
1. Add `scripts/upgrade-with-backup.sh`: run `scripts/backup-postgres.sh`, then
   `docker compose --env-file .env.prod pull && up -d`, then poll
   `curl -sf http://127.0.0.1:8080/v1/health`; on failed health after N tries, print the exact
   restore command for the dump just taken (do NOT auto-restore).
2. In `apps/api/src/main.rs`, log before/after `db::run_migrations`: count of pending
   migrations about to apply and app version (`env!("CARGO_PKG_VERSION")` is already logged at
   startup, main.rs:30) — so post-incident forensics can tell "which upgrade ran which
   migrations".
3. Add an integration test `apps/api/tests/migration_guard.rs` asserting the documented failure
   mode: applying a fake `_sqlx_migrations` row with an unknown version makes
   `run_migrations` return an error (schema-isolated harness from `apps/api/tests/common/mod.rs`).

**You have a result when:** a full drill on the local Docker stack (CLAUDE.md "Test local con
Docker Desktop") — upgrade with the script, deliberately break the image tag, restore from the
auto-taken dump — ends with `/v1/health` green and data intact, and the drill's commands are
recorded in `.claude/skills/futurefin-run-and-operate/SKILL.md` or README "Backups".

---

### 3. Projection-as-auditable-artifact (recomputable snapshot)

**Why consumer tools fall short:** no consumer planner lets you export "what the projection said
on date X, with which inputs, under which engine version" and re-verify it later. Numbers are
ephemeral; disputes ("it said I'd retire in 2041 last month") are unresolvable.

**FutureFin's asset:** the engine is pure — `ProjectionInput` fully determines
`ProjectionOutput`. `ProjectionInput` already derives `Serialize`/`Deserialize` partially
(`SimAsset` does; check remaining structs) and the encrypted user-backup layer
(`apps/api/src/handlers/backup_user/`) already ships `schema_version` + `app_version`
(`export.rs:71`) — the manifest pattern to copy.

**First three steps in this repo:**
1. Make `ProjectionInput` and `ProjectionOutput` fully `Serialize + Deserialize` in
   `crates/engine/src/projection.rs` (add derives to `AllocationRule`, `AllocationCap`,
   `ProjectionLiabilityInput`, `FireTarget`, `ProjectionInput`, `ProjectionOutput`).
2. Add engine test `snapshot_roundtrip_recomputes_identically` in the same file's `mod tests`:
   serialize input to JSON, deserialize, re-run, `assert_eq!` against the original output.
   (Serialize Decimals as strings in this artifact — the f64 wire format of
   `/v1/projection/series` is a display optimization, NOT audit-grade; see
   `serialize_decimal_as_f64`, `apps/api/src/handlers/projection.rs:177`.)
3. Add `GET /v1/projection/snapshot` (follow `.claude/adding-handler.md`): respond with
   `{engine_input, engine_output_sha256, app_version, anchor_date_ymd}` where the hash covers the
   canonical JSON of the output. Integration test in `apps/api/tests/projection_snapshot.rs`:
   fetch twice, hashes equal; mutate an asset, hash changes.

**You have a result when:** a snapshot exported from the API can be re-verified OFFLINE — a tiny
`cargo test`-level check (or example binary) that takes the artifact JSON, re-runs the engine at
the same crate version and reproduces the recorded hash bit-exactly. Only after that may any
"auditable/recomputable" wording appear publicly (see claims table).

---

### 4. Sequence-of-returns risk surfacing near the jubilación crossing — via campaign

**Why consumer tools fall short:** they show one average-return path. Two retirees with
identical average returns can have wildly different outcomes depending on WHEN bad years land;
a crash the year after retiring is far worse than the same crash ten years earlier. This
(sequence-of-returns risk, SORR) is invisible in every single-path chart — including
FutureFin's today, where each asset compounds at a constant `monthly_multiplier`
(projection.rs:154).

**FutureFin's asset:** this does NOT need randomness. The engine is pure and fast enough to run
twice per request already (main + marker simulation via `spawn_blocking` + `tokio::join!`,
handler lines ~1019–1036). A deterministic stress path — "apply a fixed −X% shock in month M" —
re-uses everything and stays bit-reproducible.

**First three steps in this repo:**
1. Write the design note as a campaign step (`futurefin-projection-realism-campaign`): shock
   size, which assets it hits (only those with `expected_annual_return_percent > 0`?), and the
   exact output metric (shift of `jubilacion_month_index` + post-retirement depletion month).
2. Engine: add an optional per-month return override hook to `ProjectionInput` (e.g.
   `shock: Option<(u32, Decimal)>` = month index + multiplier), applied in the growth step
   (projection.rs:535–538). Unit test `shock_month_reduces_growth_exactly_once` — a shock at
   month k changes `net_worth[k]` and nothing before it.
3. Handler: run the projection a third time with the shock anchored at the unstressed
   `jubilacion_month_index`; expose e.g. `sorr_jubilacion_delay_months` in
   `ProjectionSeriesResponse`. Extend `apps/api/tests/projection_marker.rs` pattern for the
   endpoint test. Cache note: any new input axis must join `ProjectionCacheKey`
   (`apps/api/src/state.rs`) or be derived from cached full series.

**You have a result when:** on a fixture household, the same shock placed 1 month before the
crossing delays jubilación strictly more than when placed 10 years earlier — asserted in a test
with numbers predicted BEFORE running (futurefin-research-methodology discipline). If the deltas
come out equal, the model is not capturing SORR and must not ship.

---

### 5. Liability interest accrual — discovered gap, via campaign

**Discovery (verified in code):** the simulation reduces each liability's principal by the full
monthly payment with NO interest accrual — `principals[i] -= pay` (projection.rs:549–553). A
250k€ mortgage at 1.200 €/month is simulated as paid off in ~208 months regardless of its rate;
real amortization would take far longer. Net-worth trajectories with large mortgages are
systematically optimistic (debt vanishes too fast). Liabilities carry no rate column today —
check `apps/api/migrations/` and `.claude/data-model.md` (noting that doc has known drift)
before designing the schema change.

**Why this ranks above Monte Carlo:** it is a correctness bug-shaped gap in axis 1 with a
closed-form ground truth (standard amortization) to test against — cheaper and more certain
value than stochastic features.

**First three steps in this repo:**
1. Campaign design note: add optional `annual_interest_rate_percent` to liabilities (migration +
   `ProjectionLiabilityInput`), semantics: monthly interest accrues on principal before the
   payment applies; payment smaller than interest ⇒ principal grows (test this case explicitly).
2. Engine: implement in the liability loop (projection.rs:540–555) + unit tests
   `liability_interest_slows_amortization` and `payment_below_interest_grows_principal`,
   asserting against hand-computed closed-form amortization values.
3. Migration `apps/api/migrations/<YYYYMMDDHHMMSS>_liability_interest_rate.sql` (nullable column,
   NULL = current zero-interest behavior — existing installs unchanged), handler plumbing in
   `apps/api/src/handlers/liabilities.rs` + `projection.rs`, gated through
   futurefin-change-control (schema + engine behavior change).

**You have a result when:** for a fixture loan, the simulated payoff month matches the standard
amortization formula within 1 month, and a NULL-rate liability reproduces today's series
bit-exactly (regression guard).

---

### 6. Monte Carlo percentile bands — candidate, gated behind item 4

**Why consumer tools fall short:** those that do offer Monte Carlo hide the return model and are
non-reproducible (fresh RNG per view — the number changes on refresh, killing trust).

**FutureFin's angle:** SEEDED stochastic projection preserving the determinism contract: the
seed is part of the input, the run is replayable, the artifact (item 3) records it. The engine
currently has **no RNG dependency at all** — that absence is the contract; any stochastic layer
must keep the deterministic path untouched and bit-identical.

**Honest cost assessment:** one 841-month Decimal simulation already needs `spawn_blocking`;
1.000 paths ≈ 1.000×. Expect to need f64 internally for the stochastic paths (precedent: the
wire arrays are f64 by deliberate exception) or drastically fewer paths — this is a real design
problem, not a footnote. Do item 4 first: if deterministic stress answers the household's actual
question ("how fragile is my date?"), Monte Carlo may never be worth it.

**First three steps in this repo:**
1. Campaign design note: return model (lognormal per asset? correlated?), path count, PRNG
   (e.g. `rand_chacha`, pinned version — cross-platform reproducibility is the point), precision
   choice with an error bound argument (futurefin-proof-and-analysis-toolkit).
2. New module `crates/engine/src/stochastic.rs` exposing
   `project_percentile_bands(&ProjectionInput, &McConfig { seed: u64, paths: u32, .. })`,
   reusing the monthly loop with injected per-month multipliers.
3. Engine tests `mc_same_seed_bit_identical_bands` and
   `mc_zero_volatility_degenerates_to_deterministic` (P10 == P50 == P90 == the
   `project_net_worth_series` output within the documented precision bound).

**You have a result when:** both tests are green AND replaying an exported artifact (item 3)
with its recorded seed reproduces the bands exactly. Only then may "Monte Carlo" appear in any
public text (claims table above).

---

### 7. Tax-aware drawdown path — candidate, via campaign

**Verified inconsistency worth fixing:** taxes exist only in FIRE-target sizing — the closed-form
gross-up (`gross_up_net_annual_fire`) inflates the target so SWR withdrawals cover taxes. But the
simulated retirement phase drains the NET need from assets with no tax
(`drain_from_assets`, projection.rs:505–513): the post-crossing depletion picture is optimistic.
The ingredient for capital-gains math already exists per asset: `SimAsset.purchase_price` (basis).

**First three steps in this repo:**
1. Campaign design note quantifying the gap on a fixture household (predict the depletion-month
   shift by hand first). Decide where `TaxBracket` lives — it is currently defined in
   `apps/api/src/handlers/installation.rs`; the engine cannot depend on the API crate, so the
   type moves to `crates/domain` or is mirrored in the engine.
2. Engine: in the drain path, gross-up the monthly shortfall through brackets using each asset's
   gain fraction `(value − basis)/value` before withdrawing; unit test
   `retirement_drain_pays_capital_gains_tax` against a hand-computed 1-asset case.
3. Extend `apps/api/tests/fixtures/fire-parity.json` with drawdown cases and regenerate expected
   values; BOTH `apps/api/tests/fire_parity.rs` and `apps/web/src/lib/fire.test.ts` must pass
   (parity discipline — see futurefin-validation-and-qa).

**You have a result when:** with taxes enabled the simulated depletion month shortens versus
today's model by exactly the hand-computed amount (±1 month), and disabling taxes reproduces the
current series bit-exactly.

### 8. Variable/dynamic SWR strategies — deferred

Guardrails-style rules (spend less after bad years) only produce different outcomes when returns
VARY — under today's constant-return model every strategy degenerates to the fixed SWR. Zero
value before item 6 (or at least item 4) exists. Revisit then; until that day, the only useful
groundwork is keeping `swr_pct` validation and the fixture cases honest. Do not build UI for it.

### 9. Multi-currency correctness — demoted

Honest inventory (verified 2026-07-02): `base_currency` validates EUR/USD/GBP
(`installation.rs:295`) and display formatting is already ISO-parametric
(`formatCurrencyAmount`, `apps/web/src/lib/format.ts:49`). What is genuinely EUR/ES-centric:
locale hardcoded `es-ES` (`format.ts:6`), Spanish IRPF savings brackets as the tax default
(`default_es_tax_brackets`, `installation.rs:91` — 19/21/23/27/30%), Spanish UI copy, and — the
real cost — a single-currency ledger: no per-asset currency column, no FX rates, no conversion
anywhere in schema or engine. True multi-currency is a schema + engine + FX-data project with
near-zero value for the actual EUR household running this installation. **Recommendation:** do
not pursue. The only cheap worthwhile slice: label the default brackets as Spanish in the
Ajustes UI (they are already user-editable via `PATCH /v1/installation` fire_settings).

---

## Adding a new frontier item

1. Check `futurefin-failure-archaeology` — is it a settled rejection?
2. Tie it to one of the three axes; if it fits none, drop it.
3. Name the repo asset that makes it tractable HERE; write the first three steps with real
   paths and test names, and a falsifiable "result when…" (a milestone you cannot fail is not
   a milestone).
4. Add it to the ranked table with value/effort/risk and update this file (doc maintenance:
   futurefin-docs-and-writing).

## Provenance and maintenance

Facts verified 2026-07-02 against v1.4.3. Re-verify before relying on them:

- Version: `grep -m1 '^version' apps/api/Cargo.toml`
- Engine purity (no RNG/IO deps): `grep -A8 '\[dependencies\]' crates/engine/Cargo.toml`
- No proptest yet: `grep -rn proptest crates/ apps/ --include=Cargo.toml` (empty = item 1 still open)
- Liability loop has no interest accrual: `grep -n 'principals\[i\] -= pay' crates/engine/src/projection.rs`
- Drain is tax-free / gross-up only in target: `grep -n 'gross_up_net_annual_fire\|drain_from_assets' apps/api/src/handlers/projection.rs crates/engine/src/projection.rs`
- Backup script defaults: `sed -n '17,19p' scripts/backup-postgres.sh`
- Migration runner (auto on startup, fails loud): `cat apps/api/src/db.rs`
- Parity fixture + consumers: `ls apps/api/tests/fixtures/fire-parity.json apps/api/tests/fire_parity.rs apps/web/src/lib/fire.test.ts`
- CI covers engine tests but NOT Postgres integration tests: `grep -n 'cargo test' .github/workflows/ci.yml`
- README still claim-free on MC/determinism: `grep -in 'monte\|stochastic\|bit-exact\|deterministic' README.md` (empty = good)
- Currency/locale state: `grep -n 'EUR.*USD.*GBP' apps/api/src/handlers/installation.rs; grep -n DISPLAY_NUMBER_LOCALE apps/web/src/lib/format.ts`
- Horizon basis strings: `grep -n '"lifespan_90"\|"fallback_no_demographics"\|"months_override"' apps/api/src/handlers/projection.rs`
- Migration count: `ls apps/api/migrations | wc -l` (31 as of 2026-07-02)
