---
name: futurefin-run-and-operate
description: >
  Operating a FutureFin installation in production: deploying from zero with Docker Compose
  (single all-in-one container with embedded PostgreSQL since 3.0.0), upgrading/rolling back
  the image (FUTUREFIN_TAG), migrating a 2.x two-container stack, reading logs and startup
  milestones, health vs readiness probes, ordered shutdown, taking and restoring backups
  (automatic pre-migration dumps, the pg_dump scripts, and the per-user encrypted .ffbackup
  layer), and knowing what data lives where. Load this skill when the task mentions: deploy,
  production, docker compose up, upgrade, rollback, downgrade, release image, Docker Hub,
  GHCR, image tags, :latest, pull, watchtower, backup, restore, pg_dump, pg_upgrade,
  automigración / automigration (retired in 4.0.0 — see §4.5), external database, db-only mode,
  .ffbackup, export/import,
  container unhealthy in the operational sense (reading healthcheck/probe status during
  deploy/upgrade, restarting, rolling back — for diagnosing WHY a container is unhealthy from
  a symptom use futurefin-debugging-playbook), healthcheck, /v1/health, /v1/ready, logs,
  RUST_LOG, volume, pgdata, ffdata, stop_grace_period, data loss, stop the stack. Do NOT load
  for: setting up a local DEV environment or building the image locally
  (futurefin-build-and-env), measuring performance or cache behavior
  (futurefin-diagnostics-and-tooling), diagnosing an app-level bug behind a failing container
  (futurefin-debugging-playbook), the meaning of each env var (futurefin-config-and-flags), or
  how releases are cut and tagged (futurefin-change-control).
---

# FutureFin — Run and Operate

Runbook for deploying, upgrading, backing up and operating a production FutureFin
installation. All commands run from the directory containing `docker-compose.yml`
(the repo root, or a server directory holding just `docker-compose.yml` + optionally `.env`).

Facts date-stamped **2026-08-16**, app version **v3.0.0**, backup `schema_version` **6**
(unchanged by 3.0.0), 34 SQL migrations in `apps/api/migrations/`.

> **3.0.0 is the single biggest operational change in this project's history.** The stack went
> from **two containers** (`futurefin` + `futurefin-database`) to **ONE**: PostgreSQL now runs
> inside the application image. Everything below assumes 3.x unless a paragraph says "2.x".

Vocabulary (defined once):
- **Installation** — the singleton row in the `installation` table; one per deployment.
  All financial data belongs to it. The first registered user becomes its **owner**.
- **Stack** — since 3.0.0, the **single** Compose service `futurefin`: entrypoint supervisor +
  embedded PostgreSQL 16 + API + web UI, one container, host port `APP_PORT` (default 8080).
- **Entrypoint** — `apps/api/docker-entrypoint.sh`, PID 1 in the container. It supervises two
  processes (postmaster and API), owns cluster adoption, `pg_upgrade`, automatic backups and
  ordered shutdown. Its log lines are prefixed `[futurefin-entrypoint]`.
- **`pgdata`** — the named volume holding the PostgreSQL cluster, mounted at
  `/var/lib/postgresql/data`. **Same name and same path as in 2.x** — upgrading reuses it as is.
- **`ffdata`** — the *new* named volume mounted at `/var/lib/futurefin`: automatic
  pre-migration backups, `pg_upgrade` staging, and the entrypoint's state files.
- **`.ffbackup`** — FutureFin's per-user encrypted application-level backup file format.
- **Embedded / external mode** — historical. Embedded = the in-container PostgreSQL, and since
  **4.0.0 the only option**: external mode (a separate server via `DATABASE_URL`) was deprecated
  in 3.0.0 and **removed** in 4.0.0 (§4.2/§4.3). `DATABASE_URL` still exists and is still required
  in split-dev; what disappeared is the container ever honouring it.
- **Migrations roll forward only** — SQLx applies pending `.sql` files at startup; there are
  no "down" migrations anywhere in this project.

## When NOT to use this skill

| You are doing… | Use instead |
|---|---|
| Setting up split-dev (cargo run + Vite), building the image locally, dev DB | `.claude/skills/futurefin-build-and-env/SKILL.md` |
| Measuring latency, cache hit rates, `smoke-projection-cache.sh`, curl recipes | `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md` |
| Triaging a crash/bug once you have logs (app-level root cause) | `.claude/skills/futurefin-debugging-playbook/SKILL.md` |
| Understanding every env var / compose variable / query param | `.claude/skills/futurefin-config-and-flags/SKILL.md` |
| Cutting a release, version bump, dev→main merge, migration discipline | `.claude/skills/futurefin-change-control/SKILL.md` |

## 1. Production deploy from zero

```bash
# 1. Get docker-compose.yml (from this repo) into an empty directory.
# 2. That's it. NO environment variable is required since 3.0.0 —
#    an empty .env, or no .env at all, is a valid configuration.
docker compose up -d

# 3. Smoke test (first start needs a moment; the healthcheck start_period is 120 s):
curl -sf http://127.0.0.1:8080/v1/health
# → {"status":"ok","service":"futurefin","version":"3.0.0"}
curl -sf http://127.0.0.1:8080/v1/ready     # 200 only when the embedded PostgreSQL answers
```

What happens on a fresh first start, in order (source: `apps/api/docker-entrypoint.sh`):

1. Compose pulls `maxlainz/futurefin:latest` (override with `FUTUREFIN_IMAGE`/`FUTUREFIN_TAG`)
   and creates the `pgdata` + `ffdata` volumes. **There is no `postgres:*` image to pull any
   more** — PostgreSQL 16 binaries ship inside the FutureFin image.
2. The entrypoint checks that a real volume is mounted at `$PGDATA` (`mountpoint`) and
   **aborts otherwise** — see §4.4.
3. `initdb` creates the cluster: `UTF8`, locale `C.UTF-8`, `--data-checksums`,
   `--auth-local=trust --auth-host=scram-sha-256`, superuser `$POSTGRES_USER`
   (default `futurefin`).
4. The postmaster starts **socket-only**: `listen_addresses=''`, sockets in
   `/var/run/postgresql`. **PostgreSQL is not reachable over TCP at all** — not from the host,
   not from other containers, not even inside the container.
5. The API is launched as the unprivileged `futurefin` user with
   `DATABASE_URL=postgres:///futurefin?host=/var/run/postgresql&user=futurefin`, connects,
   **auto-runs all pending SQL migrations**, and serves API + SPA on port 8080
   (`WEB_STATIC_ROOT=/app/web`).
6. Open `http://<host>:8080` and register. **The first user to register automatically becomes
   the installation owner** (`bootstrap_installation_as_owner_if_empty`). Later registrants
   are "pending" and see no data until the owner approves them.

Compose defaults worth knowing (all overridable in `.env`): `APP_PORT=8080`,
`FUTUREFIN_IMAGE=maxlainz/futurefin`, `FUTUREFIN_TAG=latest`, `POSTGRES_USER=futurefin`,
`POSTGRES_DB=futurefin`. **`POSTGRES_PASSWORD` is no longer needed** (local socket + `trust`);
if you set it anyway, the entrypoint applies it to the role and nothing else — harmless, and
kept for 2.x compatibility. If you terminate TLS in front of the app, set `COOKIE_SECURE=true`
on the `futurefin` service (defaults to false; details in futurefin-config-and-flags).

Two containers still in `docker ps`? You are running a 2.x compose file. Go to §2.4.

## 2. Images, tags, upgrade, rollback

### 2.1 Where images come from

`.github/workflows/publish-image.yml` publishes three ways: **auto-tag on merge** (the normal
path since 4.0.6 — every push to `main` whose `Cargo.toml` carries an untagged version waits for
that commit's CI, creates the tag and builds in the same run; a bump-less merge is a seconds-long
green no-op), an explicit git tag push matching `v[0-9]+.[0-9]+.[0-9]+`, or manual
`workflow_dispatch` (rebuilds; with `create_tag`, a new version — idempotent if the tag already
exists). It builds `apps/api/Dockerfile` for **linux/amd64 + linux/arm64** and pushes to BOTH
registries:

| Registry | Image |
|---|---|
| Docker Hub | `maxlainz/futurefin` (requires `DOCKERHUB_USERNAME`/`DOCKERHUB_TOKEN` secrets) |
| GHCR | `ghcr.io/<repo-owner>/futurefin` (uses `GITHUB_TOKEN`, always) |

Tags per release: pushing `v3.0.0` publishes `:3.0.0`, `:3.0`, `:3`, and `:latest`.
Note the **image tags have no `v` prefix** — git tag `v3.0.0` → image tag `3.0.0`.

Image shape since 3.0.0 (`apps/api/Dockerfile`):
- Runtime base `debian:bookworm-slim`, **digest-pinned**, like every third-party base here.
- PostgreSQL **16** binaries (active) **and 15** (only to auto-`pg_upgrade` old volumes) are
  `COPY`-ed from digest-pinned `postgres:1x-bookworm` stages. The image advertises this with
  the label `com.futurefin.postgres.majors="15,16"` — read it with
  `docker inspect -f '{{index .Config.Labels "com.futurefin.postgres.majors"}}' <image>`.
- JIT (`llvmjit.so`) is deliberately stripped (~120 MB of libLLVM, useless for this workload),
  and a link gate fails the build if any PG binary is missing a `.so`.
- Size ≈ **320–360 MB uncompressed** (2.x was ≈120 MB *plus* a separate ~250 MB
  `postgres:16.4-alpine`) — total bytes pulled are comparable, in one image instead of two.

Two Dockerfile rules that must not be "simplified" away (they are load-bearing):
- **Do not base the runtime on `postgres:*`.** Its inherited `VOLUME` creates anonymous
  volumes on `docker run` without an explicit volume, and watchtower loses them on recreate —
  silent data loss.
- **Do not add a `VOLUME` instruction.** The entrypoint detects a real mount with `mountpoint`
  and refuses to start without one; a `VOLUME` would defeat that guard.

GHCR housekeeping: `.github/workflows/cleanup-ghcr.yml` runs weekly (Mon 03:00 UTC),
keeps anything tagged `vX.Y.Z` or `latest`, deletes `sha-*` versions older than 30 days and
other untagged/dev versions older than 60 days. Release tags are never deleted, so pinned
deployments stay pullable.

### 2.2 Pinning advice

`FUTUREFIN_TAG=latest` (the default) means every `docker compose pull` may jump versions,
including ones with new DB migrations — and, once, across the 2.x→3.x container-shape change.
For any installation you care about, **pin to a full version** in `.env`:

```env
FUTUREFIN_TAG=3.0.0
```

`:3.0` / `:3` float within minor/major and are a middle ground; `:X.Y.Z` is fully deterministic.

### 2.3 Routine upgrade inside 3.x

```bash
# 0. Optional but cheap: scripts/backup-postgres.sh (§5.2). The container ALSO takes its own
#    pre-migration dump automatically (§5.1) — that is the real safety net.
# 1. Edit .env: FUTUREFIN_TAG=3.0.1   (or leave :latest and accept the jump)
docker compose pull && docker compose up -d
# 2. Verify:
curl -sf http://127.0.0.1:8080/v1/health     # version field must show the new version
docker compose logs futurefin | grep -E "pre-migration backup written|migrations applied|ERROR"
```

New migrations run automatically on the first start of the new image, **after** the entrypoint
has written a pre-migration dump. A migration checksum mismatch **fails loud** (startup aborts;
no auto-repair since v1.3.0) — that is deliberate.

### 2.4 Upgrading from a 2.x two-container stack

This is a one-way, one-time operation. Read it all before starting.

**Before you touch anything** (belt and braces — do both):

```bash
# 1. Application layer: have every user export their .ffbackup from Ajustes (§5.3).
# 2. Infrastructure layer: a pg_dump of the 2.x database, taken with the 2.x stack running.
ENV_FILE=.env ./scripts/backup-postgres.sh    # 2.x form of the script; see §5.2 for 3.x
```

**The upgrade itself:**

```bash
# Replace docker-compose.yml with the 3.x one from this repo, then:
docker compose up -d --remove-orphans
```

`--remove-orphans` is what retires the now-unused `futurefin-database` container. **Do not
delete the `pgdata` volume** — the 3.x compose mounts the very same volume name at the very
same path, which is precisely how your data survives.

The first 3.x start does three one-time things, in this order, before the API comes up:

1. **Ownership adoption.** The 2.x cluster was created by `postgres:16.4-alpine` (uid **70**);
   the 3.x image runs the postmaster as Debian's `postgres` (uid **999**). The entrypoint
   `chown -R`s `$PGDATA` and logs
   `adopting ownership of PGDATA (uid 70 -> 999)`.
2. **Collation reindex.** Alpine's musl and Debian's glibc sort text differently, so every
   text index inherited from 2.x is suspect (a corrupt UNIQUE index would silently accept
   duplicate usernames). The entrypoint runs `REINDEX DATABASE` + `ALTER DATABASE …
   REFRESH COLLATION VERSION`, logging
   `reindexing database after adoption (musl->glibc collation) — one-time, may take a moment`
   and then `reindex complete in Ns`. It is idempotent per cluster: the cluster's *system
   identifier* is recorded in `ffdata` (`state/cluster.env`, key `REINDEXED_SYSID`), so it
   never runs twice for the same cluster.
3. **Automatic pre-migration backup** (§5.1), written to `ffdata` before any 3.x migration
   touches the schema. If it fails, **startup aborts** rather than migrating without a net.

That is why the compose healthcheck has `start_period: 120s`. On a large database the reindex
can take longer than that; the container will report `unhealthy` for a while and then recover —
follow `docker compose logs -f futurefin` instead of watching `docker ps`.

**If your 2.x install used a custom `POSTGRES_USER`/`POSTGRES_DB`**, set the same values in
`.env` for 3.x. The adopted cluster's superuser *is* that role; without it the entrypoint dies
with `cannot connect as role 'futurefin'. If your 2.x install used a custom POSTGRES_USER, set
the same value now.`

**Watchtower / auto-updaters, the awkward case (changed in 4.0.0).** A 2.x user who never edits
their compose file but auto-pulls `:latest` gets the new image *inside their 2.x compose*: no
volume mounted at `$PGDATA`, `DATABASE_URL` pointing at `futurefin-database`. Up to 3.9.0 the
entrypoint fell back to **external compat mode** and kept serving from the old database with a
`DEPRECATED` banner. **4.0.0 refuses to start instead** (`refuse_external_database`, exit 1),
printing the boxed instructions and touching nothing — because the alternative, starting on an
empty database, reads as data loss. The fix is the same as it always was, and now mandatory:
adopt the current compose file as above, which reuses the very same `pgdata` volume. CI pins both
halves of this (§9, scenario 5).

Post-upgrade smoke test (the v1.0.10 lesson): log in, open Jubilación, and **export a
`.ffbackup`** — not just `/v1/health`.

### 2.5 Rollback 3.x → 2.x

Supported, and deliberately boring, because 3.0.0 does not change the on-disk cluster shape:

```bash
docker compose down                       # ordered shutdown; volumes are kept
# restore your 2.x docker-compose.yml, and put POSTGRES_PASSWORD back in .env
docker compose up -d
```

- The `pgdata` volume needs no conversion: `postgres:16.4-alpine` re-`chown`s the data
  directory to its own uid at startup, exactly as the 3.x image did in the other direction.
  (The collation reindex done in step 2.4 is harmless under musl — indexes are simply rebuilt
  with the collation of whichever libc is running.)
- The `ffdata` volume becomes **orphaned**. Do NOT delete it if you want to keep the automatic
  pre-migration backups; copy them out first (§5.1).
- `POSTGRES_PASSWORD` is required again in 2.x (TCP + scram). If the role's password drifted,
  the 3.x entrypoint applied whatever `POSTGRES_PASSWORD` you had set, so use that value.

**The real constraint is unchanged: migrations only roll forward.** If the 3.x release applied
migrations the 2.x binary does not embed, sqlx's default migrator (`ignore_missing = false`)
aborts startup with `VersionMissing` — the same de-facto downgrade guard as always. Check first:

```bash
# Which migrations has THIS database applied? (3.x form — no separate DB container)
docker compose exec futurefin psql -h /var/run/postgresql \
  -U "${POSTGRES_USER:-futurefin}" -d "${POSTGRES_DB:-futurefin}" \
  -c "SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;"
# Which migrations does the TARGET (older) version ship?
git ls-tree --name-only <old-tag> apps/api/migrations/
```

- Applied set == old version's set → rollback is safe.
- DB has extra migrations → do NOT downgrade. Either roll forward to a fixed release, or
  restore the pre-migration dump the container itself wrote (§5.1) with
  `scripts/restore-postgres.sh` and accept losing data written since the upgrade.

### 2.6 Rollback inside 3.x

Same rules, minus the compose-file change: set `FUTUREFIN_TAG` back, `docker compose pull &&
docker compose up -d`, and respect the forward-only migration constraint above. One extra
guard: an image can never open a cluster created by a **newer** PostgreSQL major — the
entrypoint dies with `PGDATA was created by PostgreSQL <N>, NEWER than this image's 16.` So if
a future 4.x has already `pg_upgrade`d your volume to 17, you cannot roll back to 3.x without
restoring a dump.

## 3. Day-2 operations

### 3.1 Logs — one stream now

```bash
docker compose logs -f futurefin      # entrypoint + PostgreSQL + API, interleaved
```

`docker compose logs futurefin-database` **no longer exists**. PostgreSQL logs to stderr with
`logging_collector=off`, so its lines land in the same stream; entrypoint lines are prefixed
`[futurefin-entrypoint]`; API lines are `tracing` output. Useful filters:

```bash
docker compose logs futurefin | grep '\[futurefin-entrypoint\]'   # lifecycle only
docker compose logs futurefin | grep -E 'FATAL|ERROR|WARN'
```

API verbosity comes from `RUST_LOG`. The compose file sets the production default
`futurefin_api=info,tower_http=info,sqlx=warn` (same as the binary's built-in fallback).
For temporary debugging set e.g. `RUST_LOG=futurefin_api=debug,tower_http=debug,sqlx=info`
and `docker compose up -d` to recreate. PostgreSQL verbosity has its own knob,
`FUTUREFIN_PG_LOG_LEVEL` (maps to `log_min_messages`) — debug only.

### 3.2 Startup log milestones

3.0.0 adds an entrypoint phase **before** the API milestones, which are unchanged. A healthy
start logs, in this order:

**Entrypoint phase** (`[futurefin-entrypoint]` prefix):

1. `FutureFin 3.0.0 — mode=serve db_mode=auto postgres_majors=15 16`
2. Exactly one cluster-provisioning path:
   - `initializing fresh PostgreSQL 16 cluster in /var/lib/postgresql/data` — new install; or
   - `adopting ownership of PGDATA (uid 70 -> 999)` **+**
     `reindexing database after adoption (musl->glibc collation) …` — first start over a 2.x
     volume; or
   - `pg_upgrade needed: PostgreSQL 15 -> 16` (then the pg_upgrade sequence, §4.6); or
   - nothing at all — an already-adopted 3.x cluster starts silently.
3. `starting embedded PostgreSQL 16 (socket-only at /var/run/postgresql)`
4. Optional: `creating database futurefin` (cluster present, database absent).
5. Optional: `app version change or pending migrations detected — writing pre-migration backup`
   followed by `pre-migration backup written: /var/lib/futurefin/backups/pre-migration-…sql.gz (N M)`
6. `starting FutureFin API 3.0.0`

**API phase** (unchanged since v1.0.2; source `apps/api/src/main.rs`):

7. `futurefin starting` (with `version=`)
8. `database connected`
9. `migrations applied`
10. `server config` (with `port=`, `session_ttl_days=`, `cookie_secure=`)
11. `serving web UI and API on one port` (with `root=/app/web`) — if this instead says
    `WEB_STATIC_ROOT set but path missing — API only`, the UI will 404 while the API works
12. `listening on http://0.0.0.0:8080`

Whichever milestone is missing tells you the failing phase: nothing after (1) → guard abort
(volume, PG_VERSION, role) — read the `FATAL:` line, it is written to be self-explanatory;
stuck at (3) → the postmaster did not become ready in 60 s (`PostgreSQL did not become ready
within 60s`); stuck at (5) → the pre-migration dump is failing, which **intentionally** blocks
startup; nothing after (7) → the API cannot reach the socket; stuck between (8) and (9) → migration failure (checksum mismatch or SQL error — the error
is logged; resolution discipline in futurefin-change-control); missing (12) → port bind problem.

If either supervised process dies on its own the entrypoint logs
`PostgreSQL exited unexpectedly — shutting down` or `API exited unexpectedly — shutting down`,
stops the other one cleanly and exits **1**; `restart: unless-stopped` then restarts the
container.

### 3.3 Health vs ready

| Endpoint | Checks | Use for |
|---|---|---|
| `GET /v1/health` (also `GET /health`) | Process is up; no DB touch. Returns `{status, service, version}`. Always 200 if the process runs. | Liveness, smoke test, "which version is deployed?" |
| `GET /v1/ready` | Runs `SELECT 1` against the embedded PostgreSQL. 200 if reachable, **503** if not. | Readiness / dependency check, **and the container healthcheck** |

Source: `apps/api/src/handlers/health.rs`, routes in `apps/api/src/routes/mod.rs`.
So: health OK + ready 503 = the API process is fine, the embedded database is not (a stuck
recovery, a full disk, a crashed postmaster the supervisor is about to act on).

### 3.4 Container healthcheck

`docker-compose.yml` (and the image's own `HEALTHCHECK`) probe **readiness**, not liveness:

```
test: ["CMD-SHELL", "curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null"]
interval: 15s   timeout: 5s   retries: 5   start_period: 120s
```

Two rules encoded here:
- **`CMD-SHELL` stays.** The v1.0.2 incident (exec-form `CMD` cannot resolve `curl` via PATH)
  is still live history; the image keeps `curl` installed for this.
- **No `</dev/tcp/...` fallback.** 2.x had one, because the probe hit `/v1/health` and a
  socket-level fallback was harmless. Probing `/v1/ready`, a fallback would mask a 503 and
  report `healthy` with the database down. It was removed on purpose — do not "restore" it.

Consequence worth internalising: since 3.0.0, **`docker ps` showing `healthy` does imply the
database is alive.** During the first start after a 2.x upgrade, expect `starting`/`unhealthy`
for as long as the reindex takes.

### 3.5 Ordered shutdown (and watchtower)

`docker compose stop` / `down` sends SIGTERM to PID 1 (the entrypoint), which runs:

1. `[futurefin-entrypoint] shutdown signal received — stopping API first, then PostgreSQL (fast)`
2. **SIGTERM to the API** (`FUTUREFIN_API_STOP_TIMEOUT`, default 15 s, escalating to SIGKILL).
   The API logs `shutdown signal received — draining connections` → `http server stopped` →
   `database pool closed` (axum graceful shutdown, then `pool.close()`).
3. **SIGINT to the postmaster** = PostgreSQL **fast shutdown** (checkpoint and exit).
   `FUTUREFIN_PG_STOP_TIMEOUT`, default 30 s, escalating to SIGQUIT (immediate).
   **Never SIGTERM the postmaster**: that is *smart* shutdown, which waits for clients forever.
4. PostgreSQL logs `database system is shut down`.
5. `[futurefin-entrypoint] clean shutdown complete`, exit code **0**.

`docker-compose.yml` sets `stop_grace_period: 60s` to cover the whole sequence.

> **Watchtower ignores `stop_grace_period`.** It uses its own timeout (10 s by default), which
> can SIGKILL the container mid-checkpoint. Set **`WATCHTOWER_TIMEOUT=60s`** on your watchtower
> container. A SIGKILL does not corrupt anything — PostgreSQL's WAL exists for this — but the
> next start pays for crash recovery, and on a big database that can exceed the healthcheck
> `start_period` and flap the container as `unhealthy`.

### 3.6 psql and other in-container tools

```bash
# Interactive SQL (the stack exposes PostgreSQL on NO TCP port, internal or external):
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin

# Any PG client tool is on PATH inside the image:
docker compose exec -T futurefin pg_dump -h /var/run/postgresql -U futurefin -d futurefin | gzip -9 > dump.sql.gz

# Anything that is not `serve` / `db-only` is exec'd verbatim by the entrypoint:
docker run --rm maxlainz/futurefin:3.0.0 pg_dump --version
```

### 3.7 Stop / cleanup

```bash
docker compose down --remove-orphans   # ordered shutdown; DATA IS KEPT (both volumes survive)
docker compose down -v                 # DESTRUCTIVE: deletes pgdata AND ffdata = all data + all automatic backups
docker compose restart futurefin       # full restart: PG bounces too. Clears the projection cache (harmless)
```

Note the change of blast radius: in 2.x `docker compose restart futurefin` only bounced the
API. Now it restarts PostgreSQL as well — still safe (ordered shutdown, then normal start), but
it is no longer a "free" operation on a busy instance.

## 4. The embedded database: guards, leftovers and one-shot upgrades

### 4.1 The knobs (entrypoint env vars)

| Variable | Default | Meaning |
|---|---|---|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded` — **synonyms since 4.0.0** (always the embedded cluster). `external` aborts with migration instructions; any other value aborts with `invalid FUTUREFIN_DB_MODE`. |
| `FUTUREFIN_MODE` | `serve` | `serve` \| `db-only`. Also settable as argv[1] (`CMD ["serve"]`). `db-only` = PostgreSQL only, no API (§4.7). |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | Anything else disables the automatic pre-migration dump. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | Newest N automatic backups are never pruned. |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | Beyond those N, prune files older than this. |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` = start without a volume at `$PGDATA`. **Throwaway use only** (CI, demos). |
| `FUTUREFIN_API_STOP_TIMEOUT` | `15` | Seconds to wait for the API on SIGTERM before SIGKILL. |
| `FUTUREFIN_PG_STOP_TIMEOUT` | `30` | Seconds to wait for the postmaster on SIGINT before SIGQUIT. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | *Binary* knob: how long `connect_with_retry` retries (backoff 0.5→1→2→4 s). Clamped to 1..=600. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` | Advanced: where state + backups live (the `ffdata` mount). |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | Advanced: automatic-backup directory. |
| `FUTUREFIN_PG_LISTEN` | `` (empty) | Debug: value for `listen_addresses`. Empty = socket-only. Do not set in production. |
| `FUTUREFIN_PG_LOG_LEVEL` | unset | Debug: `log_min_messages` for the embedded postmaster. |

Plus the compat/classic ones: `POSTGRES_USER`, `POSTGRES_DB`, `POSTGRES_PASSWORD` (optional,
applied to the role if present), `DATABASE_URL` (**no longer honoured in the container**, §4.2),
`APP_PORT`,
`FUTUREFIN_IMAGE`, `FUTUREFIN_TAG`, `RUST_LOG`, `COOKIE_SECURE`, `SESSION_TTL_DAYS`,
`CORS_ORIGINS`. Full catalog: futurefin-config-and-flags.

### 4.2 What a leftover `DATABASE_URL` does now (4.0.0)

The database is always the embedded one. `DATABASE_URL` is still *read*, but only to catch a
value dragged in from a 3.x or 2.x compose — "external" is defined exactly as before: a value
that does **not** contain `/var/run/postgresql`.

| Situation | What happens |
|---|---|
| `DATABASE_URL` unset, or pointing at the socket | Normal embedded start. (The entrypoint `export`s its own socket URL right before launching the API anyway, overwriting whatever was there.) |
| External value **and** `$PGDATA` already holds a cluster | **Ignored**, with `WARN: DATABASE_URL está definida pero FutureFin 4.0.0 solo usa la base embebida, que ya tiene tus datos — se ignora. Quítala de tu compose.` Startup continues normally. |
| External value **and** no cluster in `$PGDATA` | `refuse_external_database`: a boxed Spanish message on stderr + **exit 1**, before initializing anything. Nothing is written to the volume, nothing is read from the remote DB. |

Two details worth remembering:

- **The refusal runs before the no-volume guard (§4.4)**, so the watchtower-over-2.x-compose case
  reports the external message, not `no persistent volume`.
- `FUTUREFIN_DB_MODE=external` is still *parsed*, only to die with
  `FUTUREFIN_DB_MODE=external ya no existe: FutureFin 4.0.0 solo usa la base embebida…` — a
  deliberate courtesy so a 3.x compose gets an explanation instead of `invalid FUTUREFIN_DB_MODE`.

### 4.3 Getting someone off an external database

What the refusal of §4.2 tells the operator, and what you should tell them:

```
  FutureFin 4.0.0 ya no habla con bases de datos externas: PostgreSQL va dentro
  de la imagen. Tus datos NO se han tocado y siguen intactos donde están.

  Para migrarlos:
    1. Arranca UNA VEZ FutureFin 3.9.0 con esta misma DATABASE_URL y este mismo
       volumen. Copiará tus datos a la base embebida y te lo dirá en los logs
       ("automigration completed").
    2. Quita DATABASE_URL de tu compose.
    3. Vuelve a 4.0.0.
```

Triage before repeating it, because **most people do not need 3.9.0 at all**:

| Where their data actually is | Route |
|---|---|
| The `pgdata` volume of a 2.x two-container stack | Adopt the current compose (§2.4). The volume is adopted in place — no intermediate version. This is the common case. |
| A genuinely external server (managed, another host, another stack) | The 3.9.0 round trip above. It needs an **empty** volume mounted at `$PGDATA` and `DATABASE_URL` reaching the container, then dumps read-only, restores, and verifies by row census (§4.5). |
| Anywhere, but they would rather do it by hand | `pg_dump` the external DB and restore into a fresh volume with `scripts/restore-postgres.sh` (§5). |

Note that none of this repo's compose files pass `DATABASE_URL` into the container (no
`env_file:`, no `DATABASE_URL:` entry) — a 2.x compose does. Whoever runs the 3.9.0 step has to
make sure the variable actually reaches the image.

Operator-facing version of all this: `docs/actualizar.md` §«Vengo de 2.x o tengo una base de
datos externa».

### 4.4 The anti-data-loss guard

With no real mount at `$PGDATA`, the container **aborts**:

```
[futurefin-entrypoint] FATAL: no persistent volume is mounted at /var/lib/postgresql/data —
your data would be LOST when the container is recreated. Mount a volume (see docker-compose.yml)
or set FUTUREFIN_ALLOW_EPHEMERAL_DB=1 for throwaway use.
```

`FUTUREFIN_ALLOW_EPHEMERAL_DB=1` starts anyway and warns loudly — that is for CI and demos, not
for anything you would miss. A related guard: if `$PGDATA` is non-empty but has no `PG_VERSION`,
the entrypoint refuses to touch it (`refusing to touch it. Inspect the volume manually.`)
instead of guessing.

**Golden rule of the entrypoint: it never deletes a cluster.** Partial or superseded clusters
are `mv`-ed aside (`pgdata_old_15/`; before 4.0.0 also `failed-automigration-<ts>/`). The only things it deletes
are its own backups under retention and the `pg_upgrade` staging directory once copied.

### 4.5 One-shot automigration from an external database — REMOVED in 4.0.0

`automigrate_prepare` / `automigrate_restore` are gone from the entrypoint, together with
`exec_api_external` and `FUTUREFIN_EXTERNAL_WAIT_SECS`. **The last version that can do it is
3.9.0**, so this is now a description of what you send someone back to, not of what this image
does. Under 3.9.0, with `DATABASE_URL` set **and** an empty volume mounted at `$PGDATA`:

1. Wait up to `FUTUREFIN_EXTERNAL_WAIT_SECS` (60 s) for the external DB (`waiting for external
   database to answer (max 60s): migration source`). If it never answers, **abort** — starting
   empty would look like data loss.
2. `pg_dump --no-owner --no-privileges` of the external DB → `ffdata`
   (`pre-automigration-<ts>.sql.gz`). The external database is only ever **read**.
3. Row census of the source (table:count per table, dynamic — no hardcoded table list).
4. `initdb` a fresh embedded cluster, start it, restore the dump into it.
5. **Verify by census.** Any mismatch marks the automigration `failed` and aborts with both
   sides preserved.
6. `automigration completed: N rows across M tables. The external database is NO LONGER USED —
   you can retire it and remove DATABASE_URL.`

State lived in `ffdata` (`state/automigration.env`). A volume that went through it under 3.x
still carries that file and its `pre-automigration-*.sql.gz`; 4.0.0 reads neither, and prunes the
dump like any other `pre-*` backup. (Re-read the 3.9.0 code with
`git show v3.9.0:apps/api/docker-entrypoint.sh` before advising anyone on the details.)

### 4.6 Automatic `pg_upgrade` (old-major volume)

Trigger: `$PGDATA/PG_VERSION` ≠ 16 **and** the image bundles that major's binaries (15 today).
Sequence — every step is designed so the old cluster survives a failure:

1. **Space check**: needs ≈ **3× the current `$PGDATA` size** free in `ffdata`; aborts with the
   exact figures otherwise (`not enough free space for pg_upgrade: need ~NMB in …, have MMB`).
2. Start the old cluster cleanly (pg_upgrade rejects clusters in recovery), read
   `datcollate`/`datctype`/encoding/checksum settings, take a **mandatory** `pg_dumpall`
   backup → `pre-pgupgrade-15-to-16-<ts>.sql.gz`, census, stop.
3. `initdb` a staging cluster in `$STATE_DIR/pgupgrade/new` with **identical** locale, encoding
   and `--data-checksums`.
4. `pg_upgrade` in **copy mode** (deliberately *not* `--link`: the old cluster must remain
   usable). Failure ⇒ `old cluster untouched in $PGDATA; logs in …/pgupgrade/logs`.
5. Verify the staged cluster **by row census** before it is promoted.
6. **Resumable swap**: everything in `$PGDATA` is moved to `$PGDATA/pgdata_old_15/`, then the
   staging cluster is copied in. A crash mid-swap resumes from `state/pgupgrade.env`
   (`staged` → `swapping` → `copying` → `done`).
7. `vacuumdb --all --analyze-in-stages` afterwards, so the fresh statistics land before real
   traffic.

Log lines to look for: `pg_upgrade needed: PostgreSQL 15 -> 16`, `writing mandatory
pre-pg_upgrade backup (pg_dumpall)`, `running pg_upgrade 15 -> 16 (copy mode)`,
`pg_upgrade 15 -> 16 completed; old cluster preserved at /var/lib/postgresql/data/pgdata_old_15
(delete manually when satisfied)`.

**Delete `pgdata_old_15/` by hand** once you are happy — it is inside the `pgdata` volume and
costs you the old cluster's full size until you do.

**Major policy**: each image ships the current major **plus the previous one**. 3.x and 4.0.0
both ship 16 + 15 (label `com.futurefin.postgres.majors=15,16`); whenever a release moves to
17 + 16, a volume still on **15** must be brought to 16 by an image that still bundles 15. A
too-new volume is refused outright (§2.6); a too-old one aborts with the options spelled out
(`start an older FutureFin release that bundles <N>`, or dump with the official `postgres:<N>`
image and restore into a fresh volume with `scripts/restore-postgres.sh`).

### 4.7 `db-only` rescue mode

`FUTUREFIN_MODE=db-only` (or `CMD` `db-only`) starts **PostgreSQL only** — no API. The
entrypoint prints restore instructions and supervises the postmaster. Used by
`scripts/restore-postgres.sh`, and by hand when you need the database up while the app must
stay away from it.

In this mode `/v1/ready` does not answer at all, so **the container reports `unhealthy` — that
is expected**, not a symptom. It also refuses to run without a volume. (Up to 3.9.0 it also
refused to run in external mode; there is no external mode left to refuse.)

## 5. Backups — three layers

Do not confuse them:

| Layer | What | Scope | Who runs it |
|---|---|---|---|
| (1) Automatic pre-migration | Entrypoint `pg_dump` → `ffdata` before migrations run | Whole DB | The container itself, unattended |
| (2) Manual infrastructure | `scripts/backup-postgres.sh` → `pg_dump` to the host | Whole DB: all users, sessions, installation, `_sqlx_migrations` | Operator, cron |
| (3) Application | `.ffbackup` export/import over the API | ONE user's own rows, encrypted with their account password | Each user, from the UI or curl |

Layer (1) is the automatic net around upgrades — it lives *inside a Docker volume*, so it is
not off-host disaster recovery. Layer (2) is your disaster-recovery copy. Layer (3) is per-user
data portability. None replaces the others.

### 5.1 Automatic pre-migration backups

Written by the entrypoint **before the API starts**, therefore before any migration runs.

- **Trigger**: the app version changed since the last start (`ffdata` file
  `state/last-version`) **or** the image embeds migrations the database has not applied
  (compared against `/app/migration-versions.txt`, baked at build time from
  `apps/api/migrations/`). A database with an empty `_sqlx_migrations` (brand-new install) is
  skipped — there is nothing to lose yet. (Up to 3.9.0 the start right after an automigration was
skipped too.)
- **Output**: `/var/lib/futurefin/backups/pre-migration-<from>-to-<to>-<UTC ts>.sql.gz`
  (gzip -6), e.g. `pre-migration-2.3.0-to-3.0.0-20260816T031500Z.sql.gz`. `<from>` is
  `unknown` if the state file did not exist.
- **Failure aborts startup**: `pre-migration backup FAILED — refusing to start with pending
  migrations and no safety net.` Bypass deliberately with `FUTUREFIN_PREMIGRATION_BACKUP=off`.
- **Retention** (applies to all `pre-*.sql.gz` in the directory: the `pre-pgupgrade-*` dumps and
  any `pre-automigration-*` left over from a 3.x migration): the newest `FUTUREFIN_BACKUP_KEEP`
  (**10**) are untouchable; beyond those, files older than `FUTUREFIN_BACKUP_KEEP_DAYS`
  (**90**) are removed; and under **256 MB** free the oldest are pruned regardless of age,
  **never going below 3 files**.

Get them onto the host (do this before decommissioning a stack, and after any 2.x upgrade):

```bash
docker compose cp futurefin:/var/lib/futurefin/backups ./backups-auto
ls -lh backups-auto/
```

### 5.2 Manual infrastructure backup: `scripts/backup-postgres.sh`

Rewritten for the single container. Exact behavior as of 2026-08-16:

- **`ENV_FILE` is no longer required** (it was mandatory-and-defaulted-to-`.env.prod` in 2.x,
  a recurring foot-gun). Set it only if your compose needs a non-default env file.
- Checks that the `futurefin` service is running, then dumps **through the container's Unix
  socket**:
  `docker compose exec -T futurefin pg_dump -h /var/run/postgresql -U $POSTGRES_USER -d $POSTGRES_DB | gzip -9`
- Output: `${BACKUP_DIR:-./backups}/futurefin-postgres-<UTC timestamp>.sql.gz`.
- Retention: keeps the newest `KEEP_BACKUPS` (default **30**) matching files in `BACKUP_DIR`.
- Overridable: `SERVICE`, `ENV_FILE`, `BACKUP_DIR`, `KEEP_BACKUPS`, `POSTGRES_USER`,
  `POSTGRES_DB`. Backups land on the host filesystem — **ship them off-host yourself**.

Typical cron line (daily 03:15, deployment dir):

```
15 3 * * * cd /srv/futurefin && ./scripts/backup-postgres.sh >> backups/backup.log 2>&1
```

### 5.3 Restore: `scripts/restore-postgres.sh`

There is a restore script now, and it handles the awkward part (you cannot drop the database
the running API is connected to).

```bash
./scripts/restore-postgres.sh backups/futurefin-postgres-<ts>.sql.gz          # interactive
./scripts/restore-postgres.sh backups-auto/pre-migration-2.3.0-to-3.0.0-*.sql.gz --yes
```

What it does (6 steps, each echoed):

1. `docker compose stop futurefin` — ordered shutdown of the normal service.
2. `docker compose run -d --rm --no-deps -e FUTUREFIN_MODE=db-only` — a temporary container
   named `futurefin-restore` with **PostgreSQL only**, on the same volumes. Expect it to look
   `unhealthy`: `/v1/ready` has no API behind it (§4.7).
3. Row census **before**.
4. `DROP DATABASE IF EXISTS` + `CREATE DATABASE … OWNER …`, then restore the dump through the
   socket with `ON_ERROR_STOP=1`.
5. Row census **after**, then `docker stop` — a clean checkpointed shutdown.
6. `docker compose up -d futurefin` and poll `/v1/ready` for up to 120 s.

If the dump predates the running image's migrations, the normal start simply applies the
missing ones forward (`docker compose logs futurefin | grep 'migrations applied'`). If the dump
is NEWER than the image (contains migrations the binary does not ship), the downgrade rules of
§2.5 apply.

### 5.4 Application backup: per-user `.ffbackup`

Verified against `apps/api/src/routes/mod.rs` and `apps/api/src/handlers/backup_user/`.
Unchanged by 3.0.0 — same endpoints, same `schema_version` **6**. (`GET /v1/backup/export.zip`
has not existed since v1.0.9.) The real endpoints (all POST, session cookie required):

| Endpoint | Role required | Notes |
|---|---|---|
| `POST /v1/backup/user-export` | any installation member | Body `{"password": "<account pw>", "ui_preferences": {...}?}`. Verifies the account password, streams a binary `.ffbackup` (`futurefin-<user>-<YYYYMMDD>.ffbackup`). |
| `POST /v1/backup/user-import/preview` | write role (owner/member) | Body `{"file_b64": "<base64 of file>", "password": "..."}`. Decrypts and returns counts + `schema_version` without changing anything. 16 MiB body limit. |
| `POST /v1/backup/user-import` | write role | Same body **plus `"confirm_replace": true`** (400 without it). 16 MiB body limit. |

Semantics you must not misremember:

- **Scope is one user**: export contains only rows with `owner_user_id = self` (assets,
  allocation_rules, liabilities, budget_entries, planning_flows, categories used, history
  snapshots, transactions/imports/categorization rules, recurring rules) plus an *informative*
  installation snapshot (currency, tz, inflation, FIRE settings). The installation snapshot is
  NOT applied on import.
- **Import is replace-only and transactional**: it DELETEs all of the importing user's
  existing rows (allocation_rules first, then assets, etc., in FK dependency order) and
  inserts the backup's rows, all in one transaction. There is no merge mode. Always call
  `/preview` first; require the user-facing flow to show the counts before applying.
- **Encryption is password-derived**: AES-256-GCM with a key derived from the user's account
  password via Argon2id (m=19456, t=2, p=1), random salt+nonce per export, gzip-compressed
  payload. File layout: magic `FFBK`, format_version byte, plaintext JSON manifest, ciphertext.
  The manifest stays in clear so the server can reject unsupported versions without
  decrypting. Wrong password ⇒ generic decrypt failure (indistinguishable from corruption,
  by design). **If the user forgets the password that was current at export time, the file is
  unrecoverable.**
- **`schema_version` compatibility** (currently 6): v1..v5 files still import — they are
  migrated forward in memory (v1/v2 legacy per-asset contribution fields are **dropped**, not
  converted to allocation rules; v3→v4 fills an empty history-snapshot list; v4→v5 fills empty
  transactions/imports/rules; v5→v6 fills an empty recurring-rules list; the user reconfigures
  rules after import — deliberate, owner-signed-off in v1.1.0). v4 (v1.5.0) added the user's
  history snapshots; on import each snapshot item is re-linked to the freshly-created
  asset/liability UUIDs (`ledger_index`) or keeps its `item_key` verbatim. v5 (v1.6.0) added the
  spending transactions/imports/categorization rules; v6 (v1.8.0) added recurring-transaction
  rules (+ `BackupTransaction.recurring_rule_index`), all re-linked by index on import. Files with
  a schema_version NEWER than the server's are rejected with "update FutureFin to import this
  backup" — so **a v6 `.ffbackup` cannot be imported into a ≤1.7.x server** (clean rejection, not
  corruption). Format/DTO code:
  `apps/api/src/handlers/backup_user/{crypto.rs,schema.rs}`.

This is the layer to lean on when someone upgrades across the 2.x→3.x boundary and wants a
copy of their data that does not depend on any volume surviving.

## 6. Data locations: stateful vs stateless

| Thing | Where | Stateful? |
|---|---|---|
| PostgreSQL cluster (all application data) | Docker named volume `pgdata` (Compose project `futurefin` → `futurefin_pgdata`) at `/var/lib/postgresql/data`; also holds `pgdata_old_15/` after a `pg_upgrade` | **YES — the thing you must protect** |
| Automatic backups + entrypoint state + pg_upgrade staging | Docker named volume `ffdata` (`futurefin_ffdata`) at `/var/lib/futurefin`: `backups/pre-*.sql.gz`, `state/{cluster,pgupgrade}.env` (plus a legacy `automigration.env` on volumes migrated under 3.x), `state/last-version` | **Semi** — losing it loses your automatic backups and the one-time markers (a lost `REINDEXED_SYSID` makes the next start redo the adoption REINDEX: slow but safe; a lost `last-version` produces one spurious pre-migration dump) |
| PostgreSQL binaries (16 + 15) | Baked into the image (`/usr/lib/postgresql/{15,16}`) | Stateless |
| API binary | `/app/futurefin-api`, run as uid 10001 `futurefin` via `gosu` | Stateless |
| Web UI | Baked into the image at `/app/web` | Stateless |
| Projection cache | In-memory `HashMap` in `AppState` (sliding 60-min TTL, keyed installation/view/owner/density) | Lost on every restart — harmless; rebuilt on demand and warmed after login. Expect the first `/v1/projection/series` after a restart to be slower. |
| Sessions | `sessions` table in Postgres (NOT in memory) | Survive restarts; users stay logged in through upgrades |

`docker volume inspect futurefin_pgdata futurefin_ffdata` shows the host paths. Never
bind-mount over `$PGDATA` casually; never `docker compose down -v` on production (it now takes
your automatic backups with it, too).

## 7. Known operational incidents (short list — historical record, do not prune)

- **v1.0.2 — healthcheck false-unhealthy**: exec-form `CMD` couldn't find `curl`; fixed with
  `CMD-SHELL` + curl in the runtime image + a `/dev/tcp` fallback. Also added default `RUST_LOG`
  to compose (before that, containers logged nothing) and the startup milestones of §3.2.
  **Still binding in 3.x**: `CMD-SHELL` stays; the `/dev/tcp` fallback was *removed* in 3.0.0
  because the probe now checks `/v1/ready` and a fallback would mask a 503 (§3.4).
- **v1.0.10 — backup export 500 after a migration**: export SQL still selected columns a
  migration had dropped. Operational lesson: after any upgrade, smoke-test an export, not
  just `/v1/health`.
- **v1.3.0 — migration auto-repair removed**: checksum drift now aborts startup instead of
  silently "fixing" itself. If an upgrade loops on a checksum error, that is change-control
  territory, not something to patch around in production.
- **3.0.0 — musl→glibc collation (design-level, caught before release)**: a 2.x cluster built
  by `postgres:16.4-alpine` carries musl-sorted text indexes; opening it under Debian glibc
  without a `REINDEX` leaves UNIQUE indexes silently wrong (duplicate usernames would be
  accepted). Hence the one-time adoption reindex (§2.4) and the CI assertion that a duplicate
  registration still returns 409/422 after the migration.
- **3.0.0 — anonymous-volume trap (design-level)**: basing the runtime on `postgres:*`, or
  declaring `VOLUME` in the Dockerfile, would create anonymous volumes that watchtower drops on
  recreate. Both are forbidden, and the entrypoint's `mountpoint` guard is the backstop (§4.4).
- Container unhealthy / crash-looping today → follow the startup-milestone table (§3.2), read
  the entrypoint's `FATAL:` line, then `.claude/skills/futurefin-debugging-playbook/SKILL.md`.

## 8. What CI already proves about all this

`.github/workflows/ci.yml`, job **`docker-stack`** — the only automated evidence of "no data
loss". Do not weaken it. It builds the image and then exercises:

1. **Sanity**: both PG majors runnable, the `com.futurefin.postgres.majors=15,16` label, and
   the no-volume guard actually aborting; `shellcheck -S warning` over the entrypoint and
   `scripts/*.sh`.
2. **Fresh install**: `initializing fresh PostgreSQL 16` → `/v1/ready` → register/login/seed a
   category with non-ASCII text.
3. **Watchtower-style recreate**: `up -d --force-recreate` keeps the data.
4. **Clean shutdown**: `stop -t 60` must log `shutdown signal received`, `database pool closed`,
   `database system is shut down`, `clean shutdown complete`, and exit **0**.
5. **Real 2.x → 3.x**: a genuine `maxlainz/futurefin:2.3.0` two-container stack is seeded, then
   (a) the current image is dropped into the untouched 2.x compose → since 4.0.0 it **refuses to
   start** (`ya no habla con bases de datos externas`, `3.9.0`, exit ≠ 0); (b) the current compose
   takes over the volume → `adopting ownership of
   PGDATA`, `reindexing database after adoption`, same credentials still log in, a **duplicate
   username registration is rejected 409/422** (the corrupt-index detector), and a
   `pre-migration-*.sql.gz` exists in `ffdata`.
6. **Leftover `DATABASE_URL` + empty volume**: the container **aborts** (`ya no habla con bases de
   datos externas`, `docs/actualizar.md`) and the volume is asserted to be **still empty** — no
   half-initialized cluster. Replaced the old automigration scenario in 4.0.0.
7. **`pg_upgrade` 15→16**: a real PG15 volume with a marker row upgrades, `SHOW server_version`
   starts with 16, `pgdata_old_15/` exists and `pre-pgupgrade-15-to-16-*.sql.gz` was written.

Frozen fixtures live in `.github/testdata/` (`docker-compose.v2.yml`,
`docker-compose.v2-app-v3.yml`; `docker-compose.automigrate.yml` was deleted with the mode).

## Provenance and maintenance

Verified **2026-08-16** against **v3.0.0**, and §2.4/§4.1–§4.7/§9 re-verified **2026-08-22
against v4.0.0** (which removed the external-database mode: `exec_api_external`,
`automigrate_prepare`/`automigrate_restore`, `FUTUREFIN_DB_MODE=external` and
`FUTUREFIN_EXTERNAL_WAIT_SECS` no longer exist in the entrypoint), by reading: `docker-compose.yml`,
`docker-compose.dev.yml`, `docker-compose.local.yml`, `.env.example`,
`apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh`, `scripts/backup-postgres.sh`,
`scripts/restore-postgres.sh`, `.github/workflows/ci.yml`,
`.github/testdata/docker-compose.{v2,v2-app-v3}.yml`,
`apps/api/src/{main.rs,db.rs,state.rs,routes/mod.rs}`, `apps/api/src/handlers/health.rs`,
`apps/api/src/handlers/backup_user/{crypto.rs,schema.rs}`,
`.github/workflows/{publish-image.yml,cleanup-ghcr.yml}`, `CHANGELOG.md`.

Notes on facts that are asserted rather than measured: the ≈320–360 MB image size is an
estimate from the image's contents (Debian slim + two PG majors, JIT stripped), not a reading
off a published manifest — check with `docker image ls` after a local build if it matters.
`docker-compose.split-dev.yml` no longer exists; the dev database is now the standalone
`docker-compose.dev.yml` (project `futurefin-dev`, volume `devdata`) — that is
futurefin-build-and-env's territory, not this skill's.

Re-verify before trusting volatile facts:

- Current version: `grep -m1 '^version' apps/api/Cargo.toml`
- Migration count + latest: `ls apps/api/migrations/ | wc -l && ls apps/api/migrations/ | tail -1`
- Single service, grace period, readiness probe, volumes:
  `grep -n 'services:\|stop_grace_period\|/v1/ready\|start_period\|pgdata\|ffdata' docker-compose.yml`
- Shutdown contract + pg_upgrade + guards in the supervisor:
  `grep -n 'SIGINT\|stop_pid\|pg_upgrade\|no persistent volume\|clean shutdown' apps/api/docker-entrypoint.sh`
- Entrypoint env knobs and their defaults:
  `grep -n 'FUTUREFIN_[A-Z_]*:-' apps/api/docker-entrypoint.sh`
- Automatic-backup naming, retention and abort-on-failure:
  `grep -n 'pre-migration\|BACKUP_KEEP\|refusing to start' apps/api/docker-entrypoint.sh`
- Embedded DATABASE_URL and socket-only postmaster:
  `grep -n 'postgres:///\|listen_addresses\|unix_socket_directories' apps/api/docker-entrypoint.sh`
- Image healthcheck, PG majors label, no `VOLUME`:
  `grep -n 'HEALTHCHECK\|postgres.majors\|VOLUME' apps/api/Dockerfile`
- Backup/restore scripts talk to the single container over the socket:
  `grep -n 'compose exec\|FUTUREFIN_MODE=db-only\|/var/run/postgresql' scripts/*.sh`
- API startup + shutdown milestones: `grep -n 'tracing::info' apps/api/src/main.rs`
- DB connect retry knob: `grep -n 'FUTUREFIN_DB_CONNECT_TIMEOUT_SECS\|connect_with_retry' apps/api/src/main.rs apps/api/src/db.rs`
- Health/ready behavior: `grep -n 'SELECT 1' apps/api/src/handlers/health.rs`
- Backup routes still as documented: `grep -n 'backup' apps/api/src/routes/mod.rs`
- Backup schema version: `grep -n 'CURRENT_SCHEMA_VERSION' apps/api/src/handlers/backup_user/schema.rs`
- Container-path CI coverage: `grep -n 'docker-stack\|adopting ownership\|ya no habla con bases de datos externas\|pg_upgrade needed' .github/workflows/ci.yml`
- Published registries/tags: `grep -nE 'images|semver|maxlainz' .github/workflows/publish-image.yml`
- GHCR retention windows: `grep -nE 'KEEP_.*DAYS|KEEP_LATEST_DEV' .github/workflows/cleanup-ghcr.yml`
- Projection cache TTL: `grep -n 'PROJECTION_CACHE_TTL' apps/api/src/state.rs`
