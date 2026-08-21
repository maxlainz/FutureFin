---
name: futurefin-change-control
description: >
  Load this skill BEFORE making, reviewing, or merging ANY change to FutureFin — code, schema,
  API, UI, docs or release. It defines how changes are classified and gated: which tests must
  pass, which docs must be updated, and what evidence is required per change type. Triggers:
  "can I change/remove/rename this field", "write a migration", "edit a migration", "drop a
  column", "bump the version", "release", "tag", "merge to main", "publish the image", "is this
  breaking", "backup schema_version", "checksum mismatch", "_sqlx_migrations", "f64 vs Decimal",
  "hardcoded hex/color", "fire-parity fixture", "pre-merge checklist". Do NOT use it for HOW to
  run tests (futurefin-validation-and-qa), HOW to deploy/rollback (futurefin-run-and-operate),
  WHY the architecture is shaped this way (futurefin-architecture-contract), or debugging a
  failure (futurefin-debugging-playbook) — this skill is the gatekeeper, not the manual.
---

# FutureFin Change Control

How changes are classified, gated and reviewed in this repo. Counts refreshed **2026-08-20 for
the 3.8.0 train** (issue #4, ergonomía MCP: **28** integration-test files in `apps/api/tests/`
— el nuevo es `allocation_resolution.rs` —, **40** migraciones **sin cambios** (ningún bloque
del tren añade columnas), `.ffbackup` sigue en **8**, y el catálogo MCP pasa de 47 a **50**
tools. `apps/api/Cargo.toml` pasa a `3.8.0`, la versión que publica el tren). Antes,
**2026-08-19 for the 3.7.0 train** (fusión de la cuota de pasivo en el presupuesto: `budget_derived.rs` renombrado
a `budget_liability_quotas.rs`, 5 → 10 tests; **40** migraciones y **27** ficheros de test sin
cambios; `.ffbackup` sigue en **8** — la reforma no toca datos almacenados). Antes, **2026-08-19 for
the 3.6.0 train** (conciliación de transferencias + paridad MCP; 3.5.0 se cerró en el CHANGELOG
pero nunca se tagueó — 3.6.0 es la imagen que lo publica todo): **40** migration files in
`apps/api/migrations/` (la 40ª es `20260819120000_transactions_transfer_reconciliation`), **27**
integration-test files in `apps/api/tests/` (nuevo `transactions_reconcile.rs`), `.ffbackup`
`CURRENT_SCHEMA_VERSION` = **8** (v8 añade el par conciliado por índice + los rechazos del
auto-matcher; los ficheros antiguos siguen importando vía `payload_v7_to_v8`). All paths below
are from the repo root. (Previously stamped 2026-08-18 at v3.4.0 with 39/26 and schema 7;
2026-08-17 at v3.2.0 with 37/23; 2026-08-17 at v3.1.0 with 36/23 and schema 6; 2026-08-16 at
v3.0.0 with 34/20; and 2026-07-06 at v1.5.0: 32 migrations, 11 test files.) `apps/api/Cargo.toml`
reads `3.7.0` and `CHANGELOG.md` carries its `## [3.7.0] - 2026-08-19` section, so the Section 4
version/CHANGELOG gates for this release are satisfied.

Vocabulary (defined once): **installation** = the singleton row all financial data belongs to
(one per deployment). **scope / view** = `?view=mine` filters ledger queries by
`owner_user_id = current_user`; default `household` is the full installation. **FIRE target /
gross-up / SWR** = the retirement net-worth target: annual net need is grossed-up through
capital-gains tax brackets, then divided by the Safe Withdrawal Rate — see
`.claude/skills/futurefin-fire-domain-reference/SKILL.md`. **nominal vs real** = the engine
simulates in nominal euros; only the FIRE target grows with inflation (v1.2.0 model).
**cascade** = allocation rules distributing the monthly surplus ("sobrante") across assets.

## When NOT to use this skill

- Writing or running tests, TestApp harness, fixture mechanics → `.claude/skills/futurefin-validation-and-qa/SKILL.md`
- Deploy, upgrade, rollback, backups in production → `.claude/skills/futurefin-run-and-operate/SKILL.md`
- Setting up the dev environment from scratch → `.claude/skills/futurefin-build-and-env/SKILL.md`
- Why a design decision exists / whether to revisit it → `.claude/skills/futurefin-architecture-contract/SKILL.md`
- A bug you are triaging → `.claude/skills/futurefin-debugging-playbook/SKILL.md`
- Full incident history → `.claude/skills/futurefin-failure-archaeology/SKILL.md`
- CHANGELOG/doc writing style → `.claude/skills/futurefin-docs-and-writing/SKILL.md`

## 1. Change classification and gates

Classify every change into the FIRST matching row (a change may hit several rows — apply the
union of gates). "Integration tests" means `TEST_DATABASE_URL=… cargo test --workspace` against
a real Postgres (Section 6); they are **not** run in CI, so running them locally is YOUR gate.

| Class | Examples | Mandatory gates |
|---|---|---|
| **Engine math** | `crates/engine/src/projection.rs`, FIRE target, cascade, retirement drain, inflation | `cargo test -p futurefin-engine`; integration tests (esp. `projection_marker.rs`, `fire_parity.rs`, `projection_cache.rs`); if tax brackets / gross-up / target formula changed → regenerate `apps/api/tests/fixtures/fire-parity.json` expected values and BOTH suites green (Section 2.5); update `.claude/engine.md`; CHANGELOG entry with before/after numeric example. Errors here are **silent** (plausible-but-wrong numbers) — this is the owner's stated hardest problem; show worked numbers as review evidence, not just green tests. |
| **API contract** | Handler request/response fields, status codes, routes, MCP tools | Breaking-change policy check (Section 5); update `#[utoipa::path]` annotations (OpenAPI is generated); update `.claude/api-routes.md`; **MCP parity evaluation** (`.claude/skills/futurefin-mcp-parity/SKILL.md` §1 — the change must end in tool added/updated, deliberate omission recorded in that skill's register, or n/a; never silent); integration tests covering the new/changed shape (for a tool: the mcp_write quartet + frozen catalog); CHANGELOG (with a "breaking" note if applicable). |
| **DB migration** | New file in `apps/api/migrations/` | Section 3 in full (never edit shipped; data-loss sign-off; grep for query drift); integration tests — every test applies ALL migrations to a fresh schema, so a broken migration fails everything; update `.claude/data-model.md`; if exported tables change shape → check `apps/api/src/handlers/backup_user/` (Section 5) and the export SQL. |
| **UI-visual** | Anything in `apps/web/src/` that renders | `npm run typecheck:web && npm run lint:web && npm run build:web && npm test --workspace futurefin-web`; verify **light AND dark theme** before merging (owner-mandated); tokens only, no hex (Section 2.4); icons only in `components/icons.tsx`; small charts via `MiniProjection`; update `.claude/design-system.md` / `.claude/frontend-structure.md` if conventions moved; CHANGELOG. |
| **Docs-only** | `CLAUDE.md`, `.claude/*.md`, `README.md`, CHANGELOG wording | No test gates. Gate = accuracy: verify every command/path/claim against the code before writing it (docs have drifted before — eight errata were found and fixed on 2026-07-02; prefer commands over frozen counts, e.g. `ls apps/api/migrations \| wc -l`). Record unfixable drift in futurefin-docs-and-writing §7. |
| **Infra-release** | `Dockerfile`, `apps/api/docker-entrypoint.sh`, `docker-compose*.yml`, `.github/workflows/*`, version bump, tag | CI green on `dev` — **including the `docker-stack` job, which since 3.0.0 is the only automated evidence of "no data loss"; never merge on a red or skipped one**; full local Docker-stack test (Section 4.2) before tagging, **plus the V2→V3 upgrade drill with real seeded data** (4.2, step B) — the published image now carries the database, so a bad entrypoint destroys installations that never ran your code path in CI; version bump + `Cargo.lock` sync + CHANGELOG; dev→main full-mirror merge; tag from `main`. Any edit to `docker-entrypoint.sh` also needs `shellcheck -S warning` clean (CI gates it). |

CI (`.github/workflows/ci.yml`, runs on push/PR to `main` and `dev`) covers three jobs:
- `rust` — `cargo build -p futurefin-api --locked`, `cargo test -p futurefin-engine --locked`.
- `web` — `npm run typecheck:web` + `npm run build:web`.
- `docker-stack` — `shellcheck` on the entrypoint and `scripts/*.sh`, an image build, then the
  container's real data paths: image sanity + **no-volume guard** (a volume-less `docker run` must
  abort), fresh install → `/v1/ready` + seeded data, watchtower-style `--force-recreate` keeping
  data, **clean shutdown** (drain → pool closed → PG checkpoint → exit 0), **V2→V3 over a real 2.3.0
  stack** (uid adoption + collation REINDEX + duplicate-register detector + automatic
  pre-migration backup), the 3.x image over an untouched 2.x compose (deprecated external-DB
  compat), one-shot **automigration** from an external DB, and **pg_upgrade 15→16** verified by row
  census.

CI does **not** run: Postgres integration tests, `npm run lint:web`, or frontend Vitest. Those
are local gates you must run yourself (Section 6).

## 2. Non-negotiables (each with rationale and incident)

Never contradict these. If a task seems to require violating one, stop and re-plan.

### 2.1 Money is `Decimal`, never `f64` — with ONE deliberate wire exception
All monetary values in domain, engine, handlers and schema are `rust_decimal::Decimal`; the API
serializes amounts as decimal **strings**; the frontend never parses them to float for
arithmetic. RATIONALE: float rounding compounds over an ~840-month simulation and across tax
gross-up; decimal-string round-trips are bit-exact. **The exception (v1.4.0, deliberate,
boundary documented in `.claude/api-routes.md`):** the large arrays in
`GET /v1/projection/series` (`points[].net_worth`, `points[].contributed_capital`,
`fire_target_series`, `asset_series[].values`) are serialized as `f64` for wire size (~30 KB
less JSON, ~5,000 fewer client parses; measured precision <1 € over 70 years). Scalars and KPIs
(`starting_net_worth`, `jubilacion_target_net_worth`, milestones) stay Decimal-as-string. Do
not extend the f64 exception to any scalar or any value used in arithmetic.

### 2.2 Reads never mutate
GET handlers must not write. Expired liabilities (`payment_end_date < today`) are **filtered**
(`WHERE payment_end_date IS NULL OR payment_end_date >= $today`), never deleted. INCIDENT
(fixed v1.3.0): the legacy `purge_expired_liabilities` function was called from 6 GET handlers
— `GET /v1/liabilities`, `/summary`, `/budget`, `/assets`, `/projection` silently issued
`DELETE` statements, violating HTTP semantics, destroying audit data and impeding caching. It
was removed; `apps/api/tests/liabilities_purge.rs` pins the filtered-not-deleted behavior.

### 2.3 LedgerView scope helpers — never hand-write the two branches
Any handler filtering by view must use `LedgerView::scope_where(table_alias)`,
`next_arg_index()`, `bind_scope_as` / `bind_scope_scalar` from
`apps/api/src/handlers/person_view.rs`. INCIDENT (v1.3.0): six handlers hand-wrote
`match view { Household => "...$1", Mine => "...$1 AND owner_user_id = $2" }`; a live bug in
`budget.rs` had **inverted bind order between the branches** in the derived-from-liabilities
query. The helpers enforce consistent placeholder ordering and killed that bug class (~500 LOC
removed). Also: `?view=mine` is a client-side filter, **not** an authorization boundary — never
rely on it for access control.

### 2.4 No hardcoded hex in CSS/components
Consume `var(--ff-*)` / `var(--proj-*)` tokens from `apps/web/src/styles/theme.css`. RATIONALE:
the app has light/dark/auto themes via `<html data-theme>`; a hardcoded color renders correctly
in one theme and breaks in the other. INCIDENT (v1.4.0 redesign): the projection chart tooltip
showed dark text on a dark background in dark mode; the fix tokenized the whole chart palette
(`--proj-area-1..10`) and left exactly one documented literal (the theme-independent tooltip).
Verify both themes before merging anything visual.

### 2.5 fire-parity fixture regeneration rule
FIRE math is deliberately duplicated (server = source of truth in `/v1/projection/series`;
client = live form preview without a round-trip). `apps/api/tests/fixtures/fire-parity.json`
is the single canonical fixture consumed by BOTH `apps/api/tests/fire_parity.rs` (Rust) and
`apps/web/src/lib/fire.test.ts` (TS), asserting the same target ±1 €. RULE: if you change tax
brackets, the gross-up formula, or the FIRE target contract on EITHER side, regenerate the
`expected_target_nw` values in the JSON and get BOTH suites green. One suite failing alone =
drift detected — that is the fixture doing its job, not a flaky test. RELATED INCIDENT
(v1.3.0): `RetirementView` fed `expense_regular_monthly_equivalent` where the server used
`expense_retirement_monthly_equivalent` → 2–3× divergence between form preview and real target.

### 2.6 Strict deserialization — reject, don't default
Unknown enum values in requests must 422, not silently fall back. INCIDENT (fixed v1.3.0):
`fire_number_mode: "foobar"` used to silently coerce to the default mode — the user's FIRE
target was computed with a mode they did not choose, with no error. `FireNumberMode` now has a
strict custom `Deserialize` (`apps/api/src/handlers/installation.rs`); pinned by
`apps/api/tests/installation_patch.rs`. Follow this pattern for any new enum field.

### 2.7 No migration auto-repair
`sqlx::migrate!().run()` runs plain; a checksum mismatch **fails loud** at startup. INCIDENT
(removed v1.3.0): the old `IDEMPOTENT_MIGRATION_REPAIR_VERSIONS` loop (12 rounds of
checksum-repair) silently masked real drift between the embedded migrations and the DB. Never
reintroduce auto-repair. Manual fix, only when the file change is genuinely idempotent:

```bash
# Production (3.x, single container, DB on a Unix socket — there is no TCP port and no password):
docker compose exec futurefin \
  psql -h /var/run/postgresql -U futurefin -d futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>"
docker compose restart futurefin

# Split-dev (host-side API against docker-compose.dev.yml):
psql "$DATABASE_URL" -c "DELETE FROM _sqlx_migrations WHERE version = <X>"
```

If the container will not stay up long enough to `exec` into it, bring PostgreSQL up **without**
the API and do it there: `docker compose run --rm -e FUTUREFIN_MODE=db-only futurefin` (the same
rescue mode `scripts/restore-postgres.sh` uses). Never script either of these.

### 2.8 The container's data-safety rules (3.0.0) — the entrypoint is on the data path
Since 3.0.0 the published image carries PostgreSQL, so `apps/api/docker-entrypoint.sh`,
`apps/api/Dockerfile` and `docker-compose.yml` are **data-integrity code**, not packaging. Three
rules are non-negotiable; each exists because breaking it loses data silently
(full story: futurefin-failure-archaeology §2.11; normative form: futurefin-architecture-contract
D13/W8):

- **The entrypoint NEVER deletes a cluster — only moves it aside.** Old or partial clusters go to
  `$PGDATA/pgdata_old_<major>` or `$STATE_DIR/failed-automigration-<ts>` via `mv`. The only `rm`s
  in the script are its own backups under retention and the pg_upgrade staging once copied. A
  review that finds a new `rm -rf` touching `$PGDATA` blocks the change.
- **The image declares no `VOLUME`, and the runtime is not based on `postgres:*`.** The inherited
  `VOLUME` of the official images creates anonymous volumes on `docker run` without `-v`, and
  watchtower drops them on recreate — total, silent loss. The `mountpoint` guard that refuses to
  start without a real volume only works while nothing pre-mounts one.
- **The postmaster is stopped with SIGINT (fast), after the API has drained.** SIGTERM to a
  postmaster is *smart* shutdown and waits for clients forever; escalation is SIGQUIT, never
  SIGKILL. Do not remove `stop_grace_period: 60s` from compose, and keep the healthcheck as
  `CMD-SHELL` on `/v1/ready` **without** a `</dev/tcp` fallback.

Corollary for reviewers: a diff that touches any of these three files is Infra-release class even
if it "only changes a comment" — re-run the `docker-stack` job.

## 3. Migration discipline

Owner-confirmed rules, previously unwritten — now they ARE written:

1. **Never edit an already-shipped migration.** Shipped = present in any tagged release or
   applied to any real DB. Editing changes the checksum → every deployed instance fails loud on
   next start (see 2.7). Fix forward with a NEW migration.
2. **Filename format**: `YYYYMMDDHHMMSS_description.sql` in `apps/api/migrations/` (e.g.
   `20260520120000_inflation_always_on.sql`). Timestamp must sort after every existing file.
   Migrations run automatically on API startup (`db::run_migrations`).
3. **Data-losing migrations require explicit owner (maxlainz) sign-off + CHANGELOG
   documentation.** Signed-off precedent: v1.1.0's
   `20260519120100_drop_asset_contribution_columns.sql` cleanly dropped 5 per-asset
   contribution columns with NO data migration (users reconfigure as allocation rules); the
   CHANGELOG documents the loss and the recovery path, and the backup `schema_version` was
   bumped to 3 with a migrate chain. If you cannot get sign-off in-session, do not merge —
   leave the migration on a branch with the CHANGELOG draft.
4. **Test against real-shaped data**, not an empty schema. Minimum: run the integration suite
   (each test applies all migrations to a fresh schema), then load realistic rows (import a
   `.ffbackup`, or seed via the API as `fire_parity.rs` does) and exercise the affected
   endpoints. Empty-table migrations hide `NOT NULL`-backfill and FK failures.
5. **Grep for query drift after dropping/renaming columns.** INCIDENT (v1.0.10): backup export
   500'd because its SQL still selected `b.label`/`b.frequency` after
   `20260505180000_budget_entries_monthly_only` dropped them. SQLx queries here are runtime
   strings — the compiler will not catch this. After any column change:
   `grep -rn "<column_name>" apps/api/src/` and fix every hit, especially
   `handlers/backup_user/` and `handlers/backup.rs`-style export SQL.
6. **Checksum mismatch handling**: fails loud by design; manual `DELETE FROM _sqlx_migrations
   WHERE version = X` only for genuinely idempotent dev-time fixes (see 2.7). Never script it.

## 4. Release discipline

### 4.1 dev→main mirror flow (quoted from CLAUDE.md — follow verbatim)

`main` is the production/publishing branch and a **full mirror** of `dev` (no divergence, no
branch-exclusive files). CLAUDE.md, "Releases":

> 1. Desarrollar en `dev`, hacer commit y push.
> 2. Bumpar versión en `apps/api/Cargo.toml` (sincronizar `Cargo.lock`) y añadir entrada en `CHANGELOG.md`.
> 3. **Merge completo `dev` → `main`** (`git checkout main && git merge dev`). Nunca copias parciales de archivos: `main` debe quedar idéntico a `dev`.
> 4. Push tag `vX.Y.Z` **desde `main`** → el workflow `publish-image.yml` (que vive en `main`) publica la imagen.
> 5. Volver a `dev` (`git checkout dev`) y seguir; mantener `dev` al día con `main`.

The tag push triggers `.github/workflows/publish-image.yml`: multi-arch (amd64+arm64) image to
GHCR (always) and Docker Hub `maxlainz/futurefin` (if secrets set), tags `:X.Y.Z`, `:X.Y`,
`:X`, `:latest`. Version bump: edit `version` in `apps/api/Cargo.toml`, then sync the lockfile
(`cargo build` or `cargo update -p futurefin-api` regenerates `Cargo.lock`) — a tag whose
`Cargo.lock` disagrees with `Cargo.toml` breaks `--locked` builds in CI.

### 4.2 Before tagging: full local Docker-stack test (owner-mandated)

Since 3.0.0 the image *is* the stack — one container, PostgreSQL inside it. Three drills, in
order. **A is always required; B is required for any Infra-release; C at least once per release
train that changes migrations or the entrypoint.**

**A — fresh stack from the locally built image.**

```bash
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .
# .env only needs: FUTUREFIN_IMAGE=futurefin-local, FUTUREFIN_TAG=dev
# (POSTGRES_PASSWORD is NO LONGER required — the embedded DB is socket-only.)
# TRAP: leave DATABASE_URL commented out. A split-dev .env with it uncommented flips the
# image into deprecated external-DB mode and you will test the wrong code path.
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d
curl -sf http://127.0.0.1:8080/v1/ready          # readiness, not /v1/health: it round-trips the DB
docker compose logs futurefin | grep -E "migrations applied|initializing fresh PostgreSQL"
```

Then click through the app once (light **and** dark, 4.3), and finish with a clean stop to prove
the shutdown contract holds with your changes:

```bash
docker compose stop -t 60 futurefin
docker compose logs futurefin | grep -E "database pool closed|database system is shut down|clean shutdown complete"
docker inspect -f '{{.State.ExitCode}}' futurefin      # must be 0
```

**B — the V2→V3 upgrade drill with real data (do this before tagging any Infra-release).**
Mirror what CI does, on your machine, so you see the adoption path a real installation will take:
bring up the frozen 2.x topology from `.github/testdata/docker-compose.v2.yml`, seed it through
the API (register a user, create a category with accented characters), then swap in the new
compose and let the entrypoint adopt the volume.

```bash
# 1. Real 2.x stack (two containers, image pinned to 2.3.0)
POSTGRES_PASSWORD=drill docker compose -p futurefin -f .github/testdata/docker-compose.v2.yml up -d
# … register + log in + create data via curl or the UI at :8080 …

# 2. Migrate to the 3.x compose reusing the same `pgdata` volume
POSTGRES_PASSWORD=drill docker compose -p futurefin -f .github/testdata/docker-compose.v2.yml down --remove-orphans
FUTUREFIN_IMAGE=futurefin-local FUTUREFIN_TAG=dev \
  docker compose -p futurefin -f docker-compose.yml -f docker-compose.local.yml up -d --remove-orphans
#   (`--remove-orphans` is what retires the old `futurefin-database` container — the same step a
#    real self-hoster runs; the `local.yml` override keeps compose from pulling your local tag.)
curl -sf http://127.0.0.1:8080/v1/ready

# 3. What must be true afterwards
docker compose -p futurefin logs futurefin | grep -E "adopting ownership of PGDATA|reindexing database after adoption"
#    …log in with the SAME credentials and see the SAME data…
#    …re-registering an existing username must return 409/422 (unique index survived the
#      musl→glibc collation change — failure-archaeology §2.11 trap 3)…
docker compose -p futurefin exec -T futurefin sh -c 'ls /var/lib/futurefin/backups/pre-migration-*.sql.gz'
```

**C — restore drill.** Take one of those automatic dumps (or `./scripts/backup-postgres.sh`) and
put it back with `./scripts/restore-postgres.sh backups/<file>.sql.gz`. The script stops the
service, starts a rescue container in `FUTUREFIN_MODE=db-only`, prints a row census **before and
after**, restarts the stack and waits for `/v1/ready`. Compare the two censuses — that is the
evidence, not the absence of an error message.

Full flow, traps and the rebuild loop: futurefin-build-and-env §4. Operational detail on backups,
`db-only` mode and rollback: futurefin-run-and-operate.

### 4.3 Visual changes: verify light AND dark

Owner-mandated, also in CLAUDE.md: before merging any visual change, check both themes
(`Ajustes → Datos y sistema → Apariencia`, or set `<html data-theme="dark|light">` in
devtools). Rationale in 2.4.

## 5. Breaking-change policy

No breaking change ships without a version bump + a CHANGELOG entry explicitly marked
**breaking**. What counts as breaking:

- **API**: removing/renaming a request or response field; changing nullability or type;
  rejecting previously-accepted input. Precedent — v1.2.0 CHANGELOG has explicit "API breaking"
  and "Engine breaking" notes: `PATCH /v1/installation` stopped accepting
  `projection_includes_inflation`, `annual_inflation_assumption_percent` went from nullable to
  required-when-sent, and the response dropped a field. v1.3.0 shows the boundary case: it
  documented even the removal of a consumer-less field (`fire_number_expense_adjustment_pct`)
  and the new strict 422 as the only non-bit-exact changes.
- **`.ffbackup` schema**: any shape change to exported user data requires bumping
  `CURRENT_SCHEMA_VERSION` in `apps/api/src/handlers/backup_user/schema.rs` (currently **8**: v4
  added `snapshots` via `payload_v3_to_v4` (v1.5.0), v5 added transactions/imports/rules (v1.6.0),
  v6 added `recurring_transaction_rules` + `BackupTransaction.recurring_rule_index` via
  `payload_v5_to_v6` (v1.8.0), v7 — the first NON-additive bump — dropped `day_of_month` from
  recurring rules via `payload_v6_to_v7` (3.2.0), and v8 added the reconciled transfer pairing +
  `transfer_match_rejections` via `payload_v7_to_v8` (3.5.0)) and extending the
  `migrate_to_current` chain so ALL older versions still import (v1..v7 remain importable today;
  dropped fields are discarded on import — the documented v1.1.0 pattern). `parse_payload` rejects versions
  newer than the server supports — keep that. Never break import of an old backup; it is users'
  only recovery path.
- **Engine input struct** (`ProjectionInput` and friends in `crates/engine`): field
  removals/replacements are breaking for the handler layer and must be noted (precedent:
  v1.2.0 replaced `inflation_annual_percent` + `fire_target_net_worth` with
  `fire_target: Option<FireTarget>`).

Non-breaking by convention: additive optional response fields (e.g. `milestones_real` in
v1.4.2, `fire_target_series` in v1.2.0), new endpoints, new optional request fields.

## 6. Pre-merge checklist (execute literally, from repo root)

```bash
# 0. Fresh base (CLAUDE.md rule: pull before resuming work)
git pull --ff-only

# 1. Rust build + engine unit tests (no DB needed)
cargo build -p futurefin-api --locked
cargo test -p futurefin-engine

# 2. Postgres integration tests — NOT covered by CI, run them yourself.
#    (Test DB not running? One-time ff-test-db setup: futurefin-validation-and-qa §2.)
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace

# 3. Frontend gates (lint and Vitest are also NOT in CI)
npm run typecheck:web
npm run lint:web
npm run build:web
npm test --workspace futurefin-web
```

Then, per change class:

- [ ] Class-specific gates from the Section 1 table applied (fixture regeneration, both themes,
      migration discipline, breaking-note…).
- [ ] `.claude/` doc of record for the touched area updated (api-routes / data-model / engine /
      design-system / tests / env-and-config…). CLAUDE.md: "Keep these files up to date".
- [ ] If routes/handlers or `apps/api/src/mcp/` changed: the MCP parity evaluation ran and its
      outcome is recorded (futurefin-mcp-parity §1), and that skill's §5 counters still agree
      with the code (`grep -c '#\[tool(' apps/api/src/mcp/server.rs` vs the doc counters).
- [ ] `CHANGELOG.md` entry under `[Unreleased]` (or the release version), root-cause style —
      say WHY, not just what; mark breaking changes explicitly.
- [ ] No non-negotiable (Section 2) violated; no gate routed around.
- [ ] If you touched `apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh` or any
      `docker-compose*.yml`: `shellcheck -S warning apps/api/docker-entrypoint.sh scripts/*.sh`
      clean, `docker-stack` green in CI, and Section 2.8 re-read (no cluster deletion, no `VOLUME`,
      SIGINT to the postmaster).
- [ ] If releasing: Section 4 in order — bump, mirror-merge, local Docker-stack test **and the
      V2→V3 upgrade drill (4.2 B)**, tag from `main`, `git checkout dev` afterwards. After push:
      pull again.

## Provenance and maintenance

Facts above verified against the repo on 2026-07-02 (v1.4.3, branch
`claude/skill-library-handoff-rtfotl`); version/migration/test-file counts and the backup
schema version refreshed for v1.5.0 on 2026-07-06 (history-snapshots feature); §1 Infra-release
gates, §2.7 recovery commands, new §2.8, §4.2 drills and §6 checklist refreshed **2026-08-16 for
v3.0.0** (self-contained image), and the version/migration/test-file counts re-counted **2026-08-17
for v3.1.0** (embedded OAuth 2.1; `CURRENT_SCHEMA_VERSION` unchanged), against `apps/api/Dockerfile`,
`apps/api/docker-entrypoint.sh`, `docker-compose.yml`, `.github/workflows/ci.yml`,
`.github/testdata/` and `scripts/`. Re-verify before trusting:

- Current version: `grep '^version' apps/api/Cargo.toml` (**3.8.0** on 2026-08-21; 3.5.0 nunca se publicó)
- Migration count/list: `ls apps/api/migrations | wc -l && ls apps/api/migrations` (**40** on 2026-08-19 — la 40ª es `20260819120000_transactions_transfer_reconciliation`)
- Integration-test count: `ls apps/api/tests/*.rs | wc -l` (**28** on 2026-08-21 — el alta de 3.8.0 es `allocation_resolution.rs`); test-fn count: `grep -c "#\[tokio::test\]" apps/api/tests/*.rs | awk -F: '{s+=$2} END {print s}'` (312 on 2026-08-21 tras el tren del issue #4; 289 at 3.7.0, 284 at 3.6.0, 283 at 3.5.0)
- CI actually run: `cat .github/workflows/ci.yml` (jobs: rust / web / docker-stack) and
  `grep -n '^      - name:' .github/workflows/ci.yml` for the docker-stack scenario list
- Compose topology (one service since 3.0.0):
  `awk '/^services:/{f=1;next} /^volumes:/{f=0} f && /^  [a-z]/' docker-compose.yml`;
  overlays: `ls docker-compose*.yml`; frozen 2.x topologies for the drill: `ls .github/testdata/`
- §2.8 rules still hold: `grep -n '^VOLUME' apps/api/Dockerfile` (empty),
  `grep -n 'rm -rf "\$PGDATA"' apps/api/docker-entrypoint.sh` (empty),
  `grep -n 'stop_pid "\$PG_PID" INT' apps/api/docker-entrypoint.sh`,
  `grep -n 'stop_grace_period\|CMD-SHELL' docker-compose.yml`
- Rescue mode used by 4.2 C: `grep -n 'FUTUREFIN_MODE=db-only' scripts/restore-postgres.sh` and
  `grep -n 'db-only' apps/api/docker-entrypoint.sh`
- Shellcheck gate reproduces locally: `shellcheck -S warning apps/api/docker-entrypoint.sh scripts/*.sh .claude/skills/futurefin-diagnostics-and-tooling/scripts/*.sh`
- Publish trigger + registries: `cat .github/workflows/publish-image.yml`
- Backup schema version + chain: `grep -n 'CURRENT_SCHEMA_VERSION\|migrate_to_current' apps/api/src/handlers/backup_user/schema.rs`
- Scope helpers exist: `grep -n 'pub fn scope_where\|bind_scope' apps/api/src/handlers/person_view.rs`
- f64 wire exception boundary: `grep -n 'f64' .claude/api-routes.md`
- Fixture + both consumers: `ls apps/api/tests/fixtures/fire-parity.json apps/api/tests/fire_parity.rs apps/web/src/lib/fire.test.ts`
- Strict enum precedent: `grep -n "impl<'de> Deserialize" apps/api/src/handlers/installation.rs`
- npm script names: `grep -n '"typecheck:web"\|"lint:web"\|"build:web"' package.json`
- Release-flow wording drift: `grep -n 'Merge completo' CLAUDE.md`
