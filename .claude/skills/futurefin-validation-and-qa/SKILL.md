---
name: futurefin-validation-and-qa
description: >
  Load this skill whenever you need to PROVE a FutureFin change is correct: running or adding
  tests, deciding what evidence a change needs before merge, writing backend integration tests
  (TestApp harness), engine unit tests, or frontend Vitest tests; regenerating or extending the
  fire-parity.json cross-language fixture; capturing regression values before a refactor; or
  answering "did CI cover this?" / "which tests must I run locally?". Symptom keywords: test
  fails only locally, cargo test hangs on TEST_DATABASE_URL, schema ff_test_* piling up, parity
  test fails on one side only, Decimal string "1000.0000" vs "1000" assertion mismatch, 146/156
  test count confusion. Do NOT use for: getting the app running or a
  dev environment (futurefin-build-and-env), measuring live behavior with curl/scripts and
  interpreting the numbers (futurefin-diagnostics-and-tooling), deciding whether a change is
  allowed at all (futurefin-change-control), or the FIRE math itself
  (futurefin-fire-domain-reference).
---

# FutureFin — Validation & QA

How to prove things in this repo: what counts as evidence, the exact test inventory and
harness, and how to add tests. Verified against the code on 2026-07-02 (v1.4.3); counts and the
history-snapshots test files refreshed for v1.5.0 on 2026-07-06.

Why this matters here: the hardest live problem in FutureFin is **projection correctness** —
errors are silent (numbers look plausible but wrong). Eyeballing a chart is never acceptance.

## 1. Evidence standards — what counts as proof

| Situation | Required evidence | Not acceptable |
|---|---|---|
| Refactor that must not change output | **Bit-exact regression capture**: write the test asserting the value FIRST (it fails or you print the actual), run against pre-refactor code, commit the captured expected value, then refactor until green. Example: `apps/api/tests/projection_marker.rs` (captured `compound_outpaces_true_savings_month_index == Some(1)` before the spawn_blocking/tokio::join perf refactor). | "The chart looks the same" |
| Model/behavior change (engine math, FIRE formula) | **Predict-then-measure**: write down the expected number (hand calculation, `python3`, or independent derivation) BEFORE running, then assert against it. Full discipline: `.claude/skills/futurefin-research-methodology/SKILL.md`. | Running first and asserting whatever came out |
| Logic duplicated client & server (FIRE target) | **Parity fixture**: one canonical JSON both suites consume (`apps/api/tests/fixtures/fire-parity.json`). A failure on one side only = drift. | Updating one side and re-deriving the other "by inspection" |
| Bug fix | One test that fails on the old code and passes on the new. One test per surprising behavior — no "while we're here" assertion bundles. | Fix without a pinning test |
| Comparing money values in tests | API serializes `Decimal` as strings like `"1000.0000"`, never `"1000"`. Parse to `f64` and compare with tolerance: `let v: f64 = body["x"].as_str().unwrap().parse().unwrap(); assert!((v - 1000.0).abs() < 0.01)`. Or `starts_with("15000")` for coarse checks. | `assert_eq!(body["x"], "1000")` — will fail on scale |
| Visual change | Verify light AND dark theme manually (`<html data-theme>`); there are no rendering tests. | Checking one theme |

Jargon used below, defined once: **SWR** = safe withdrawal rate (annual % of net worth
withdrawn in retirement); **gross-up** = inflating a net annual need to the pre-tax gross
amount using progressive tax brackets; **installation** = the singleton row all data belongs
to; **cascade** = the ordered allocation-rules pipeline distributing monthly surplus to assets.

## 2. Test inventory (as of 2026-07-06, v1.5.0)

Three suites. None share infrastructure; run all three before merging.

| Suite | Location | Needs | Command (from repo root) |
|---|---|---|---|
| Engine unit tests (43) | `crates/engine/src/{projection.rs (22), history.rs (21)}` `mod tests` | Nothing (pure `Decimal` math, no I/O) | `cargo test -p futurefin-engine` |
| Backend integration (109 tests, 17 files, as of 2026-07-08) | `apps/api/tests/*.rs` | Postgres reachable via `TEST_DATABASE_URL` | See below |
| Frontend Vitest (137) | `apps/web/src/**/*.test.ts` | Node only (`environment: "node"`, no jsdom) | `npm test --workspace futurefin-web` |

Plus API lib unit tests run by `cargo test --workspace` (no Postgres): notably
`apps/api/src/handlers/backup_user/schema.rs` `mod tests` (10; 2 added in v1.6.0 for `.ffbackup` v5
and 2 in v1.8.0 for v6 migration/round-trip) and the `handlers/transactions/` unit tests (CSV presets,
fingerprint/ordinal, rule precedence).

### Backend integration — full invocation (verbatim)

```bash
# One-time: dedicated test DB on port 5433 (avoids clashing with dev on 5432)
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine

# Run everything (engine + integration + api unit):
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

If `TEST_DATABASE_URL` is unset, tests default to that exact URL
(`apps/api/tests/common/mod.rs::test_database_url`). If no Postgres is listening there, every
integration test panics at "connect to TEST_DATABASE_URL" — that is the "hangs/fails without
DB" symptom. Single test: append `-- <test_fn_name>` or `--test <file_stem>`.

### `isolated_pool()` mechanics (read before touching the harness)

`apps/api/tests/common/mod.rs::isolated_pool()`:
1. Creates schema `ff_test_<uuid-simple>` in the test DB.
2. Opens a pool (max 5 conns) with `after_connect` hook: `SET search_path TO "<schema>", public`
   on every connection — so all queries in the test hit only that schema.
3. Runs `sqlx::migrate!("./migrations")` inside it (32 migration files as of 2026-07-06 —
   count with `ls apps/api/migrations | wc -l`).
4. Returns `(PgPool, schema_name)`. **Schemas leak intentionally** — no teardown, so a failed
   test leaves its state inspectable.

Cleanup when they pile up (note: the `make clean-test-schemas` / `scripts/clean-test-schemas.sh`
mentioned in the `common/mod.rs` doc comment do NOT exist as of 2026-07-02 — clean manually):

```bash
# Nuke everything: wipe the container
docker rm -f ff-test-db   # then re-run the docker run one-liner above

# Or drop schemas surgically:
docker exec ff-test-db psql -U futurefin -d futurefin_test -tAc \
  "SELECT 'DROP SCHEMA ' || quote_ident(nspname) || ' CASCADE;' FROM pg_namespace WHERE nspname LIKE 'ff_test_%'" \
  | docker exec -i ff-test-db psql -U futurefin -d futurefin_test
```

### Integration test files (all 11)

| File | Tests | Covers |
|---|---|---|
| `smoke.rs` | 5 | health/ready, 401 unauth, register→login→me roundtrip, first-user bootstrap → owner |
| `liabilities_purge.rs` | 2 | expired liabilities hidden from GET/summary but **persist in DB** (reads never mutate) |
| `body_limits.rs` | 2 | 1 MiB global body cap → 413; `/backup/user-import` accepts up to 16 MiB |
| `installation_patch.rs` | 3 | unknown `fire_number_mode` rejected; legacy `annual_expense_adjusted` alias accepted; valid mode change |
| `unique_violation.rs` | 2 | duplicate username / duplicate category name → 409 via central `From<sqlx::Error>` |
| `projection_marker.rs` | 1 | regression capture: stable marker + starting NW across the perf refactor (the template for capture-first) |
| `fire_parity.rs` | 1 (×6 fixture cases) | server `jubilacion_target_net_worth` matches `fire-parity.json` ± 1 € |
| `projection_cache.rs` | 5 | cache hit faster than miss + identical body; invalidation on mutation; logout drops only `view=mine` entries; `density=hybrid` decimation (months 0–12 monthly, then 24,36,48…); monthly/hybrid cached as separate keys |
| `history_snapshots.rs` | 20 | snapshot capture (copied terms) / same-day upsert / exclude shared+expired / backfill CRUD roundtrip with `year` filter + cascade / 400 validations (future, `duplicate_item_id`, terms-on-asset) / 409 date taken / 404 cross-user / 403 viewer on every mutation / GET never mutates / `snapshot_mutations_do_not_touch_projection_cache` (cache stays HIT — history is NOT a projection input) |
| `history_series.rs` | 7 | `GET /v1/history/series`: empty→200, exact linear interpolation between two asset snapshots, join to live values (deleted asset→0 at k=0), amortization curve above the chord with exact endpoints, household sums two users + `?view=mine` filters, markers carry date/kind/total, single today snapshot. Numbers predicted before running |
| `backup_user_roundtrip.rs` | 11 | `.ffbackup` v4/v5/v6: roundtrip with identical history series, item re-link to fresh asset UUIDs, null `ledger_index` keeps `item_key`, v3 still imports (0 snapshots), out-of-range index → 400 + rollback, import invalidates projection cache (pre-existing bug fix), preview reports snapshot/item counts, viewer 403; plus v5 (v1.6.0) transactions/imports/rules round-trip with index re-link and preserved `fingerprint_ordinal`; plus v6 (v1.8.0) `recurring_transaction_rules` round-trip with `recurring_rule_index` re-link and preserved `last_materialized_month` |
| `transactions_import.rs` | 15 | CSV import: MyInvestor/N26 header autodetection, preview flags `already_imported` (omitted by default), confirm inserts with ordinals, same-file re-confirm → 0 new, `force` appends a fresh ordinal, internal-transfer heuristic, learned rule pre-assigns on next preview, non-EUR rejected on confirm, viewer 403, preview↔confirm sha mismatch → 400; **accent folding** (post-2.0.0): savings hint + learned-rule matching are diacritic-insensitive (`savings_hint_accent_insensitive_*`, `learned_rule_matches_accent_insensitive*`) |
| `transactions_crud.rs` | 14 | manual create/batch, `savings` requires NULL category, income/expense scope validation, **PATCH edits op_date/amount/concept on imported rows with the fingerprint anchored to the CSV** (`patch_imported_fields_editable_fingerprint_anchored`, ex `patch_imported_op_date_is_immutable`; no more `immutable_field`) while manuals recompute the fingerprint and free the ordinal (`patch_manual_op_date_recomputes_and_allows_reuse`), deleting linked asset/liability SET NULLs the link keeping the row, category delete remaps transactions, viewer 403 |
| `transactions_summary.rs` | 9 | exact Decimal per-category actual/budget/avg, **weighted average** (denominator = `months_with_data`, not the window width → short history no longer dilutes to 0), `avg_window` 3/6/12/`ytd`/`all` + legacy `avg_months` alias, invalid `avg_window` → 400, partial month flagged, savings excluded from expense, «Sin categoría» bucket. **No** derived-debt line anymore: `totals.expense_budget` = Σ expense-category budget |
| `transactions_projection_cache.rs` | 3 | mode-conditioned cache contract (`fire_settings.savings_source`): `mode_a_mutations_do_not_touch_projection_cache` (mode `budget`: cache stays HIT across import/create/edit/delete/rule writes + recurring endpoints — transactions not engine inputs), `mode_b_each_mutation_invalidates_projection_cache` (mode `transactions_avg`: every mutation invalidates), `flipping_savings_source_invalidates_projection_cache` (switching mode invalidates) |
| `transactions_recurring.rs` | 16 | recurring rules (v1.8.0): `recurrence` create makes rule + linked origin instance, idempotent `materialize` (2nd call → 0), never a future `op_date` (current month only once its day arrived), `day_of_month` clamped to month end, deleting an instance is not recreated on re-materialize (cursor), rule `DELETE` keeps instances (SET NULL), viewer 403 on materialize/delete, out-of-range `recurrence.day_of_month` → 400; **create-time backfill** (post-2.0.0): `create_with_past_date_backfills_instances` (past date fills intermediate months in the same commit), `recurrence_op_date_within_bound_created`, and the 10-year bound `recurrence_op_date_too_old_*` → 422 `recurrence_too_old` |
| `history_cashflow.rs` | 5 | `GET /v1/history/cashflow`: exact monthly aggregates (Decimal-string, household + mine), fine series passes through snapshots, `/v1/history/series` byte-identical with and without transactions (tier-1 regression), `daily` with window >6m → 400, `fine` absent without links |

### Frontend Vitest files (no congeles el total aquí — cuéntalo con `npm test --workspace futurefin-web 2>&1 | grep Tests`; 277 a 2026-07-11)

Config: `apps/web/vitest.config.ts` — `environment: "node"`, `include: ["src/**/*.test.{ts,tsx}"]`,
`globals: false` (import `describe/it/expect` from `vitest` explicitly).

| File | Tests | Covers |
|---|---|---|
| `apps/web/src/lib/format.test.ts` | 29 | es-ES Intl formatting, null/NaN/empty edges, Decimal string preservation |
| `apps/web/src/lib/dates.test.ts` | 29 | civil calendar (leap years, day clamping, age around birthday), TZ fallback, payment intervals, negative `addMonthsCivil` deltas (v1.5.0) |
| `apps/web/src/api/client.test.ts` | 10 | fetch mocks: credentials, body serialization, 4xx propagation, 204 handling |
| `apps/web/src/lib/fire.test.ts` | 7 | FIRE parity vs the shared fixture (1 sanity + 6 cases) |
| `apps/web/src/lib/history-merge.test.ts` | 11 | `mergeProjectionWithHistory`: identity-by-reference (null/empty/anchor-mismatch → byte-identical render), drops `month_index ≥ 0`, asset-series union by id, future offset |
| `apps/web/src/lib/projection-chart.test.ts` | 10 | `deflationFactorAt` (0 / ±12 / inflation 0) + tick-builders with `startMonth=-24` and the `startMonth=0` regression (identical to prior behavior) |
| `apps/web/src/lib/snapshot-tracker.test.ts` | 8 | `liquidCoverageComplete` (empty→false, full coverage→true, stale after `pruneEditLog`→false, new asset within the window) |

## 3. CI reality

`.github/workflows/ci.yml` runs on push/PR to `main` and `dev`. Verified against the file on
2026-07-02:

**CI DOES run** (three jobs):
- `rust`: `cargo build -p futurefin-api --locked` + `cargo test -p futurefin-engine --locked`
- `web`: `npm install`, `npm run typecheck:web`, `npm run build:web`
- `docker-stack`: builds the API image, `docker compose up`, polls `GET /v1/health` (90×2 s)

**CI does NOT run** — verified absent from `ci.yml`:
- Backend integration tests (`apps/api/tests/`) — no Postgres service, no `TEST_DATABASE_URL`
- Frontend Vitest (`npm test`) — the web job only typechecks and builds
- ESLint (`npm run lint:web`)

**Therefore, your pre-merge local obligation list** (green CI is NOT enough):

```bash
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace                 # engine + integration
npm test --workspace futurefin-web       # Vitest
npm run lint:web                         # eslint
npm run typecheck:web                    # (CI also runs this, run anyway — it's fast)
```

Before tagging a release, additionally run the full local Docker-stack test (CLAUDE.md § "Test
local con Docker Desktop"). Release gates live in
`.claude/skills/futurefin-change-control/SKILL.md`.

## 4. Golden / certified inventory

### `apps/api/tests/fixtures/fire-parity.json` — the canonical cross-language fixture

The FIRE target math is **deliberately duplicated**: the client (`apps/web/src/lib/fire.ts`)
computes a live form preview without a round-trip; the server (`/v1/projection/series`) is the
source of truth. One JSON pins both:

- Backend consumer: `apps/api/tests/fire_parity.rs` — for each case, PATCHes
  `fire_settings` on the installation, seeds an asset + budget entries reproducing `monthly`,
  calls `GET /v1/projection/series`, asserts `jubilacion_target_net_worth` ≈
  `expected_target_nw` ± 1 € (`null` must match `null`).
- Frontend consumer: `apps/web/src/lib/fire.test.ts` — loads the same file via
  `readFileSync` (relative path `../../../api/tests/fixtures/fire-parity.json`), computes
  `grossUpNetAnnualFire(computeFireAnnualNeedNetEur(...)) / (swr/100)`, same ± 1 € tolerance.

Formula pinned (from the fixture's `_formula`): `target_nw = gross_up(annual_need_net,
brackets, taxes_enabled) / (swr_pct / 100)`, where `annual_need_net` depends on
`fire_number_mode` (`manual` / `annual_expense` / `current_income`). 6 cases as of 2026-07-02,
covering all three modes, taxes on/off, multi-bracket gross-up, and the null-target case.

**Discipline:**
- If you change `tax_brackets` defaults, the gross-up formula, or the target contract on
  EITHER side → regenerate every `expected_target_nw` from an independent reference
  (`python3` hand calc or the Rust engine), then **both suites must pass**. One suite failing
  = drift; find which side moved.
- Every case carries a `_calc_note` documenting how its expected value was derived (e.g.
  `"500000 / 0.035 (sin taxes)"`). **Never commit a case without one** — it is the audit trail.
- **Adding a case**: append to `cases[]` with `name`, `fire_settings`, `monthly`
  (`income`/`income_retirement`/`expense_retirement`, all decimal strings),
  `expected_target_nw` (number or `null`), `_calc_note`. Re-run both suites.
- Historical motivation: the client once passed `expense_regular_monthly_equivalent` where
  the server used `expense_retirement_monthly_equivalent` → 2–3× preview divergence, found
  during the v1.3.0 refactor. The fixture exists so that class of drift fails a test.

### `projection_marker.rs` — the regression-capture exemplar

Deterministic setup (100 k asset at 15 % TAE, +100 €/month net savings) with hand-derivable
expectations: the compound-outpaces-savings marker at `month_index = 1`, 25 points for a
24-month horizon, NW at month 12 in a justified range. Copy this pattern whenever a refactor
must preserve outputs: seed deterministic state → assert exact/current values → refactor →
values must not move.

### History series: server-computed, NOT duplicated on the client (no parity fixture — yet)

Unlike the FIRE target (deliberately duplicated Rust↔TS, held by `fire-parity.json`), the
historical net-worth interpolation lives **only** in `crates/engine/src/history.rs` and the
`GET /v1/history/series` handler. The server returns the series **ready to paint**; the client
(`lib/history-merge.ts`) merely splices those points onto the projection's month-0 vertex and
does **no** interpolation of its own. Consequences for testing:
- The interpolation math is proven by engine unit tests (`history.rs`, exact `Decimal`) plus the
  `history_series.rs` integration tests (predict-then-measure numbers). There is **no**
  cross-language fixture because there is no second implementation to keep in sync.
- **Rule**: if a client-side interpolation preview is ever added (e.g. to redraw the past while a
  snapshot save is in flight), it becomes a duplicated computation → a parity fixture of the
  `fire-parity.json` kind (one canonical JSON both sides consume, ±1 € tolerance) becomes
  **mandatory**, and the D8/§2.5 drift discipline applies to it.

## 5. How to add tests

### Backend integration test

Create `apps/api/tests/my_feature.rs`. Helper names verified against
`apps/api/tests/common/mod.rs` on 2026-07-02:

```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn my_endpoint_does_x() {
    let app = TestApp::spawn().await;                       // fresh schema + router
    let owner = app.register_and_login_owner("alice").await; // first user → owner (bootstrap)

    // arrange via the API, not raw SQL
    let cat = app.create_category(&owner, "asset", "Cash").await; // scope: asset|income|expense

    // act
    let resp = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "EUR", "current_value": "1000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;

    // assert
    assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
    let body = resp.json();
    assert_eq!(body["name"], "EUR");
    let v: f64 = body["current_value"].as_str().unwrap().parse().unwrap();
    assert!((v - 1000.0).abs() < 0.01);                     // NOT assert_eq on the string
}
```

Available on `TestApp` (all in `common/mod.rs`): `spawn()` → `{router, pool, schema, state}`;
`register_and_login_owner(name)` → `LoggedInOwner {username, cookie}`;
`create_category(&owner, scope, name)` → id string; `count_rows(table)` → i64 (direct DB
check — how `liabilities_purge.rs` proves rows persist); `get`, `get_with_cookie`,
`post_json`, `post_json_with_cookie`, `patch_json_with_cookie`, `delete_with_cookie` → all
return `ResponseParts {status, headers, body}` with `.json()` and `.session_cookie()`.
`app.state` exposes `AppState` internals (e.g. `projection_cache`) — see `projection_cache.rs`
for asserting cache keys directly. Mutation-triggered background work runs in `tokio::spawn`;
sleep ~100 ms before asserting its effects (pattern in `projection_cache.rs`).

### Engine unit test

Add to `mod tests` in `crates/engine/src/projection.rs`. Use the existing builders
`mk_asset`, `rule_fixed`, `rule_percent`, `rule_remainder`, `base_input` — do not
hand-construct `ProjectionInput`. Assert exact `Decimal` values (pure math, no tolerance
needed) and derive them by hand in a comment first (predict-then-measure). Run:
`cargo test -p futurefin-engine -- <name>`.

### Frontend test

Colocate beside the module: `lib/foo.ts` ↔ `lib/foo.test.ts` (Vitest `include` picks up
`src/**/*.test.{ts,tsx}`). Import `describe/it/expect` from `vitest` (`globals: false`).
Stub network with `vi.stubGlobal("fetch", ...)`-style mocks as in `api/client.test.ts`.

**What NOT to test on the frontend**: component rendering. There is no jsdom/happy-dom
configured — `environment: "node"` only. The config comment says: if component render tests
are ever added, switch to `happy-dom` or `jsdom` in `apps/web/vitest.config.ts`. Until then,
test pure functions only; extract logic out of components to make it testable.

## 6. Coverage gaps — be honest (as of 2026-07-02)

- **No E2E browser tests.** Nothing drives the real SPA; auth-flow + UI regressions are
  caught only manually. The docker-stack CI job proves the server boots, not that the UI works.
- **Integration tests not in CI.** A PR can go green with every `apps/api/tests/` test broken.
  This is the biggest gap; until fixed, the local obligation list in § 3 is mandatory.
- **No property-based tests** on the engine (e.g. invariants like "cascade never allocates
  more than the surplus", "NW series is deterministic under input permutation"). Labeled a
  candidate direction — see `.claude/skills/futurefin-research-frontier/SKILL.md`.
- **No load/performance tests.** The projection-cache tests assert relative hit/miss speed
  only; there is no throughput or memory baseline.

## When NOT to use this skill

- Getting the app or a dev/test environment running from scratch (Docker, `.env`, split-dev):
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- Measuring live behavior (curl recipes, `scripts/smoke-projection-cache.sh`) and interpreting
  the numbers: `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.
- Deciding whether/how a change may proceed (migrations, releases, gates):
  `.claude/skills/futurefin-change-control/SKILL.md`.
- Understanding the FIRE/retirement math being tested:
  `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- The hunch→accepted-result research discipline (evidence bar, predict-then-run):
  `.claude/skills/futurefin-research-methodology/SKILL.md`.
- Deploy/upgrade/backup smoke tests in production: `.claude/skills/futurefin-run-and-operate/SKILL.md`.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`); `.claude/tests.md` was corrected
the same day (CI claim, migration count, missing `projection_cache.rs` row). Re-verify volatile
facts with:

- Test file inventory: `ls apps/api/tests/` and `ls apps/web/src/lib/*.test.ts apps/web/src/api/*.test.ts`
- Engine test count: `grep -c "#\[test\]" crates/engine/src/projection.rs crates/engine/src/history.rs` (22 + 21 = 43)
- Integration test count: `grep -c "#\[tokio::test\]" apps/api/tests/*.rs` (90 total across 16 files)
- Migration count: `ls apps/api/migrations | wc -l` (32)
- CI coverage claims: read `.github/workflows/ci.yml` (jobs: rust, web, docker-stack; grep it
  for `TEST_DATABASE_URL` — absent means integration tests still not in CI; grep for
  `npm test`/`vitest` — absent means Vitest still not in CI)
- TestApp helper names: `grep -n "pub async fn\|pub fn" apps/api/tests/common/mod.rs`
- Vitest env: `grep -n environment apps/web/vitest.config.ts` (still `"node"`?)
- Fixture case count: `grep -c '"name"' apps/api/tests/fixtures/fire-parity.json` (6 cases)
- Cleanup script existence: `ls scripts/` (clean-test-schemas.sh did NOT exist as of 2026-07-02)
- Default TEST_DATABASE_URL: `grep -n "5433" apps/api/tests/common/mod.rs`
