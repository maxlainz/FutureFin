---
name: futurefin-build-and-env
description: >
  Load this skill when you need to recreate the FutureFin development environment from scratch,
  build or run any part of the stack, or debug environment/build failures. Triggers: fresh clone /
  new machine / "how do I run this", split-dev setup (cargo run + Vite), API-only mode, building
  the Docker image locally, running the compose stack locally, cargo build/test commands, npm
  typecheck/lint/build/test commands, TEST_DATABASE_URL, ff-test-db; and symptoms like
  "connection refused 5432", "DATABASE_URL must be set", "Port 8080 is in use", Vite proxy
  ECONNREFUSED, "migration ... was previously applied but has been modified" (checksum mismatch,
  VersionMismatch), stale UI being served, `docker compose up` pulling instead of using a local
  image, or leftover ff_test_* schemas. Do NOT use for: the catalog of env vars / query params /
  fire_settings axes (futurefin-config-and-flags), production deploy/upgrade/rollback/backups
  (futurefin-run-and-operate), or writing/extending tests (futurefin-validation-and-qa).
---

# FutureFin — build and environment recreation

Runbook to go from a bare machine to a fully working FutureFin dev environment, plus every
build/verify command and the known traps. Facts date-stamped 2026-07-02 (repo at v1.4.3,
31 migration files). All commands are run from the repo root unless stated otherwise.

Vocabulary used below:
- **split-dev** — the two-process dev mode: Rust API via `cargo run` on port 8081 + Vite dev
  server on port 8080 proxying API paths. This is the normal way to develop.
- **installation** — FutureFin's single-tenant unit: one row in the `installation` table per
  deployment; all financial data belongs to it.
- **migrations** — SQLx SQL files embedded into the API binary from `apps/api/migrations/`,
  applied automatically at startup.

## 1. Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | stable | Pinned by `rust-toolchain.toml` (`channel = "stable"`); rustup picks it up automatically. CI uses `dtolnay/rust-toolchain@stable`. The Docker image builds with `rust:bookworm` pinned by digest. |
| Node.js | 20+ works; prefer 24 | README says 20+, but CI runs Node 24 and the Docker build stage is `node:24.15-bookworm-slim` (upgraded from 22.14 in v1.0.7 to align with CI). Use 24 to match what actually gets tested/shipped. |
| npm | 10+ | Workspaces feature required (root `package.json` declares `apps/web`). |
| Docker + Compose v2 | any recent | Needed for Postgres in dev and for the local image build. BuildKit assumed (default in modern Docker). |
| PostgreSQL | — | Never installed on the host; always runs in Docker (`postgres:16.4-alpine`, digest-pinned in `docker-compose.yml`). |

Workspace layout: Cargo workspace members `apps/api`, `crates/domain`, `crates/engine`
(root `Cargo.toml`); npm workspace `apps/web` (package name `futurefin-web`).

## 2. From zero to split-dev (the normal dev setup)

Copy-pasteable sequence from a fresh clone:

```bash
cp .env.example .env
```

Edit `.env`: uncomment the three dev vars (they ship commented out) and set any
`POSTGRES_PASSWORD` (compose refuses to start the DB without one):

```env
POSTGRES_PASSWORD=dev_only_password
PORT=8081
DATABASE_URL=postgres://futurefin:futurefin@127.0.0.1:5432/futurefin
RUST_LOG=futurefin_api=info,tower_http=info
```

Note: the dev `DATABASE_URL` uses user/password `futurefin:futurefin` — if you set a different
`POSTGRES_PASSWORD`, either keep `POSTGRES_PASSWORD=futurefin` for dev or update the password
inside `DATABASE_URL` to match. They must agree.

Start Postgres only, **with the split-dev override** (required — see trap T4):

```bash
docker compose -f docker-compose.yml -f docker-compose.split-dev.yml up -d futurefin-database
```

`docker-compose.yml` deliberately does not map the DB port to the host;
`docker-compose.split-dev.yml` adds `127.0.0.1:5432:5432` so your local `cargo run` can connect.
(CLAUDE.md/README show the short form `docker compose up -d futurefin-database` — that starts the
DB but leaves it unreachable from the host.)

```bash
# Terminal 1 — API on :8081 (applies migrations on startup)
cd apps/api && cargo run

# Terminal 2 — UI on :8080, from repo root
npm install
npm run dev:web
```

Open `http://127.0.0.1:8080`. Register a user — the **first user to register becomes the
installation owner** automatically; later registrants are "pending" until the owner approves them.

### Dual-port architecture (why two ports)

- Vite dev server listens on `WEB_DEV_PORT` (default **8080**) and serves the React SPA with hot
  reload.
- The Rust API listens on `PORT` (**8081** in split-dev).
- `apps/web/vite.config.ts` loads the **repo-root** `.env` (it resolves two levels up from
  `apps/web`) and proxies exactly three path prefixes to `http://127.0.0.1:${FUTUREFIN_API_PORT ?? 8081}`:
  `/v1`, `/health`, `/openapi.json`. Everything else is the SPA.
- `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` are read without a `VITE_` prefix and are not in
  `.env.example`; the defaults (8081/8080) match the split-dev values. If you change `PORT`, you
  must set `FUTUREFIN_API_PORT` to the same value or the proxy points at a dead port.
- In the Docker image there is no Vite: the API serves the prebuilt SPA itself from
  `WEB_STATIC_ROOT=/app/web` on a single port (8080).

## 3. API-only mode (no Vite)

Set `PORT=8080` in `.env`, then `cd apps/api && cargo run`. You get the API + `/openapi.json` on
`http://127.0.0.1:8080` with no UI (unless `WEB_STATIC_ROOT` points to an existing `dist/` — see
trap T6). Useful for curl-driven backend work.

## 4. Full local Docker-stack build ("Test local con Docker Desktop")

Validates the complete production artifact (API + embedded frontend + DB) without waiting for CI
to publish an image. This flow is also the release gate: run it before tagging any release.

```bash
# 1. Build the image locally (slow the first time; cached on rebuilds)
#    --load is mandatory with BuildKit so the image lands in Docker's local store
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .

# 2. Make sure .env contains:
#      FUTUREFIN_IMAGE=futurefin-local
#      FUTUREFIN_TAG=dev
#      POSTGRES_PASSWORD=<anything>

# 3. Start the stack with the local override (stops Compose from pulling the local-only image)
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d

# 4. Smoke test
curl -sf http://127.0.0.1:8080/v1/health

# 5. Rebuild loop after changes (Docker layer cache reuses unchanged stages)
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev . \
  && docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env \
     up -d --no-deps futurefin
```

`docker-compose.local.yml` only adds `pull_policy: never` to the `futurefin` service so Compose
uses the image that exists only locally. The Dockerfile is three stages: `node:24.15` builds the
SPA (`npm ci && npm run build:web`, asserts `dist/index.html` exists), `rust:bookworm` builds
`cargo build --release -p futurefin-api --locked`, and a `debian:bookworm-slim` runtime runs as
`nobody` with `PORT=8080` and `WEB_STATIC_ROOT=/app/web`. All base images are digest-pinned.

CI (`.github/workflows/ci.yml`) proves this exact flow: it builds the image, starts
`docker compose up -d` with `FUTUREFIN_IMAGE=futurefin-ci`, and polls `/v1/health` for up to
180 s. If your local stack passes step 4, you match CI's `docker-stack` job.

## 5. Build / verify command reference

| What | Command (from repo root) | Needs DB? |
|------|--------------------------|-----------|
| Build API | `cd apps/api && cargo build` (CI: `cargo build -p futurefin-api --locked`) | No |
| Engine unit tests | `cargo test -p futurefin-engine` | No — pure Decimal math, no I/O |
| One engine test | `cargo test -p futurefin-engine -- <name>` | No |
| Full workspace tests (incl. `apps/api/tests/` integration) | see below | Yes — ff-test-db |
| Frontend typecheck | `npm run typecheck:web` | No |
| Frontend lint | `npm run lint:web` | No |
| Frontend prod build | `npm run build:web` → `apps/web/dist/` | No |
| Frontend unit tests (Vitest, node env, pure functions) | `npm test --workspace futurefin-web` | No |

Integration tests need a dedicated Postgres on port **5433** (so it never clashes with the dev DB
on 5432). Start it once, reuse forever:

```bash
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine

TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

Each integration test creates its own schema `ff_test_<uuid>` inside `futurefin_test`, applies all
migrations there, and runs against the real router. Schemas are leaked on purpose (see trap T8).

What CI runs (as of 2026-07-02, `.github/workflows/ci.yml`): `cargo build -p futurefin-api
--locked`, `cargo test -p futurefin-engine --locked`, `npm install` + `typecheck:web` +
`build:web`, and the Docker-stack build + health smoke. **CI does NOT run the Postgres
integration tests** — no `TEST_DATABASE_URL` in CI — so run `cargo test --workspace` locally
before considering backend changes verified. (`.claude/tests.md` claims "There is no CI yet";
that is stale — CI exists, it just skips the integration suite.)

## 6. How migrations run in dev

- `apps/api/src/db.rs::run_migrations` calls `sqlx::migrate!("./migrations")` — the SQL files are
  **embedded in the binary at compile time** and applied automatically every time the API starts
  (`main.rs` runs it right after connecting, before serving). There is no separate migrate step.
- Applied migrations are recorded in `_sqlx_migrations` with a checksum. SQLx only applies
  versions it hasn't seen.
- **Branch-switching implication**: "auto-migrates on start" means merely running the API mutates
  the dev DB's schema. If branch A adds migration `2026...X.sql` and you switch to branch B that
  lacks it, B's binary starts fine (extra applied rows are tolerated) but the schema has objects
  B's code doesn't know about — usually harmless, occasionally confusing. If branch B has a
  *different file for the same version* you get a checksum mismatch (trap T5). When in doubt,
  recreate the dev DB: `docker compose down` + `docker volume rm futurefin_pgdata` + restart.
- There is **no auto-repair**: a checksum-repair loop existed and was deliberately removed in
  v1.3.0 because it masked drift. Mismatches now fail loud at startup and must be fixed by hand.
  Never reintroduce auto-repair.

## 7. Known traps (symptom → cause → fix)

**T1 — Port clash on 8080.** Symptom: Vite prints "Port 8080 is in use, trying another one..."
and the UI appears on 8081+, or the API fails to bind. Cause: the API's `PORT` defaults to 8080
(`main.rs::port()`); if you left `PORT` commented in `.env`, `cargo run` grabs 8080 and collides
with Vite's `WEB_DEV_PORT` default 8080. Fix: in split-dev always set `PORT=8081` (and leave
`FUTUREFIN_API_PORT` unset or equal to it). If Vite relocated itself, kill it and restart after
fixing `.env` — a relocated Vite still proxies to 8081, but bookmarks/cookies get confusing.

**T2 — `.env` edits seem ignored / wrong DB used.** Cause: `main.rs::load_env()` loads, in order,
(1) `{CARGO_MANIFEST_DIR}/../../.env` (repo root, path baked in at compile time — works from any
CWD on the machine that built it) then (2) `.env` in the current working directory. dotenvy
**never overrides a variable that is already set**, so precedence is: shell environment > repo-root
`.env` > CWD `.env`. Fix: keep exactly one `.env` at the repo root; check `env | grep -E
'DATABASE_URL|PORT'` for stale exports in your shell before blaming the file.

**T3 — Exported shell vars override `.env`.** Symptom: you change `DATABASE_URL` in `.env` and the
API still connects to the old DB. Cause: same dotenvy rule — process env wins. Fix: `unset
DATABASE_URL PORT RUST_LOG` (or start a fresh shell), rerun.

**T4 — API can't reach Postgres: "Connection refused (os error 111)" on 127.0.0.1:5432.** Cause:
you started the DB with plain `docker compose up -d futurefin-database`; the base compose file
does not publish the DB port to the host (production hardening). Fix: restart with the override —
`docker compose -f docker-compose.yml -f docker-compose.split-dev.yml up -d futurefin-database`.
Never use that override in production.

**T5 — Startup fails with migration checksum/version mismatch** (`VersionMismatch` /
"was previously applied but has been modified"). Cause: a migration file changed after it was
applied to this DB (edited migration, branch switch where the same version differs, or a squash).
Fix (dev DB only, and only if the current file is genuinely equivalent/idempotent):

```bash
docker exec -it futurefin-database psql -U futurefin -d futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>"
```

then restart the API. This is intentionally manual — the auto-repair loop was removed in v1.3.0
because it silently papered over drift. If the file legitimately differs in effect, recreate the
DB instead. Never edit an already-shipped migration (see futurefin-change-control).

**T6 — Browser shows an old UI.** Cause: `WEB_STATIC_ROOT` points at a stale `apps/web/dist/`
(from an old `npm run build:web`), so the API serves that snapshot instead of your live code; or
you're on the API port (8081) instead of Vite (8080). If `WEB_STATIC_ROOT` is set but the path is
missing, the API logs a warning and runs API-only. Fix: in split-dev leave `WEB_STATIC_ROOT`
unset and use the Vite port; if you deliberately serve `dist/`, rebuild it first.

**T7 — First Docker build is very slow / built image "not found" by compose.** Cause: cold cache
(full Rust release compile + npm ci) — expect many minutes on first build; and with BuildKit the
result stays in the builder cache unless you pass `--load`, so `docker compose` can't see
`futurefin-local:dev` and tries to pull it. Fix: always `docker build --load ...` and start the
stack with `-f docker-compose.local.yml` (its `pull_policy: never` stops pull attempts). Rebuilds
reuse cached layers as long as `Cargo.lock`/`package-lock.json` didn't change.

**T8 — ff-test-db fills up with `ff_test_<uuid>` schemas.** Cause: intentional — each integration
test creates an isolated schema and leaks it so failures can be inspected post-mortem. Fix when it
bothers you: wipe the whole container (`docker rm -f ff-test-db` and recreate, simplest) or drop
selectively:

```bash
docker exec ff-test-db psql -U futurefin -d futurefin_test -Atc \
  "SELECT 'DROP SCHEMA ' || nspname || ' CASCADE;' FROM pg_namespace WHERE nspname LIKE 'ff_test_%'" \
  | docker exec -i ff-test-db psql -U futurefin -d futurefin_test
```

## 8. When NOT to use this skill

- Cataloging or adding **env vars, compose knobs, query params, `fire_settings` axes** →
  `.claude/skills/futurefin-config-and-flags/SKILL.md` (this skill only touches the vars needed to
  boot a dev environment).
- **Production** deploy, upgrade, rollback, backups, logs, health monitoring →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- **Writing or extending tests**, TestApp harness, parity fixtures, evidence standards →
  `.claude/skills/futurefin-validation-and-qa/SKILL.md` (here you only learn how to *run* them).
- Deciding whether a change is safe to make at all → `.claude/skills/futurefin-change-control/SKILL.md`.

## 9. Provenance and maintenance

Written 2026-07-02 against v1.4.3 from: `CLAUDE.md`, `README.md`, `.env.example`,
`rust-toolchain.toml`, `package.json` + `apps/web/package.json`, `apps/api/Cargo.toml`,
`apps/api/Dockerfile`, `apps/web/vite.config.ts`, `apps/api/src/{main.rs,db.rs}`,
`docker-compose{,.local,.split-dev}.yml`, `.github/workflows/ci.yml`, `.claude/env-and-config.md`,
`.claude/tests.md` (stale on "no CI"), `CHANGELOG.md` (v1.0.7 Node bump, v1.3.0 auto-repair
removal). Re-verify before trusting volatile facts:

- Version: `grep '^version' apps/api/Cargo.toml`
- Migration count/list: `ls apps/api/migrations | wc -l`
- Node versions: `grep node-version .github/workflows/ci.yml` and `grep 'FROM node' apps/api/Dockerfile`
- Vite proxy paths + port defaults: `grep -n 'FUTUREFIN_API_PORT\|WEB_DEV_PORT\|proxy' apps/web/vite.config.ts`
- API port default + env loading: `grep -n 'fn port\|fn load_env' -A 5 apps/api/src/main.rs`
- Migration runner (no auto-repair): `grep -n 'migrate!' apps/api/src/db.rs`
- Split-dev DB port mapping: `cat docker-compose.split-dev.yml`
- CI jobs actually run: `grep -n 'run:' .github/workflows/ci.yml`
- Test DB recipe: TL;DR block at top of `.claude/tests.md`
