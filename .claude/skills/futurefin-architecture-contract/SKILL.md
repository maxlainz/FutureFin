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
snapshots) and the migration/backup-schema counts were added/refreshed for **v1.5.0** on 2026-07-06,
again for **v1.6.0** (transactions module, backup schema_version 5) on 2026-07-07, for **v1.8.0**
(recurring-transaction rules, backup schema_version 6) on 2026-07-08, for **Unreleased** on
2026-07-09 (D12a: the transactions no-cache contract became conditional on `savings_source`), and for
**v3.0.0** on 2026-08-16 (D13: the image now contains the store; W8: the container is a two-process
supervisor), and for the **Fase 3 (issue #84) train** on 2026-08-28 (D20: append-only MCP write
audit, subtractive API-token scope, a real two-phase `confirm_token`, and a third semaphore around
projection simulations — same pass fixed a standing "`?months=` is clamped" drift in D11), for
the **Fase 4 (issue #85) train** on 2026-08-28 (D21 + I17: the MCP transport — a kill-switch that
changes the handler and not the route table, two CORS layers over one origin list, `Origin`
validation, an explicit body cap on `/mcp`, and an issuer that accepts a subpath; I4 and I11 were
false before this pass), and for the **Fase 5 (issue #86) train** on 2026-08-28 (D22 + I18:
suppressed/capped/derived content must declare itself in the payload — window caps, pagination,
item suppression, scope echo, basis fields — plus the D4/I3 f64-exception boundary splitting into
two definitions with a publication-rounding step). This is the
contract a retiring principal engineer would make you sign: the decisions
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

Postgres 16 is the only store — and **since v3.0.0 the Docker image also CONTAINS it**. The
published image serves API + built SPA on one port via `WEB_STATIC_ROOT`
(`main.rs::web_static_root` → `ServeDir` fallback) *and* runs the PostgreSQL that backs them, in
the same container: one container, two supervised processes, no database service in
`docker-compose.yml`. See **D13** for the runtime shape and **W8** for what that costs. In
split-dev the API still talks to a separate Postgres (`docker-compose.dev.yml`) over TCP — the
embedded database exists only inside the image.

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
**Exception (v1.4.0; extended 2026-07-06; publication rounding added Fase 5/issue #86, 4.4.0)**: the
large parallel arrays in `GET /v1/projection/series` — `points[].net_worth`,
`points[].contributed_capital`, `fire_target_series`, `asset_series[].values` — AND the per-point
arrays of `GET /v1/history/series` (`points[].net_worth/assets_total/liabilities_total`,
`asset_series[].values`, `markers[].total`) serialize as `f64`. **Two separate
`serialize_decimal_as_*` definitions since 4.4.0, not one**: `serialize_decimal_as_f64`
(`pub(crate)`, `handlers/projection.rs`, full f64 precision) for projection responses, and
`serialize_decimal_as_chart_f64` (private, `handlers/history.rs`) for history responses — the
history one additionally rounds to `CHART_DP = 2` decimal places (`Decimal::round_dp` before the
`to_f64` cast) before serializing: interpolation and anchoring still compute exact, only the
published copy is clipped. Same family as `money_out`/`round_ratio` — a raw
`78012.333333333333333333333` per point was pure context/wire cost, no consumer reads a history
chart to 13 decimals. `month_fraction` on history markers gets the analogous publication-rounding
treatment at 4 decimals (`MONTH_FRACTION_DP`, `round_month_fraction`, same file). Documented in
`.claude/api-routes.md`: "**`net_worth` y
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
`sqlx::migrate!("./migrations")` runs at startup (`db.rs::run_migrations`). 34 files in
`apps/api/migrations/` as of 2026-07-08 (`ls apps/api/migrations | wc -l`; v1.8.0 added
`20260708090000_recurring_transaction_rules.sql`; v1.6.0 added
`20260707120000_transactions_and_rules.sql`; v1.5.0 added `20260706203746_history_snapshots.sql`).
There is **no
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
- **Key**: `{installation_id, view, owner_user_id, density}` — **`owner_user_id` is ALWAYS `Some(_)`,
  including `household`, since 4.0.0.** Until then it was `Some` only for `view=mine`, and that was a
  cross-member data leak in the response: a `household` payload carries the **requester's**
  demographics (`viewer_birth_date`, the horizon derived from their age, `jubilacion_age`, the age
  axis), so the first member to ask left *their* projection cached for the whole household.
  Reproduced in test: bob received alice's birth date and, with the order reversed, alice got a
  360-month horizon instead of 648 — if her FIRE crossing fell on month 400, the app told her she
  never retires. **Rule to keep**: anything the response derives from *who is asking* belongs in the
  key, not just in the query. Regression: `apps/api/tests/projection_cache.rs`.
- **TTL**: `PROJECTION_CACHE_TTL = 60 min`, **sliding** — refreshed on every hit; expired entries
  removed lazily on access.
- **Invalidation**: every mutating handler calls and **awaits**
  `refresh_projection_after_mutation` (`handlers/projection.rs`), which runs
  `invalidate_projection_by_installation` — drops ALL entries for the installation (household +
  every member's mine). Logout drops that user's `mine` entries only.
  Until 3.8.0 this was a `tokio::spawn`, which left a real window: the order was
  `commit → respond → (eventually) invalidate`, so a GET landing in between served the stale
  projection — the user edits something and the figure does not move. Awaiting it costs a `retain`
  over a small `HashMap` under an uncontended lock (microseconds) and makes the cache state final
  by the time the mutation responds. As a side effect it removed every timing-dependent sleep from
  the integration tests.
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
- Explicit `?months=` → must be **12–840** or the request is **rejected** (400
  `months_out_of_range`), `horizon_basis = "months_override"`, uncached (D7). **Since 4.4.0 it is
  a rejection, not a clamp**: until then `resolve_projection_context` did `m.clamp(12, 840)` and
  returned 200 with `horizon_basis: "months_override"` on an out-of-range value — the response
  claimed "I did what you asked" while silently substituting a different horizon
  (`validate_months_override`, `handlers/projection.rs`).
(The old target-age model lingered in `.claude/data-model.md`, `.claude/engine.md` and the
`horizon_basis` doc comment in `projection.rs` until 2026-07-02 — all fixed since. The clamp→reject
change (4.4.0) itself lingered as a stale "clamped 12–840" claim in this file,
`futurefin-config-and-flags` and `.claude/api-routes.md` until 2026-08-28 — fixed in the same
sweep as the Fase 3 (issue #84) docs. If in doubt, `projection_horizon_months` /
`validate_months_override` and their unit tests are the ground truth.)

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
`.ffbackup` **schema_version 6** (additive chain: v4 added snapshots over v3; v5 added
`transactions`/`transaction_imports`/`categorization_rules` — v1.6.0; v6 added
`recurring_transaction_rules` + `BackupTransaction.recurring_rule_index` — v1.8.0; import re-links
snapshot items to fresh asset/liability UUIDs via `ledger_index`, else keeps `item_key`). The same
D12 no-cache contract **originally** covered the **transactions** module too
(`handlers/transactions/`, incl. the v1.8.0 recurring-rule endpoints, guarded by
`transactions_projection_cache.rs`). **Breaks if violated**: wiring snapshot writes
into `refresh_projection_after_mutation` couples two independent subsystems and needlessly evicts
a hot projection cache; conversely, ever feeding snapshot data into the engine would make past
"observations" silently reshape the *future* projection — a category error.

**D12a — the transactions half of the no-cache contract became CONDITIONAL (D12a v2 2026-07-09; mode
C added Unreleased).** The blanket "transactions are never a projection input" rule now holds **only** in
`fire_settings.savings_source = budget` (mode A, the default — unchanged, still guarded by
`transactions_projection_cache.rs`). In the **modes that use transactions** — `transactions_avg` (mode B)
and `budget_income_real_expense` (mode C), i.e. `SavingsSource::uses_transactions()` — the projection
derives the monthly saving from the **weighted 12-month average** of **non-reconciled** transactions
(`transactions_avg`, raw since the 3.4.0 reform — paid cuotas count as ordinary spending and
liabilities only subtract their principal from net worth; mode C keeps the budget income and only takes
the real expense; since 3.5.0 reconciled transfer legs — `transfer_counterpart_id IS NOT NULL` — are
excluded from numerator AND denominator: an internal transfer is not income or expense), so
**transactions ARE an engine input** and every mutation that changes the
transaction set (`crud.rs` create/batch/patch/delete + delete_import, `import.rs` confirm, `recurring.rs`
materialize, and since 3.5.0 `reconcile.rs` — reconciling/unreconciling changes WHAT counts in the
average) **must** invalidate the projection cache — via
`invalidate_projection_if_savings_uses_transactions` (mod.rs), which reads
`projection_savings_source(pool, iid)`, checks `uses_transactions()`, and only then calls
`refresh_projection_after_mutation`. It is **best-effort post-commit**: the write is already persisted, so
a failing `savings_source` SELECT is logged and swallowed — it must never turn a successful mutation into
a 5xx (a retry could double-insert). `rules.rs`, previews, and deleting a recurring rule never invalidate
(the set is unchanged). This is a **superseding decision, not a contradiction**: the mode toggle is
precisely what turns display-only history into a real engine input, so the invalidation must follow the
mode. Still no warm-up after mutation (D7 / failure-archaeology §2.7): B/C invalidation is delete-only.
Regression for **all three** modes (A = no mutation invalidates; B and C = each invalidates; A↔B/C
flip via `PATCH /v1/installation` invalidates): `apps/api/tests/transactions_projection_cache.rs`.
The snapshot half of D12 (`/v1/history/*`) is **unaffected** — snapshots are never an engine input
in any mode.

### D13. The image CONTAINS the store — one container, two supervised processes (v3.0.0)
Until 2.x the store was a second compose service (`futurefin-database`, `postgres:16.4-alpine`).
Since v3.0.0 PostgreSQL 16 runs **inside** the published image (plus PostgreSQL 15 binaries, used
only for auto-`pg_upgrade` of older volumes), supervised by `apps/api/docker-entrypoint.sh`.
`docker-compose.yml` has exactly one service and two volumes: `pgdata:/var/lib/postgresql/data`
(**same name and path as 2.x** — the upgrade reuses the existing volume as-is) and
`ffdata:/var/lib/futurefin` (automatic pre-migration backups, entrypoint state files, pg_upgrade
staging). **Why**: a self-hosted household app whose stated axis is "upgrades and backups that
never lose data" was shipping an upgrade path with two moving parts, an externally-managed
password, and no snapshot before migrations ran. Four sub-decisions are load-bearing, each with a
trap behind it (all five traps: futurefin-failure-archaeology §2.11):

- **The runtime is NOT based on `postgres:*`.** Base is `debian:bookworm-slim`; the PG 15/16
  binaries are `COPY --from=` build stages of the official images. `postgres:*` declares
  `VOLUME /var/lib/postgresql/data`, so a `docker run` without `-v` silently gets an **anonymous**
  volume — and watchtower drops it on recreate: total, silent data loss.
- **The image declares NO `VOLUME` of its own**, precisely so the entrypoint's `mountpoint` check
  (`is_mounted`) can tell "a real volume is mounted" from "nothing is". Without one it **aborts**
  (`no persistent volume is mounted at $PGDATA …`) instead of booting onto the container's
  ephemeral layer; `FUTUREFIN_ALLOW_EPHEMERAL_DB=1` is the deliberate opt-out for throwaway runs.
  Declaring `VOLUME` would pre-mount an anonymous volume and blind the guard.
- **Healthcheck is `/v1/ready`, `CMD-SHELL`, with NO `</dev/tcp` fallback.** `/v1/ready`
  round-trips the pool (`SELECT 1` in `handlers/health.rs`, 503 on failure), so it actually
  reports the embedded database. The `CMD-SHELL` form stays (incident v1.0.2: the exec form does
  not resolve `curl` via PATH); the `/dev/tcp` fallback that shipped alongside it was **removed** —
  it would answer "healthy" from the TCP listener while `/v1/ready` was returning 503 with the
  database down. Both `docker-compose.yml` and the Dockerfile carry the comment; do not "restore"
  it.
- **The embedded database authenticates by trust over a local Unix socket.** `initdb
  --auth-local=trust --auth-host=scram-sha-256`; the postmaster runs with `listen_addresses=''`
  and `unix_socket_directories=/var/run/postgresql`, so there is **no TCP listener at all**, and
  the only things inside the namespace are the two processes the entrypoint started. The API's
  `DATABASE_URL` is exported by the entrypoint as
  `postgres:///$POSTGRES_DB?host=/var/run/postgresql&user=$POSTGRES_USER`. A password here would
  not defend against anything an attacker in that namespace could not already do — it would only
  add a secret to generate, inject, rotate and lose: **one more failure mode, not one fewer**.
  `POSTGRES_PASSWORD` is still honored if present (applied to the role) purely so 2.x installs
  that set it are not surprised.

**Breaks if violated**: basing the runtime on `postgres:*` or declaring `VOLUME` re-arms the
anonymous-volume loss for every `docker run`/watchtower user; adding a TCP listener or a password
converts a two-process namespace into an exposed service for zero security gain; re-adding the
`/dev/tcp` fallback makes a container with a dead database report healthy — the exact failure the
healthcheck exists to catch.

**Amendment (2026-08-27, branch `feat/home-assistant-addon`) — a SECOND distribution channel over
the SAME image.** The repo is also a Home Assistant add-on store (`repository.yaml` at the root +
`addon/futurefin/config.yaml`). The add-on **builds nothing**: `image: maxlainz/futurefin` (Docker Hub; GHCR remains private)
(no `{arch}` — the GHCR manifest is multi-arch, the registry picks amd64/aarch64), and the
Supervisor uses the add-on's `version:` as the image tag. None of D13's rules are weakened, only
re-anchored:
- **The layout moves, the model does not.** The Supervisor mounts exactly one persistent bind at
  `/data`, so under the add-on the entrypoint exports `PGDATA=/data/pgdata` and
  `FUTUREFIN_STATE_DIR=/data/state` (detection = `/data/options.json` exists). Because `$PGDATA` is
  now a *subdirectory* of the mountpoint, the volume guard is `is_persisted` — an ancestor walk that
  stops **before `/`** — instead of a plain `mountpoint` check on `$PGDATA`. Same refusal, same
  message; still no `VOLUME` in the Dockerfile.
- **`init: false`** in the add-on config: the image's entrypoint stays PID 1, so the ordered
  shutdown of W8 (API SIGTERM → postmaster **SIGINT**) survives. Putting s6/tini in front breaks it.
- **`backup: cold`**: the Supervisor stops the add-on before copying `/data`. A hot copy of a live
  data directory is not consistent — this is D13's "the store is inside" paying its bill.
- **No `watchdog`**: the only probe candidate (`/v1/ready`) is reachable only over the *optional*
  direct port `8080/tcp`, which ships `null` (unpublished). A watchdog pointed there would restart
  the add-on forever on a normal ingress-only install.
- **CI guards the store shape**: HA reads *any* `config.{yaml,yml,json}` in the repo as an add-on,
  so `ci.yml` (job `secrets-scan`) pins the exact list to two files. Adding a third `config.yaml`
  anywhere would publish a phantom add-on to every subscriber.

### D16. Transfer reconciliation is continuous, with a periodic retry net (v3.8.1)
El pase de conciliación (`handlers/transactions/reconcile.rs`) corre **tras cada mutación** del
conjunto de transacciones: alta, alta en lote, PATCH de `amount`/`op_date`, borrado, borrado de
lote, confirm de import CSV, materialización de recurrentes e import de `.ffbackup`.

- **Los pases post-mutación son best-effort a propósito**: un fallo se loguea y NO convierte una
  escritura ya persistida en un 5xx (el cliente reintentaría y duplicaría el movimiento — el
  `fingerprint_ordinal` no lo impide). El precio es que ese par se queda sin conciliar de forma
  **permanente y silenciosa**, porque nada lo reintenta y el usuario no puede enterarse.
- **La red de reintento** es `sweep_all_owners`, lanzado por la **única tarea periódica del
  binario** (`main.rs::spawn_reconcile_sweep`, `FUTUREFIN_RECONCILE_SWEEP_HOURS`, default 24 h,
  0 = off). Recorre cada `(installation, owner)` con movimientos sin conciliar; un owner que falla
  no aborta el barrido (se cuenta y se reintenta a la siguiente). Se **aborta antes de cerrar el
  pool** en el apagado ordenado.
- **Primera pasada tras el primer intervalo, no al arrancar**: en el arranque no ha pasado nada
  que conciliar y competir con migraciones y warm-up no compra nada.
- **El barrido obedece D12a como cualquier otra mutación.** Toma `Arc<AppState>` (no un `PgPool`)
  y, por cada owner cuyo pase **crea pares**, llama a
  `invalidate_projection_if_savings_uses_transactions`: conciliar cambia QUÉ cuenta en el promedio
  12m, así que en modos B/C es una mutación de inputs del engine. La invalidación va **condicionada
  a `pairs_created > 0`** — en una instalación sana el barrido no encuentra nada y desalojar una
  cache caliente cada 24 h a cambio de nada sería peor que el bug. El gating por modo vive dentro
  del helper, así que en modo A no invalida jamás. Regresión (los cuatro casos, verificados con
  mutantes): `apps/api/tests/transactions_projection_cache.rs::*_sweep_*`.
  **Incidente**: la primera versión del barrido (3.8.1, antes de mergear) recibía solo el pool, así
  que estructuralmente NO podía invalidar. Un par recuperado por el barrido dejaba la proyección
  cacheada obsoleta y, con el TTL deslizante de D7, un usuario que la mirase una vez por hora la
  mantenía viva indefinidamente.
- **La UI no tiene botón** desde 3.8.1: con el pase en cada mutación más el barrido, «Conciliar
  ahora» no tenía trabajo (su mensaje habitual ya era «Sin transferencias que conciliar»). La ruta
  `POST /v1/transactions/reconcile` y la tool MCP `reconcile_transfers` **se mantienen** como
  recuperación manual.
**Breaks if violated**: hacer que el pase post-mutación propague su error convierte una mutación
correcta en 5xx y provoca duplicados por reintento del cliente; quitar el barrido devuelve el fallo
silencioso permanente; darle solo el pool (o invalidar sin mirar `pairs_created`) rompe D12a por un
lado o desaloja la cache a diario por el otro.

### D14. Second auth scheme: per-user Bearer API tokens, hash-only, live role (v3.0.0)
The embedded MCP server (`/mcp`, module `apps/api/src/mcp/`, official `rmcp` SDK) needed a
non-cookie credential. The decision mirrors D3 (sessions-in-DB, not JWT), deliberately:

- **The stored credential is only the SHA-256 hex of the secret** (`api_tokens.token_hash`
  UNIQUE). The secret (`ffp_` + 43 chars base64url of 32 `OsRng` bytes) travels once, in the
  `POST /v1/api-tokens` 201. Lookup is an O(1) hash-equality in SQL — no secret comparison in
  Rust, no timing surface.
- **The token freezes NOTHING**: `require_api_token` (handlers/api_tokens.rs) returns only
  `{user_id, token_id}`; every `/mcp` request re-runs `require_installation_member` for the live
  role/installation. Revoking a token (`revoked_at`) or a membership cuts access on the next
  request — same revocation semantics as deleting a session row.
- **Any member (viewer included) may mint their own tokens** via the cookie-authed CRUD: a token
  can never do more than its owner. Since the #2/#3 MCP expansion (2026-08-18) tools also WRITE:
  every write tool re-checks `require_mcp_write` per request (`role_can_write` on the live role +
  the DB kill-switch `installation.mcp_write_enabled`, toggle in Ajustes → Integraciones), so a viewer's
  token still cannot write and flipping the toggle cuts writes for ALL tokens on the next call.
  Pending users hit the same 403 gate.
- **MCP tools call the SAME core fns as the HTTP handlers** (`summary_core`,
  `projection_series_cached`, `budget_snapshot_core`, …): read handlers were split into
  extractors+auth vs `*_core(pool, iid, user_id, view, …)`. A tool with its own SQL or its own
  response type is the D2/D8 dual-branch drift bug reborn — don't.
- `/mcp` is **deliberately not in OpenAPI** (JSON-RPC, self-described via `tools/list`).
  `FUTUREFIN_MCP_ENABLED=0` does **not** unmount the router — since 4.4.0 the route is mounted
  either way and the handler answers 404 JSON `mcp_disabled` (D21).
- This decision governs **how** a tool is built, never **whether** it should exist. The catalog is
  a derived surface of the HTTP API, so every API-surface change owes a parity evaluation (tool
  added/updated, deliberate omission recorded, or n/a) — that discipline, its rubric and the
  omission register live in `.claude/skills/futurefin-mcp-parity/SKILL.md`.

**Breaks if violated**: storing the secret (or a reversible form) turns a DB leak into credential
theft; caching role/installation in the token resurrects stale-privilege bugs the session design
already killed; duplicating query logic in tools reintroduces silent handler↔tool divergence
(plausible-but-different numbers, the owner's stated worst failure mode).

*(v3.1.0 makes this the second of THREE schemes: OAuth access tokens — D15 — are dispatched by
Bearer prefix in `mcp/auth.rs::authenticate` and obey every rule above.)*

*(Fase 3/4.4.0 — D20 — adds a scope axis to the API-token half of this pair (`read_write` |
`read_only`, subtractive only), an append-only audit trail on every `require_mcp_write` call, and
a real two-phase `confirm_token` on the seven destructive tools whose preview can't be undone by
re-asking the model. None of it touches the credential contract above — it's what sits between
"the gate passed" and "the write happened".)*

### D15. FutureFin as its own OAuth 2.1 authorization server for MCP (v3.1.0)
The claude.ai web connector requires the MCP authorization spec (OAuth 2.1 + PKCE S256 + RFC
8414/9728 metadata + DCR RFC 7591). The decision: the **same binary is authorization server AND
resource server** (`apps/api/src/oauth/`) — no external IdP, no new container, zero signing keys.
This is NOT a login mechanism: username+password Argon2id stays the only way to authenticate a
person (the failure-archaeology "OAuth login" rejection is untouched); OAuth here *delegates*
read-only access to a client app after explicit consent.

- **Same credential contract as D3/D14, extended**: authorization codes, access tokens (`ffo_`,
  1 h) and refresh tokens (`ffr_`, 90 days idle, rotated on every use) are opaque, hash-only
  (`auth/secret.rs::sha256_hex`), and freeze nothing — every `/mcp` request re-runs
  `require_installation_member`. All expiries are computed by Postgres (`now() + interval`),
  never by Rust's clock.
- **The grant (`oauth_grants`, one row per client+user, partial-UNIQUE on the live pair) is the
  unit of consent and revocation**: auth lookups JOIN it and require `revoked_at IS NULL`, so one
  UPDATE kills every token of a connection — the session-row philosophy at grant scale. Reusing a
  consumed code or a rotated refresh token revokes the whole grant (`revoked_reason` audit).
- **Open DCR is safe by design**: a client row grants nothing — the gate is the user's login +
  consent screen. Anti-flood: lazy GC of grant-less clients >24 h inside `POST /oauth/register`
  (never in a GET — D5), 1000-client cap → 503. Unknown `client_id` at the token endpoint is
  **401 `invalid_client`** — the exact signal that makes claude.ai re-register, and what lets a
  backup restore (which excludes `oauth_*` tables) self-heal.
- **`resource` (RFC 8707) validated hard at issuance, deliberately NOT re-validated at `/mcp`**:
  we are the only AS and only RS of our tokens; re-checking against the per-request Host would
  break "consent via tunnel domain, query via LAN IP" for zero real security.
- **Issuer derived from the request** (`X-Forwarded-Proto`/`X-Forwarded-Host`/`Host`, strict host
  charset) so no env var became mandatory; `FUTUREFIN_PUBLIC_URL` is an optional fail-loud
  override, which **since 4.4.0 accepts a subpath** — the only way to run OAuth behind a
  prefix-mounting proxy (D21). The `/mcp` 401 advertises `resource_metadata` (RFC 9728 §5.1) —
  **only the 401**: a 403 (pending user) with that header sends clients into an infinite re-auth
  loop.
- **The consent screen is the SPA** (`/oauth/authorize` served by the static fallback, resolved in
  `main.tsx`); protocol endpoints are flat root routes and there is **never** a backend route at
  `/oauth/authorize` (an axum 405 does not fall through to the SPA fallback). The seven protocol
  routes are **always mounted** and answer 404 JSON `mcp_disabled` when `mcp_enabled` is false
  (D21); what still unmounts under the flag are the two consent-flow endpoints
  (`/v1/oauth/authorize-details`, `POST /v1/oauth/authorize`), which live under `/v1` whose
  fallback already returned JSON. `GET/DELETE /v1/oauth/connections` is mounted always regardless
  (precedent `/v1/api-tokens`: killing MCP must not strand existing grants unrevocable).

**Breaks if violated**: a JWT access token (or any role/installation claim) resurrects the exact
stale-privilege class D3 killed, plus key management; validating `resource` against the request
Host breaks LAN access with tunnel-issued tokens; a backend route at `/oauth/authorize` kills the
consent screen in production with a silent 405; `WWW-Authenticate` on 403 loops claude.ai forever;
prefix-or-substring redirect matching (instead of exact string) is an open redirect.

### D17. Anti-clickjacking condicional en modo ingress (2026-08-27)
**Context.** The original invariant was absolute: *nothing* in FutureFin is embeddable, implemented
as a fixed `X-Frame-Options: DENY` layered over the **final** router (outside `api`, because the
thing that most needs it — the OAuth consent screen — is served by the SPA fallback, D15). Home
Assistant's Ingress renders every add-on inside a **same-origin iframe** of the HA frontend. Under
`DENY` the app renders blank.

**Decision.** The header is now computed per request by `handlers/frame.rs`
(`with_frame_policy(router, state)`, wrapping the same final router). Exactly one condition relaxes
it: **trusted peer AND `X-Ingress-Path` present** ⇒ `Content-Security-Policy: frame-ancestors 'self'`
and `X-Frame-Options` **removed**. Everywhere else — including a request that carries
`X-Ingress-Path` from an untrusted peer — the response still carries `X-Frame-Options: DENY`.

**Rationale (and why it is not a weakening).** `frame-ancestors 'self'` still forbids cross-origin
framing, which is the actual clickjacking vector; same-origin embedding of our own app buys an
attacker nothing. The two details that are load-bearing:
- **The header alone cannot relax it.** If `X-Ingress-Path` were sufficient, any client could turn
  the protection off by sending one header. The gate is `PeerPolicy` over
  `FUTUREFIN_TRUSTED_PROXY_IPS` (`prefix.rs`), default `Disabled` = nobody is trusted.
- **`X-Frame-Options` must be *removed*, not just accompanied by the CSP.** Browsers that read both
  let `DENY` win, and the add-on would still render blank — a bug that looks like "the CSP didn't
  apply".

Note the deliberate asymmetry with the **prefix** detection (`prefix::request_prefix`), which does
NOT require a trusted peer: a forged `X-Forwarded-Prefix` only deforms the attacker's own response
(assets that fail to load). Relaxing frame policy and accepting identity (D18) are the two things
that do require the peer. The **OAuth issuer** (`X-Forwarded-Proto`/`X-Forwarded-Host`) falls on the
prefix side of that line for the same reason — it reflects, it does not grant — and what made the
reflection dangerous was cacheability, closed in 4.4.0 with `no-store` + `Vary` (D21).

**Consequences / breaks if violated.** Restoring a static `SetResponseHeaderLayer` of
`X-Frame-Options: DENY` breaks the HA add-on silently (blank panel, no console error that names the
cause). Dropping the peer condition, or keeping `X-Frame-Options` alongside the CSP, breaks the
protection or the add-on respectively. Pinned by `apps/api/tests/frame_options.rs`, which asserts
**both halves** (untrusted peer + header ⇒ `DENY`; trusted peer + header ⇒ CSP and no XFO).

### D18. Confianza en cabeceras de proxy: opt-in doble (2026-08-27)
**Context.** Behind HA's Ingress the Supervisor has already authenticated the person and injects
`X-Remote-User-Id` (stripping any client-supplied copy). `POST /v1/auth/sso` (`handlers/sso.rs`)
turns that assertion into a **normal FutureFin session**: same `sessions` row, same `ff_session`
cookie, same installation gate; the only difference in the account is `password_hash IS NULL`
(migration `20260827120000_users_trusted_header_identity.sql`, plus a partial-UNIQUE
`external_user_id`).

**Decision.** A header of identity is an **unproven assertion**, so honoring it requires **two
independent opt-ins**, both of which the operator must set:
1. `FUTUREFIN_TRUSTED_PROXY_AUTH=1` — otherwise the endpoint answers 401 `sso_disabled`.
2. The peer IP in `FUTUREFIN_TRUSTED_PROXY_IPS` — otherwise 401 `sso_untrusted_peer`.
Setting (1) without (2) is not a half-configuration, it is a **startup panic** (`main.rs`): an
enabled SSO with nobody trusted would either be dead code or, worse, read as enabled.
The route is mounted **unconditionally** — the shape of the router must not depend on the
environment, or the tests stop describing the binary that ships; what the environment changes is
the *state*, not the route table.

**Rationale.** A spoofed `X-Remote-User-Id` on an open port is not a privilege escalation, it is
**total impersonation of any account, without a password**. Hence: the add-on's **direct port never
qualifies** — the add-on exports `FUTUREFIN_TRUSTED_PROXY_IPS=172.30.32.2` (the Supervisor's ingress
address on HA's internal network) and a LAN peer reaching the optional published `8080/tcp` is not
on that list, so the same running process accepts SSO through the ingress and rejects it from the
LAN. `PeerPolicy::Any` exists for tests and for deployments where the proxy is physically the only
path to the process; it is never the default (`Disabled`), and an unknown peer (`None`, e.g. axum
`oneshot` without `ConnectInfo`) passes **only** under `Any`.

**Consequences.**
- Auto-provisioning follows the password path exactly: first identity through the door **creates the
  installation and is owner** (`bootstrap_installation_as_owner_if_empty`), everyone after is
  *pending* until approved. SSO does not bypass D1 or membership.
- Password login and `POST /v1/auth/password` reject a NULL-hash account with
  `sso_account_no_password` instead of a generic 401 — otherwise the person hunts for a password
  that does not exist.
- **Breaks if violated**: honoring `X-Remote-User-*` on the strength of the header alone (or on a
  `PeerPolicy::Any` default) is a full authentication bypass on any port the container publishes.
  Pinned by `apps/api/tests/sso_login.rs` — whose *first* assertion is that the door is closed by
  default.

### D19. «Entrar con Home Assistant»: HA como fuente de identidad, nunca de autorización (2026-08-27, v4.3.1)

- **Context**: las cuentas SSO del add-on no tienen contraseña (decisión 4.3.0) y por tanto no
  podían iniciar sesión en el origen directo — donde vive la pantalla de consentimiento OAuth del
  conector MCP. La opción «que se pongan contraseña» fue rechazada por el owner; la paridad exigía
  un login con la cuenta de HA fuera del panel. Esto **reabre estrechamente** la fila 12 de la
  arqueología (OAuth-as-login, settled desde mayo 2026) — ver su second scope note.
- **Decision** (cuatro pilares, todos owner-confirmados):
  1. **Identidad, no autorización**: del flujo de HA solo se toma `auth/current_user.id` (el mismo
     `User.id` que el Supervisor manda en `X-Remote-User-Id` — ambos caminos convergen en la misma
     fila vía `users.external_user_id` y la misma `resolve_or_provision`). Roles, membership,
     aprobación de pendientes y el bootstrap del owner siguen siendo 100 % de FutureFin;
     `is_admin`/`is_owner` de HA se ignoran.
  2. **IdP puro**: el refresh token de HA se revoca (`/auth/revoke`, best-effort) inmediatamente
     después de leer la identidad y ANTES de tocar nuestra BD — FutureFin no retiene ninguna
     credencial de HA. Un access token de HA es plenos poderes sobre la domótica: se usa una vez
     y se tira.
  3. **Modelo CSRF por cookie de estado**: HA no soporta PKCE ni client secret; la defensa es el
     mismo-origen exacto `client_id`↔`redirect_uri` (con eso HA nunca fetchea nuestra URL) + la
     cookie `ff_ha_state` (HttpOnly, **`SameSite=Lax` obligatorio** — el callback es un GET
     top-level cross-site; `Strict` rompería todos los logins —, Max-Age 600, un solo uso, nonce
     comparado en tiempo constante). La ruta de retorno `next` viaja DENTRO de la cookie (jamás en
     el `state` tamperable) y `sanitize_next` la re-valida al emitirla.
  4. **Solo add-on, una sola URL**: `FUTUREFIN_HA_SSO_URL` (origen público de HA, fail-loud) solo
     se honra con `FUTUREFIN_HA_ADDON=1` — que únicamente exporta el entrypoint en modo add-on;
     la URL sin el flag aborta el arranque. Sin optimización de URL interna
     (`http://homeassistant:8123`): un solo origen para navegador y servidor, un solo modo de fallo.
- **Consequences**: primera dependencia de red saliente del binario (reqwest/rustls +
  tokio-tungstenite; gate `cargo tree -d` sin rustls duplicado); el seam de test es el trait
  `HaIdp` con `FakeHaIdp` en el harness (sin wiremock: oneshot no abre sockets y la pata WS no la
  cubre un mock HTTP); el login-con-IdP **genérico** sigue rechazado (arqueología fila 12).
- **Breaks if violated**: guardar el refresh token convierte la BD de finanzas en un llavero de
  domótica; derivar roles de `is_admin` fusionaría dos modelos de autorización distintos; aceptar
  la env fuera del add-on pintaría el botón en instalaciones compose (decisión explícita del
  owner en contra); meter `next` en el `state` es una fábrica de open-redirects.

### D20. Escrituras MCP: auditoría append-only, scope como resta, y una confirmación en dos fases de verdad (Fase 3, issue #84, 4.4.0)

- **Contexto**: hasta aquí, un token de API podía vaciar el ledger del hogar sin dejar rastro
  persistente — `delete_transaction` es hard delete, y `api_tokens.last_used_at` (throttle 60 s) no
  cuenta llamadas ni dice cuáles. Y el patrón preview/confirm del issue #3 se leía como una
  salvaguarda de dos fases sin serlo: `confirm` es un booleano del propio esquema de la tool, así
  que el modelo podía escribirlo en la PRIMERA llamada — un `delete_import` con `confirm: true` de
  entrada borraba el lote y sus movimientos sin que nadie hubiera visto nunca el preview. Tres
  decisiones cierran ambos huecos sin tocar D14 (el contrato de credenciales) ni D2 (view scoping
  sigue sin ser un límite de autorización):
  1. **Auditoría append-only, nunca de los argumentos.** `mcp_write_audit` (una fila por llamada a
     `require_mcp_write`) registra quién, con qué credencial, con qué **rol vivo**, qué tool, el
     desenlace y los UUIDs mutados — **nunca** el contenido de los argumentos, ni en claro ni como
     digest. Dos razones estructurales, no solo de gusto: los argumentos llevan texto escrito por
     la persona (conceptos, notas) y guardarlos crearía un segundo domicilio para ese contenido
     fuera del `.ffbackup` cifrado que, al ser append-only, convertiría el borrado del usuario en
     una mentira; y un digest tampoco vale porque el espacio de entrada (fecha + importe + un
     concepto de vocabulario corto) es lo bastante pequeño para fuerza-bruta un SHA-256. El
     esquema es tipado sin JSONB ni texto libre **a propósito**, para que la disciplina de higiene
     no dependa de que el siguiente que toque la tabla se acuerde de ella. El orden que hace el log
     imposible de falsear: `attempted` (el gate dejó pasar, aún no hay desenlace) → `settle` cierra
     UNA vez a `ok`/`failed`; `denied` nace ya cerrado (el gate ES toda la operación); un
     `CHECK ((settled_at IS NULL) = (outcome = 'attempted'))` hace el resto write-once. Retención
     365 días, poda perezosa dentro del propio camino de escritura (D5: nunca en un GET).
  2. **El scope de un token de API solo RESTA.** `api_tokens.scope ∈ {read_write, read_only}`,
     default `read_write` (preserva byte a byte todo token ya emitido), leído **vivo** en el mismo
     SELECT que autentica — la misma filosofía de D14, aplicada a un eje nuevo. Es la puerta
     intermedia de `require_mcp_write`, entre el rol vivo (puerta 1) y el toggle de la instalación
     (puerta 3): nunca puede conceder lo que el rol de la persona no concede ya, así que degradar a
     `viewer` sigue siendo el techo real. Los access tokens OAuth (`ffo_…`) **no** negocian scope
     propio (siempre `read_write`) por una asimetría deliberada: en un token de API el scope lo
     elige la PERSONA con su propia cookie de sesión; en OAuth el `scope` del authorization request
     lo elige la APLICACIÓN CLIENTE, así que anunciarlo en `scopes_supported` sin una pantalla de
     consentimiento que lo recorte no restringiría nada — solo mentiría en la metadata RFC 8414.
  3. **`confirm_token`: la confirmación deja de ser un booleano.** Solo el preview (`confirm:
     false`, la única llamada honesta) puede emitir un secreto hash-only (`ffpv_…`, un solo uso,
     TTL 10 min) ligado a la tool, a los argumentos normalizados y a la **huella de los efectos que
     acaba de enseñar**. La confirmación exige ese token, y el servidor **recalcula** los efectos
     en ese instante y compara huellas: si el mundo se movió entre las dos llamadas (el lote
     creció, el pasivo ganó movimientos vinculados), `confirm_token_stale` en vez de ejecutar sobre
     algo distinto de lo que se enseñó. El precedente exacto es `oauth_authorization_codes` (mismo
     patrón hash-only + un solo uso + `consumed_at` marcado dentro del propio UPDATE que valida);
     la diferencia deliberada es el TTL — 10 min, no los 2 min de un code OAuth, porque aquí hay
     una PERSONA leyendo un preview en un chat, no una máquina respondiendo al instante. Se exige
     solo en 7 de las 14 tools con preview: cascadas de tamaño no acotado y puertas de un solo
     sentido — nunca en un borrado de una fila cuyo contenido íntegro ya viajó en el preview,
     porque encarecer cada borrado trivial a dos viajes es la forma más rápida de que la ceremonia
     se lea como ruido y de que la gente aprenda a ignorarla.
- **Consequences**: `materialize_recurring`, `reconcile_transfers` y `unreconcile_transfer` — las
  tres tools destructivas que llevaban desde el issue #3 SIN preview porque sus cores calculan y
  escriben en la misma transacción — ganan preview aunque no puedan dar cifras (publican
  `would_materialize`/`would_prune: null` **con el motivo**, en vez de inventar un número: un
  número inventado sería peor que ninguno, porque el humano aprobaría un borrado creyendo conocer
  su tamaño). Las 15 escrituras que invalidan FULL devuelven además un bloque `impact` —
  antes/después/delta de las cuatro cifras de `get_summary` medidas con la MISMA core, best-effort
  — pero **nunca** la fecha de jubilación: eso costaría una simulación de hasta 840 meses justo
  después de invalidar la cache, así que un tercer semáforo (`heavy::run_projection_sim`, mismo
  módulo que ya acotaba Argon2id y el cripto de `.ffbackup`) pasa a envolver también las
  simulaciones de proyección — sin él, `simulate_projection` en bucle desde un agente (o cualquier
  `GET /v1/projection/series?months=…`, que salta la cache por D7) podía agotar el pool de blocking
  de Tokio y tumbar `/v1/ready`, con el PostgreSQL embebido dentro del mismo contenedor.
- **Breaks if violated**: auditar los argumentos (o un digest suyo) convierte el log en un segundo
  domicilio de datos personales que el borrado del usuario no puede alcanzar; dejar que `confirm`
  siga siendo un booleano puro deja el patrón preview/confirm en pura decoración — un modelo con
  prisa siempre puede saltárselo escribiendo `true` a la primera; dejar que el scope de un token
  conceda más que el rol vivo (o anunciar `scopes_supported` en OAuth sin consentimiento real)
  rompe la propiedad central de D14: que ninguna credencial pueda hacer más que su dueño.

### D21. El transporte de `/mcp`: la forma del router no depende del entorno, y una lista de orígenes no es un privilegio (Fase 4, issue #85, 4.4.0)

**Context.** Cuatro fallos distintos con una raíz común: el binario que se prueba no era el binario
que se publica, y una lista de configuración se estaba usando para dos cosas con consecuencias
distintas. Ninguno era explotable sin credencial válida; los cuatro se diagnosticaban como averías.

**Decision (cinco reglas, cada una con su incidente).**

1. **Un kill-switch cambia el handler, nunca la tabla de rutas.** `FUTUREFIN_MCP_ENABLED=0` monta
   `/mcp` y las siete rutas de protocolo OAuth igual, y responde **404 JSON `mcp_disabled`** con
   cualquier método. Es la generalización literal de la frase que D18 ya escribió para
   `/v1/auth/sso`. *El incidente*: desmontarlas solo se veía mal en la imagen publicada, cuyo
   fallback final es un `ServeDir` con fallback al `index.html` — y **`ServeDir` no llama a su
   fallback para métodos distintos de GET/HEAD**. `POST /mcp` devolvía **405 con cuerpo vacío** y
   `GET /.well-known/oauth-authorization-server` devolvía **`200 text/html`** (el shell de la SPA).
   El conector fallaba al parsear JSON y enseñaba «connection failed» sin causa. El test que lo
   cubría montaba el router *sin* SPA, así que **afirmaba una ausencia contra un fallback que la
   producción no tiene**: la lección general es que un test que afirma que algo NO existe solo vale
   si monta la misma pila de fallback que la imagen (por eso `spa::mount_static_spa` es ahora una
   función de la lib que llaman `main.rs` **y** los tests).
2. **Una lista, dos capas CORS, dos privilegios.** `CORS_ORIGINS` alimenta `api_cors_layer` **con**
   `allow_credentials(true)` (credencial = cookie) y `mcp_cors_layer` **sin** credenciales
   (credencial = header `Authorization`). *El incidente*: con una sola capa sobre todo el router,
   añadir un origen para que funcionara un cliente MCP de navegador concedía de paso acceso **con
   cookie** a `/v1/backup/user-export` y `/v1/api-tokens`. Compartir la lista está bien; compartir
   el privilegio no. Dos trampas de axum que esto fija: `Router::layer` solo envuelve las rutas ya
   registradas (el `merge` de `mcp` va **después**), y dentro del router de `/mcp` se usa
   `route_layer` y **jamás** `layer` — `layer` envuelve también el *fallback*, y un `merge` lo
   arrastra al router destino, mandando toda ruta desconocida (incluida `/oauth/authorize`, la
   pantalla de consentimiento) a la auth Bearer del MCP: 401 en vez de la pantalla.
3. **El `Origin` se valida; el `Host` no, y las dos mitades tienen su razón.**
   `disable_allowed_hosts()` sigue puesto porque el despliegue objetivo es LAN/túnel con `Host`
   arbitrario y el gate es el Bearer. `with_allowed_origins(CORS_ORIGINS)` se enciende porque es la
   mitad del anti-DNS-rebinding que sí se puede exigir sin conocer el `Host`, y su default en rmcp
   era lista vacía = apagada. **No rompe a ningún cliente sin navegador**: rmcp deja pasar una
   request **sin** `Origin` aunque la lista no esté vacía, y Claude Desktop, Claude Code y `curl`
   no la mandan. Ese hecho es el que hace aceptable la regla — sin él sería un cambio de ruptura.
4. **Un invariante enforced por un `Layer` solo vale para las rutas que ese layer alcanza.**
   `DefaultBodyLimit` de axum actúa vía **extractores**; `/mcp` es un `route_service` que lee el
   body por su cuenta. El «1 MiB global» de I11 era falso justo ahí (regía el default de rmcp, 4
   MiB). Ahora se fija explícitamente en `mcp::MCP_MAX_REQUEST_BODY_BYTES`.
5. **El issuer OAuth es una identidad: se declara, no se refleja.** `FUTUREFIN_PUBLIC_URL` acepta
   subpath (validado con `prefix::normalize_prefix`, la misma función de `FUTUREFIN_BASE_PATH`), y
   el prefijo **no** se compone de `X-Forwarded-Prefix`. Dos razones: un valor de operador
   (fail-loud al arrancar) no lo puede mover una cabecera; y bajo el Ingress de HA el prefijo lleva
   un **token efímero de sesión** que quedaría horneado dentro del issuer.

**Rationale — la asimetría con D17/D18 es aparente, no real.** Aquellas dos exigen peer de
confianza porque las cabeceras en juego **conceden autoridad**: relajar el anti-clickjacking, o
aceptar una identidad. `X-Forwarded-Host` en el issuer **no concede nada, refleja**: un valor
falsificado deforma la respuesta del propio atacante y de nadie más — el argumento literal que
`prefix.rs` ya daba para no exigirle peer al prefijo. Lo único que convertía esa reflexión en algo
más era la **cacheabilidad**, y eso lo cierran dos líneas: `Cache-Control: no-store` +
`Vary: X-Forwarded-Proto, X-Forwarded-Host` en las dos metadatas. Exigir peer aquí sería fail-closed
contra el caso mayoritario (Cloudflare Tunnel, nginx) para cerrar algo que ya no está abierto.

**Deliberately NOT done, with a named trigger.** La sesión de Streamable HTTP **no** se liga a la
credencial. Hoy no compra nada: el Bearer corre antes del protocolo en *cada* request, la identidad
se re-resuelve viva (D14) y el servidor no emite nada por iniciativa propia — ese último hecho es el
único que lo hace seguro. Y la capa de sesión la está retirando el propio protocolo (SEP-2567).
**Trigger para reabrirlo: la primera capacidad server→cliente** (notificaciones, `progress`, SSE
reanudable con datos). Entonces, o un `SessionManager` propio que ate sesión→credencial, o
`legacy_session_mode: false`. No antes: sería un cambio de comportamiento del transporte sin un
riesgo que lo justifique.

**Breaks if violated**: volver a desmontar rutas bajo un flag devuelve el 405 mudo y el
`200 text/html` — un apagado indistinguible de una avería; devolver `/mcp` a la capa CORS del API
(o mover su `merge` por encima del `.layer`) le regala la cookie a un origen añadido para otra
cosa; cambiar `route_layer` por `layer` en el router de `/mcp` manda toda ruta desconocida a la auth
Bearer; confiar en `DefaultBodyLimit` para un `route_service` deja el tope real en manos del SDK; y
componer el issuer con un prefijo de cabecera hornea un token de sesión efímero dentro de una
identidad pública. Pinned por `mcp_http.rs` (kill-switch **con SPA montada**, `Origin`, preflight),
`oauth_flow.rs` (metadata sin caché, subpath, GC, y el 401 que vigila `get_oauth_authorize_is_not_handled_by_the_api`)
y `body_limits.rs::oversized_mcp_body_returns_413`.

### D22. Suppressed/capped/derived content declares itself — never inferred from shape (Fase 5, issue #86, 4.4.0)

**Context.** Six unrelated symptoms in the same pass shared one root cause: a server-side cap,
window, suppression or derivation that a client (human or an MCP-driven agent) could not
distinguish from "there is nothing more to show". `GET /v1/history/series` omitted `window_months`
and returned the **entire** history — the worst case by default (~290 points for a household whose
backfill anchor sits at a birth date, the first ~200 interpolating between €0 and a few hundred).
Bounding that default to 120 months is what would have *created* the ambiguity — a short array
meaning either "that's all there is" or "the server stopped there" — so the cap shipped **with** the
fields that resolve it: `window_months`, `window_truncated` and `first_snapshot_date_ymd`. That is
the shape of this decision: a cap is only allowed to exist alongside the field that declares it. `GET /v1/history/cashflow`'s fine-grain curve simply
vanished above a size cap, with no field naming why. A `SnapshotResponse` with `items` suppressed
for list-view efficiency was byte-for-byte the same shape as a snapshot with zero items. Untagged
liabilities grouped under the Spanish string literal `"(sin etiqueta)"` — a label impersonating
data. And every `?view=mine` response was shaped identically to its `household` counterpart: a
caller that lost track of which it asked for had nothing in the body to recover that from.

**Decision.** Whenever a response's content depends on something the client did not fully choose or
fully see — a default window, a size cap, a suppressed field, an applied scope, a derivation basis
— that fact travels IN the payload as a named field, never as an inferred property of shape (array
length, presence/absence, a string that reads as a label). Concretely, since Fase 5:
`window_months` + `window_truncated` + `first_snapshot_date_ymd`/`first_snapshot_month_index`
(`GET /v1/history/series`); `fine_absent_reason` (`GET /v1/history/cashflow`, `null` ⟺ the fine
curve travels); `item_count`/`items_included` (`SnapshotResponse`); `events_truncated` (`GET
/v1/projection/series`); `total_count`/`offset`/`truncated` on every paginated `list_*` MCP tool
(including the two new ones, `list_snapshots` and `list_transaction_imports`); `financial_health.basis`
/ `totals.basis` (plan vs actual vs mixed); and `view` echoed by every scope-dependent core
(`LedgerView::as_str`, `handlers/person_view.rs`) in `SummaryResponse`, `BudgetSnapshotResponse`,
`ProjectionSeriesResponse`, `AllocationResolutionResponse` — plus an envelope wrapper on the seven
MCP list tools whose HTTP twin still returns a bare array (§I4 note below), so the tool side gets
the same self-declared scope a JSON-RPC caller cannot otherwise infer. Same principle,
smaller blast radius: `liabilities_by_type_tag[].type_tag` moved from `String` (with the
`"(sin etiqueta)"` literal) to `Option<String>` → `null`, a typed absence instead of a string
dressed as a label.

**Breaks if violated**: adding a new cap, window, suppression or derivation without a field that
names it reintroduces the exact class this pass closed — an empty list or a short array stays
silently ambiguous between "nothing exists" and "the server didn't show you everything", and a
caller has no way to tell the difference without a second, disambiguating request it has no reason
to make.

Pinned by `apps/api/tests/context_fields.rs` (11 endpoint-level contract tests, one per field
family above) and `apps/api/tests/mcp_http.rs::list_tools_echo_the_applied_view_and_keep_content_parity`
/ `list_snapshots_paginates_and_declares_item_suppression`.

## 3. Invariants table

| # | Invariant | Enforced where | How to check |
|---|-----------|----------------|--------------|
| I1 | Exactly one **uncapped `remainder`** allocation rule per scope, always **last** in the cascade (the "sink") | `handlers/allocation_rules.rs` create/patch/delete/reorder; API errors `remainder_required`, `uncapped_remainder_exists`, `sink_must_be_last` | `grep -n "remainder_required\|uncapped_remainder_exists\|sink_must_be_last" apps/api/src/handlers/allocation_rules.rs` |
| I2 | `fire_target_at_month_index` is the ONLY FIRE-target formula — engine crossover and API `fire_target_series` both call it | `crates/engine/src/projection.rs` (public fn + regression test for the old off-by-one) | `grep -rn "fire_target_at_month_index" crates/ apps/api/src/` — every inflation-compounding of a FIRE target must route through it |
| I3 | Amounts serialize as decimal strings, EXCEPT the documented f64 arrays of `/v1/projection/series` and the per-point arrays of `/v1/history/series` (D4) | `serialize_decimal_as_f64` (`pub(crate)`, `handlers/projection.rs`, full precision) for projection responses; `serialize_decimal_as_chart_f64` (private, `handlers/history.rs`, since Fase 5/issue #86, additionally rounds to `CHART_DP = 2`) for history responses — two definitions, no cross-use | `grep -rn "serialize_decimal_as_f64\|serialize_decimal_as_chart_f64" apps/api/src/` (one definition in projection.rs, one in history.rs) |
| I4 | All routes live under `/v1/`, except root `/health`, `/openapi.json`, `/mcp` (v3.0.0) and the OAuth protocol routes (v3.1.0: `/.well-known/oauth-protected-resource[/mcp]`, `/.well-known/oauth-authorization-server[/mcp]`, `/oauth/register`, `/oauth/token`, `/oauth/revoke` — root-level because RFC 8414/9728 fix the `.well-known` URLs and the metadata advertises the rest). **`/mcp` and the seven protocol routes are mounted UNCONDITIONALLY** since 4.4.0 — `mcp_enabled` picks the handler (404 JSON `mcp_disabled`), not the route table (D21). `/oauth/authorize` has NO backend route (SPA fallback serves it; a 405 would not fall through). Plus the SPA static fallback when `WEB_STATIC_ROOT` is set | `routes/mod.rs` (`nest("/v1", v1)` + unconditional `merge(mcp)` + `merge(oauth_protocol(state.mcp_enabled))`); note `/health` is ALSO mirrored at `/v1/health`, and `/v1/ready` exists | `grep -n "route\|nest\|mcp\|oauth" apps/api/src/routes/mod.rs`; `cargo test -p futurefin-api --test mcp_http -- mcp_disabled_answers_json_even_with_the_spa_mounted` |
| I5 | Reads never mutate (D5): expired liabilities filtered, never deleted, by GETs — since 3.4.0 the projection input query also filters them (fix C-10: an expired principal used to depress net worth forever, diverging from `/v1/summary`), pinned by `projection_excludes_expired_liability_principal` | WHERE clauses in liabilities/summary/budget/assets/projection handlers | `TEST_DATABASE_URL=... cargo test --workspace liabilities_purge` (en local; **desde 4.0.0 también en CI**, job `integration`) |
| I6 | In charts, the stacked per-asset areas sum EXACTLY to the (visible) net-worth line at every x | `MiniProjection.tsx` rescales each asset share by `visibleNw × (asset_i / Σassets)` — necessary because raw engine `net_worth = Σassets + surplus_cash − Σprincipals − undrained`, so raw `per_asset_series` does NOT sum to NW | Read the `cumulative` block in `apps/web/src/components/charts/MiniProjection.tsx` (~lines 164–190); any new stacked chart must reuse `MiniProjection`, not re-derive |
| I7 | `planning_monthly_cash_adjustment.len() == horizon_months`; allocation `target_index` in bounds; horizon ≥ 1 | Engine input validation → `EngineError::{InvalidPlanningAdjustments, InvalidAllocationRuleTarget, InvalidHorizon}` → 400 | `cargo test -p futurefin-engine` |
| I8 | Engine has zero I/O/async deps (purity, §1) | `crates/engine/Cargo.toml` deps are exactly: chrono, rust_decimal, serde, thiserror, uuid | `grep -E "tokio\|sqlx\|reqwest\|axum" crates/engine/Cargo.toml` → must be empty |
| I9 | Milestones, `milestones_real` and the FIRE crossover are computed on the FULL monthly series, never on density-decimated points | `handlers/projection.rs` (`points_full`, crossover loop over `output.net_worth`) — v1.4.2 incident: client deflated by array index instead of `month_index`, wrong under `hybrid` | `grep -n "points_full" apps/api/src/handlers/projection.rs` |
| I10 | SQLSTATE→HTTP mapping only in `error.rs` (D9) | `impl From<sqlx::Error> for ApiError` | `grep -rn "23505\|23503" apps/api/src/ --include=*.rs` → only `error.rs` |
| I11 | Body limits: 1 MiB global, 16 MiB on `/v1/backup/user-import*`, **1 MiB on `/mcp` fijado aparte**. `DefaultBodyLimit` actúa vía **extractores**, así que NO alcanza a `/mcp` (un `route_service` que lee el body con el tope del SDK, 4 MiB por defecto): hasta 4.4.0 este invariante era falso justo ahí (D21) | `routes/mod.rs` constants + `mcp::MCP_MAX_REQUEST_BODY_BYTES` vía `with_max_request_body_bytes` | `apps/api/tests/body_limits.rs` (local; la fila de `/mcp` es `oversized_mcp_body_returns_413`). **Toda ruta nueva que no pase por un extractor necesita su propia fila aquí** |
| I12 | No hardcoded hex colors; tokens `var(--ff-*)` only; icons only in `components/icons.tsx` | frontend convention (CLAUDE.md, design-system.md) | `grep -rn "#[0-9a-fA-F]\{6\}" apps/web/src/App.css apps/web/src/components/ \| grep -v icons.tsx` |
| I13 | Every response carries `X-Frame-Options: DENY` **except** trusted peer + `X-Ingress-Path`, which instead gets `Content-Security-Policy: frame-ancestors 'self'` **and no `X-Frame-Options`** (D17). The header alone never relaxes it | `handlers/frame.rs::frame_policy`, applied via `with_frame_policy` to the FINAL router (after the SPA fallback) in both `main.rs` and the test harness | `apps/api/tests/frame_options.rs` (both halves); `grep -n "X_FRAME_OPTIONS\|frame-ancestors" apps/api/src/handlers/frame.rs` |
| I14 | Identity from `X-Remote-User-*` is honored only with `FUTUREFIN_TRUSTED_PROXY_AUTH=1` **and** a peer in `FUTUREFIN_TRUSTED_PROXY_IPS`; `AUTH` without `IPS` panics at startup (D18) | `handlers/sso.rs` (`sso_disabled` / `sso_untrusted_peer`), `prefix.rs::PeerPolicy`, guard in `main.rs` | `apps/api/tests/sso_login.rs`; `grep -n "sso_disabled\|sso_untrusted_peer" apps/api/src/handlers/sso.rs`; `grep -n "TRUSTED_PROXY_AUTH=1 requires" apps/api/src/main.rs` |
| I15 | Without proxy headers the shell HTML is served **byte-identical** to the file on disk and the session cookie keeps `Path=/` — the compose deployment is unchanged by the subpath machinery | `handlers/spa.rs::inject` returns `Cow::Borrowed`; `handlers/auth.rs::session_cookie_path` | `apps/api/tests/base_path.rs`, `apps/api/tests/session_cookie_path.rs`; `cargo test -p futurefin-api --lib prefix::` |
| I16 | An HA-IdP login and an ingress header-SSO with the same HA user resolve to the **same** `users` row (`external_user_id`, one `resolve_or_provision`); and the HA refresh token is revoked before any DB write (D19) | `handlers/ha_sso.rs::ha_callback` step order; `handlers/sso.rs::resolve_or_provision` (single provisioning path) | `apps/api/tests/ha_idp_login.rs::header_sso_and_ha_login_resolve_to_the_same_user`; call-order assertion `[Exchange, Identity, Revoke]` in the same suite |
| I17 | **`/mcp` never carries `Access-Control-Allow-Credentials`**, and the API surface always does — one `CORS_ORIGINS` list, two layers (D21). Adding an origin for a browser MCP client must never grant cookie access to `/v1` | `routes/mod.rs` (`api_cors_layer`, applied **before** `merge(mcp)`) + `mcp::mcp_cors_layer` (applied with `route_layer`, never `layer`) | `apps/api/tests/mcp_http.rs::mcp_preflight_is_complete_and_grants_no_cookie_access` (asserts both halves: absent on `/mcp`, `true` on `/v1/backup/user-export`, same origin); `oauth_flow.rs::get_oauth_authorize_is_not_handled_by_the_api` guards the `layer`/`route_layer` half — a **401** there means the MCP auth escaped onto the fallback |
| I18 | A response never lets suppressed/capped/derived content be indistinguishable from "there is nothing here": every window, cap, suppression, scope or basis is echoed as a named field, never inferred from array length or absence (D22) | Field-level, no single choke point: `handlers/history.rs` (`window_truncated`, `fine_absent_reason`, `item_count`/`items_included`), `handlers/projection.rs` (`events_truncated`), `handlers/person_view.rs` (`LedgerView::as_str` echoed view), `mcp/server.rs` (pagination envelopes) | `apps/api/tests/context_fields.rs` (one test per field family); `grep -n "window_truncated\|fine_absent_reason\|items_included\|events_truncated" apps/api/src/handlers/` |

## 4. Known weak points (stated plainly, as of 2026-07-02)

- **W1 — RESOLVED in 4.0.0: the Postgres integration suite now runs in CI.** For most of this
  project's life it did not: `ci.yml` ran cargo build, `cargo test -p futurefin-engine`, npm
  typecheck+build and the Docker-stack scenarios, while `apps/api/tests/` — fire parity, cache,
  purge, body limits, unique violations, the tests guarding D5/D7/D8/I11 — needed a local
  `TEST_DATABASE_URL`, so a green CI did NOT mean those invariants held. The job `integration`
  closes that hole. Still run them locally before
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
  **Since v3.0.0 "keep it that way" stopped being a convention and became a physical
  impossibility** (D13): the database lives in the same container, so a second replica would not
  merely carry its own stale cache — it would carry its **own PostgreSQL and its own data
  directory**, two divergent installations behind one load balancer. Scaling replicas now requires
  extracting BOTH the database and the cache out of the process, in that order; doing only the
  cache is worse than doing nothing.
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
- **W8 — the container is a two-process bash supervisor** (new with D13, v3.0.0).
  `apps/api/docker-entrypoint.sh` is PID 1 and runs as **root** — only to `chown` and then `gosu`
  down: the postmaster runs as `postgres` (uid 999, matching the Debian official image), the API as
  `futurefin` (uid 10001). Neither workload ever runs as root. What this adds to the contract:
  - **Ordered shutdown is now an invariant, not a nicety.** `on_term` (trapped on TERM/INT) stops
    the **API first** with SIGTERM — which `main.rs` turns into
    `axum::serve(...).with_graceful_shutdown(shutdown_signal())` followed by `pool.close()`, i.e.
    the log pair `shutdown signal received — draining connections` → `database pool closed` — and
    **only then** sends **SIGINT to the postmaster**. SIGINT is PostgreSQL's *fast* shutdown
    (roll back, checkpoint, exit). **SIGTERM to a postmaster is *smart* shutdown**: it waits for
    clients to disconnect, indefinitely, so the container would hang until Docker SIGKILLed it
    mid-checkpoint. That is why the official image sets `STOPSIGNAL SIGINT`, and why the escalation
    here is SIGQUIT (immediate), never SIGKILL. Timeouts:
    `FUTUREFIN_API_STOP_TIMEOUT` (15 s) and `FUTUREFIN_PG_STOP_TIMEOUT` (30 s), under compose's
    `stop_grace_period: 60s`. **Watchtower ignores compose's grace period** — self-hosters running
    auto-updates must set `WATCHTOWER_TIMEOUT=60s` or every unattended update kills the postmaster
    mid-checkpoint.
  - **The entrypoint NEVER deletes a cluster.** Old or partial clusters are moved aside with `mv`
    (`$PGDATA/pgdata_old_<major>` after pg_upgrade; before 4.0.0 also
    `$STATE_DIR/failed-automigration-<ts>` after an interrupted automigration). The only `rm`s in the script are its own backups under retention
    and the pg_upgrade staging directory once its contents are safely copied in. A "cleanup" patch
    that turns any of those `mv`s into `rm -rf` is a data-loss patch.
  - **A dead process is a restart, not a repair.** `supervise` tears the other process down and
    exits 1 so `restart: unless-stopped` recovers the container. A restart loop here is a real
    incident, not self-healing.
  - **The weak point proper**: hundreds of lines of bash sit on the data path (adoption `chown`,
    collation REINDEX, pre-migration `pg_dump`, `pg_upgrade` swap; 4.0.0 removed the external-DB
    mode and its one-shot automigration, which shortened it). CI
    gates it with `shellcheck -S warning` and the `docker-stack` job exercises fresh install,
    watchtower-style recreate, clean shutdown, 2.x adoption, the two external-`DATABASE_URL`
    refusals and pg_upgrade 15→16 — but a bug in it loses data in a way **no Rust test can
    catch**. Treat every edit to `docker-entrypoint.sh` / `Dockerfile` / `docker-compose.yml` as
    Infra-release class (futurefin-change-control §1) and never merge one on a red or skipped
    `docker-stack` job.

## Provenance and maintenance

Written 2026-07-02 against branch `claude/skill-library-handoff-rtfotl` at v1.4.3, by reading the
files cited inline (not from memory of the docs — docs can drift, see W6). D13 and W8 written
2026-08-16 for **v3.0.0** against branch `claude/docker-self-contained-v3-skg8jm`, by reading
`apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh`, `docker-compose.yml`,
`apps/api/src/main.rs`, `apps/api/src/handlers/health.rs` and `.github/workflows/ci.yml`. D20
written 2026-08-28 for the Fase 3 (issue #84) train, branch `feat/mcp-fase-3-escritura-segura`, by
reading `apps/api/src/mcp/auth.rs`, `apps/api/src/confirm_token.rs`, `apps/api/src/heavy.rs`, the
four migrations under `apps/api/migrations/20260828*.sql` and the Fase 3 diff of
`apps/api/src/mcp/server.rs`. Same pass fixed a standing drift in D11: several docs (this one
included) still said `?months=` was *clamped* to 12–840 — it has been a **rejection**
(400 `months_out_of_range`) since 4.4.0, and the doc lagged for a while. Re-verify volatile claims with:

- Version: `grep -n '^version' apps/api/Cargo.toml` and top of `CHANGELOG.md` (4.2.1 on
  2026-08-27; 3.1.0 on 2026-08-17).
- Migration count: `ls apps/api/migrations/*.sql | wc -l` (49 on 2026-08-28, Fase 3/issue #84; 44 on 2026-08-27; 36 on 2026-08-17; 34 on 2026-08-16; 33 on 2026-07-07; 32 on 2026-07-06; 31 on 2026-07-02).
- Engine purity deps (I8): `grep -E "tokio|sqlx|reqwest|axum" crates/engine/Cargo.toml` → empty.
- Horizon rule (D11): `grep -n "fn validate_months_override\|months_out_of_range\|LIFESPAN_AGE\|FALLBACK_YEARS\|fallback_no_demographics" apps/api/src/handlers/projection.rs`. **`?months=` is a REJECTION since 4.4.0, not a clamp** — `clamp(12, 840)` as a live pattern now finds only the doc comment noting it was retired.
- Cache TTL/keys (D7): `grep -n "PROJECTION_CACHE_TTL\|ProjectionCacheKey\|invalidate_projection" apps/api/src/state.rs`.
- No-warm-up-after-mutation rationale: `grep -n -A6 "refresh_projection_after_mutation" apps/api/src/handlers/projection.rs`.
- f64 exception boundary (D4/I3): `grep -rn "serialize_decimal_as_f64\|serialize_decimal_as_chart_f64" apps/api/src/` (two separate definitions since Fase 5/issue #86 — `serialize_decimal_as_f64` in projection.rs, `serialize_decimal_as_chart_f64` in history.rs with the `CHART_DP`/`MONTH_FRACTION_DP` publication rounding) and `.claude/api-routes.md` §projection + §history series.
- Sink invariant (I1): `grep -n "sink_must_be_last" apps/api/src/handlers/allocation_rules.rs`.
- CI coverage (W1): `cat .github/workflows/ci.yml` — check whether a job now sets `TEST_DATABASE_URL`; if so, update W1 here and `.claude/tests.md` §CI.
- Session mechanics (D3): `grep -n "expires_at\|SESSION_COOKIE" apps/api/src/handlers/session.rs` and `grep -n "SESSION_TTL_DAYS" apps/api/src/main.rs`.
- Default ES brackets (W4): `grep -n -A24 "default_es_tax_brackets" apps/api/src/handlers/installation.rs` vs `DEFAULT_ES_TAX_BRACKETS_API` in `apps/web/src/lib/fire.ts`.
- App.tsx size (W2): `wc -l apps/web/src/App.tsx`.
- Doc drift (W6): the standing-errata table in futurefin-docs-and-writing §7 is the record.
- **D14 — API tokens + MCP (added 2026-08-16, v3.0.0)**: `grep -n "token_hash\|require_api_token" apps/api/src/handlers/api_tokens.rs`
  (hash-only storage, single 401); `grep -n "require_installation_member" apps/api/src/mcp/auth.rs`
  (live role per request); `grep -rn "_core\b" apps/api/src/mcp/server.rs` (tools call handler
  cores, no SQL in tools); `grep -n "mcp" apps/api/src/routes/mod.rs` (conditional mount);
  `grep -rn "api_tokens" apps/api/src/handlers/backup_user/` → **must be empty** (excluded from
  `.ffbackup` on purpose).
- **D15 — embedded OAuth 2.1 AS/RS (added 2026-08-17, v3.1.0)**:
  `grep -n "ACCESS_TOKEN_PREFIX\|REFRESH_TOKEN_PREFIX" apps/api/src/oauth/mod.rs` (prefixes);
  `grep -n "sha256_hex" apps/api/src/oauth/*.rs` (hash-only everywhere);
  `grep -n "now() + " apps/api/migrations/20260817090000_oauth.sql apps/api/src/oauth/token.rs`
  (Postgres computes expiries); `grep -n "revoked_reason" apps/api/src/oauth/token.rs` (reuse
  detection revokes the grant); `grep -n "oauth_grants_active_uniq" apps/api/migrations/20260817090000_oauth.sql`
  (partial-UNIQUE grant); `grep -n "UNAUTHORIZED" apps/api/src/mcp/auth.rs` (WWW-Authenticate
  only on 401); `grep -rn "oauth" apps/api/src/handlers/backup_user/` → **must be empty**;
  `grep -n "oauth/authorize" apps/api/src/routes/mod.rs apps/api/src/oauth/mod.rs` → no backend
  route registered at that path (only the /v1 consent endpoints);
  `grep -n "connections" apps/api/src/handlers/oauth_consent.rs` (panel mounted unconditionally).
- **D13 — the image is the store**: `grep -n '^FROM' apps/api/Dockerfile` (runtime is
  `debian:bookworm-slim`; `postgres:15/16-bookworm` appear only as `AS pg15`/`AS pg16` COPY
  sources); `grep -n '^VOLUME' apps/api/Dockerfile` → **must be empty** (the only `VOLUME` hits in
  that file are the header comment explaining why there is none);
  `grep -n 'HEALTHCHECK' -A2 apps/api/Dockerfile` and the `healthcheck:` block of
  `docker-compose.yml` (both `/v1/ready`, both without `</dev/tcp`);
  `awk '/^services:/{f=1;next} /^volumes:/{f=0} f && /^  [a-z]/' docker-compose.yml` → one service.
- **D13 — socket-only trust auth**: `grep -n 'auth-local=trust\|listen_addresses\|unix_socket_directories' apps/api/docker-entrypoint.sh`.
- **W8 — volume guard / no-delete / SIGINT**:
  `grep -n 'no persistent volume' apps/api/docker-entrypoint.sh`;
  `grep -n 'rm -rf "\$PGDATA"' apps/api/docker-entrypoint.sh` → **must be empty**;
  `grep -n 'stop_pid "\$PG_PID" INT' apps/api/docker-entrypoint.sh` (two call sites: `on_term`
  and `supervise`); `grep -n 'stop_grace_period' docker-compose.yml`.
- **W8 — API graceful shutdown**: `grep -n 'with_graceful_shutdown\|pool closed\|draining connections' apps/api/src/main.rs`.
- **W8 — CI coverage of the container paths**: `grep -n '^      - name:' .github/workflows/ci.yml`
  (job `docker-stack`: image sanity + no-volume guard, fresh install, watchtower-style recreate,
  clean shutdown, 2.x adoption, the two external-`DATABASE_URL` refusals — 4.0.0 replaced the old
  compat/automigration scenarios with them — and pg_upgrade 15→16) and
  `ls .github/testdata/`.
- **D17 — conditional anti-clickjacking (added 2026-08-27, branch `feat/home-assistant-addon`)**:
  `grep -n "X_FRAME_OPTIONS\|frame-ancestors\|trusted_peers.allows" apps/api/src/handlers/frame.rs`
  (relaxation gated on peer AND header; XFO removed, not accompanied);
  `grep -n "with_frame_policy" apps/api/src/main.rs apps/api/tests/common/mod.rs` (same layer in
  binary and harness, applied to the final router); `apps/api/tests/frame_options.rs` pins both
  halves.
- **D17/D18 — peer policy and prefix precedence**: `grep -n "enum PeerPolicy\|fn allows\|fn request_prefix\|X_INGRESS_PATH\|X_FORWARDED_PREFIX" apps/api/src/prefix.rs`
  (default `Disabled`; precedence `X-Ingress-Path` > `X-Forwarded-Prefix` > `FUTUREFIN_BASE_PATH` >
  `""`); `cargo test -p futurefin-api --lib prefix::` (unit tests for normalization + precedence).
- **D18 — header SSO double opt-in (added 2026-08-27)**:
  `grep -n "sso_disabled\|sso_untrusted_peer\|sso_bad_identity" apps/api/src/handlers/sso.rs`;
  `grep -n "TRUSTED_PROXY_AUTH=1 requires" apps/api/src/main.rs` (startup panic when AUTH is set
  without IPS); `grep -n "sso" apps/api/src/routes/mod.rs` (route mounted unconditionally);
  `grep -n "sso_account_no_password" apps/api/src/handlers/auth.rs` (2 call sites: login and
  password change); `grep -n "external_user_id\|DROP NOT NULL" apps/api/migrations/20260827120000_users_trusted_header_identity.sql`;
  `grep -rn "external_user_id" apps/api/src/handlers/backup_user/` → **must be empty** (the SSO
  identity is not part of `.ffbackup`).
- **D13 amendment — the HA add-on channel (added 2026-08-27)**: `cat repository.yaml`;
  `grep -n "^image:\|^version:\|init:\|backup:\|ingress\|^ports:" addon/futurefin/config.yaml`
  (prebuilt GHCR image, `init: false`, `backup: cold`, ingress 8080, direct port `null`);
  `grep -n "options.json\|PGDATA=/data/pgdata\|FUTUREFIN_STATE_DIR=/data/state\|is_persisted" apps/api/docker-entrypoint.sh`;
  `grep -n "Guardia de config" -A20 .github/workflows/ci.yml` (the store-shape guard);
  `find . -name .git -prune -o -name 'config.yaml' -o -name 'config.yml' -o -name 'config.json' -print`
  → exactly `.github/ISSUE_TEMPLATE/config.yml` and `addon/futurefin/config.yaml`.
- **D20 — MCP write safety: audit, scope, two-phase confirm (added 2026-08-28, Fase 3/issue #84,
  4.4.0)**: `grep -n "outcome\|settled_at\|CHECK" apps/api/migrations/20260828140000_mcp_write_audit.sql`
  (the write-once shape); `grep -n "pub async fn settle" -A20 apps/api/src/mcp/auth.rs` (`WHERE id =
  \$1 AND settled_at IS NULL`); `grep -c 'settled(&self.state.pool, audit' apps/api/src/mcp/server.rs`
  → must equal the write count (31); `grep -n "TokenScope\|can_write" apps/api/src/handlers/api_tokens.rs`
  (scope reads live, subtractive only); `grep -n "evaluate_write_gate" -A15 apps/api/src/mcp/auth.rs`
  (three gates in order: role → scope → toggle); `grep -n "scopes_supported" apps/api/src/oauth/metadata.rs`
  (still absent, reasoning updated); `cat apps/api/src/confirm_token.rs | grep -n "pub fn digest\|pub async fn issue\|pub async fn consume"`
  (canonical-order hash, single-use `consumed_at`, TTL 10 min); `grep -c 'confirm_token.as_deref()' apps/api/src/mcp/server.rs`
  → 7 (the token-gated subset); `grep -n "fn projection_permits" -A10 apps/api/src/heavy.rs` (third
  semaphore, floor 2 ceiling 8, same `available_parallelism()` pattern as the KDF one).
- **D21 + I17 — MCP transport: kill-switch shape, CORS split, `Origin`, body cap, issuer subpath
  (added 2026-08-28, Fase 4/issue #85, 4.4.0)**:
  `grep -n "MCP_DISABLED_MESSAGE\|fn mcp_disabled" apps/api/src/mcp/mod.rs` and
  `grep -n "fn oauth_disabled\|oauth_protocol_router" apps/api/src/oauth/mod.rs` (routes mounted
  either way; the switch picks the handler); `grep -n "route_layer\|\.layer(" apps/api/src/mcp/mod.rs`
  → **`route_layer` only**, never `layer` (a `layer` here escapes onto the merged fallback);
  `grep -rn '\.allow_credentials(' apps/api/src/` → **exactly one hit, in `routes/mod.rs`**
  (`mcp/mod.rs` mentions it only in prose); `grep -n "fn api_cors_layer\|fn cors_origins" apps/api/src/routes/mod.rs`;
  `grep -n "with_allowed_origins\|disable_allowed_hosts\|with_max_request_body_bytes" apps/api/src/mcp/mod.rs`
  (all three present: Host off, Origin on, body cap explicit);
  `grep -n "normalize_prefix" apps/api/src/main.rs` (the issuer accepts a subpath, validated by the
  same function as `FUTUREFIN_BASE_PATH`);
  `grep -n "CACHE_CONTROL\|VARY\|ISSUER_VARY" apps/api/src/oauth/metadata.rs` (`no-store` + `Vary`);
  `grep -n "fn gc_expired_tokens" -A8 apps/api/src/oauth/token.rs` (lazy GC in the POST, D5);
  `grep -n "fn mount_static_spa" apps/api/src/handlers/spa.rs` — **must be called from both
  `main.rs` and `apps/api/tests/common/mod.rs`**, which is the mechanism that keeps the tested
  router the same shape as the shipped one.
- **D22 + I18 — suppressed data declares itself (added 2026-08-28, Fase 5/issue #86, 4.4.0)**:
  `grep -n "DEFAULT_HISTORY_WINDOW_MONTHS\|MAX_HISTORY_WINDOW_MONTHS\|window_truncated" apps/api/src/handlers/history.rs`
  (series default 120, max 1200, echoed truncation flag); `grep -n "MAX_FINE_CURVE_WINDOW_MONTHS\|fine_absent_reason" apps/api/src/handlers/history.rs`
  (cashflow fine-curve cap at 36 months, four named reasons); `grep -n "item_count\|items_included" apps/api/src/handlers/history.rs`
  (`SnapshotResponse` suppression flag); `grep -n "PROJECTION_EVENTS_MAX\|events_truncated" apps/api/src/handlers/projection.rs`
  (dated-flow events, capped at 100); `grep -n "fn as_str" -A6 apps/api/src/handlers/person_view.rs`
  (`LedgerView::as_str`, the view-echo helper, and its round-trip test
  `as_str_round_trips_through_resolve`); `grep -n "NOTA-VIEW-ENVELOPE" -A20 apps/api/src/mcp/server.rs`
  (why 7 of the paginating/enveloping list tools wrap their content while `list_categories` and
  `list_recurring_rules` stay bare arrays); `grep -n "type_tag" apps/api/src/handlers/summary.rs`
  (the `Option<String>` change, `null` replacing the `"(sin etiqueta)"` literal);
  `cargo test -p futurefin-api --test context_fields` (the 11-test contract suite).

Update this skill whenever: a decision above is overturned (record the new incident), a new
cross-cutting mechanism appears (cache backend, auth scheme, second crate consumer of the
engine), the container's process model or shutdown contract changes (D13/W8), the set of
distribution channels changes (D13 amendment), or CI starts running the Postgres integration suite.
