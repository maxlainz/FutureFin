---
name: futurefin-run-and-operate
description: >
  Operating a FutureFin installation in production: deploying from zero with Docker Compose,
  upgrading/rolling back the image (FUTUREFIN_TAG), reading logs and startup milestones,
  health vs readiness probes, taking and restoring backups (both the pg_dump layer and the
  per-user encrypted .ffbackup layer), and knowing what data lives where. Load this skill when
  the task mentions: deploy, production, docker compose up, upgrade, rollback, downgrade,
  release image, Docker Hub, GHCR, image tags, :latest, pull, backup, restore, pg_dump,
  .ffbackup, export/import, container unhealthy in the operational sense (reading
  healthcheck/probe status during deploy/upgrade, restarting, rolling back — for diagnosing WHY
  a container is unhealthy from a symptom use futurefin-debugging-playbook), healthcheck,
  /v1/health, /v1/ready, logs, RUST_LOG, volume, pgdata, data loss, stop the stack. Do NOT load for: setting up a local
  DEV environment or building the image locally (futurefin-build-and-env), measuring
  performance or cache behavior (futurefin-diagnostics-and-tooling), diagnosing an app-level
  bug behind a failing container (futurefin-debugging-playbook), the meaning of each env
  var (futurefin-config-and-flags), or how releases are cut and tagged
  (futurefin-change-control).
---

# FutureFin — Run and Operate

Runbook for deploying, upgrading, backing up and operating a production FutureFin
installation. All commands run from the directory containing `docker-compose.yml`
(the repo root, or a server directory holding just `docker-compose.yml` + `.env`).

Facts date-stamped 2026-07-02 (backup/version facts refreshed 2026-07-06 for v1.5.0),
app version **v1.5.0**, backup `schema_version` **4**, 32 SQL migrations in
`apps/api/migrations/`.

Vocabulary (defined once):
- **Installation** — the singleton row in the `installation` table; one per deployment.
  All financial data belongs to it. The first registered user becomes its **owner**.
- **Stack** — the two Compose services: `futurefin` (API + embedded web UI, one container,
  port 8080) and `futurefin-database` (Postgres 16.4).
- **`.ffbackup`** — FutureFin's per-user encrypted application-level backup file format.
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
# 2. Create .env next to it — POSTGRES_PASSWORD is the ONLY required variable:
cat > .env <<'EOF'
POSTGRES_PASSWORD=a_strong_password_here
EOF

# 3. Start the stack:
docker compose up -d

# 4. Smoke test (the container needs ~15 s to pass its start_period):
curl -sf http://127.0.0.1:8080/v1/health
# → {"status":"ok","service":"futurefin","version":"1.5.0"}
```

What happens on first start, in order (see `apps/api/src/main.rs`):
1. Compose pulls `maxlainz/futurefin:latest` (override with `FUTUREFIN_IMAGE`/`FUTUREFIN_TAG`)
   and `postgres:16.4-alpine` **pinned by digest** in `docker-compose.yml`.
2. Postgres initializes the `pgdata` volume; its healthcheck (`pg_isready`) gates the API
   via `depends_on: condition: service_healthy`.
3. The API connects, **auto-runs all pending SQL migrations**, then serves API + web UI on
   internal port 8080 (host port `APP_PORT`, default 8080). `WEB_STATIC_ROOT=/app/web`
   makes the container serve the SPA and the API on the same port.
4. Open `http://<host>:8080` and register. **The first user to register automatically becomes
   the installation owner** (`bootstrap_installation_as_owner_if_empty`). Later registrants
   are "pending" and see no data until the owner approves them.

Compose defaults worth knowing (all overridable in `.env`): `POSTGRES_USER=futurefin`,
`POSTGRES_DB=futurefin`, `APP_PORT=8080`, `FUTUREFIN_IMAGE=maxlainz/futurefin`,
`FUTUREFIN_TAG=latest`. `DATABASE_URL` is composed automatically from the Postgres vars.
If you terminate TLS in front of the app, also set `COOKIE_SECURE=true` on the `futurefin`
service (defaults to false; details in futurefin-config-and-flags).

## 2. Images, tags, upgrade, rollback

### Where images come from

`.github/workflows/publish-image.yml` publishes on every git tag matching `v[0-9]+.[0-9]+.[0-9]+`
pushed to the repo (tags are cut **from `main`** per the release process), plus manual
`workflow_dispatch` on an existing tag. It builds `apps/api/Dockerfile` for
**linux/amd64 + linux/arm64** and pushes to BOTH registries:

| Registry | Image |
|---|---|
| Docker Hub | `maxlainz/futurefin` (requires `DOCKERHUB_USERNAME`/`DOCKERHUB_TOKEN` secrets) |
| GHCR | `ghcr.io/<repo-owner>/futurefin` (uses `GITHUB_TOKEN`, always) |

Tags per release: pushing `v1.2.3` publishes `:1.2.3`, `:1.2`, `:1`, and `:latest`.
Note the **image tags have no `v` prefix** — git tag `v1.4.3` → image tag `1.4.3`.

GHCR housekeeping: `.github/workflows/cleanup-ghcr.yml` runs weekly (Mon 03:00 UTC),
keeps anything tagged `vX.Y.Z` or `latest`, deletes `sha-*` versions older than 30 days and
other untagged/dev versions older than 60 days. Release tags are never deleted, so pinned
deployments stay pullable.

### Pinning advice

`FUTUREFIN_TAG=latest` (the default) means every `docker compose pull` may jump versions,
including ones with new DB migrations. For any installation you care about, **pin to a full
version** in `.env`:

```env
FUTUREFIN_TAG=1.4.3
```

`:1.4` / `:1` float within minor/major and are a middle ground; `:X.Y.Z` is fully deterministic.

### Upgrade

```bash
# 0. Take a DB backup first (section 4a) — migrations run automatically and only forward.
# 1. Edit .env: FUTUREFIN_TAG=1.5.0   (or leave :latest and accept the jump)
docker compose pull && docker compose up -d
# 2. Verify:
curl -sf http://127.0.0.1:8080/v1/health     # version field must show the new version
docker compose logs futurefin | grep -E "migrations applied|ERROR"
```

New migrations run automatically on the first start of the new image. A migration checksum
mismatch **fails loud** (startup aborts; no auto-repair since v1.3.0) — that is deliberate.

### Rollback — read before downgrading

Mechanically it is the same operation: set `FUTUREFIN_TAG` back to the previous version and
`docker compose pull && docker compose up -d`.

**BUT: migrations only roll forward.** If the newer version applied a migration, the database
schema now belongs to the newer version and the old binary will not accept it: sqlx's default
migrator (`ignore_missing = false`) aborts startup with `VersionMissing` when
`_sqlx_migrations` contains versions the binary does not embed — a de-facto downgrade guard.
(Even if it did start, dropped/renamed columns would break the old queries — see the v1.0.10
incident where SQL drifted from schema.) Before downgrading:

```bash
# Which migrations has THIS database applied?
docker compose exec futurefin-database \
  psql -U "${POSTGRES_USER:-futurefin}" -d "${POSTGRES_DB:-futurefin}" \
  -c "SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;"
# Which migrations does the TARGET (older) version ship?
git ls-tree --name-only <old-tag> apps/api/migrations/
```

- If the applied set equals the old version's set → rollback is safe.
- If the DB has extra migrations → do NOT just downgrade. Either roll forward to a fixed
  release instead, or restore the pre-upgrade `pg_dump` backup (section 4a) and accept losing
  data written since the upgrade. This is why the upgrade runbook starts with a backup.

## 3. Day-2 operations

### Logs

```bash
docker compose logs -f futurefin            # follow API logs
docker compose logs -f futurefin-database   # Postgres logs
```

Log verbosity comes from `RUST_LOG`. The compose file sets the production default
`futurefin_api=info,tower_http=info,sqlx=warn` (same as the binary's built-in fallback).
For temporary debugging set e.g. `RUST_LOG=futurefin_api=debug,tower_http=debug,sqlx=info`
on the `futurefin` service and `docker compose up -d` to recreate.

### Startup log milestones (present since v1.0.2; source: `apps/api/src/main.rs`)

A healthy start logs, in this order:

1. `futurefin starting` (with `version=`)
2. `database connected`
3. `migrations applied`
4. `server config` (with `port=`, `session_ttl_days=`, `cookie_secure=`)
5. `serving web UI and API on one port` (with `root=/app/web`) — if this instead says
   `WEB_STATIC_ROOT set but path missing — API only`, the UI will 404 while the API works
6. `listening on http://0.0.0.0:8080`

Whichever milestone is missing tells you the failing phase: nothing after (1) → DB
unreachable/bad `DATABASE_URL`; stuck between (2) and (3) → migration failure (checksum
mismatch or SQL error — the error is logged; resolution discipline in
futurefin-change-control); missing (6) → port bind problem.

### Health vs ready

| Endpoint | Checks | Use for |
|---|---|---|
| `GET /v1/health` (also `GET /health`) | Process is up; no DB touch. Returns `{status, service, version}`. Always 200 if the process runs. | Liveness, smoke test, "which version is deployed?" |
| `GET /v1/ready` | Runs `SELECT 1` against Postgres. 200 if DB reachable, **503** if not. | Readiness / dependency check |

Source: `apps/api/src/handlers/health.rs`, routes in `apps/api/src/routes/mod.rs`.
So: health OK + ready 503 = the API process is fine, the database is not.

### Container healthcheck

`docker-compose.yml` healthcheck for `futurefin`:
`curl -sf http://localhost:8080/v1/health || bash -c '</dev/tcp/localhost/8080'`
— interval 15 s, timeout 5 s, 5 retries, start_period 15 s. It probes liveness only
(not the DB), so `docker ps` can show "healthy" while `/v1/ready` returns 503.

Historical incident (v1.0.2, CHANGELOG): the healthcheck originally used exec-form `CMD`,
where `curl` did not resolve via shell PATH, so containers sat "unhealthy" while serving
fine. Fixed by switching to `CMD-SHELL`, adding `curl` to the runtime image, and keeping the
bash `/dev/tcp` fallback. If you see "unhealthy" today, it is real — triage with
`docker compose logs futurefin` and the milestone list above; deeper app-level triage lives
in `.claude/skills/futurefin-debugging-playbook/SKILL.md`.

### Stop / cleanup

```bash
docker compose down --remove-orphans   # stop containers; DATA IS KEPT (volume survives)
docker compose down -v                 # DESTRUCTIVE: also deletes the pgdata volume = all data
docker compose restart futurefin       # bounce just the API (clears projection cache, harmless)
```

## 4. Backups — two independent layers

Do not confuse them:

| Layer | What | Scope | Who runs it |
|---|---|---|---|
| (a) Infrastructure | `scripts/backup-postgres.sh` → `pg_dump` of the whole DB | Everything: all users, sessions, installation, `_sqlx_migrations` | Operator, cron |
| (b) Application | `.ffbackup` export/import over the API | ONE user's own rows, encrypted with their account password | Each user, from the UI or curl |

Layer (a) is your disaster-recovery and pre-upgrade safety net. Layer (b) is per-user data
portability (e.g. moving one person's data to another installation). Neither replaces the other.

### 4a. Infrastructure backup: `scripts/backup-postgres.sh`

Read the script before relying on it; exact behavior as of 2026-07-02:

- Env-file: reads `POSTGRES_USER` and `POSTGRES_DB` from **`ENV_FILE`, default `.env.prod`**
  — NOT `.env`. If your deployment uses `.env` (the normal case), run it as
  `ENV_FILE=.env ./scripts/backup-postgres.sh` or it exits with
  "Faltan POSTGRES_USER/POSTGRES_DB". Note it needs those two keys **literally present** in
  the file (it awk-greps them; compose defaults don't count), so add
  `POSTGRES_USER=futurefin` / `POSTGRES_DB=futurefin` to the env file if absent.
- Dump: `docker compose --env-file "$ENV_FILE" exec -T futurefin-database pg_dump -U $POSTGRES_USER -d $POSTGRES_DB | gzip -9`
  (plain-SQL dump through the running container; no TTY, cron-safe).
- Output: `${BACKUP_DIR:-./backups}/futurefin-postgres-<UTC timestamp>.sql.gz`
  (e.g. `futurefin-postgres-20260702T031500Z.sql.gz`).
- Retention: keeps the newest `KEEP_BACKUPS` (default **30**) matching files in
  `BACKUP_DIR`, deletes the rest. Backups live on the host filesystem — ship them off-host
  yourself (the script does not).

Typical cron line (daily 03:15, deployment dir):

```
15 3 * * * cd /srv/futurefin && ENV_FILE=.env ./scripts/backup-postgres.sh >> backups/backup.log 2>&1
```

Restore (standard pg_dump restore; there is no restore script in the repo). Restore into an
**empty** database — cleanest is to stop the API, drop/recreate the DB, replay, restart:

```bash
docker compose stop futurefin
docker compose exec -T futurefin-database psql -U futurefin -d postgres \
  -c "DROP DATABASE futurefin;" -c "CREATE DATABASE futurefin OWNER futurefin;"
gunzip -c backups/futurefin-postgres-<ts>.sql.gz \
  | docker compose exec -T futurefin-database psql -U futurefin -d futurefin
docker compose start futurefin   # replays no migrations: _sqlx_migrations came with the dump
```

If the dump predates the running image's migrations, startup will simply apply the missing
ones forward. If the dump is NEWER than the image (contains migrations the binary doesn't
ship), downgrade rules from section 2 apply.

### 4b. Application backup: per-user `.ffbackup`

Verified against `apps/api/src/routes/mod.rs` and `apps/api/src/handlers/backup_user/` as of
v1.4.3. (`GET /v1/backup/export.zip` no longer exists — replaced in v1.0.9 by the `.ffbackup`
endpoints and never re-added; the README described it until 2026-07-02, now fixed.) The real
endpoints (all POST, session cookie required):

| Endpoint | Role required | Notes |
|---|---|---|
| `POST /v1/backup/user-export` | any installation member | Body `{"password": "<account pw>", "ui_preferences": {...}?}`. Verifies the account password, streams a binary `.ffbackup` (`futurefin-<user>-<YYYYMMDD>.ffbackup`). |
| `POST /v1/backup/user-import/preview` | write role (owner/member) | Body `{"file_b64": "<base64 of file>", "password": "..."}`. Decrypts and returns counts + `schema_version` without changing anything. 16 MiB body limit. |
| `POST /v1/backup/user-import` | write role | Same body **plus `"confirm_replace": true`** (400 without it). 16 MiB body limit. |

Semantics you must not misremember:

- **Scope is one user**: export contains only rows with `owner_user_id = self` (assets,
  allocation_rules, liabilities, budget_entries, planning_flows, categories used, and — since
  v4 — the user's **history snapshots**) plus an *informative* installation snapshot (currency,
  tz, inflation, FIRE settings). The installation snapshot is NOT applied on import.
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
- **`schema_version` compatibility** (currently 4): v1, v2 and v3 files still import — they are
  migrated forward in memory (v1/v2 legacy per-asset contribution fields are **dropped**, not
  converted to allocation rules; v3→v4 just fills an empty history-snapshot list; the user
  reconfigures rules after import — deliberate, owner-signed-off in v1.1.0). v4 (v1.5.0) added
  the user's history snapshots to the payload; on import each snapshot item is re-linked to the
  freshly-created asset/liability UUIDs (`ledger_index`) or keeps its `item_key` verbatim.
  Files with a schema_version NEWER than the server's are rejected with "update FutureFin to
  import this backup" — so **a v4 `.ffbackup` cannot be imported into a ≤1.4.x server** (clean
  rejection, not corruption). Format/DTO code:
  `apps/api/src/handlers/backup_user/{crypto.rs,schema.rs}`.

## 5. Data locations: stateful vs stateless

| Thing | Where | Stateful? |
|---|---|---|
| All application data | Docker named volume `pgdata` (Compose project `futurefin` → actual volume name `futurefin_pgdata`) mounted at `/var/lib/postgresql/data` | YES — the only thing you must protect |
| API container | `maxlainz/futurefin` image, runs as `nobody`, entrypoint execs `/app/futurefin-api` | Stateless — safe to destroy/recreate anytime |
| Web UI | Baked into the image at `/app/web` | Stateless |
| Projection cache | In-memory `HashMap` in `AppState` (sliding 60-min TTL, keyed installation/view/owner/density) | Lost on every restart — harmless; rebuilt on demand and warmed after login. Expect the first `/v1/projection/series` after a restart to be slower. |
| Sessions | `sessions` table in Postgres (NOT in memory) | Survive restarts; users stay logged in through upgrades |

`docker volume inspect futurefin_pgdata` shows the host path. Never bind-mount over it
casually; never `docker compose down -v` on production.

## 6. Known operational incidents (short list)

- **v1.0.2 — healthcheck false-unhealthy**: exec-form `CMD` couldn't find `curl`; fixed with
  `CMD-SHELL` + curl in the runtime image + `/dev/tcp` fallback. Also added default `RUST_LOG`
  to compose (before that, containers logged nothing) and the startup milestones of section 3.
- **v1.0.10 — backup export 500 after a migration**: export SQL still selected columns a
  migration had dropped. Operational lesson: after any upgrade, smoke-test an export, not
  just `/v1/health`.
- **v1.3.0 — migration auto-repair removed**: checksum drift now aborts startup instead of
  silently "fixing" itself. If an upgrade loops on a checksum error, that is change-control
  territory, not something to patch around in production.
- Container unhealthy / crash-looping today → follow the startup-milestone table (section 3),
  then `.claude/skills/futurefin-debugging-playbook/SKILL.md`.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 by reading: `docker-compose.yml`, `docker-compose.local.yml`,
`.env.example`, `README.md`, `CHANGELOG.md` (v1.0.2, v1.0.9, v1.0.10, v1.1.0, v1.3.0, v1.4.0),
`scripts/backup-postgres.sh`, `apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh`,
`apps/api/src/{main.rs,state.rs,routes/mod.rs}`, `apps/api/src/handlers/health.rs`,
`apps/api/src/handlers/backup_user/{mod.rs,crypto.rs,schema.rs,export.rs,import.rs}`,
`.github/workflows/{publish-image.yml,cleanup-ghcr.yml}`.

(README's "Backups" section used to claim the removed `GET /v1/backup/export.zip`; fixed on
2026-07-02 to describe the `.ffbackup` endpoints + `scripts/backup-postgres.sh`.)

Re-verify before trusting volatile facts:

- Current version: `grep -m1 '^version' apps/api/Cargo.toml`
- Migration count + latest: `ls apps/api/migrations/ | wc -l && ls apps/api/migrations/ | tail -1`
- Backup routes still as documented: `grep -n "backup" apps/api/src/routes/mod.rs`
- Backup schema version: `grep -n "CURRENT_SCHEMA_VERSION" apps/api/src/handlers/backup_user/schema.rs`
- Compose image/tag defaults, volume, healthchecks: `grep -nE "image:|volumes:|healthcheck|FUTUREFIN" docker-compose.yml`
- Backup script env-file default and retention: `grep -nE "ENV_FILE|KEEP_BACKUPS|BACKUP_DIR|pg_dump" scripts/backup-postgres.sh`
- Published registries/tags: `grep -nE "images|semver|maxlainz" .github/workflows/publish-image.yml`
- GHCR retention windows: `grep -nE "KEEP_.*DAYS|KEEP_LATEST_DEV" .github/workflows/cleanup-ghcr.yml`
- Health/ready behavior: `grep -n "SELECT 1" apps/api/src/handlers/health.rs`
- Startup milestones: `grep -n "tracing::info" apps/api/src/main.rs`
- Projection cache TTL: `grep -n "PROJECTION_CACHE_TTL" apps/api/src/state.rs`
