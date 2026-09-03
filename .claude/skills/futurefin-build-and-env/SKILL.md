---
name: futurefin-build-and-env
description: >
  Load this skill when you need to recreate the FutureFin development environment from scratch,
  build or run any part of the stack, or debug environment/build failures. Triggers: fresh clone /
  new machine / "how do I run this", split-dev setup (cargo run + Vite against
  docker-compose.dev.yml), API-only mode, building the self-contained Docker image locally (the
  one that bundles PostgreSQL 16/15), running the compose stack locally, cargo build/test
  commands, npm typecheck/lint/build/test commands, TEST_DATABASE_URL, ff-test-db; and symptoms
  like "connection refused 5432", "DATABASE_URL must be set", "no persistent volume is mounted",
  "Port 8080 is in use", Vite proxy ECONNREFUSED, "migration ... was previously applied but has
  been modified" (checksum mismatch, VersionMismatch), stale UI being served, `docker compose up`
  pulling instead of using a local image, a stray DATABASE_URL making the production image refuse
  to start ("ya no habla con bases de datos externas"), `ldd` "not found" failures during the image
  build, or leftover
  ff_test_* schemas. Do NOT use for: the catalog of env vars / entrypoint vars / query params /
  fire_settings axes (futurefin-config-and-flags), production deploy/upgrade/rollback/backups
  (futurefin-run-and-operate), or writing/extending tests (futurefin-validation-and-qa).
---

# FutureFin — build and environment recreation

Runbook to go from a bare machine to a fully working FutureFin dev environment, plus every
build/verify command and the known traps. Facts re-verified **2026-08-16 for v3.0.0** (34
migration files). All commands are run from the repo root unless stated otherwise.

**What 3.0.0 changed for anyone setting up or building:** the published Docker image is now
**self-contained** — PostgreSQL 16 runs inside the single `futurefin` container over a Unix
socket, so `docker-compose.yml` has exactly one service, no env var is required in production, and
the compose service `futurefin-database` is gone. `docker-compose.split-dev.yml` was **deleted**
and replaced by the standalone `docker-compose.dev.yml` (own project `futurefin-dev`, service
`db`, volume `devdata`). Everything in §5 (cargo/npm commands) and the ff-test-db recipe is
unchanged.

Vocabulary used below:
- **split-dev** — the two-process dev mode: Rust API via `cargo run` on port 8081 + Vite dev
  server on port 8080 proxying API paths, both talking to the standalone dev Postgres on
  127.0.0.1:5432. This is the normal way to develop; the embedded-Postgres image is *not* used in
  dev.
- **installation** — FutureFin's single-tenant unit: one row in the `installation` table per
  deployment; all financial data belongs to it.
- **migrations** — SQLx SQL files embedded into the API binary from `apps/api/migrations/`,
  applied automatically at startup.
- **entrypoint** — `apps/api/docker-entrypoint.sh`, PID 1 in the image since 3.0.0: it
  initializes/adopts/upgrades the embedded cluster, takes automatic backups, and supervises both
  PostgreSQL and the API. It exists only inside the container — split-dev never runs it.

## 1. Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | stable | Pinned by `rust-toolchain.toml` (`channel = "stable"`); rustup picks it up automatically. CI uses `dtolnay/rust-toolchain@stable`. The Docker image builds with `rust:bookworm` pinned by digest. |
| Node.js | 20+ works; prefer 24 | README says 20+, but CI runs Node 24 and the Docker build stage is `node:24.15-bookworm-slim` (upgraded from 22.14 in v1.0.7 to align with CI). Use 24 to match what actually gets tested/shipped. |
| npm | 10+ | Workspaces feature required (root `package.json` declares `apps/web`). |
| Docker + Compose v2 | any recent | Needed for Postgres in dev and for the local image build. BuildKit assumed (default in modern Docker). |
| PostgreSQL | — | Never installed on the host. In **dev** it runs as its own container (`postgres:16.4-alpine`, digest-pinned in `docker-compose.dev.yml`). In **production** it is *inside* the FutureFin image (PostgreSQL 16 active, 15 bundled only for automatic `pg_upgrade` of older volumes — `LABEL com.futurefin.postgres.majors="15,16"`). |

Workspace layout: Cargo workspace members `apps/api`, `crates/domain`, `crates/engine`
(root `Cargo.toml`); npm workspace `apps/web` (package name `futurefin-web`).

## 2. From zero to split-dev (the normal dev setup)

**The happy path is OWNED by `docs/desarrollo.md`** (2026-08-30 consolidation) — follow it there:
`cp .env.example .env` + uncomment the three dev vars, `./scripts/dev-db.sh` (wraps
`docker-compose.dev.yml` and waits for `pg_isready`), then `cargo run` in `apps/api` and
`npm run dev:web`. This section keeps only what an agent needs BEYOND the manual:

- **The `POSTGRES_*` agreement**: `docker-compose.dev.yml` defaults user/password/db to
  `futurefin`/`futurefin`/`futurefin`, exactly what the example `DATABASE_URL` expects. If you set
  `POSTGRES_USER`/`POSTGRES_PASSWORD`/`POSTGRES_DB` in `.env`, the dev compose picks them up and
  your `DATABASE_URL` must be updated to match — they must agree.
- **The dev `DATABASE_URL` vs the production compose on the same machine**: the image reads any
  `DATABASE_URL` not pointing at `/var/run/postgresql` as "external", and since 4.0.0 those are
  gone (populated volume → ignored with a warning; empty volume → refuses to start). See trap T10
  for why a `.env` alone usually cannot leak it into the container.
- **`docker-compose.dev.yml` is standalone, not an override** (trap T4): project `futurefin-dev`,
  service `db`, container `futurefin-dev-db`, volume `devdata`, publishes `127.0.0.1:5432`. It
  replaced the deleted `docker-compose.split-dev.yml` — production no longer has a DB service to
  override. Keeping 2.x dev data: a comment inside the file shows the `external: true` +
  `name: futurefin_pgdata` recipe.
- **First registered user becomes the installation owner**; later registrants stay "pending" until
  approved.

### Dual-port architecture (why two ports)

- Vite dev server listens on `WEB_DEV_PORT` (default **8080**) and serves the React SPA with hot
  reload.
- The Rust API listens on `PORT` (**8081** in split-dev).
- `apps/web/vite.config.ts` loads the **repo-root** `.env` (it resolves two levels up from
  `apps/web`) and proxies the path prefixes enumerated in its `server.proxy` block to
  `http://127.0.0.1:${FUTUREFIN_API_PORT ?? 8081}` — `/v1`, `/health`, `/openapi.json` plus, since
  the embedded MCP/OAuth, `/.well-known`, `/oauth/token`, `/oauth/register`, `/oauth/revoke` and
  `/mcp` (**never `/oauth` bare**: it would capture `/oauth/authorize`, which is an SPA view — the
  config says so in a comment). Everything else is the SPA. Count them in the file, not here:
  `grep -c 'target: apiTarget' apps/web/vite.config.ts`.
- `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` are read without a `VITE_` prefix and are not in
  `.env.example`; the defaults (8081/8080) match the split-dev values. If you change `PORT`, you
  must set `FUTUREFIN_API_PORT` to the same value or the proxy points at a dead port.
- In the Docker image there is no Vite: the API serves the prebuilt SPA itself from
  `WEB_STATIC_ROOT=/app/web` on a single port (8080). Since 2026-08-27 `index.html` is served by a
  **handler** (`apps/api/src/handlers/spa.rs`), not `ServeFile`, because the public subpath is a
  **per-request** property (the same image serves compose at `/` and HA's Ingress under
  `/api/hassio_ingress/<token>` simultaneously). Assets keep coming from `ServeDir`. Without proxy
  headers the shell is returned **byte-identical** to the file on disk, so the build output is
  unaffected — see the note in §9.

## 3. API-only mode (no Vite)

Set `PORT=8080` in `.env`, then `cd apps/api && cargo run`. You get the API + `/openapi.json` on
`http://127.0.0.1:8080` with no UI (unless `WEB_STATIC_ROOT` points to an existing `dist/` — see
trap T6). Useful for curl-driven backend work.

## 4. Full local Docker-stack build ("Test local con Docker Desktop")

Validates the complete production artifact (API + embedded frontend + **embedded PostgreSQL**)
without waiting for CI to publish an image. This flow is also the release gate: run it before
tagging any release.

**The step sequence is OWNED by `docs/desarrollo.md` §Construir la imagen en local** (2026-08-30
consolidation), and has a citable name: **`./scripts/build-local-image.sh`** — build with `--load`
(mandatory with BuildKit or Compose pulls a nonexistent image), stack up with the local override,
poll `/v1/ready`, print the served version. Prereqs it assumes: `.env` with
`FUTUREFIN_IMAGE=futurefin-local` + `FUTUREFIN_TAG=dev` and **no uncommented `DATABASE_URL`**
(trap T10). Rebuild loop after changes: re-run the script (layer cache reuses unchanged stages) or
`docker compose … up -d --no-deps futurefin` after a rebuild.

`docker-compose.local.yml` still only adds `pull_policy: never` to the `futurefin` service so
Compose uses the image that exists only locally. Note there is **no `futurefin-database` service
to wait for** any more — one container, two volumes (`pgdata` for the cluster, `ffdata` for
automatic backups and pg_upgrade staging), `stop_grace_period: 60s`, healthcheck on `/v1/ready`
with `start_period: 120s`.

### What the 3.0.0 Dockerfile actually does

Five stages, all bases digest-pinned:

1. `node:24.15-bookworm-slim` (`web`) — `npm ci && npm run build:web`, asserts `dist/index.html`
   and a non-empty `dist/assets`.
2. `rust:bookworm` (`rust-builder`) — `cargo build --release -p futurefin-api --locked`, and also
   emits two manifests the entrypoint consumes without starting the API: `/version.txt` (app
   version) and `/migration-versions.txt` (embedded migration versions, used to decide whether a
   pre-migration backup is needed).
3. `postgres:16-bookworm` (`pg16`) and 4. `postgres:15-bookworm` (`pg15`) — **source stages only**,
   never a base. Their digests must be the ones of the multi-arch **index** (manifest list), not a
   single-platform manifest, or the arm64 build fails.
5. `debian:bookworm-slim` (`runtime`) — copies `/usr/lib/postgresql/{15,16}` +
   `/usr/share/postgresql/{15,16}` + `libpq.so.5` out of those stages, installs the runtime libs
   plus `gosu`/`curl`, and runs `localedef … en_US.UTF-8` (without that locale glibc cannot open
   the `datcollate=en_US.utf8` clusters produced by the 2.x official image).

Three build-time rules that will bite you if you edit the Dockerfile:

- **The `ldd` gate.** After the COPYs, a `RUN` loop runs `ldd` over every PG binary and `.so` and
  **fails the build** if any prints "not found". If you add a PG extension or drop a lib from the
  `apt-get install` list, this is what catches it — read the printed list, add the missing package.
- **No `VOLUME` instruction, deliberately.** Basing the runtime on `postgres:*` (or declaring
  `VOLUME` yourself) creates anonymous volumes on a plain `docker run`, which Watchtower silently
  loses on recreate. The entrypoint instead uses `mountpoint` to detect a real volume and refuses
  to start without one (trap T9).
- **`llvmjit.so` is deleted on purpose** (~120 MB of libLLVM this workload never uses). Expect a
  final image around 320–360 MB uncompressed; check with `docker image ls futurefin-local:dev`.

Two unprivileged users exist in the image: `postgres` (uid 999, like the official Debian image) for
the postmaster and `futurefin` (uid 10001) for the API. PID 1 is root only so it can `chown` an
adopted 2.x volume and then `gosu` down.

CI (`.github/workflows/ci.yml`, job `docker-stack`) proves far more than a health poll now:
`shellcheck -S warning` over the entrypoint and scripts, an image-sanity step (both PG majors
report `--version`, the `com.futurefin.postgres.majors=15,16` label is `15,16`, and a volume-less
`docker run` **must abort** with "no persistent volume"), a fresh install polled on `/v1/ready`, a
Watchtower-style `--force-recreate`, an ordered-shutdown check, a real 2.x stack upgraded reusing
the volume, the **refusal** paths of the retired external mode (the current image over an untouched
2.x compose, and a leftover `DATABASE_URL` with an empty volume — both must exit non-zero and leave
the volume untouched), and a 15→16 `pg_upgrade`. It is
the only automated evidence of "no data loss" — do not weaken it. If your local stack passes step
4, you match the first CI scenario only.

## 5. Build / verify command reference

| What | Command (from repo root) | Needs DB? |
|------|--------------------------|-----------|
| Build API | `cd apps/api && cargo build` (CI: `cargo build -p futurefin-api --locked`) | No |
| Engine unit tests | `cargo test -p futurefin-engine` | No — pure Decimal math, no I/O |
| One engine test | `cargo test -p futurefin-engine -- <name>` | No |
| **Stochastic engine** (5.0.0: Monte Carlo, `f64`) | `cargo test -p futurefin-engine-stochastic` | No — pure math, no I/O |
| The **degeneration gate** alone (Decimal ↔ f64 son la misma simulación) | `cargo test -p futurefin-engine-stochastic --test degeneration` | No |
| The **golden pins** of the engine | `cargo test -p futurefin-engine --test golden_pins` | No |
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

What CI runs (**re-verified 2026-08-29; `.github/workflows/ci.yml` has FIVE jobs**, recount with
`sed -n '/^jobs:/,$p' .github/workflows/ci.yml | grep -E '^  [a-z_-]+:$'`): `secrets-scan` (blocking, first); `rust` =
`cargo build -p futurefin-api --locked` + `cargo test -p futurefin-engine --locked` + —desde 5.0.0—
`cargo test -p futurefin-engine-stochastic --locked`; `web` = Node
24, `npm install` + `typecheck:web` + `lint:web` + `npm test --workspace futurefin-web` +
`build:web`; **`integration` = a `postgres:16.4-alpine` service + `cargo test --workspace
--locked`**; `docker-stack` = the shellcheck + image-sanity + container suite described in §4.
**This paragraph said "three jobs" and "CI still does NOT run the Postgres integration tests"
until the Fase-7 sweep — both false since 4.0.0.** Running `cargo test --workspace` locally is
still the fast loop, but it is no longer the only place those tests run.

**5.0.0 — el workspace tiene un crate más** (`crates/engine-stochastic`, el Monte Carlo en `f64`):
`grep -n 'members' -A 1 Cargo.toml` lo enumera y `grep -n 'futurefin-engine' .github/workflows/ci.yml
scripts/test-all.sh` debe imprimir **≥ 4 líneas**. Corre sin base de datos y `./scripts/test-all.sh`
lo ejecuta **fuera** del bloque de integración, para que lo vea también un `SKIP_DB=1`: la puerta de
degeneración (`degeneration.rs`, todos los casos de la batería, ≤ 1 € por mes) es lo único que
garantiza que el camino de coma flotante y el exacto son la misma simulación, y saltársela por no
tener Postgres levantado sería saltarse el gate que la arqueología exigió para readmitir el `f64`.

## 6. How migrations run in dev

- `apps/api/src/db.rs::run_migrations` calls `sqlx::migrate!("./migrations")` — the SQL files are
  **embedded in the binary at compile time** and applied automatically every time the API starts
  (`main.rs` runs it right after connecting, before serving). There is no separate migrate step.
- Applied migrations are recorded in `_sqlx_migrations` with a checksum. SQLx only applies
  versions it hasn't seen.
- **Branch-switching implication**: "auto-migrates on start" means merely running the API mutates
  the dev DB's schema. If branch A adds migration `2026...X.sql` and you switch to branch B that
  lacks it, B's binary **fails to start**: sqlx's default migrator (`ignore_missing = false`;
  `db.rs` passes no override) errors with `VersionMissing` ("migration X was previously applied
  but is missing in the resolved migrations") when `_sqlx_migrations` contains versions the
  binary doesn't embed. The same mechanism is why downgrading a production image past a
  migration is blocked (see futurefin-run-and-operate). If branch B has a *different file for
  the same version* you get a checksum mismatch instead (trap T5). Recovery for branch
  switching: recreate the dev DB — note the volume name **changed in 3.0.0**, the dev compose
  project is `futurefin-dev` and its volume is `devdata`:

  ```bash
  docker compose -f docker-compose.dev.yml down
  docker volume rm futurefin-dev_devdata      # was futurefin_pgdata before 3.0.0
  docker compose -f docker-compose.dev.yml up -d
  ```

  (or, only if branch A's migration was genuinely additive and harmless, delete its row from
  `_sqlx_migrations` by hand). `futurefin_pgdata` is now a *production* volume name — never
  `docker volume rm` it to fix a dev problem.
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

**T4 — API can't reach Postgres: "Connection refused (os error 111)" on 127.0.0.1:5432.** Cause
(**changed in 3.0.0**): you never started the dev database. There is no split-dev override any
more and the production compose has no database service at all — its PostgreSQL lives inside the
app container on a Unix socket, unreachable from the host by design. Fix: start the standalone dev
compose, which is the only thing that publishes 5432:

```bash
docker compose -f docker-compose.dev.yml up -d
docker compose -f docker-compose.dev.yml ps      # db should be healthy
```

Related near-misses: `docker compose up -d` (the production file) starts the all-in-one container
and gives you *nothing* on 5432; and if you also have a production stack running, its `APP_PORT`
8080 will collide with Vite (T1).

**T5 — Startup fails with migration checksum/version mismatch** (`VersionMismatch` /
"was previously applied but has been modified"). Cause: a migration file changed after it was
applied to this DB (edited migration, branch switch where the same version differs, or a squash).
Fix (dev DB only, and only if the current file is genuinely equivalent/idempotent) — note **how
you reach psql changed in 3.0.0**:

```bash
# dev (standalone Postgres, port published on the host — no container needed)
psql postgres://futurefin:futurefin@127.0.0.1:5432/futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>"

# same thing from inside the dev container, if you prefer
docker compose -f docker-compose.dev.yml exec db psql -U futurefin -d futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>"

# production / local all-in-one image: PostgreSQL is INSIDE the futurefin container,
# socket-only — there is no futurefin-database container and no TCP port to hit
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
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

**T9 — The container exits immediately with "no persistent volume is mounted at
/var/lib/postgresql/data".** (New in 3.0.0.) Cause: **not a bug — a deliberate anti-data-loss
guard.** The embedded PostgreSQL writes into `$PGDATA`; if that path is not a real mountpoint, the
cluster would live in the container's ephemeral layer and vanish on the next `docker run` /
Watchtower recreate. The image declares no `VOLUME` precisely so anonymous volumes can't hide the
problem, and the entrypoint checks with `mountpoint` before doing anything. Fix: run it through
compose (`docker-compose.yml` mounts `pgdata` and `ffdata`) or add `-v` yourself on a bare
`docker run`. Only for genuinely throwaway containers, set `FUTUREFIN_ALLOW_EPHEMERAL_DB=1` —
it starts with a warning and the data dies with the container. CI asserts this abort happens, so
do not "fix" it by relaxing the guard.

**T10 — A stray `DATABASE_URL` reaches the production image.** (New in 3.0.0; **behaviour changed
in 4.0.0**.) Symptom: the container logs `DATABASE_URL está definida pero FutureFin 4.0.0 solo usa
la base embebida… se ignora` and carries on, or — with a volume that has no cluster yet — it exits
non-zero with the boxed `ya no habla con bases de datos externas` message. Cause: the image treats
any `DATABASE_URL` that doesn't point at `/var/run/postgresql` as external, and external databases
were retired in 4.0.0 (up to 3.9.0 the same input entered the deprecated compat mode, or triggered
a one-shot automigration from a database you didn't mean to migrate).

**How it actually gets in**, verified 2026-08-22: *not* through a `.env` sitting next to the
production compose — none of this repo's compose files declare `env_file:` or list `DATABASE_URL`
under `environment:` (`grep -n 'env_file\|DATABASE_URL' docker-compose*.yml` prints nothing), and
Compose does not inject `.env` into containers. It gets in through a compose that declares it (the
2.x one does), a `docker run -e DATABASE_URL=…`, or your own edit. Fix: remove it from whatever is
passing it. Keeping separate `.env` files — the dev one (`PORT=8081` + `DATABASE_URL`) and the
prod/local one (`FUTUREFIN_IMAGE`/`FUTUREFIN_TAG`/`APP_PORT`) passed with `--env-file` — is still
good hygiene. `FUTUREFIN_DB_MODE=embedded` no longer buys you anything: it is a synonym of `auto`.

## 8. When NOT to use this skill

- Cataloging or adding **env vars, entrypoint `FUTUREFIN_*` vars, compose knobs, query params,
  `fire_settings` axes** → `.claude/skills/futurefin-config-and-flags/SKILL.md` (this skill only
  touches the vars needed to boot a dev environment or build the image).
- **Production** deploy, upgrade, rollback, the 2.x migration path, getting someone off an
  external database, `pg_upgrade`, backups, logs, health monitoring →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- **Writing or extending tests**, TestApp harness, parity fixtures, evidence standards →
  `.claude/skills/futurefin-validation-and-qa/SKILL.md` (here you only learn how to *run* them).
- Deciding whether a change is safe to make at all → `.claude/skills/futurefin-change-control/SKILL.md`.

## 9. Provenance and maintenance

Written 2026-07-02 against v1.4.3; **fully re-verified 2026-08-16 against v3.0.0** (self-contained
image); trap **T10, the split-dev `DATABASE_URL` note and the CI paragraph re-verified 2026-08-22
against v4.0.0**, which removed the external-database mode from `apps/api/docker-entrypoint.sh`.
Sources: `CLAUDE.md`, `README.md`, `.env.example`, `rust-toolchain.toml`, `package.json` +
`apps/web/package.json`, `apps/api/Cargo.toml`, `apps/api/Dockerfile`,
**`apps/api/docker-entrypoint.sh`**, `apps/web/vite.config.ts`, `apps/api/src/{main.rs,db.rs}`,
`docker-compose{,.local,.dev}.yml`, `.github/workflows/ci.yml`, `.claude/env-and-config.md`,
`.claude/tests.md`, `CHANGELOG.md` (v1.0.7 Node bump, v1.3.0 auto-repair removal, 3.0.0
self-contained image). Re-verify before trusting volatile facts — every command below was run on
2026-08-16:

- Version: `grep '^version' apps/api/Cargo.toml` (3.0.0)
- Migration count/list: `ls apps/api/migrations | wc -l` (**59** on 2026-09-03, rama `release/5.0.0`; 49 on 2026-08-30; 34 on 2026-08-16)
- Compose files present — must be exactly three, and **no** `split-dev`: `ls docker-compose*.yml`
- Dev DB definition (project name, service, port, volume): `cat docker-compose.dev.yml`
- Production stack is one service, two volumes, `/v1/ready` healthcheck: `cat docker-compose.yml`
- Image stages, `ldd` gate, locale, users, label, absence of `VOLUME`:
  `grep -n '^FROM\|^ENV\|^LABEL\|^HEALTHCHECK\|localedef\|ldd\|llvmjit\|useradd\|VOLUME' apps/api/Dockerfile`
- Entrypoint modes, guards and defaults (incl. the 4.0.0 external-DB refusal):
  `grep -n 'FUTUREFIN_[A-Z_]*:-\|no persistent volume\|invalid FUTUREFIN_DB_MODE\|refuse_external_database\|export DATABASE_URL' apps/api/docker-entrypoint.sh`
- Node versions: `grep node-version .github/workflows/ci.yml` and `grep 'FROM node' apps/api/Dockerfile`
- Vite proxy paths + port defaults: `grep -n 'FUTUREFIN_API_PORT\|WEB_DEV_PORT\|proxy' apps/web/vite.config.ts`
- API port default + env loading + connect timeout: `grep -n 'fn port\|fn load_env\|FUTUREFIN_DB_CONNECT_TIMEOUT_SECS' -A 5 apps/api/src/main.rs`
- Migration runner (no auto-repair) + connect retry: `grep -n 'migrate!\|connect_with_retry' apps/api/src/db.rs`
- CI jobs actually run: `grep -n 'name:\|run:' .github/workflows/ci.yml`
- Workspace crates (5.0.0 añadió el cuarto): `sed -n '/^members/p' Cargo.toml` → `apps/api`,
  `crates/domain`, `crates/engine`, `crates/engine-stochastic`
- Test DB recipe: TL;DR block at top of `.claude/tests.md`
- **`apps/web/vite.config.ts` still declares no `base` — and since 2026-08-27 that is deliberate,
  not an omission** (`grep -n 'base' apps/web/vite.config.ts` → nothing). A Vite `base` is baked at
  build time and would pin ONE public prefix into the bundle; the prefix is per request
  (`apps/api/src/prefix.rs`), so the server rewrites the absolute refs of `index.html` on the way
  out (`apps/api/src/handlers/spa.rs::inject`, which returns `Cow::Borrowed` — literally the same
  bytes — when there is no prefix and no SSO). If someone adds a `base`, the add-on and the plain
  compose deployment can no longer be served by the same image: check
  `cargo test -p futurefin-api --lib prefix::` and `apps/api/tests/base_path.rs` before believing
  otherwise.
