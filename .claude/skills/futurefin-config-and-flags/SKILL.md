---
name: futurefin-config-and-flags
description: >
  Catalog of every configuration axis in FutureFin: environment variables of the API binary (PORT,
  DATABASE_URL, SESSION_TTL_DAYS, COOKIE_SECURE, CORS_ORIGINS, WEB_STATIC_ROOT, RUST_LOG,
  FUTUREFIN_DB_CONNECT_TIMEOUT_SECS, FUTUREFIN_MCP_ENABLED, FUTUREFIN_PUBLIC_URL,
  FUTUREFIN_RECONCILE_SWEEP_HOURS,
  FUTUREFIN_API_PORT, WEB_DEV_PORT,
  TEST_DATABASE_URL) and of
  the self-contained container entrypoint (FUTUREFIN_DB_MODE, FUTUREFIN_MODE, FUTUREFIN_BACKUP_KEEP*,
  FUTUREFIN_PREMIGRATION_BACKUP, FUTUREFIN_ALLOW_EPHEMERAL_DB, FUTUREFIN_STATE_DIR, POSTGRES_*),
  deployment knobs (FUTUREFIN_IMAGE/TAG, APP_PORT), the three docker-compose files
  (prod single-service, dev standalone, local-image override), API query-parameter flags
  (?view=mine, ?months, ?density=hybrid), request-body limits, and per-installation runtime
  settings (PATCH /v1/installation: base_currency, calendar_tz, show_age_mode,
  annual_inflation_assumption_percent, the fire_settings JSONB with swr_pct and tax_brackets
  bounds, and mcp_write_enabled — the live kill-switch of the MCP write tools). Load this skill when you need to know what an option is called, its default, its
  validation bounds, WHICH FILE parses it (Rust binary vs entrypoint vs compose), whether it is
  prod or dev-only, why production needs no env var at all since 3.0.0, why setting DATABASE_URL
  flips the image into deprecated external-DB mode, why a setting change returns 400, why an env
  var "isn't taking effect" (.env precedence), why CORS panics at startup, or when ADDING a new
  env var / installation setting / query param. Do NOT load it for step-by-step environment setup
  (use futurefin-build-and-env), deployment/upgrade/backup operations (futurefin-run-and-operate),
  or the MEANING of the FIRE math these settings feed (futurefin-fire-domain-reference).
---

# FutureFin configuration and flags

Env/compose/entrypoint facts re-verified on **2026-08-16 for v3.0.0** (the self-contained-image
release), plus the **v3.1.0 additions of 2026-08-17** (`FUTUREFIN_PUBLIC_URL`, the widened
`FUTUREFIN_MCP_ENABLED` scope); the query-param, body-limit and installation-settings sections were
last verified 2026-07-02 (v1.4.3) plus the v1.5.x/v1.6.0/v1.8.0/v2.x additions noted inline — none
of those introduced env vars. This skill is the single home for "what can be configured, where, with
what bounds".

**What 3.1.0 changed**: the embedded OAuth 2.1 authorization server added exactly **one** optional
env var, `FUTUREFIN_PUBLIC_URL` (§1.1) — production still needs none, because the issuer is derived
from the request headers by default. `FUTUREFIN_MCP_ENABLED` now gates OAuth too, with one
deliberate exception (§1.1). No new installation setting, no new query param.

**What 3.0.0 changed (read this before trusting any older note):** the Docker image is
**self-contained** — PostgreSQL 16 runs *inside* the single `futurefin` container over a Unix
socket (`/var/run/postgresql`, no TCP), so the compose service `futurefin-database` is gone,
**no environment variable is required in production** (an empty `.env` works), `DATABASE_URL` is
no longer composed for you (setting it now means "use an external database", deprecated), and a
new family of `FUTUREFIN_*` variables is parsed by the container **entrypoint**, not by the Rust
binary (§1.2). `docker-compose.split-dev.yml` was replaced by the standalone
`docker-compose.dev.yml` (§3).

Vocabulary used below:
- **Installation** — the singleton row in table `installation`; one per deployment; all financial
  data belongs to it. Its columns are the *runtime* settings (changed via API, stored in DB).
- **SWR** — Safe Withdrawal Rate: the % of net worth withdrawn per year in retirement.
  `FIRE number = annual expenses / (SWR/100)`.
- **split-dev** — the two-process dev mode: `cargo run` API on :8081 + Vite dev server on :8080,
  against the standalone dev Postgres of `docker-compose.dev.yml` on 127.0.0.1:5432.
- **Embedded / external DB** — embedded = the PostgreSQL inside the image (default since 3.0.0);
  external = a separate Postgres reached via `DATABASE_URL` (2.x behavior, **deprecated**,
  removed in 4.0.0).
- **Nominal vs real** — nominal = future euros; real = deflated to today's purchasing power.

## When NOT to use this skill

- Recreating a working dev environment from scratch, toolchain issues, local image builds →
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- Deploying, upgrading, rollback, backups, logs, smoke tests →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- What `swr_pct`, gross-up, the inflation-growing FIRE target, or the allocation cascade *mean*
  and how the engine consumes them → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- Changing behavior behind a config axis (new semantics, migrations) → gates in
  `.claude/skills/futurefin-change-control/SKILL.md`.
- curl recipes to observe cache hits, densities, etc. →
  `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.

## 1. Environment variable catalog

Three different processes parse configuration, and confusing them is the usual source of "my env
var does nothing": **§1.1** the Rust binary (`apps/api/src/main.rs`), **§1.2** the container
entrypoint (`apps/api/docker-entrypoint.sh`, Docker image only), **§1.3** compose itself
(`docker-compose*.yml`, substituted before any container starts).

### 1.1 API runtime (parsed in `apps/api/src/main.rs`)

| Variable | Default | Bounds / parsing | Prod or dev | Notes |
|---|---|---|---|---|
| `DATABASE_URL` | **none — the binary still panics with `expect` if unset** | any Postgres URL | both | **Changed in 3.0.0.** In the image you no longer set it: the entrypoint exports `postgres:///$POSTGRES_DB?host=/var/run/postgresql&user=$POSTGRES_USER` (Unix socket) right before launching the binary. Setting it yourself in production switches the image to **external-DB mode** — deprecated, removed in 4.0.0, and with an empty mounted volume it triggers the one-shot automigration instead (§1.2). In split-dev you still set it by hand in `.env`: `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | u64, **1–600**; out of range or unparseable → **silently** 30 | both (new in 3.0.0) | Total budget for `db::connect_with_retry` (`apps/api/src/db.rs`), which retries with backoff 0.5s → 1s → 2s → 4s → 4s… instead of crash-looping. Matters in external-DB compat mode, where no `depends_on: service_healthy` guarantees ordering; with the embedded DB the entrypoint already waited on `pg_isready`. |
| `PORT` | `8080` | u16; unparseable → silently falls back to 8080 | both | API listen port, binds `0.0.0.0`. Use `8081` in split-dev so Vite can take 8080. Container always runs with `PORT=8080` — since 3.0.0 that comes **only** from the Dockerfile `ENV` (the prod compose no longer restates it); the host side is `APP_PORT`. |
| `SESSION_TTL_DAYS` | `30` | integer **1–400**; out-of-range or unparseable → **silently** 30 | both | Session cookie/DB row lifetime. Stored in `AppState.session_ttl_days`. |
| `COOKIE_SECURE` | `false` | true only for exact strings `1`, `true`, `TRUE`, `yes`, `YES` (`parse_bool_env`). `True`, `Yes`, `on` etc. parse as **false** | prod (behind HTTPS) | Sets the `Secure` attribute on the `ff_session` cookie. |
| `FUTUREFIN_MCP_ENABLED` | `true` | `parse_bool_env` (same quirk as `COOKIE_SECURE`: only `1/true/TRUE/yes/YES` are true — but here **unset → true**, any other string → false) | both (new in 3.0.0) | Parsed by `main.rs` into `AppState.mcp_enabled`. `false` means `routes::app_router` never mounts the `/mcp` router (404 from the fallback). **Widened in 3.1.0**: the same flag also drops `oauth::oauth_protocol_router()` — the 7 root routes (`/.well-known/oauth-protected-resource[/mcp]`, `/.well-known/oauth-authorization-server[/mcp]`, `POST /oauth/register|token|revoke`) — and the two consent endpoints `GET /v1/oauth/authorize-details` + `POST /v1/oauth/authorize`. **Exception, on purpose**: `/v1/api-tokens` and `GET/DELETE /v1/oauth/connections[/{id}]` stay mounted (`oauth_consent_router(mcp_enabled)` only gates the flow half) — turning MCP off must never strip your ability to revoke credentials you already granted. Default enabled: the surface is inert without credentials (everything 401s) and prod keeps its zero-required-vars story. Tested without touching the environment: the suites build the router by hand with `mcp_enabled = false` (`oauth_flow.rs::oauth_protocol_disabled_with_mcp_but_connections_panel_survives`, `mcp_http.rs::mcp_disabled_returns_404`). |
| `FUTUREFIN_RECONCILE_SWEEP_HOURS` | `24` | integer **0–168**; **0 = disabled**; out-of-range or unparseable → silently 24 (same laxity as `SESSION_TTL_DAYS`) | both (new in 3.8.1) | Horas entre **barridos de conciliación de transferencias**, la primera tarea periódica del binario (`main.rs::spawn_reconcile_sweep` → `handlers::transactions::reconcile::sweep_all_owners`). NO es el mecanismo principal: el pase automático ya corre tras **cada** mutación (alta, edición, borrado, import CSV, materialización de recurrentes). El barrido es la **red de reintento** de esos pases, que son best-effort y se tragan sus errores para no convertir una escritura ya persistida en un 5xx — sin él, un fallo puntual deja el par sin conciliar de forma permanente y silenciosa. La primera pasada corre **tras el primer intervalo, no al arrancar** (en el arranque no ha pasado nada que conciliar, y competir con migraciones y warm-up no compra nada). Se aborta ANTES de cerrar el pool en el apagado ordenado. En una instalación sana no encuentra nada y loguea a `debug`; solo sube a `info` si concilió algo o si algún owner falló. |
| `FUTUREFIN_PUBLIC_URL` | unset → **derived per request** | must parse as a URL, scheme `http`/`https`, host present, and be a **bare origin** (no path, query or fragment); normalized to `Url::origin().ascii_serialization()` (no trailing slash). Present-but-invalid → **panic at startup** (fail-loud, like `CORS_ORIGINS`) | prod, **optional** (new in 3.1.0) | Parsed by the **Rust binary** (`main.rs::public_url()`) into `AppState.public_url`; consumed by `oauth::url::public_base_url` as the OAuth **issuer** and as the base of every URL in the `.well-known` metadata, the `iss` of the authorize redirect, and the `resource_metadata` of the `/mcp` 401 challenge. Unset (the default and the normal case) → derived from the request: `X-Forwarded-Proto` + `X-Forwarded-Host` (first value of each) else the `Host` header, through a strict charset (`host[:port]`, bracketed IPv6, ≤255 chars, no `/ @`, spaces or control chars) → a bad host is **400 `invalid_request`**, no host at all likewise. **Set it only when your reverse proxy sends neither `X-Forwarded-*` nor a public `Host`** — otherwise the metadata would advertise an unreachable issuer and claude.ai reports "connection failed". Irrelevant with `FUTUREFIN_MCP_ENABLED=0`. Echoed in the startup `"server config"` line as `public_url=…` or `(derived from request)`. Tests never set it (`TestApp::spawn` passes `None`), which is what makes the forwarded-header derivation testable. |
| `CORS_ORIGINS` | `http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080` | comma-separated origins, entries trimmed, empties dropped; an unparseable entry **panics at startup**; empty result panics | prod, only for cross-origin API access | `allow_credentials(true)`, methods GET/POST/PATCH/DELETE/OPTIONS, headers `content-type`/`accept`/`authorization`/`mcp-session-id` (the last two added in 3.0.0 for browser MCP clients; `mcp-session-id` is also exposed). Same-origin deployments (the normal Docker image) never send CORS preflights, so the default is fine. |
| `WEB_STATIC_ROOT` | unset | path; empty/whitespace value treated as unset; set-but-missing path → startup warning, API-only mode | prod (Docker sets `/app/web`) | When the path exists, the SPA is served from it with `index.html` fallback (single-port mode). Omit in split-dev — Vite serves the UI. |
| `RUST_LOG` | `futurefin_api=info,tower_http=info,sqlx=warn` | tracing `EnvFilter` syntax; invalid filter → the default is used | both | Default is applied in `main.rs` when the env filter can't be built from the env. |

Not env-configurable (hardcoded constants — changing them is a code change):
- DB pool: `max_connections=10, min=1, acquire_timeout=5s, idle_timeout=600s, max_lifetime=1800s` (`apps/api/src/db.rs`).
- Projection cache TTL: 60 min sliding (`PROJECTION_CACHE_TTL`, `apps/api/src/state.rs`).
- Body limits: 1 MiB global, 16 MiB for backup import (`apps/api/src/routes/mod.rs`, see §4).
- Gzip compression for responses >1 KB (`main.rs`, `CompressionLayer`).

### 1.2 Container entrypoint (parsed in `apps/api/docker-entrypoint.sh`) — new in 3.0.0

The entrypoint is PID 1 in the image: it decides embedded vs external DB, initializes/adopts/
upgrades the cluster, takes the automatic pre-migration backup, launches the postmaster and the
API, and shuts both down in order. **None of these variables is required** — the defaults below
are exactly what production runs with an empty `.env`.

| Variable | Default | Values / bounds | Notes |
|---|---|---|---|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded` \| `external`; anything else **aborts** at startup (`invalid FUTUREFIN_DB_MODE`) | `auto` = embedded unless `DATABASE_URL` points somewhere other than the socket; then, with a **mounted but empty** volume it runs the one-shot automigration (dump external → restore embedded → row census verification), with an existing cluster the **embedded DB wins** and a warning is logged, and with **no** volume mounted it silently stays external (the watchtower-over-2.x-compose case). `external` forces the deprecated path and never automigrates. |
| `FUTUREFIN_MODE` | `serve` (or `argv[1]`) | `serve` \| `db-only` | `db-only` = rescue mode: PostgreSQL up, API **not** started, restore instructions printed. Any other `argv` is `exec`'d verbatim (`docker run … pg_dump --version`). |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | `on` = enabled; any other value disables it | Automatic `pg_dump` + gzip into `$FUTUREFIN_BACKUP_DIR` whenever the app version changed or migrations are pending. A **failing** backup aborts startup on purpose ("refusing to start with pending migrations and no safety net") — set it to `off` only to bypass deliberately. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | integer | The newest N automatic backups are never pruned. |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | integer (days) | Beyond the newest `KEEP`, files older than this are deleted. Plus an emergency prune when the volume drops under 256 MB free, which never goes below 3 files. |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` = allow | Guard against silent data loss: if `$PGDATA` is **not a real mountpoint** the container **aborts** ("no persistent volume is mounted"). `1` runs with a throwaway DB that dies with the container — never for real data. |
| `FUTUREFIN_EXTERNAL_WAIT_SECS` | `60` | seconds | How long automigration waits for the external DB to answer `pg_isready` before refusing to start empty. |
| `FUTUREFIN_API_STOP_TIMEOUT` | `15` | seconds | SIGTERM grace for the API before escalating to SIGKILL. |
| `FUTUREFIN_PG_STOP_TIMEOUT` | `30` | seconds | SIGINT (**fast** shutdown — never SIGTERM, which is *smart* and can hang) grace for the postmaster before SIGQUIT. Keep compose's `stop_grace_period: 60s` above `API_STOP_TIMEOUT + PG_STOP_TIMEOUT`. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` (Dockerfile `ENV`) | path | Volume `ffdata`: `state/` files, automatic backups, pg_upgrade staging. |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | path | Where `pre-migration-*`, `pre-pgupgrade-*` and `pre-automigration-*` dumps land. |
| `FUTUREFIN_PG_LISTEN` | empty = socket only | postgres `listen_addresses` | **Debug only.** Setting it opens TCP inside the container; production is socket-only by design. |
| `FUTUREFIN_PG_LOG_LEVEL` | unset | postgres `log_min_messages` (e.g. `debug1`) | Debug only. |
| `POSTGRES_USER` | `futurefin` | role name | Compat with 2.x: set it only if your 2.x install customized it, otherwise the adopted cluster's superuser won't match and startup dies with a clear message. |
| `POSTGRES_DB` | `futurefin` | database name | Same 2.x-compat rationale; created on first boot if missing. |
| `POSTGRES_PASSWORD` | unset | any string | **No longer required** (local socket, `trust`). If present it is only `ALTER ROLE … PASSWORD`-applied, for people who reach the role from outside. |

Dockerfile `ENV`s the entrypoint reads but you should not override: `PGDATA=/var/lib/postgresql/data`,
`PG_MAJOR=16`, `WEB_STATIC_ROOT=/app/web`, `PORT=8080`. The image also carries
`LABEL com.futurefin.postgres.majors="15,16"` (16 active, 15 bundled only to auto-`pg_upgrade`
older volumes) and a `HEALTHCHECK` on `/v1/ready`, and deliberately declares **no `VOLUME`** — the
mountpoint guard above depends on that.

### 1.3 Compose / deployment level (substituted by `docker-compose*.yml`, never seen by the binary)

| Variable | Default | Prod or dev | Notes |
|---|---|---|---|
| `FUTUREFIN_IMAGE` | `maxlainz/futurefin` | prod | Set to `futurefin-local` for the local-image test flow (§3). |
| `FUTUREFIN_TAG` | `latest` | prod | Pin to `X.Y.Z` for stability; rollback = change tag + `up -d`. |
| `APP_PORT` | `8080` | prod | **Host** port mapped to the container's fixed internal `:8080`. This is the distinction: `APP_PORT` = host side of the mapping, `PORT` = what the binary listens on inside the container (always 8080 there). |
| `POSTGRES_USER` / `POSTGRES_DB` | `futurefin` / `futurefin` | prod + dev | Passed through to the container (§1.2) and, in `docker-compose.dev.yml`, to the dev Postgres and its `pg_isready` healthcheck. |
| `POSTGRES_PASSWORD` | dev compose defaults it to `futurefin`; prod compose does not pass it at all | dev (prod: optional) | **Changed in 3.0.0**: the old `${POSTGRES_PASSWORD:?Set POSTGRES_PASSWORD in .env}` guard is gone — production no longer needs it. It still matters in split-dev, where it must match the password inside your `DATABASE_URL`. |

### Dev-only (Vite, tests, scripts)

| Variable | Default | Consumed where | Notes |
|---|---|---|---|
| `FUTUREFIN_API_PORT` | `8081` | `apps/web/vite.config.ts` | Vite proxy target port for `/v1`, `/health`, `/openapi.json` and — since 3.1.0 — `/.well-known`, `/oauth/token`, `/oauth/register`, `/oauth/revoke`, `/mcp`. Read **without** `VITE_` prefix — the config uses `loadEnv(mode, repoRoot, "")`, i.e. all vars, from the **repo root** `.env` (not `apps/web/.env`). **Never add a bare `"/oauth"` proxy key**: keys are prefixes, so it would hijack `/oauth/authorize` — an SPA view, not a backend route — and dev would 404 instead of showing the consent screen. |
| `WEB_DEV_PORT` | `8080` | `apps/web/vite.config.ts` | Vite dev-server port. `strictPort: false` — if 8080 is busy Vite silently picks the next port; check the terminal banner. |
| `TEST_DATABASE_URL` | `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test` | `apps/api/tests/common/mod.rs` | Postgres for integration tests (each test creates its own schema). Still not run in CI as of 2026-08-16 (no `TEST_DATABASE_URL` in any job) — see `.claude/skills/futurefin-validation-and-qa/SKILL.md`. |
| `BASE`, `SMOKE_USER`, `SMOKE_PASS` | `http://127.0.0.1:8080`, auto-registers throwaway user | `scripts/smoke-projection-cache.sh` | Owned by futurefin-diagnostics-and-tooling. |
| `ENV_FILE`, `BACKUP_DIR`, `KEEP_BACKUPS` | `.env.prod`, `./backups`, `30` | `scripts/backup-postgres.sh` | Owned by futurefin-run-and-operate. |

`.env.example` at the repo root is the canonical template: since 3.0.0 **every line in it is
commented out** — production runs with an empty `.env` or none at all. It documents the optional
prod knobs (`FUTUREFIN_TAG`, `APP_PORT`, `FUTUREFIN_IMAGE`, the two backup-retention vars, and since
3.1.0 `FUTUREFIN_MCP_ENABLED` + `FUTUREFIN_PUBLIC_URL`), the
2.x compat trio (`POSTGRES_USER`/`POSTGRES_DB`/`POSTGRES_PASSWORD`), the deprecated external-DB
pair (`DATABASE_URL`, `FUTUREFIN_DB_MODE=external`) and the dev block (`PORT=8081`,
`DATABASE_URL`, `RUST_LOG`) — with an explicit warning not to leave the dev `DATABASE_URL`
uncommented next to the production compose, because the image would read it as "I want an
external database".

## 2. `.env` loading order and precedence

API side — `main.rs::load_env()` runs before anything else:
1. `dotenvy::from_filename({CARGO_MANIFEST_DIR}/../../.env)` — the **repo-root** `.env`, resolved
   at compile time relative to `apps/api/Cargo.toml`. This is why `cargo run` from `apps/api`
   still picks up the root `.env`.
2. `dotenvy::dotenv()` — `.env` in the current working directory (a fallback; from repo root this
   is the same file).

dotenvy never overwrites variables already set: **real environment > repo-root `.env` > CWD
`.env`**. If a change to `.env` "isn't taking effect", check for the variable exported in your
shell or injected by compose — that wins. Both loads are `let _ = ... .ok()`: a missing `.env` is
silent, so in Docker (no `.env` in the image) only real env vars apply — and since 3.0.0 the one
that matters most, `DATABASE_URL`, is `export`ed by the entrypoint immediately before launching
the binary, so inside the container it always wins.

Vite side — `apps/web/vite.config.ts` computes `repoRoot = apps/web/../..` and calls
`loadEnv(mode, repoRoot, "")`. The empty-string third argument disables the `VITE_` prefix filter,
so `FUTUREFIN_API_PORT` / `WEB_DEV_PORT` are plain names in the root `.env`. These are dev-server
settings only; they are not baked into the client bundle.

## 3. Docker Compose file matrix

Three files, but only **one** of them is an override now (3.0.0 replaced
`docker-compose.split-dev.yml` — it no longer exists — with the standalone `docker-compose.dev.yml`):

| File | Project name | Scenario | What it does |
|---|---|---|---|
| `docker-compose.yml` | `futurefin` | Production / normal run | **One** service, `futurefin`: the published image, host `${APP_PORT:-8080}` → container 8080, `restart: unless-stopped`, `stop_grace_period: 60s` (the embedded postmaster needs room to checkpoint; Watchtower ignores it — set `WATCHTOWER_TIMEOUT=60s`). Volumes `pgdata:/var/lib/postgresql/data` (**same name and path as 2.x**, so upgrading reuses the data as-is) and `ffdata:/var/lib/futurefin` (automatic backups + pg_upgrade staging). Environment: only `RUST_LOG`, `POSTGRES_USER`, `POSTGRES_DB` — `PORT`/`WEB_STATIC_ROOT`/`PGDATA` come from the Dockerfile `ENV` and **`DATABASE_URL` is deliberately absent**. Healthcheck: `["CMD-SHELL", "curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null"]`, `interval 15s`, `timeout 5s`, `retries 5`, `start_period 120s` (first boot after a 2.x upgrade does chown + REINDEX + backup). CMD-SHELL is mandatory (v1.0.2 incident: the exec form doesn't resolve `curl` via PATH) and **no `</dev/tcp/…>` fallback** may be added — it would mask a 503 from `/v1/ready` and report healthy with the DB down. |
| `docker-compose.local.yml` | (inherits `futurefin`) | Test a locally built image without publishing | Unchanged: an override adding **`pull_policy: never`** to service `futurefin` — otherwise compose tries to pull `futurefin-local:dev` from Docker Hub and fails. Use with `FUTUREFIN_IMAGE=futurefin-local`, `FUTUREFIN_TAG=dev` in `.env`: `docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d`. Full recipe: CLAUDE.md "Test local con Docker Desktop". |
| `docker-compose.dev.yml` | `futurefin-dev` | split-dev (`cargo run` + `npm run dev:web`) | **Standalone, not an override** — the production file has no DB service left to override. Single service `db` (`postgres:16.4-alpine`, digest-pinned, container `futurefin-dev-db`) published on **`127.0.0.1:5432`**, volume `devdata`, `pg_isready` healthcheck, creds defaulting to `futurefin`/`futurefin`/`futurefin`. Usage: `docker compose -f docker-compose.dev.yml up -d` (no `-f docker-compose.yml`). Never in production. A comment inside explains how to keep your pre-3.0.0 dev data: replace the `devdata:` entry with `devdata: {external: true, name: futurefin_pgdata}`. |

Only `docker-compose.local.yml` is combined with the base file (`-f docker-compose.yml -f
docker-compose.local.yml`, base first). `docker-compose.dev.yml` is passed **alone** and lives in
its own compose project, so it never collides with a production stack on the same host — but note
the two projects would both want host port 5432/8080 respectively, and the dev volume is
`futurefin-dev_devdata`, *not* `futurefin_pgdata`.

The postgres image digest now appears in exactly two places: `docker-compose.dev.yml` (dev DB) and
`apps/api/Dockerfile` (the pg15/pg16 COPY source stages). It is **no longer** in
`docker-compose.yml`.

## 4. API query-parameter flags and body limits

### `?view=household|mine` — ledger scope (all ledger endpoints)

Defined in `apps/api/src/handlers/person_view.rs` (`LedgerViewQuery::resolve`). The value is
trimmed; exactly `mine` → `LedgerView::Mine` (adds `AND owner_user_id = <session user>`); **any
other value, including typos, silently means `household`** (full installation). Accepted by
assets, liabilities, summary, budget, planning, allocation-rules and projection handlers.
Non-negotiable semantics: this is a client-side display filter, **not** an authorization
boundary — every member sees household data. Handlers must build the WHERE via
`LedgerView::scope_where(alias)` + `bind_scope_as/scalar` (placeholder indices start at
`next_arg_index()`: 2 for household, 3 for mine); never hand-write the two branches.

### `GET /v1/projection/series?months=&density=&view=` (`apps/api/src/handlers/projection.rs`)

| Param | Values | Default | Semantics |
|---|---|---|---|
| `months` | u32, **clamped to 12–840** (no error on out-of-range) | omitted | Horizon override. Omitted → horizon derived from demographics: years until age 90 from ONE resolved birth date (session user's `users.birth_date`, else the first `persons` row by `is_primary DESC, sort_index ASC` — NOT the oldest member), clamped 5–70 years; no birth date at all → 30 years. `horizon_basis` in the response reports which path: `lifespan_90`, `fallback_no_demographics`, or `months_override`. (Implementation: `projection_horizon_months()`, `handlers/projection.rs` ~599–627.) |
| `density` | `monthly` \| `hybrid` (trimmed; anything else → `monthly`) | `monthly` | Serialization-only decimation: `monthly` ≈ one point per month (~841 at max horizon); `hybrid` = months 0..12 monthly + 24, 36, … annually (~82 points, ~5× smaller JSON). The engine always computes the **full** series; milestones/crossover indices are computed pre-decimation, so a `reached_month_index` may not exist as a point in a hybrid response — match by `month_index`, never by array position (the v1.4.2 chart bug). |
| `view` | as above | `household` | Also selects the cache partition. |

Cache-key implications (`apps/api/src/state.rs`): the in-memory projection cache is keyed by
`(installation_id, view, owner_user_id [Some only for mine], density)` with a 60-min sliding TTL.
**Any `?months=` override bypasses the cache entirely** (computed fresh, never stored). Adding a
new query param that changes response content requires either joining `ProjectionCacheKey` or
bypassing the cache — otherwise users get stale cross-contaminated responses. Every mutating
handler invalidates the whole installation's entries (`refresh_projection_after_mutation`).

### `GET /v1/history/*` — snapshot query params (`apps/api/src/handlers/history.rs`, v1.5.0)

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/history/snapshots` | `year` | `i32`, validated **1900–3000** (out of range → 400) | omitted → all years | Filters by a civil-date range (index-friendly), always own-user (no `?view`). |
| `GET /v1/history/snapshots` | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | omitted → both | Note: **stricter than `?view`/`?density`** — an unknown `kind` here **errors 400**, it does not silently fall back. |
| `GET /v1/history/series` | `view` | `household` \| `mine` (standard `LedgerViewQuery::resolve`) | `household` | Standard scope filter (§4.1). `mine` = own series; `household` = server-side sum of every user's interpolated series. |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | **required** | Which ledger side to pre-populate the backfill modal with. Always own-user (no `?view`). |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `date` | civil date `YYYY-MM-DD`; a future date → 400 `snapshot_date_in_future` | **required** | Target date the suggested values are interpolated to (same math as `/v1/history/series`). Each item returns a `value` + `basis` ∈ `interpolated`\|`first_snapshot`\|`live`\|`not_owned`; items that didn't exist yet arrive `value:"0"`, `existed:false`. |
| `GET /v1/history/cashflow` (v1.6.0) | `view` | standard `LedgerViewQuery::resolve` | `household` | Standard scope filter (§4.1) over transactions + snapshots. |
| `GET /v1/history/cashflow` (v1.6.0) | `window_months` | `i64`, **clamped 1..=120** (no error) | `24` | Months of monthly aggregate + fine-grid window. |
| `GET /v1/history/cashflow` (v1.6.0) | `resolution` | `weekly` \| `daily` (trimmed; anything else → `weekly`) | `weekly` | `daily` **requires `window_months <= 6`** → else **400 `daily_window_too_large`** (grid cost). `daily` runs in `spawn_blocking`; `weekly` inline. |

No new **env vars** and no new installation settings ship with the history feature (series,
prefill or cashflow) — it is entirely per-user request/data surface. The series and prefill
endpoints have **no cache** (sub-ms compute) and take no `?months`/`?density`; cashflow is also
uncached.

### `GET /v1/transactions/*` — histórico de gasto query params (`apps/api/src/handlers/transactions/`, v1.6.0)

Most read endpoints accept `?view` (standard §4.1 scope: `GET /v1/transactions`, `/months`,
`/summary`, `/imports`); the **rules** GET is always own-user (no `?view`), and all writes are
`owner_user_id = session user`. Additional filters, all optional unless noted:

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/transactions` | `month` | `YYYY-MM` (invalid → 400) | omitted → all | Filters `op_date` to that calendar month. Plus `kind` (`expense`\|`income`\|`savings`, invalid → 400), `category_id` (uuid), `import_id` (uuid). |
| `GET /v1/transactions/summary` | `year` + `month` | `year` 1900–3000, `month` 1–12; **provided together or neither** (else 400) | omitted → last **complete** calendar month | Selected month of the comparison. |
| `GET /v1/transactions/summary` | `avg_window` | `3` \| `6` \| `12` \| `ytd` \| `all` (trim + case-insensitive; anything else → **400 `avg_window must be one of 3, 6, 12, ytd, all`**) | `6` | Historical-average window (v1.8.0). Weighted average: denominator = months in the window with ≥1 transaction, not the window width. `ytd` = calendar months of the selected year strictly before the selected month (Jan → empty); `all` = since the first transaction. |
| `GET /v1/transactions/summary` | `avg_months` | u32, **1–24** (out of range → 400) | `6` | **Legacy alias** for `avg_window` (fixed-month window only). `avg_window` wins when both are sent. |
| `DELETE /v1/transactions/imports/{id}` | `confirm` | `bool` | `false` | Must be `true` or **400 `confirm_required`** (undo cascades to the batch's transactions). |

None of these query params (nor any transactions mutation) touch the projection cache — transactions
are not engine inputs (regression `transactions_projection_cache.rs`).

### Body limits (`apps/api/src/routes/mod.rs`)

- Global: `DEFAULT_BODY_LIMIT_BYTES` = 1 MiB (`DefaultBodyLimit` on the outer router).
- `POST /v1/backup/user-import` and `/v1/backup/user-import/preview`, plus (v1.6.0)
  `POST /v1/transactions/import/preview` and `/v1/transactions/import/confirm`:
  `BACKUP_IMPORT_BODY_LIMIT_BYTES` = 16 MiB (base64 `.ffbackup`/CSV payloads inflate ~33%).
- Symptom of hitting the limit: HTTP 413 on an otherwise valid request.

## 5. Per-installation runtime settings (`apps/api/src/handlers/installation.rs`)

Stored on the singleton `installation` row; read back in every `InstallationSnapshot`
(`GET /v1/installation`, `GET /v1/installation/session-context`). Amounts/percentages travel as
**strings** on the wire (Decimal-as-string; never floats).

`PATCH /v1/installation` — **owner role only** (403 otherwise); at least one field required (400
otherwise); a successful PATCH **invalidates** the projection cache (like every mutation — it does
NOT warm it; warm-up happens only after login, see futurefin-architecture-contract D7). Frontend surface:
`apps/web/src/views/SettingsView.tsx` (Ajustes) and `RetirementView.tsx` (FIRE settings).

| Setting | Set where | Validation | Default | Meaning |
|---|---|---|---|---|
| `base_currency` | **setup only** (`POST /v1/installation/setup`); not in the PATCH body → immutable afterwards without a migration | trimmed, exactly 3 ASCII letters, uppercased; MVP whitelist `EUR`/`USD`/`GBP` | `EUR` (auto-bootstrap path) | Display currency. |
| `calendar_tz` | setup + PATCH | trimmed, length 3–64, must parse as an IANA zone via `chrono_tz` (e.g. `Europe/Madrid`, `UTC`); DB CHECK mirrors the length/trim rules | `UTC` (serde default at setup; DB column `DEFAULT 'UTC'`) | Civil "today" for the whole installation (projection anchor month, liability expiry filtering, derive-principal). |
| `show_age_mode` | setup + PATCH | `dates` \| `ages` | `dates` | Whether the projection X axis shows calendar dates or the viewer's age. |
| `annual_inflation_assumption_percent` | PATCH only | sent as a **string** (`"2.5"`); empty string → `0`; must parse as decimal, bounds **0–50** (negative rejected) | `0` (column `NOT NULL DEFAULT 0`) | Annual % applied to the **moving FIRE target only** — `target(month_index) = base × (1+pct/100)^(month_index/12)` (`fire_target_at_month_index`; the engine evaluates month k against the target at index k−1 — see futurefin-fire-domain-reference §4); incomes/expenses/contributions stay nominal. `0` = flat target. Semantics owned by futurefin-fire-domain-reference. |
| `fire_settings` | PATCH only | JSONB, shape below | column nullable; `NULL` → defaults applied on read (`resolve_fire_settings`) | FIRE target computation config. |
| `mcp_write_enabled` | PATCH only | bool | `TRUE` (column `NOT NULL DEFAULT TRUE`, migración `20260818120000`) | Kill-switch **vivo** de las tools de escritura del servidor MCP (issue #3): `require_mcp_write` lo lee de la DB en cada llamada de escritura → apagarlo corta la escritura en el siguiente request sin reiniciar (`FUTUREFIN_MCP_ENABLED` sigue siendo el kill-switch de `/mcp` entero, en el entorno; este es un **DB setting**, no una env var — deliberado: tiene toggle en la GUI, Ajustes → MCP). Las lecturas MCP no lo consultan. |

### `fire_settings` JSONB shape (as of 2026-07-09; `savings_source` added)

```json
{
  "fire_number_mode": "annual_expense",          // "manual" | "annual_expense" | "current_income"
  "fire_number_manual_amount": null,             // decimal string; REQUIRED and > 0 when mode = "manual"
  "swr_pct": "3.5",                              // decimal string, 0–4 (PERCENT, not ratio)
  "taxes_enabled": true,
  "tax_brackets": [                              // capital-gains schedule used for gross-up
    { "up_to": "6000",   "pct": "19" },
    { "up_to": "50000",  "pct": "21" },
    { "up_to": "200000", "pct": "23" },
    { "up_to": "300000", "pct": "27" },
    { "up_to": null,     "pct": "30" }           // last bracket MUST be open-ended (up_to null)
  ],
  "savings_source": "budget"                     // "budget" (default) | "transactions_avg" | "budget_income_real_expense"
}
```

Validation (`validate_fire_settings` / `validate_tax_brackets`, all 400 on failure):
- `swr_pct` ∈ [0, 4].
- mode `manual` ⇒ `fire_number_manual_amount` present and > 0.
- When `taxes_enabled`: `tax_brackets` non-empty; each `pct` ∈ [0, 99]; only the **last** bracket
  may (and must) have `up_to: null`; non-last `up_to` values must be > 0 and strictly increasing.
- Brackets are **not validated when `taxes_enabled` is false** — stale brackets can sit dormant.

Consumers beyond the FIRE target (v2.3.0 widened them):
- **`swr_pct`** feeds the Jubilación target *and* `GET /v1/summary` → `financial_health.runway_is_indefinite`:
  the runway is indefinite ⟺ the grossed-up annual expense fits inside `swr_pct` × liquid balance
  (`.claude/engine.md` §Runway). `swr_pct = 0` is a valid setting that makes the flag unreachable.
- **`tax_brackets` / `taxes_enabled`** likewise reach the runway through the *same* gross-up
  (`gross_up_net_annual_fire`, `pub(crate)` in `handlers/projection.rs`), so editing brackets moves the
  FIRE target and the runway threshold together. Dormant brackets (`taxes_enabled = false`) affect
  neither.

Deserialization details that matter:
- `fire_number_mode` is **strict**: unknown strings → 422. Sole legacy alias
  `annual_expense_adjusted` (old backup schemas) maps to `annual_expense`.
- **`savings_source`** (`SavingsSource` enum, `installation.rs`, `rename_all = "snake_case"`, `Default = Budget`) — source of the simulation's monthly saving, **three modes**:
  - `budget` (mode A, default — from budget entries, historical behavior).
  - `transactions_avg` (mode B — income and expense from the weighted average of the last 12 complete calendar months of transactions, used **raw** since the 3.4.0 reform: paid cuotas count as ordinary spending; liabilities only subtract their pending principal from net worth, constant across the horizon).
  - `budget_income_real_expense` (mode C — income from the **budget** + **real** expense, same raw average as B; FIRE target `annual_expense` uses the real expense, `current_income` uses budget income).

  The 12m average that feeds the engine (`transactions_avg`) counts only **real months** (≥1 transaction with `recurring_rule_id IS NULL`); pseudo-empty / recurring-only months are excluded entirely. Strict deserialize like `FireNumberMode`: unknown value → **422** (error lists all three valid variants); absent → `budget` (via the struct-level `#[serde(default)]`; old backups load). No extra `validate_fire_settings` bound — any of the three enum values is accepted. **What it affects** (modes B and C, gated by `SavingsSource::uses_transactions()`): `GET /v1/projection/*` (engine income/expense + FIRE target base), `GET /v1/summary` `financial_health` (income/expense/net/savings_rate + fields `savings_source`, `savings_source_months_with_data`; in mode C income stays the budget income; since the 3.4.0 reform also `expense_derived`/`expense_total` — derived = 0, total = the raw `expense_avg` — and therefore `runway_months`), `GET /v1/assets` (`contribution_nominal_monthly` **and**, since v2.2.0, the `months_expense`/`income_multiple` caps behind `contribution_target_amount`), `GET /v1/projection/series` (echoes the effective mode in `savings_source` + `savings_source_months_with_data`), and — crucially — the **projection-cache invalidation contract**: in B/C transaction mutations invalidate the cache (`invalidate_projection_if_savings_uses_transactions`), in mode A they never do (D12/D12a in futurefin-architecture-contract). Read without a round-trip via `projection_savings_source(pool, iid)`. FIRE-math meaning owned by futurefin-fire-domain-reference.
- The struct has `#[serde(default)]`: omitted fields fill with defaults
  (mode `annual_expense`, `swr_pct` 3.5, `taxes_enabled` true, Spanish IRPF brackets above).
- In the PATCH body `fire_settings` is `Option<Option<FireSettings>>`: **omit** = unchanged,
  JSON **`null`** = clear stored JSON (defaults apply on read), **object** = validate + replace
  wholesale (no deep merge — send the full object).

Note: `installation.projection_target_age` no longer exists — dropped by migration
`20260516120000_drop_projection_target_age.sql` (v1.0.6). The FIRE crossover is the sole
retirement trigger; do not reintroduce an age setting.

## 6. How to add a new configuration axis

First decide the layer: **env var** = per-deployment, operator-set, needs restart;
**installation setting** = per-household runtime data, owner-set via UI, persisted in DB;
**query param** = per-request presentation/scoping only. Behavior changes ride
futurefin-change-control gates regardless of layer.

### New env var — checklist

**Step 0 (new in 3.0.0): decide the consumer, and say so in the docs.** A variable is parsed in
exactly one of three places, and the reader must be told which: the **Rust binary**
(`apps/api/src/main.rs`, needs a restart of the API), the **container entrypoint**
(`apps/api/docker-entrypoint.sh`, only exists in the Docker image, affects DB lifecycle/backups
before the API ever starts), or **compose substitution** (`docker-compose*.yml`, resolved on the
host before the container exists — so it is *not* visible inside the container unless also passed
through `environment:`). Anything touching cluster init, adoption, pg_upgrade, backups or process
supervision belongs to the entrypoint; anything the handlers read at request time belongs to the
binary.

1. **Binary consumer**: `apps/api/src/main.rs` — parse next to the existing helpers
   (`parse_bool_env`, `port()`, `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS`). Follow house style: explicit
   default, bounds via `.filter(...)`, never panic except for truly required values (only
   `DATABASE_URL` and bad `CORS_ORIGINS` panic today).
   **Entrypoint consumer**: add it to the `── Configuración ──` block at the top of
   `apps/api/docker-entrypoint.sh` as `NAME="${FUTUREFIN_X:-default}"` — all of them are defaulted
   there in one place, and CI runs `shellcheck -S warning` over the script.
2. If handlers need it at request time: add a field to `AppState` (`apps/api/src/state.rs`) and
   thread it through `AppState::new(...)` in `main.rs`.
3. Log it: the binary's startup `tracing::info!(... , "server config")` line, or an entrypoint
   `log ...` line, so deployments are auditable.
4. `.env.example` — add it, **commented out** (production must keep working with an empty `.env`),
   with the default noted and in the right block (prod / 2.x compat / external-DB / dev).
5. If production-relevant *and* it must reach the container: `docker-compose.yml` `environment:`
   block. Compose-only knobs (image, tag, host port) stay in the `${VAR:-default}` interpolations.
   Do **not** reintroduce a `${VAR:?…}` hard requirement — 3.0.0's contract is that production
   needs no variable at all.
6. Docs of record: `.claude/env-and-config.md` table + `README.md` "Environment variables" table,
   plus §1.1/§1.2/§1.3 here, stating **which file parses it**.
7. If integration tests need it: `apps/api/tests/common/mod.rs` follows the
   default-with-override pattern (`TEST_DATABASE_URL`).

### New installation setting — checklist
1. Migration `apps/api/migrations/YYYYMMDDHHMMSS_description.sql`: `ALTER TABLE installation ADD
   COLUMN ... NOT NULL DEFAULT ...` (a default keeps the existing singleton row valid). Never edit
   a shipped migration; data-losing migrations need explicit owner sign-off (change-control rule).
2. `apps/api/src/handlers/installation.rs`: add the field to `InstallationMemberRow`,
   `InstallationSnapshot`, `PatchInstallationBody` (as `Option<...>`, omit = unchanged); write a
   `validate_*` function with explicit bounds; extend the `UPDATE` in `patch_my_installation`, the
   "at least one field" guard, and **all three** `SELECT i.id, i.base_currency, ...` queries (they
   are duplicated in session-context / get / patch — miss one and reads return stale shape).
   Also update `setup_installation`'s hardcoded response snapshot if the field has a setup default.
3. New struct types → register in the schema list in `apps/api/src/openapi.rs`; utoipa path
   annotations pick up body changes automatically.
4. Frontend: `apps/web/src/api/types.ts` (snapshot + patch types) and the editing UI in
   `apps/web/src/views/SettingsView.tsx` (Ajustes) or `RetirementView.tsx` for FIRE-related knobs.
5. If projection math consumes it: thread through `build_installation_projection_input` and
   remember PATCH already invalidates the projection cache.
6. Integration test in `apps/api/tests/` covering the validation bounds (accept boundary, reject
   out-of-range) — see futurefin-validation-and-qa for the TestApp harness.
7. Docs of record: `.claude/data-model.md` (installation row + invariants) and
   `.claude/api-routes.md`; CHANGELOG entry per futurefin-docs-and-writing.

### New query param — checklist
Add the field to the handler's `#[derive(Deserialize)]` query struct with `#[serde(default)]`,
resolve with the trim-then-match pattern (unknown values fall back to the default, they don't
error — match existing `view`/`density` behavior), document it in the `#[utoipa::path]` `params`,
and if the endpoint is the cached projection route, extend `ProjectionCacheKey` in
`apps/api/src/state.rs` or bypass the cache (see §4). Update `.claude/api-routes.md`.

## Provenance and maintenance

Env/compose/entrypoint rows re-verified **2026-08-16 against v3.0.0**, the two OAuth-related
rows (`FUTUREFIN_PUBLIC_URL`, `FUTUREFIN_MCP_ENABLED`) **2026-08-17 against v3.1.0**, and the
`mcp_write_enabled` installation-setting row **2026-08-18** (issue #3; re-verify with
`grep -n "mcp_write_enabled" apps/api/src/handlers/installation.rs apps/api/src/mcp/auth.rs`);
the rest of the tables carry their own dates inline. Every row is re-verifiable — run these from the repo root when
auditing for drift (all confirmed working on 2026-08-17):

- Env parsing, defaults, bounds, load order: `grep -n "env::var\|unwrap_or\|contains(&d)\|load_env" apps/api/src/main.rs`
- DB connect budget + retry backoff: `grep -n "FUTUREFIN_DB_CONNECT_TIMEOUT_SECS" -A 6 apps/api/src/main.rs` and `grep -n "connect_with_retry" -A 20 apps/api/src/db.rs`
- **Entrypoint variables and their defaults (§1.2)**: `grep -n 'FUTUREFIN_[A-Z_]*:-\|FUTUREFIN_MODE\|FUTUREFIN_PG_LISTEN\|FUTUREFIN_PG_LOG_LEVEL' apps/api/docker-entrypoint.sh` (the whole config block is lines ~17–34)
- Entrypoint guards and abort messages (mountpoint guard, invalid db_mode, embedded-wins warning): `grep -n 'no persistent volume\|invalid FUTUREFIN_DB_MODE\|already contains an embedded cluster\|DEPRECATED' apps/api/docker-entrypoint.sh`
- Socket `DATABASE_URL` the entrypoint exports: `grep -n 'export DATABASE_URL' apps/api/docker-entrypoint.sh`
- CORS default origin list + panic + MCP headers: `grep -n "CORS_ORIGINS" -A 6 apps/api/src/main.rs` and `grep -n "mcp-session-id\|AUTHORIZATION" apps/api/src/main.rs`
- MCP kill-switch (added 2026-08-16, v3.0.0; widened to OAuth 2026-08-17, v3.1.0): `grep -n "FUTUREFIN_MCP_ENABLED" apps/api/src/main.rs` and `grep -n "mcp_enabled" apps/api/src/routes/mod.rs apps/api/src/state.rs apps/api/src/handlers/oauth_consent.rs` — the last hit must show `oauth_consent_router(mcp_enabled)` gating ONLY `/authorize-details` + `/authorize`, with `/connections` mounted unconditionally
- `FUTUREFIN_PUBLIC_URL` parsing, bounds and the four panics: `grep -n "FUTUREFIN_PUBLIC_URL" -A 14 apps/api/src/main.rs`; where it is consumed: `grep -n "public_url\|state.public_url" apps/api/src/state.rs apps/api/src/oauth/url.rs`
- Request-derived issuer (the default path) + strict host charset: `grep -n "x-forwarded-proto\|x-forwarded-host\|fn is_valid_host" -A 8 apps/api/src/oauth/url.rs`
- The 7 OAuth protocol routes gated by the kill-switch: `grep -n "route(" apps/api/src/oauth/mod.rs`
- Vite proxy keys (must list `/oauth/token|register|revoke` one by one and **no bare `/oauth`**): `grep -n "proxy\|/oauth\|well-known\|/mcp" apps/web/vite.config.ts`
- Pool constants: `grep -n "connections\|timeout\|lifetime" apps/api/src/db.rs`
- Cache TTL + key + Density docs: `grep -n "PROJECTION_CACHE_TTL\|pub enum Density\|ProjectionCacheKey" -A 6 apps/api/src/state.rs`
- Body limits: `grep -n "BODY_LIMIT" apps/api/src/routes/mod.rs`
- `?months` clamp + horizon: `grep -n "clamp(12, 840)\|LIFESPAN_AGE\|FALLBACK_YEARS\|lifespan_90" apps/api/src/handlers/projection.rs`
- `?density` / hybrid indices: `grep -n "resolve_density\|density_month_indices" -A 10 apps/api/src/handlers/projection.rs`
- `?view` resolution: `grep -n "fn resolve" -A 5 apps/api/src/handlers/person_view.rs`
- Installation validation bounds: `grep -n "normalize_currency\|validate_show_age_mode\|validate_annual_inflation\|normalize_calendar_tz\|swr_pct\|from(99u32)" apps/api/src/handlers/installation.rs`
- fire_settings defaults + legacy alias: `grep -n "default_fire_settings\|annual_expense_adjusted" -A 8 apps/api/src/handlers/installation.rs`
- `savings_source` enum + reader + conditional cache gating: `grep -n "enum SavingsSource\|savings_source\|projection_savings_source" apps/api/src/handlers/installation.rs apps/api/src/handlers/transactions/mod.rs`
- Compose file matrix (should list exactly three, no `split-dev`): `ls docker-compose*.yml`
- Compose services, volumes, healthcheck, ports, project names: `grep -n 'name:\|image:\|test:\|start_period\|stop_grace_period\|pull_policy\|5432\|/var/lib' docker-compose*.yml`
- Compose interpolation defaults / absence of hard requirements: `grep -n ":-\|:?" docker-compose*.yml`
- Dockerfile env, label, healthcheck, stages: `grep -n "^ENV\|^LABEL\|^HEALTHCHECK\|^FROM\|^CMD\|^ENTRYPOINT" apps/api/Dockerfile`
- Vite env reading: `grep -n "loadEnv\|FUTUREFIN_API_PORT\|WEB_DEV_PORT\|strictPort" apps/web/vite.config.ts`
- Test DB default: `grep -n "TEST_DATABASE_URL" apps/api/tests/common/mod.rs`
- Version stamp: `grep -n "^version" apps/api/Cargo.toml`

(The previously stale docs on these topics — data-model.md's `projection_target_age`,
env-and-config.md's fake `DATABASE_URL` "default", and the `mac_*` `horizon_basis` doc comment in
`handlers/projection.rs` — were all fixed on 2026-07-02. The standing-errata record lives in
futurefin-docs-and-writing §7.)

**Drift check for 3.0.0**: any doc, script or skill that still says
`docker-compose.split-dev.yml`, `docker compose up -d futurefin-database`, "`POSTGRES_PASSWORD`
is required", or composes `DATABASE_URL` from `POSTGRES_*` is stale — none of those exist any
more. `grep -rn 'split-dev\|futurefin-database' --include='*.md' --include='*.sh' .` finds them
(legitimate survivors: CHANGELOG history, `README`/`run-and-operate` telling you `--remove-orphans`
retires the old container, and `.github/testdata/docker-compose.v2*.yml`, which recreate a real
2.x stack on purpose to test the upgrade path).

When you change anything cataloged here, update this file in the same change, plus the matching
doc of record (`.claude/env-and-config.md`, `.claude/data-model.md`, `README.md`).
