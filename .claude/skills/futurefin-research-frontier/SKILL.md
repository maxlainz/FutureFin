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
better than consumer tools. Refreshed **2026-08-16 for v3.0.0**: 34 migrations, engine at
`crates/engine/src/projection.rs` (~1122 lines incl. unit tests), 20 integration-test files in
`apps/api/tests/` (originally written 2026-07-02 at v1.4.3, 31 migrations, 8 test files).

**Implementation status — read before quoting this file.** Every item here was a pure candidate
until 3.0.0. That is no longer true of **item 2**: its *pre-migration backup* half **shipped in
3.0.0** (automatic, inside the container) and its remaining two pieces — the **explicit downgrade
guard with an operator message** and **`apps/api/tests/migration_guard.rs`** — **shipped 2026-08-27**
on branch `feat/home-assistant-addon`. Item 2 is now essentially done; what is left is a drill, not
a feature (see the item). Nor of **item 5**: liability interest accrual (`RepaymentModel` +
`apr_percent`) **shipped in full in 4.2.0** (2026-08-25) — see the item for what landed and what
tests prove it. Items 1, 3, 4 and 6–9 remain **CANDIDATES — not implemented**; do not describe any
of them as existing features anywhere.

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

- Executing a projection-model change end-to-end → `.claude/skills/futurefin-projection-realism-campaign/SKILL.md` (items 4, 6, 7 below EXECUTE through it; item 5 already did — shipped 4.2.0)
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
it traces to **in-repo, runnable evidence**. Re-checked 2026-08-16: `README.md` still makes no
claims about Monte Carlo, stochastic modeling or bit-exactness — keep it that way until the
evidence exists. Its 3.0.0 backup wording ("el contenedor escribe **automáticamente** un backup
`pre-migration-*.sql.gz` antes de aplicar migraciones nuevas") is inside the rule, not an
exception: it names a mechanism CI asserts, and it deliberately stops short of "never lose data".

| Claim you might be tempted to write | Minimum evidence required first |
|---|---|
| "Monte Carlo / percentile bands" | Engine module + tests proving seeded reproducibility AND zero-volatility degeneration to the deterministic series (item 6) |
| "Deterministic / bit-exact projections" | A replay test asserting two runs of `project_net_worth_series` on the same input are `assert_eq!`-identical, running in CI (`cargo test -p futurefin-engine --locked` — CI runs engine tests **and, since 4.0.0, the Postgres integration ones too** — job `integration`, `cargo test --workspace --locked` against a service Postgres; this parenthesis claimed the opposite until the Fase-7 sweep) |
| "Property-tested engine invariants" | proptest suite merged and green (item 1), named properties listed in the CHANGELOG entry |
| "Tax-aware withdrawals" | Engine drawdown tax model + regenerated parity fixture with both suites green (item 7) |
| "Safe upgrades / never lose data" | Pre-migration dump hook + a restore actually exercised in a test or documented drill (item 2). **Partly earned in 3.0.0**: the automatic pre-migration dump exists and the CI `docker-stack` job exercises V2→V3 with real data, automigration and pg_upgrade 15→16. **The downgrade guard was earned on 2026-08-27** (`db.rs` → `MigrationError::Downgrade` + operator banner, pinned by `apps/api/tests/migration_guard.rs`), so "refuses to start instead of running an old schema over new data, and says so in words you can act on" is now claimable. Still unearned, and therefore still unclaimable: a restore drill run against a *production-shaped* dump. Word claims to what the evidence covers — "backs itself up before every migration" and "refuses to downgrade" are provable today; "never lose data" is not. |
| "The MCP catalog fits in context" / "context-efficient MCP" | Split the claim by which half you mean. **Descriptions**: earned, but the margin is now thin — `apps/api/tests/mcp_http.rs::tool_descriptions_stay_within_the_context_budget` (`PER_TOOL_MAX = 600`, `TOTAL_BUDGET = 24_000`) is green in CI; Fase 5 (issue #86) cut 37.214 → 21.319 chars over 52 tools, and Fase 6 (issue #87) added 16 tools that pushed the raw total to **28.884** (+4.884 over budget) before the prescribed rebalancing brought it to **23.874 / 24.000, max 596** — **126 characters of headroom**, so "fits in context" is true today and one tool away from needing work again. **`inputSchema`**: NOT earned — measured after that cut it is ~55 KB, ~2,7× the descriptions, and no guard test exists for it (item 10). Do not let "the catalog is context-efficient" stand as a whole claim until item 10's guard exists too — today it is true for one half of the payload and open for the larger half. |

Rule of thumb: the CHANGELOG entry that introduces a capability must name the test(s) that prove
it. If you cannot name the test, the claim is not ready.

## Ranked frontier items

Ranking = value for a solo self-hosted household app ÷ effort, discounted by risk. Items 4–7
change the economic model and therefore execute through
`futurefin-projection-realism-campaign` (item 5 already did — shipped 4.2.0); ALL items gate
through `futurefin-change-control`. Item 10 touches the MCP catalog surface, so it additionally
gates through `futurefin-mcp-parity` when executed.

| # | Item | Axis | Value | Effort | Risk | Verdict |
|---|---|---|---|---|---|---|
| 1 | Property-based engine invariants (proptest) | 2 | High | Low | Low | Do first |
| 2 | Migration-safety tooling (pre-migration dump ✅ 3.0.0; downgrade guard + `migration_guard.rs` ✅ 2026-08-27) | 3 | High | Low | Low | **Done except the restore drill** |
| 3 | Projection-as-auditable-artifact (recomputable snapshot) | 2, 3 | Med–High | Med | Low | Do |
| 4 | Sequence-of-returns risk surfacing (deterministic stress) | 1 | High | Med | Med | Via campaign |
| 5 | Liability interest accrual (`RepaymentModel` + `apr_percent`) | 1 | High | — | — | **✅ Done — shipped 4.2.0** |
| 6 | Monte Carlo percentile bands (seeded) | 1 | Med | High | High | Candidate, gate behind 4 |
| 7 | Tax-aware drawdown path | 1 | Med | Med–High | Med | Candidate, via campaign |
| 8 | Variable/dynamic SWR strategies | 1 | Low–Med | Med | Med | Deferred until 6 exists |
| 9 | Multi-currency correctness | — | Low | High | Med | Demoted — see inventory |
| 10 | MCP catalog context budget: descriptions done, `inputSchema` next | 3 | Med | Low | Low | Do next |

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
   `prop_series_lengths_align`, `prop_contributed_capital_bounded` (OJO: la monotonía que aquí
   se proponía MURIÓ con #120/4.10.0 — vender baja la base a propósito; el invariante que sí
   sobrevive es `contributed(k) ≤ Σ purchase_price + Σ aportaciones acumuladas`),
   `prop_zero_rate_accounting_identity`, `prop_replay_bit_identical`; run with
   `cargo test -p futurefin-engine` (CI already runs this target, `.github/workflows/ci.yml`).

**You have a result when:** all six properties are green in CI AND a deliberate mutation (e.g.
change `k - 1` to `k` in the `fire_reached` check at projection.rs:478, or drop the
`live_values[target] += take` line) is caught by at least one property. If no property catches
either mutation, the suite is decorative — fix the generators before merging.

---

### 2. Migration-safety tooling for self-hosted installs — PARTLY SHIPPED IN 3.0.0

**Why consumer tools fall short:** SaaS handles upgrades invisibly; most self-hosted apps just
say "back up first". FutureFin auto-migrates on startup (`apps/api/src/db.rs`, `sqlx::migrate!`) —
an upgrade IS a migration.

#### ✅ Shipped in 3.0.0 — the automatic pre-migration backup

The original framing of this item ("today nothing snapshots the DB before it runs") is **no longer
true**. With the self-contained image, the container backs *itself* up before letting the API touch
the schema. Mechanism, in `apps/api/docker-entrypoint.sh::premigration_backup`, running after
PostgreSQL is up and before the API is launched:

- **Detection without starting the API.** The image ships a manifest `/app/migration-versions.txt`
  (built in the Dockerfile from `ls apps/api/migrations/*.sql`) plus `/app/VERSION`. The entrypoint
  takes a backup when **either** the app version changed since the last boot
  (`$STATE_DIR/state/last-version`) **or** `comm` shows a migration in the manifest that is absent
  from `_sqlx_migrations`. A brand-new database (no `_sqlx_migrations` rows) skips it — there is
  nothing to lose yet.
- **Output**: `pg_dump … | gzip -6` to
  `/var/lib/futurefin/backups/pre-migration-<from>-to-<to>-<UTC ts>.sql.gz`, i.e. inside the
  **`ffdata` volume**, separate from `pgdata`. Retention `prune_backups`: keep the newest
  `FUTUREFIN_BACKUP_KEEP` (10), then drop anything older than `FUTUREFIN_BACKUP_KEEP_DAYS` (90),
  plus a disk-pressure sweep below 256 MB free that never deletes the last 3.
- **Failure aborts the boot.** If `pg_dump` fails, the partial file is removed and the entrypoint
  `die`s: *"refusing to start with pending migrations and no safety net"*. Deliberate bypass:
  `FUTUREFIN_PREMIGRATION_BACKUP=off`. This is the part that makes it a safety property rather than
  a convenience.
- **Sibling guarantees from the same release**: `pg_upgrade` writes a **mandatory** `pg_dumpall`
  first and verifies the new cluster by row census before swapping; the one-shot external
  automigration dumps the source read-only and verifies by census too. Both preserve the old
  cluster (`mv`, never `rm`).
- **Evidence**: CI `docker-stack` asserts `ls /var/lib/futurefin/backups/pre-migration-*.sql.gz`
  after the V2→V3 upgrade, and `pre-pgupgrade-15-to-16-*.sql.gz` after the major upgrade.

**Consequence for the old step 1 — `scripts/upgrade-with-backup.sh` is obsolete as designed.** An
external wrapper that dumps *before* `docker compose pull && up -d` no longer adds anything: the
new container takes its own dump after it starts and before it migrates, which is strictly better
(it cannot be forgotten, and it runs at the exact moment the schema is at risk). Do not build it.
What the host-side scripts are for now: `scripts/backup-postgres.sh` (ad-hoc dump via
`compose exec … pg_dump -h /var/run/postgresql`) and `scripts/restore-postgres.sh` (restores into a
temporary `FUTUREFIN_MODE=db-only` container, printing a row census before and after).

#### ✅ Shipped 2026-08-27 (branch `feat/home-assistant-addon`) — the two pieces this item had left

1. **Explicit downgrade guard with a friendly message — SHIPPED.** `apps/api/src/db.rs` now maps
   sqlx's `MigrateError::VersionMissing` (which *is* the signature of "the database went past this
   binary") to its own `MigrationError::Downgrade { version, message }`. No extra check was added on
   top: sqlx already fails: the work was (a) not losing that failure in a generic error and (b)
   translating it. The message is a boxed operator banner —
   `FutureFin NO ARRANCA: esta base de datos viene de una versión MÁS NUEVA.` — naming the unknown
   migration version and the binary's own version, stating explicitly that **nothing was touched**,
   and giving both exits: go back to the newer tag, or restore the matching
   `pre-migration-*.sql.gz` (`ffdata`, or `/data/state/backups` under the Home Assistant add-on),
   pointing at `docs/backups.md`. Same quality bar as the entrypoint's own guards, as the item asked.
2. **`apps/api/tests/migration_guard.rs` — SHIPPED** (2 tests). It applies the real migrations to a
   schema-isolated DB with `common::isolated_pool()` (deliberately *not* `TestApp` — no router
   needed), inserts a fake `_sqlx_migrations` row with version `99_999_999_999_999`, and asserts
   `run_migrations` returns `MigrationError::Downgrade` carrying the operator banner. It tests the
   contract, not the implementation: keep the `VersionMissing` arm and the message quality, and the
   test stays honest if the mechanism changes.

Still not done from the optional half: the pending-migration count logged around
`db::run_migrations` (below), and the **restore drill** of the "you have a result when" clause (c) —
running an actual `pre-migration-*.sql.gz` back through `scripts/restore-postgres.sh` end to end.
Until (c) is exercised, the "safe upgrades" claim is well-evidenced but not fully drilled.

Optional and cheap: log pending-migration count + app version around `db::run_migrations` in
`apps/api/src/main.rs`, so post-incident forensics can answer "which upgrade ran which migrations"
from the app log as well as from the backup filename.

**You have a result when:** (a) `migration_guard.rs` is green — **done**, (b) starting an older tag
over a migrated volume prints a message a non-Rust user can act on — **done** (`db.rs`
`downgrade_message`), and (c) a restore drill — **still pending** —
`scripts/restore-postgres.sh` against an automatic `pre-migration-*.sql.gz` pulled out of `ffdata`
— ends with `/v1/ready` green and the row census matching, with the commands recorded in
`.claude/skills/futurefin-run-and-operate/SKILL.md` or README "Backups". Only then does the
"safe upgrades" claim cover the whole item (claims table above).

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
3. Add `GET /v1/projection/snapshot` (follow `.claude/backend-structure.md` §Cómo añadir un handler): respond with
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

### 5. Liability interest accrual — ✅ SHIPPED IN 4.2.0

**Discovery that started this item (verified in code as of 2026-08-16, no longer true):** the
simulation used to reduce each liability's principal by the full monthly payment with **no**
interest accrual — `principals[i] -= pay`. A 250k€ mortgage at 1.200 €/month was simulated as
paid off in ~208 months regardless of its rate; real amortization would take far longer.
Net-worth trajectories with large mortgages were systematically optimistic (debt vanished too
fast).

**What shipped, verified against `CHANGELOG.md` §[4.2.0] (2026-08-25) and
`crates/engine/src/projection.rs`:** every liability now declares HOW it is paid via
`RepaymentModel` — `fixed_payments` (the old 1:1 behavior; stays the column default so upgrading
moves no number), `french` (standard amortization), `interest_only`, `revolving` (shares
`french`'s recurrence in 4.2.0, deliberately, pinned by a test) — plus `apr_percent` (nominal
annual rate, `i = apr_percent / 1200`, same convention the historical snapshot interpolation
already used). The accruing models apply `P' = P·(1+i) − M` on the **opening** balance; a payment
below the interest makes the principal **grow**, the exact case the old model could not represent
at all. `POST`/`PATCH /v1/liabilities` derive the outstanding principal differently per model when
`derive_principal_from_plan` is set — `Σ cuotas` for `fixed_payments`, the annuity's present value
for `french` (engine-exported `present_value_of_payments`). Migration
`20260825120000_liabilities_repayment_model.sql` adds the column, `NOT NULL DEFAULT
'fixed_payments'`, no data loss; `.ffbackup schema_version` unaffected (already 10). Engine unit
tests: `fixed_payments_with_apr_is_bit_identical_to_the_pre_4_2_0_pin` (bit-exact regression pin),
`french_two_months_hand_checked`, `french_extinction_at_month_278`,
`french_payment_below_interest_grows_the_principal`,
`interest_only_principal_constant_and_cash_is_the_quota`,
`revolving_matches_french_recurrence`. MCP: `create_liability`/`update_liability` gained
`repayment_model` — the catalog stayed at **52 tools** at the time (a field on an already-covered
resource, not a new one; it reached 68 in the 4.4.0 train's Fase 6).

**Extended, not reopened, in 4.4.0 (Fase 6, issue #87)** — the accrual shipped in 4.2.0 but was
**invisible**: the engine derived every liability's closing principal up to 840 times per request
and `ProjectionOutput` never published it, so «¿cuánto interés pago?» and «¿cuándo termino?» had no
answer. `liability_amortization_schedule` (engine) + `GET /v1/liabilities/{id}/schedule` (handler)
now serve it, and `simulate_projection` gained `liability_overrides` (extra monthly principal and
lump sums) so «¿me compensa amortizar antes?» is answerable at all — until then the 12 what-if axes
touched **no liability**. Zero new math: both reuse `liability_month`, and the interest is derived
as a **residual** from the balances (`payment − (opening − closing)`), which is what makes
`payment + extra == interest + principal` exact by construction in all four models. Cross-surface
pin: `simulate_liability_kpis.rs::the_what_if_debt_kpis_agree_with_the_liability_schedule`.

**Why this counts as fully closed, not partly:** opt-in (existing liabilities keep
`fixed_payments`, bit-identical to pre-4.2.0 — the exact "you have a result when" bar this item
originally set), a closed-form ground truth checked by hand
(`french_extinction_at_month_278`, `french_two_months_hand_checked`), and the payment-below-interest
case this item flagged as untestable before is now both representable and tested. What remains
named-but-out-of-scope is not unfinished business from this item: a liability without an active
payment plan simply does not accrue (documented behavior), and `revolving` sharing `french`'s math
is a tracked, tested simplification for a future release, not a gap in this one.

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

### 10. MCP catalog context budget: descriptions are done, `inputSchema` is next

**Why it belongs on axis 3:** for a self-hosted household operated (in part) through Claude, the
MCP catalog IS the product's AI-native interface — every tool description and every parameter
description inside `tools/list` travels in full into any conversation that touches `/mcp`,
whether or not a single tool ends up called. That is a real, ongoing product cost, the same way a
slow endpoint or an unreadable error is: it degrades the app for its AI-agent users specifically.

Fase 5 (issue #86) proved the pattern that fixes this — move what can be checked in the RESPONSE
out of prose that gets paid every turn — and applied it to tool-level descriptions:
**37.214 → 21.319 characters (−42,7 %)**, every tool now ≤ 600 chars (before, 26 tools exceeded
600, worst at 3.821 — re-derive this pre-Fase-5 figure yourself before quoting it; a number
already circulating in this repo's own CHANGELOG for it, 12, does not match a direct count of the
pre-Fase-5 source). Gate:
`apps/api/tests/mcp_http.rs::tool_descriptions_stay_within_the_context_budget`
(`PER_TOOL_MAX = 600`, `TOTAL_BUDGET = 24_000`) — its failure message is explicit: do not raise
the constant, move the overflow to a response provenance field or to the server's `instructions`.

**The confession that makes this credible — read before pitching a shortcut here:** phases 1–4 of
this same effort made the description number WORSE, not better — **27,280 raw literal chars at
`v4.3.1` → 37,228 at `main`** (measure any revision with
`git show <rev>:apps/api/src/mcp/server.rs | python3 -c "import re,sys;d=re.findall(r'description = \"((?:[^\"\\\\]|\\\\.)*)\",', sys.stdin.read());print(len(d), sum(map(len,d)))"`)
— because every fixed silent-wrong-number incident bolted its own warning onto some tool's prose.
That one-liner counts the **raw** string literal, escapes included, so it runs a few characters
above the authoritative figure (`description_len` in `apps/api/tests/fixtures/mcp-catalog.json`,
which the guard test uses: 37,214 at `main`, 21,319 now). Use it for the trend across revisions,
the fixture for the number of record. The
generalizable lesson, and the one this item exists to keep applying: **an MCP server is not
defended by prose repeated every turn — it is defended by provenance fields in the response that
tell the model where a figure came from at the moment it is looking at that figure** (the
`*_basis`, `*_absent_reason`, `source: capture|backfill` and `events` fields this same Fase 5
added are the pattern already applied once). Reach for a response field or an `instructions` line
before reaching for more words in a description — that instinct is what Fase 5 had to unlearn the
hard way, and it is exactly the instinct this item asks you to apply one layer deeper.

**What Fase 5 did NOT touch — the asset that makes this item tractable HERE:** measured with a
live `tools/list` AFTER the description cut, the `inputSchema` block of the then-52-tool catalog
was **~55 KB — roughly 2,7× the descriptions**. Prose stopped being the dominant cost; the lever
left is the **~250 parameter doc-comments** that `schemars` publishes as the `description` of
each JSON-Schema field (Fase 2, issue #83, already put the *structural* constraints — `enum`,
`pattern`, numeric bounds — into the schema itself; the doc-comment sitting on top of that is
often now pure duplication). Treat 55 KB as a one-time Fase-5 audit measurement, NOT a frozen
constant: nothing in the repo asserts it today, and the catalog fixture
(`apps/api/tests/fixtures/mcp-catalog.json`) deliberately stores only a canonicalized
`constraints` summary plus a description hash — never the full schema — precisely so its diff
stays readable (see the comment above `tool_signature` in `mcp_http.rs`). Re-derive both halves
against a live server instead of quoting these numbers going forward:

```bash
BASE=http://127.0.0.1:8080
curl -s -X POST "$BASE/mcp" \
  -H "Authorization: Bearer ffp_…" -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
| python3 -c '
import json, sys
d = json.load(sys.stdin)
tools = d["result"]["tools"]
schema_bytes = sum(len(json.dumps(t["inputSchema"])) for t in tools)
desc_bytes = sum(len(t.get("description", "")) for t in tools)
print("tools", len(tools), "schema_bytes", schema_bytes, "desc_bytes", desc_bytes)
'
```

The description half alone re-derives cheaply, no running server needed, straight from the
frozen fixture:

```bash
python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];l=[x['description_len'] for x in t];print(len(t),sum(l),max(l))"
```
→ today `68 23874 596` (`52 21319 596` right after Fase 5). **Fase 6 turned this item's headroom into work**: the 16 new tools took the raw total to 28.884, +4.884 over `TOTAL_BUDGET`, and the fix was the one the guard prescribes — provenance fields and `instructions`, never raising the constant. What is left is 126 characters, i.e. the description half is no longer self-financing either: the next tool pays for itself out of somebody else's prose.

**First three steps in this repo:**
1. Extend the pattern of `tool_descriptions_stay_within_the_context_budget` in `mcp_http.rs` with
   a throwaway, printed-not-asserted per-tool breakdown of `len(inputSchema)` vs
   `len(description)` (the curl recipe above, turned into a `#[tokio::test]`). The aggregate says
   the schema dominates; trimming needs to know WHICH tools carry the weight before touching any
   doc-comment.
2. From that breakdown, group by shared parameter TYPE rather than by tool. Request/response
   fragments reused across many tools (`view`, pagination cursors, `confirm`/`confirm_token`,
   `category_id`, date ranges…) surface as `$defs` entries in the JSON Schema and repeat the SAME
   doc-comment once per tool that references them. Counting occurrences per shared `$defs` entry
   shows where trimming one doc-comment saves bytes across the whole catalog, not just one tool —
   the same multiplication effect that made tool descriptions expensive, one layer down.
3. For each parameter doc-comment, classify it PAYLOAD (states a bound or enum the schema ALREADY
   declares structurally via Fase 2's `#[schemars(range/pattern/extend)]` — pure duplication,
   safe to shorten or drop) or PROSE (explains something the schema cannot express structurally —
   candidate for the server's `instructions` block if it is transversal to several tools, or for
   a response provenance field if it is checkable in the output — the exact move Fase 5 made for
   tool descriptions, not yet made for parameters).

**You have a result when:** a guard test analogous to
`tool_descriptions_stay_within_the_context_budget`, but over total `inputSchema` bytes, is green
in CI with a documented, deliberately-chosen budget, AND a first trimming pass (the top-N
duplicated `$defs` doc-comments identified in step 2) lands without any test in
`mcp_http.rs`/`mcp_write.rs` losing coverage — `constraints_sha256_12` may change (the prose
moved), the underlying contract (`enum`, `pattern`, bounds, `required`) must not. Until that guard
exists, do not describe the MCP catalog's context cost as solved anywhere public: the description
half is (cite `tool_descriptions_stay_within_the_context_budget`); the schema half — ~2,7× larger
— is not (claims table above).

---

## Observables — watched, not proposed

Things worth *noticing* that do not yet clear the bar for a ranked item (no evidence of need at
this scale). Listing them here keeps them from being re-discovered as "obvious wins".

- **Memory tuning of the embedded PostgreSQL** (`shared_buffers`, `work_mem`, `effective_cache_size`).
  Since 3.0.0 the postmaster runs inside the app container with PostgreSQL's stock defaults — the
  entrypoint sets only `listen_addresses`, `unix_socket_directories` and `logging_collector`. For a
  single-household ledger (thousands of rows, one user at a time, the heavy work being pure-Decimal
  CPU in the engine, not I/O) there is **no measured evidence that any of this is a bottleneck**.
  Do not tune blind: if it ever matters, measure first (futurefin-diagnostics-and-tooling), and
  remember every knob added to the entrypoint is more bash on the data path
  (futurefin-architecture-contract W8).

## Adding a new frontier item

1. Check `futurefin-failure-archaeology` — is it a settled rejection?
2. Tie it to one of the three axes; if it fits none, drop it.
3. Name the repo asset that makes it tractable HERE; write the first three steps with real
   paths and test names, and a falsifiable "result when…" (a milestone you cannot fail is not
   a milestone).
4. Add it to the ranked table with value/effort/risk and update this file (doc maintenance:
   futurefin-docs-and-writing).

## Provenance and maintenance

Facts verified 2026-07-02 against v1.4.3; item 2, the claims table and the counts re-verified
**2026-08-16 for v3.0.0** against `apps/api/docker-entrypoint.sh`, `apps/api/Dockerfile`,
`.github/workflows/ci.yml` and `scripts/`.

**Item 5 corrected 2026-08-28 (MCP Fase 4 doc sweep, issue #88) — it had shipped in 4.2.0
(2026-08-25) and this file still called it an open candidate.** Verified against
`CHANGELOG.md` §[4.2.0] and `crates/engine/src/projection.rs` (`RepaymentModel`, `apr_percent`,
the six tests named in the item). Lesson for this skill specifically: a "candidate" item is only
as fresh as the last time someone checked whether the campaign it points at already closed it —
`futurefin-projection-realism-campaign`/`futurefin-change-control` gate the *shipping*, but
nothing automatically un-lists a shipped item here. Re-verify before relying on them:

- Version: `grep -m1 '^version' apps/api/Cargo.toml` — **read `2.3.0` on 2026-08-16; the 3.0.0
  bump is part of the release gate and had not been applied yet** (futurefin-change-control §4).
- Item 2 shipped half (pre-migration backup):
  `grep -n 'premigration_backup\|migration-versions.txt\|refusing to start with pending migrations' apps/api/docker-entrypoint.sh`
  and `grep -n 'FUTUREFIN_BACKUP_KEEP\|FUTUREFIN_PREMIGRATION_BACKUP' apps/api/docker-entrypoint.sh`
  (defaults 10 / 90 days / `on`); CI assertion: `grep -n 'pre-migration-\|pre-pgupgrade-' .github/workflows/ci.yml`.
- Item 2 formerly-open half — **SHIPPED 2026-08-27** (branch `feat/home-assistant-addon`), so the
  old "must still be missing" check is inverted: `ls apps/api/tests/migration_guard.rs` → **must
  exist** (2 tests); `grep -n 'MigrationError::Downgrade\|VersionMissing\|NO ARRANCA' apps/api/src/db.rs`
  → must print the mapping and the operator banner; `ls apps/api/tests/*.rs | wc -l` (**62** on
  2026-08-28 after the whole 4.4.0 MCP-audit train; 43 on 2026-08-27; 20 on 2026-08-16; 8 on
  2026-07-02). Run it with
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test migration_guard`.
- Host-side backup/restore scripts (no `upgrade-with-backup.sh` — deliberately): `ls scripts/`.
- Embedded PG runs with stock memory settings (Observables): `grep -n -A9 '^start_postgres()' apps/api/docker-entrypoint.sh`
  → the postmaster gets only `listen_addresses`, `unix_socket_directories`, `logging_collector`
  (+ optional `log_min_messages`); no `shared_buffers`/`work_mem` anywhere.
- Engine purity (no RNG/IO deps): `grep -A8 '\[dependencies\]' crates/engine/Cargo.toml`
- No proptest yet: `grep -rn proptest crates/ apps/ --include=Cargo.toml` (empty = item 1 still open)
- Item 5 shipped (no longer a gap): `grep -n 'enum RepaymentModel\|apr_percent' crates/engine/src/projection.rs`
  (must hit; the old marker `principals[i] -= pay` is gone — `grep -n` for it now returns nothing,
  which is the expected, correct state, not drift) and
  `grep -n '^## \[4.2.0\]' CHANGELOG.md`.
- Drain is tax-free / gross-up only in target: `grep -n 'gross_up_net_annual_fire\|drain_from_assets' apps/api/src/handlers/projection.rs crates/engine/src/{projection,sim_core}.rs`
- Backup script defaults: `grep -n 'BACKUP_DIR=\|KEEP_BACKUPS=\|ENV_FILE=' scripts/backup-postgres.sh`
  (since 3.0.0 it `compose exec`s into the single `futurefin` service and `ENV_FILE` is optional)
- Migration runner (auto on startup, fails loud): `cat apps/api/src/db.rs`
- Parity fixture + consumers: `ls apps/api/tests/fixtures/fire-parity.json apps/api/tests/fire_parity.rs apps/web/src/lib/fire.test.ts`
- ~~CI covers engine tests but NOT Postgres integration tests~~ — **falso desde 4.0.0**: el job `integration` corre `cargo test --workspace --locked` contra un Postgres de servicio. Corregido en la Fase 7: `grep -n 'cargo test' .github/workflows/ci.yml`
  (since 3.0.0 the `docker-stack` job also exercises the container's data paths end-to-end:
  `grep -n '^      - name:' .github/workflows/ci.yml`)
- README still claim-free on MC/determinism: `grep -in 'monte\|stochastic\|bit-exact\|deterministic' README.md` (empty = good)
- Currency/locale state: `grep -n 'EUR.*USD.*GBP' apps/api/src/handlers/installation.rs; grep -n DISPLAY_NUMBER_LOCALE apps/web/src/lib/format.ts`
- Horizon basis strings: `grep -n '"lifespan_90"\|"fallback_no_demographics"\|"months_override"' apps/api/src/handlers/projection.rs`
- Migration count: `ls apps/api/migrations | wc -l` (34 as of 2026-08-16; 31 as of 2026-07-02)

**Item 10 added 2026-08-28 (MCP Fase 5 doc sweep, issue #86, branch `feat/mcp-fase-5-contexto`,
unreleased at verification time — `Cargo.toml` still 4.3.1).** Verified against
`apps/api/tests/mcp_http.rs`, `apps/api/tests/fixtures/mcp-catalog.json` and
`CHANGELOG.md` (unreleased Fase-5 entry). Re-verify before relying on either headline number:

- Description budget + guard: `grep -n 'PER_TOOL_MAX\|TOTAL_BUDGET' apps/api/tests/mcp_http.rs`
  (must show `600` / `24_000`); cheap re-derivation:
  `python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];l=[x['description_len'] for x in t];print(len(t),sum(l),max(l))"`
  (`68 23874 596` on 2026-08-28 after Fase 6 — **126 from the ceiling**; `52 21319 596` at the
  close of Fase 5, same day).
- `inputSchema` has NO guard yet — this absence IS the item, do not treat it as an oversight to
  silently "fix" outside this item's steps: `grep -rn 'schema_bytes\|inputSchema.*len' apps/api/tests/mcp_http.rs`
  (empty = still open). Re-derive the ~55 KB figure with the live-server curl recipe inside the
  item itself; it is not stored in any fixture on purpose (`apps/api/tests/fixtures/mcp-catalog.json`
  only carries `constraints` + a description hash — confirm with
  `grep -n '"constraints"\|"description_len"\|inputSchema' apps/api/tests/fixtures/mcp-catalog.json | head`).
- Fixture regeneration command (only if the catalog's contract itself changed, never to chase a
  budget failure): `grep -n 'UPDATE_MCP_CATALOG' apps/api/tests/mcp_http.rs`.
