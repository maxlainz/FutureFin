---
name: futurefin-architecture-contract
description: >
  The load-bearing design decisions of FutureFin, WHY each exists (with the incident that forged
  it), the invariants that must hold, and the known weak points. Load this skill BEFORE any change
  that touches architecture: adding/moving a route or handler, touching money serialization
  (Decimal vs f64), sessions/auth/cookies, the installation singleton, view scoping (?view=mine),
  the projection cache (TTL/keys/invalidation/warm-up), allocation-rule semantics (remainder/sink),
  the engine/handler boundary, migrations, or error mapping. Also load it when you are ABOUT to
  violate something without knowing it — symptoms: "why is this amount a string?", "why f64 here?",
  "can a GET delete expired rows?", "why not JWT?", "why does the cache not warm up after a
  mutation?", "is view=mine a security boundary?", "where does the projection horizon come from?".
  NOT for: how to build/run the dev environment (use futurefin-build-and-env), FIRE math details
  like SWR/gross-up/cascade formulas (use futurefin-fire-domain-reference), step-by-step change
  gates (futurefin-change-control), or debugging a live symptom (futurefin-debugging-playbook).
---

# FutureFin Architecture Contract

Facts date-stamped **as of 2026-07-02, v1.4.3** (`apps/api/Cargo.toml`); D12 (historical
snapshots) and the migration/backup-schema counts were added/refreshed for **v1.5.0** on
2026-07-06. This is the contract a retiring principal engineer would make you sign: the decisions
below are settled, most of them by a documented incident. Do not re-litigate them casually; if you must change one, go through
`.claude/skills/futurefin-change-control/SKILL.md`.

Vocabulary used throughout: **installation** = the single household deployment (one DB row owns
all data). **Scope** = the row-set a query sees (household vs mine). **FIRE** = Financial
Independence / Retire Early; the app projects net worth until it crosses a "FIRE target".
**SWR** = safe withdrawal rate (% of net worth withdrawable per year). **Gross-up** = inflating a
net annual need to the pre-tax gross that yields it after per-bracket capital-gains tax.
**Cascade** = the ordered list of allocation rules that route each month's savings surplus
("sobrante") into assets. **Nominal** = euros of the future month; **real** = deflated to today's
purchasing power.

## When NOT to use this skill

- Setting up, building or running the app → `.claude/skills/futurefin-build-and-env/SKILL.md`.
- FIRE/retirement math as implemented (target modes, gross-up derivation, cascade mechanics,
  nominal-vs-real model) → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- Classifying a change and passing release/migration gates → `.claude/skills/futurefin-change-control/SKILL.md`.
- Triaging a live bug → `.claude/skills/futurefin-debugging-playbook/SKILL.md`; historical
  incidents in depth → `.claude/skills/futurefin-failure-archaeology/SKILL.md`.
- Env vars / query params / fire_settings axes → `.claude/skills/futurefin-config-and-flags/SKILL.md`.

## 1. System map

```
Cargo workspace (Cargo.toml: members = ["apps/api", "crates/domain", "crates/engine"])
├── crates/domain    futurefin-domain: UserId newtype over Uuid; re-exports Decimal + Uuid.
│                    Deps: rust_decimal (serde-with-str), serde, uuid. Nothing else.
├── crates/engine    futurefin-engine: pure projection + history-interpolation math
│                    (projection.rs, 1114 LOC incl. tests; history.rs — snapshot timelines).
│                    Deps: chrono (no default features), rust_decimal(+maths), serde, thiserror, uuid.
│                    Public API (crates/engine/src/lib.rs): project_net_worth_series,
│                    first_month_per_asset_contribution_nominals, fire_target_at_month_index,
│                    evaluate_timeline (+ history types/helpers),
│                    plus input/output types (ProjectionInput, SimAsset, AllocationRule, FireTarget…).
└── apps/api         futurefin-api: Axum server. lib.rs modules: auth, db, error, handlers,
                     openapi, routes, state. main.rs = bin (env loading, CORS, gzip, static SPA).

npm workspace
└── apps/web         futurefin-web: React 19 + TS + Vite SPA. App.tsx (3229 LOC) is the
                     composition root; lib/, api/, components/, views/ per .claude/frontend-structure.md.
```

Postgres 16 is the only store. The Docker image serves API + built SPA on one port via
`WEB_STATIC_ROOT` (`main.rs::web_static_root` → `ServeDir` fallback).

### The engine purity contract (load-bearing)

`crates/engine` is **pure**: no I/O, no DB, no async, no clock reads (the civil date arrives as
`ProjectionInput::ref_date`), no randomness, no `f64` in the math — only `rust_decimal::Decimal`.
Same input → bit-identical output (argued from purity — see the audit greps in
futurefin-proof-and-analysis-toolkit Recipe 6; a replay regression test pinning it is still an
unimplemented candidate, futurefin-research-frontier item 1). This is not a style preference;
three things depend on it:

1. **Testability**: 22+ engine unit tests run with `cargo test -p futurefin-engine` — no DB, runs
   in CI (`.github/workflows/ci.yml`, job `rust`). This is currently the only simulation-math
   safety net that CI executes (see weak points).
2. **Parity**: deterministic Decimal math is what makes the client/server FIRE parity fixture
   (decision D8) meaningful — a shared JSON of expected values can bind both implementations.
3. **The `spawn_blocking` boundary**: `handlers/projection.rs` runs the two CPU-bound simulations
   (main series + "compound outpaces savings" marker) inside `tokio::task::spawn_blocking`,
   joined with `tokio::join!`. That only works because the engine never awaits and never touches
   shared state. Adding async or I/O to the engine breaks the boundary and can stall the reactor.

If you need data inside a simulation, fetch it in the handler
(`build_installation_projection_input`) and pass it in as a value. Never the other way around.

## 2. Decision register

Each entry: **decision → why → precedent/incident → what breaks if violated**.

### D1. Installation singleton
One row in `installation` per deployment; every ledger row hangs off it. First registered user
bootstraps it and becomes owner (`bootstrap_installation_as_owner_if_empty`, wired from
`handlers/auth.rs`/`installation.rs`); later registrants are "pending" until approved.
`handlers/installation.rs::singleton_installation_id` literally resolves "the one installation".
**Why**: self-hosted single-household product; the singleton collapses authorization to one check
(`require_installation_member`) and makes every query scope trivial. **Breaks if violated**:
any code path that assumes "the installation" (backup import/export, membership, warm-up,
cache invalidation) silently misbehaves with >1 row. Multi-tenancy is a rewrite, not a patch.

### D2. `owner_user_id` + `?view=mine` is a display filter, NOT authorization
Ledger rows carry `owner_user_id`; `?view=mine` filters to the session user's rows. Any member
can always request `household` and see everything. The authorization boundary is installation
membership + role (`role_can_write` in `handlers/membership.rs`), nothing finer.
Handlers MUST build both branches via `LedgerView::scope_where(alias)` + `next_arg_index()` +
`bind_scope_as/_scalar` (`handlers/person_view.rs`) — never hand-written `match view` blocks.
**Incident**: v1.3.0 found a live inverted-binds bug in `budget.rs` where the two hand-written
branches bound placeholders in different orders; the helpers exist to kill that bug class.
**Breaks if violated**: hand-rolled branches reintroduce placeholder-ordering bugs (wrong data
returned, silently); treating `mine` as a privacy/security boundary is a design error — it is not
enforced server-side beyond filtering.

### D3. Sessions in the DB, not JWT
`ff_session` cookie = a UUID (HttpOnly, SameSite=Lax, Secure per `COOKIE_SECURE`); the row lives
in `sessions` with `expires_at`; `require_session_user` (`handlers/session.rs`) joins
`sessions → users` and checks `expires_at > now()` on every request. TTL default 30 days
(`SESSION_TTL_DAYS`, accepted range 1–400 in `main.rs`; out-of-range or unparseable values are
NOT clamped — they silently fall back to 30 via `.filter(...).unwrap_or(30)`). **Why**: instant revocation (logout deletes the
row; owner can nuke sessions), zero signing-key management, and on a single-node self-host the DB
round-trip is cheap. **Breaks if violated**: switching to stateless JWT makes logout and
pending-user demotion non-immediate and adds key rotation surface for no benefit at this scale.

### D4. Money is `Decimal` end-to-end — with ONE deliberate `f64` exception
Domain/schema/engine: `rust_decimal::Decimal`, never `f64` (see `crates/domain/src/lib.rs` header).
API serializes amounts as decimal **strings** (`rust_decimal::serde::str`); the frontend does
arithmetic via `parseDisplayDecimal`-style helpers, never `parseFloat` on money.
**Exception (v1.4.0; extended 2026-07-06)**: the large parallel arrays in `GET /v1/projection/series` —
`points[].net_worth`, `points[].contributed_capital`, `fire_target_series`,
`asset_series[].values` — AND the per-point arrays of `GET /v1/history/series`
(`points[].net_worth/assets_total/liabilities_total`, `asset_series[].values`,
`markers[].total`) serialize as `f64` via `serialize_decimal_as_f64` — ONE `pub(crate)`
definition in `handlers/projection.rs`, consumed only by projection and history
responses. Documented in `.claude/api-routes.md`: "**`net_worth` y
`contributed_capital` se serializan como `f64`** (no Decimal-as-string) por rendimiento: ~30 KB
menos en JSON y evita ~5.000 `parseDisplayDecimal` cliente. Precisión <1 € en horizontes de 70
años." Scalars/KPIs (`starting_net_worth`, `jubilacion_target_net_worth`, milestone targets)
stay Decimal-as-string. **Breaks if violated**: `f64` upstream (engine, DB, KPI fields) causes
silent cent drift that compounds over 840 months; conversely, re-stringifying the big arrays
regresses wire size and client parse cost for zero precision benefit at display resolution.

### D5. Reads never mutate
Liabilities whose `payment_end_date < today` are **filtered** out of GET `/v1/liabilities`,
`/summary`, `/budget` (derived lines), `/assets`, `/projection` — never deleted.
**Incident**: until v1.3.0, `purge_expired_liabilities` made GET handlers silently issue
`DELETE`s — violating HTTP semantics, breaking cacheability, and destroying data as a side effect
of looking at it. It was removed, not fixed. **Breaks if violated**: any mutation inside a GET
reintroduces the class; `apps/api/tests/liabilities_purge.rs` guards this (local-only, see W1).

### D6. Migrations are embedded and fail loud
`sqlx::migrate!("./migrations")` runs at startup (`db.rs::run_migrations`). 32 files in
`apps/api/migrations/` as of 2026-07-06 (`ls apps/api/migrations | wc -l`; v1.5.0 added
`20260706203746_history_snapshots.sql`). There is **no
auto-repair**: a checksum mismatch aborts startup and must be fixed by hand
(`DELETE FROM _sqlx_migrations WHERE version = X` via psql, only if genuinely idempotent).
**Incident**: v1.3.0 deleted the old auto-repair loop — it masked drift. Related incident
v1.0.10: backup export 500 because SQL still selected columns a migration had dropped — queries
drift from schema; migrations must be followed by a grep for the dropped columns.
**Breaks if violated**: editing an already-shipped migration changes its checksum and bricks
every existing deployment's startup. Never edit shipped migrations; never drop user data without
owner sign-off (the v1.1.0 `allocation_rules` drop was explicitly signed off and documented).

### D7. Projection cache: in-memory, sliding TTL, invalidate-don't-warm
`state.rs`: `projection_cache: RwLock<HashMap<ProjectionCacheKey, ProjectionCacheEntry>>`.
- **Key**: `{installation_id, view, owner_user_id (Some only for view=mine), density}`.
- **TTL**: `PROJECTION_CACHE_TTL = 60 min`, **sliding** — refreshed on every hit; expired entries
  removed lazily on access.
- **Invalidation**: every mutating handler calls
  `refresh_projection_after_mutation` (`handlers/projection.rs`), which spawns
  `invalidate_projection_by_installation` — drops ALL entries for the installation (household +
  every member's mine). Logout drops that user's `mine` entries only.
- **Warm-up**: post-**login** only (`warm_up_household_projection`, both densities, household
  view, spawned; failures logged, never propagated).
- **Deliberately NO warm-up after mutation**: two consecutive mutations M1, M2 could spawn two
  concurrent warm-ups and M1's (computed on pre-M2 data) may finish after M2's, leaving the cache
  stale. The comment on `refresh_projection_after_mutation` documents this rejection. The first
  GET after a mutation eats one on-demand compute (~500 ms), then it's cached again.
- `?months=` override **bypasses the cache entirely** (computed, not stored).
**Breaks if violated**: adding post-mutation warm-up without a versioned/compare-and-swap scheme
reintroduces the stale-cache race; caching `months_override` responses explodes the key space.

### D8. FIRE math is duplicated client/server — held together by the parity fixture
Server source of truth: `handlers/projection.rs` (`compute_fire_target_nw`,
`gross_up_net_annual_fire` — closed-form per-bracket gross-up, which in v1.3.0 replaced a
90-iteration binary search). Client mirror for the live Settings→FIRE preview:
`apps/web/src/lib/fire.ts` (its header states the server stays source of truth).
One canonical fixture, `apps/api/tests/fixtures/fire-parity.json`, is consumed by BOTH
`apps/api/tests/fire_parity.rs` and `apps/web/src/lib/fire.test.ts`.
**Incident**: v1.3.0 found `RetirementView` feeding `expense_regular_monthly_equivalent` where
the server used `expense_retirement_monthly_equivalent` — a 2–3× FIRE-preview divergence.
Also: an engine/handler off-by-one (engine `years=(k-1)/12` vs handler `years=month_index/12`)
was fixed by making `fire_target_at_month_index` the single public helper both consume.
**Breaks if violated**: changing brackets/gross-up/SWR semantics on one side only. If you touch
them, regenerate the fixture's expected values and run BOTH suites — note the Rust side needs a
local `TEST_DATABASE_URL` (W1); CI will NOT catch server-side drift.

### D9. Error mapping is centralized in `From<sqlx::Error>`
`error.rs`: SQLSTATE 23505 (unique violation) → 409 `Conflict`; 23503 (FK violation) → 400
`BadRequest("referenced record missing")`; everything else → `Db(_)` → 500 with sanitized message
`"internal error"` (raw error only logged). Handlers just `?` any `sqlx::Error`.
**Breaks if violated**: per-handler manual mapping diverges status codes/bodies across endpoints
and risks leaking DB internals in responses. Add new mappings in `error.rs`, nowhere else.

### D10. Spanish UI, English code
UI copy and number formatting are es-ES (`apps/web/src/lib/format.ts`:
`DISPLAY_NUMBER_LOCALE = "es-ES"`; `1.234 €`, `3,5 %`); identifiers, API fields and route names
are English; docs and code comments are mixed ES/EN. Keep the project's Spanish product
vocabulary (Resumen, Jubilación, "sobrante", "Próximos") — it IS the vocabulary.
**Breaks if violated**: English user-facing strings or Spanish identifiers both read as defects.

### D11. Projection horizon: 90-year lifespan rule (`projection_target_age` is GONE)
Removed by migration `20260516120000_drop_projection_target_age.sql` (v1.0.6). The **FIRE
crossover is the sole retirement trigger** — the engine flips to retirement income/expense the
first month net worth ≥ the inflation-adjusted target (`fire_reached` in
`crates/engine/src/projection.rs`). Current horizon logic, verified in
`handlers/projection.rs::projection_horizon_months` + `compute_projection_series_response`:
- Resolve ONE birth date: session user's `users.birth_date`, else the first household person with
  a `birth_date` (`persons` ordered `is_primary DESC, sort_index ASC`).
- Horizon = `(90 − completed_age)` years, clamped to **[5, 70]**, × 12 months;
  `horizon_basis = "lifespan_90"`.
- No birth date anywhere → **30 years** (360 months), `horizon_basis = "fallback_no_demographics"`.
- Explicit `?months=` → clamped **12–840**, `horizon_basis = "months_override"`, uncached (D7).
(The old target-age model lingered in `.claude/data-model.md`, `.claude/engine.md` and the
`horizon_basis` doc comment in `projection.rs` until 2026-07-02 — all fixed since. If in doubt,
`projection_horizon_months` and its unit tests are the ground truth.)

### D12. Historical snapshots are per-user and are NOT a projection input (v1.5.0)
Each user manually captures net-worth **snapshots** (their asset + liability items) into
`history_snapshots` / `history_snapshot_items`; the engine (`crates/engine/src/history.rs`,
`evaluate_timeline`) interpolates the past net-worth series between them (linear for assets,
French-amortization for liabilities), served by `GET /v1/history/series` and spliced onto the
projection's month-0 vertex in the chart. Handlers live in `handlers/history.rs` under
`/v1/history`; the series fetch uses the standard `LedgerView` helpers (household = server-side
sum of every user's interpolated series). **Why it is a separate decision, not just another
ledger surface**: snapshots are **display history**, never inputs to `project_net_worth_series`.
Therefore, unlike every ledger mutation (D7), snapshot CRUD (`capture` / backfill `POST` / `PUT` /
`DELETE`) **must NOT call `refresh_projection_after_mutation`** — invalidating the projection
cache on a snapshot write would be pure waste (the projection does not depend on history) and was
deliberately omitted, with an explicit comment in the handler and a regression test
`apps/api/tests/history_snapshots.rs::snapshot_mutations_do_not_touch_projection_cache`
(projection stays cache-HIT across a snapshot mutation). Snapshots have **no cache of their own**
(sub-ms compute) and the series endpoint takes no `?months`/`?density`. Shared rows
(`owner_user_id IS NULL`) are never captured — a documented limitation. Snapshots are included in
`.ffbackup` **schema_version 4** (additive over v3; import re-links items to fresh asset/liability
UUIDs via `ledger_index`, else keeps `item_key`). **Breaks if violated**: wiring snapshot writes
into `refresh_projection_after_mutation` couples two independent subsystems and needlessly evicts
a hot projection cache; conversely, ever feeding snapshot data into the engine would make past
"observations" silently reshape the *future* projection — a category error.

## 3. Invariants table

| # | Invariant | Enforced where | How to check |
|---|-----------|----------------|--------------|
| I1 | Exactly one **uncapped `remainder`** allocation rule per scope, always **last** in the cascade (the "sink") | `handlers/allocation_rules.rs` create/patch/delete/reorder; API errors `remainder_required`, `uncapped_remainder_exists`, `sink_must_be_last` | `grep -n "remainder_required\|uncapped_remainder_exists\|sink_must_be_last" apps/api/src/handlers/allocation_rules.rs` |
| I2 | `fire_target_at_month_index` is the ONLY FIRE-target formula — engine crossover and API `fire_target_series` both call it | `crates/engine/src/projection.rs` (public fn + regression test for the old off-by-one) | `grep -rn "fire_target_at_month_index" crates/ apps/api/src/` — every inflation-compounding of a FIRE target must route through it |
| I3 | Amounts serialize as decimal strings, EXCEPT the documented f64 arrays of `/v1/projection/series` and the per-point arrays of `/v1/history/series` (D4) | ONE `pub(crate)` definition of `serialize_decimal_as_f64` in `handlers/projection.rs`, used by the projection and history responses only | `grep -rn "serialize_decimal_as_f64" apps/api/src/` (definition in projection.rs; uses only in projection.rs + history.rs) |
| I4 | All routes live under `/v1/`, except root `/health` and `/openapi.json` (plus the SPA static fallback when `WEB_STATIC_ROOT` is set) | `routes/mod.rs` (`nest("/v1", v1)`); note `/health` is ALSO mirrored at `/v1/health`, and `/v1/ready` exists | `grep -n "route\|nest" apps/api/src/routes/mod.rs` |
| I5 | Reads never mutate (D5): expired liabilities filtered, never deleted, by GETs | WHERE clauses in liabilities/summary/budget/assets/projection handlers | `TEST_DATABASE_URL=... cargo test --workspace liabilities_purge` (local; not in CI) |
| I6 | In charts, the stacked per-asset areas sum EXACTLY to the (visible) net-worth line at every x | `MiniProjection.tsx` rescales each asset share by `visibleNw × (asset_i / Σassets)` — necessary because raw engine `net_worth = Σassets + surplus_cash − Σprincipals − undrained`, so raw `per_asset_series` does NOT sum to NW | Read the `cumulative` block in `apps/web/src/components/charts/MiniProjection.tsx` (~lines 164–190); any new stacked chart must reuse `MiniProjection`, not re-derive |
| I7 | `planning_monthly_cash_adjustment.len() == horizon_months`; allocation `target_index` in bounds; horizon ≥ 1 | Engine input validation → `EngineError::{InvalidPlanningAdjustments, InvalidAllocationRuleTarget, InvalidHorizon}` → 400 | `cargo test -p futurefin-engine` |
| I8 | Engine has zero I/O/async deps (purity, §1) | `crates/engine/Cargo.toml` deps are exactly: chrono, rust_decimal, serde, thiserror, uuid | `grep -E "tokio\|sqlx\|reqwest\|axum" crates/engine/Cargo.toml` → must be empty |
| I9 | Milestones, `milestones_real` and the FIRE crossover are computed on the FULL monthly series, never on density-decimated points | `handlers/projection.rs` (`points_full`, crossover loop over `output.net_worth`) — v1.4.2 incident: client deflated by array index instead of `month_index`, wrong under `hybrid` | `grep -n "points_full" apps/api/src/handlers/projection.rs` |
| I10 | SQLSTATE→HTTP mapping only in `error.rs` (D9) | `impl From<sqlx::Error> for ApiError` | `grep -rn "23505\|23503" apps/api/src/ --include=*.rs` → only `error.rs` |
| I11 | Body limits: 1 MiB global, 16 MiB on `/v1/backup/user-import*` | `routes/mod.rs` constants | `apps/api/tests/body_limits.rs` (local) |
| I12 | No hardcoded hex colors; tokens `var(--ff-*)` only; icons only in `components/icons.tsx` | frontend convention (CLAUDE.md, design-system.md) | `grep -rn "#[0-9a-fA-F]\{6\}" apps/web/src/App.css apps/web/src/components/ \| grep -v icons.tsx` |

## 4. Known weak points (stated plainly, as of 2026-07-02)

- **W1 — Postgres integration tests are NOT in CI.** `.github/workflows/ci.yml` runs: cargo build
  of the API, `cargo test -p futurefin-engine`, npm typecheck+build, and a Docker-stack
  `/v1/health` smoke test. The `apps/api/tests/` suite (fire parity, cache, purge, body limits,
  unique violations — the tests guarding D5/D7/D8/I11) requires a local `TEST_DATABASE_URL`
  (see CLAUDE.md §Rust). A green CI does NOT mean those invariants held. Run them locally before
  any release.
- **W2 — `App.tsx` is still a 3229-LOC composition root** (down from 10,384 pre-v1.3.0, but still
  the single riskiest frontend file: auth gate, global state, route dispatch, two-phase projection
  loading all live there). Prefer extracting into `lib/`/`views/` per
  `.claude/frontend-structure.md` over growing it.
- **W3 — Single-installation assumption is baked in** (D1). `singleton_installation_id`,
  backup import, warm-up and cache invalidation all assume exactly one installation. Any
  multi-household ambition is a rewrite-class change.
- **W4 — EUR/Spain-centric defaults despite a `base_currency` column** (EUR/USD/GBP accepted).
  Default tax brackets are the Spanish IRPF savings scale (6.000/50.000/200.000/300.000 € at
  19/21/23/27/30% — `default_es_tax_brackets` in `handlers/installation.rs`, mirrored in
  `apps/web/src/lib/fire.ts`), and formatting is hardcoded `es-ES`. Non-Spanish deployments get
  plausible-looking but wrong-tax FIRE targets unless the user edits brackets.
- **W5 — Projection cache is process-local** (D7): lost on every restart/deploy (first GET per
  key pays ~500 ms recompute) and NOT multi-replica-safe — invalidation only reaches the local
  `HashMap`, so running >1 API replica behind a load balancer serves stale projections. The
  compose stack runs one replica; keep it that way or move the cache out of process first.
- **W6 — reference docs can drift from code** (the eight errata found while authoring this
  library — stale CI claim, `projection_target_age` remnants, dead README route, etc. — were all
  fixed on 2026-07-02, but the mechanism that produced them remains: docs are hand-maintained).
  When docs and code disagree, the code is ground truth; record unfixable drift in the
  standing-errata table of futurefin-docs-and-writing §7.
- **W7 — Errors in projection math are silent** (owner-identified hardest problem): wrong
  economic modeling produces plausible-looking numbers. Stochastic returns, sequence-of-returns
  risk, tax-aware withdrawal and variable SWR are all candidate directions and currently
  UNIMPLEMENTED. Any work here goes through
  `.claude/skills/futurefin-projection-realism-campaign/SKILL.md`.

## Provenance and maintenance

Written 2026-07-02 against branch `claude/skill-library-handoff-rtfotl` at v1.4.3, by reading the
files cited inline (not from memory of the docs — docs can drift, see W6). Re-verify
volatile claims with:

- Version: `grep -n '^version' apps/api/Cargo.toml` and top of `CHANGELOG.md`.
- Migration count: `ls apps/api/migrations | wc -l` (32 on 2026-07-06; 31 on 2026-07-02).
- Engine purity deps (I8): `grep -E "tokio|sqlx|reqwest|axum" crates/engine/Cargo.toml` → empty.
- Horizon rule (D11): `grep -n "LIFESPAN_AGE\|FALLBACK_YEARS\|clamp(12, 840)\|fallback_no_demographics" apps/api/src/handlers/projection.rs`.
- Cache TTL/keys (D7): `grep -n "PROJECTION_CACHE_TTL\|ProjectionCacheKey\|invalidate_projection" apps/api/src/state.rs`.
- No-warm-up-after-mutation rationale: `grep -n -A6 "refresh_projection_after_mutation" apps/api/src/handlers/projection.rs`.
- f64 exception boundary (D4/I3): `grep -rn "serialize_decimal_as_f64" apps/api/src/` (definition in projection.rs; uses only in projection.rs + history.rs) and `.claude/api-routes.md` §projection + §history series.
- Sink invariant (I1): `grep -n "sink_must_be_last" apps/api/src/handlers/allocation_rules.rs`.
- CI coverage (W1): `cat .github/workflows/ci.yml` — check whether a job now sets `TEST_DATABASE_URL`; if so, update W1 here and `.claude/tests.md` §CI.
- Session mechanics (D3): `grep -n "expires_at\|SESSION_COOKIE" apps/api/src/handlers/session.rs` and `grep -n "SESSION_TTL_DAYS" apps/api/src/main.rs`.
- Default ES brackets (W4): `grep -n -A24 "default_es_tax_brackets" apps/api/src/handlers/installation.rs` vs `DEFAULT_ES_TAX_BRACKETS_API` in `apps/web/src/lib/fire.ts`.
- App.tsx size (W2): `wc -l apps/web/src/App.tsx`.
- Doc drift (W6): the standing-errata table in futurefin-docs-and-writing §7 is the record; empty as of 2026-07-02.

Update this skill whenever: a decision above is overturned (record the new incident), a new
cross-cutting mechanism appears (cache backend, auth scheme, second crate consumer of the
engine), or CI starts running the Postgres integration suite.
