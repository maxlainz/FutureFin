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

How changes are classified, gated and reviewed in this repo. As of 2026-07-06: version **v1.5.0**
(`apps/api/Cargo.toml`), **32** migration files in `apps/api/migrations/`, **11** integration-test
files in `apps/api/tests/`. All paths below are from the repo root.

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
| **API contract** | Handler request/response fields, status codes, routes | Breaking-change policy check (Section 5); update `#[utoipa::path]` annotations (OpenAPI is generated); update `.claude/api-routes.md`; integration tests covering the new/changed shape; CHANGELOG (with a "breaking" note if applicable). |
| **DB migration** | New file in `apps/api/migrations/` | Section 3 in full (never edit shipped; data-loss sign-off; grep for query drift); integration tests — every test applies ALL migrations to a fresh schema, so a broken migration fails everything; update `.claude/data-model.md`; if exported tables change shape → check `apps/api/src/handlers/backup_user/` (Section 5) and the export SQL. |
| **UI-visual** | Anything in `apps/web/src/` that renders | `npm run typecheck:web && npm run lint:web && npm run build:web && npm test --workspace futurefin-web`; verify **light AND dark theme** before merging (owner-mandated); tokens only, no hex (Section 2.4); icons only in `components/icons.tsx`; small charts via `MiniProjection`; update `.claude/design-system.md` / `.claude/frontend-structure.md` if conventions moved; CHANGELOG. |
| **Docs-only** | `CLAUDE.md`, `.claude/*.md`, `README.md`, CHANGELOG wording | No test gates. Gate = accuracy: verify every command/path/claim against the code before writing it (docs have drifted before — eight errata were found and fixed on 2026-07-02; prefer commands over frozen counts, e.g. `ls apps/api/migrations \| wc -l`). Record unfixable drift in futurefin-docs-and-writing §7. |
| **Infra-release** | `Dockerfile`, `docker-compose*.yml`, `.github/workflows/*`, version bump, tag | CI green on `dev`; full local Docker-stack test (Section 4.2) before tagging; version bump + `Cargo.lock` sync + CHANGELOG; dev→main full-mirror merge; tag from `main`. |

CI (`.github/workflows/ci.yml`, runs on push/PR to `main` and `dev`) covers only:
`cargo build -p futurefin-api --locked`, `cargo test -p futurefin-engine --locked`,
`npm run typecheck:web` + `npm run build:web`, and a Docker-stack build + `/v1/health` smoke.
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
`psql "$DATABASE_URL" -c "DELETE FROM _sqlx_migrations WHERE version = <X>"` then restart.

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

Run the CLAUDE.md "Test local con Docker Desktop" flow — it validates API + frontend + DB
exactly as production, without waiting for CI to publish:

```bash
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .
# .env must have: FUTUREFIN_IMAGE=futurefin-local, FUTUREFIN_TAG=dev, POSTGRES_PASSWORD=<any>
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d
curl -sf http://127.0.0.1:8080/v1/health
```

Full flow, traps and the rebuild loop: futurefin-build-and-env §4. Then click through the app
once, including migrations applying on first boot against a DB restored from real data if the
release contains migrations.

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
  `CURRENT_SCHEMA_VERSION` in `apps/api/src/handlers/backup_user/schema.rs` (currently **6** — every
  bump has been additive, each older file importing with the new collections empty: v4 added
  `snapshots` via `payload_v3_to_v4` (v1.5.0), v5 added transactions/imports/rules (v1.6.0), v6
  added `recurring_transaction_rules` + `BackupTransaction.recurring_rule_index` via
  `payload_v5_to_v6` (v1.8.0)) and extending the `migrate_to_current` chain so ALL older versions
  still import (v1..v5 remain importable today; legacy per-asset contribution fields are dropped
  on import, user reconfigures — the documented v1.1.0 pattern). `parse_payload` rejects versions
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
- [ ] `CHANGELOG.md` entry under `[Unreleased]` (or the release version), root-cause style —
      say WHY, not just what; mark breaking changes explicitly.
- [ ] No non-negotiable (Section 2) violated; no gate routed around.
- [ ] If releasing: Section 4 in order — bump, mirror-merge, local Docker-stack test, tag from
      `main`, `git checkout dev` afterwards. After push: pull again.

## Provenance and maintenance

Facts above verified against the repo on 2026-07-02 (v1.4.3, branch
`claude/skill-library-handoff-rtfotl`); version/migration/test-file counts and the backup
schema version refreshed for v1.5.0 on 2026-07-06 (history-snapshots feature). Re-verify before
trusting:

- Current version: `grep '^version' apps/api/Cargo.toml`
- Migration count/list: `ls apps/api/migrations | wc -l && ls apps/api/migrations`
- CI actually run: `cat .github/workflows/ci.yml` (jobs: rust / web / docker-stack)
- Publish trigger + registries: `cat .github/workflows/publish-image.yml`
- Backup schema version + chain: `grep -n 'CURRENT_SCHEMA_VERSION\|migrate_to_current' apps/api/src/handlers/backup_user/schema.rs`
- Scope helpers exist: `grep -n 'pub fn scope_where\|bind_scope' apps/api/src/handlers/person_view.rs`
- f64 wire exception boundary: `grep -n 'f64' .claude/api-routes.md`
- Fixture + both consumers: `ls apps/api/tests/fixtures/fire-parity.json apps/api/tests/fire_parity.rs apps/web/src/lib/fire.test.ts`
- Strict enum precedent: `grep -n "impl<'de> Deserialize" apps/api/src/handlers/installation.rs`
- npm script names: `grep -n '"typecheck:web"\|"lint:web"\|"build:web"' package.json`
- Release-flow wording drift: `grep -n 'Merge completo' CLAUDE.md`
