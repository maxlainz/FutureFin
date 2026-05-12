# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development (split-dev: API + Vite hot reload)
```bash
cp .env.example .env
docker compose up -d futurefin-database   # Postgres only

# Terminal 1 — API at :8081 (auto-migrates DB on start)
cd apps/api && cargo run

# Terminal 2 — UI at :8080 with proxy to API
npm install
npm run dev:web
```
Open `http://127.0.0.1:8080`. The Vite proxy routes `/v1`, `/health`, `/openapi.json` to the API port.

### API only (no Vite)
Set `PORT=8080` in `.env`, then `cd apps/api && cargo run`.

### Full stack via Docker Compose
```bash
make docker-rebuild    # build + up (preferred to avoid stale containers)
make docker-up         # up without rebuild
make docker-smoke      # curl /v1/health
```
Or directly: `docker compose up -d --build`

### Rust
```bash
cd apps/api && cargo build
cd apps/api && cargo test
cargo test -p futurefin-engine           # engine unit tests only
cargo test -p futurefin-engine -- <name> # single test
```

### Frontend
```bash
npm run typecheck:web   # tsc --noEmit
npm run lint:web        # eslint
npm run build:web       # Vite production build → apps/web/dist/
```

### Production deploy (NAS)
```bash
docker compose --env-file .env.prod \
  -f docker-compose.prod.yml \
  -f docker-compose.tls.yml \
  up -d
```

## Architecture

### Workspace layout
```
Cargo workspace: apps/api + crates/domain + crates/engine
npm workspace:   apps/web (futurefin-web)
```

**crates/domain** — shared primitives: `UserId` (newtype over `Uuid`), re-exports `Decimal` and `Uuid`. No `f64` for monetary values anywhere in the domain.

**crates/engine** — pure projection math (`project_net_worth_series`, `first_month_per_asset_contribution_nominals`). No I/O, no DB; only `Decimal` arithmetic. Has unit tests.

**apps/api** — Axum HTTP server. Entry point: `main.rs`. Key modules:
- `routes/mod.rs` — full route map; all routes under `/v1/` except `/health` and `/openapi.json`
- `state.rs` — `AppState` (pool, cookie_secure, session_ttl_days, version)
- `error.rs` — `ApiError` → `(StatusCode, JSON {error, message})` via `IntoResponse`
- `auth/` — password hashing (Argon2id)
- `handlers/session.rs` — `require_session_user` reads cookie `ff_session` → validates against `sessions` table
- `handlers/installation.rs` — singleton installation, FIRE settings, `require_installation_member`
- `handlers/membership.rs` — roles: `owner`, `member`, `viewer`; `role_can_write` used by handlers
- `handlers/person_view.rs` — `LedgerView` enum (`Household` / `Mine`); `?view=mine` query param scopes data to `owner_user_id`
- `db.rs` — pool setup + migration runner with checksum-repair logic for known idempotent migrations

**apps/web** — single `App.tsx` (monolithic SPA, all types and components in one file). React 19 + TypeScript + Vite.

### Key design decisions

**Authentication**: cookie `ff_session` (UUID), `HttpOnly`, `SameSite=Lax`. Session stored in DB with expiry. First user to register becomes installation owner automatically (`bootstrap_installation_as_owner_if_empty`).

**Installation singleton**: one row in `installation` per deployment. All financial data belongs to it. Users who register but aren't in `installation_memberships` are "pending" — they see no data until the owner approves them.

**Money**: always `rust_decimal::Decimal`. API serializes amounts as decimal strings (`serde-with-str`). The frontend receives and sends strings, never floats. Never use `f64` for financial values.

**Dual-port dev**: Vite `:8080`, API `:8081` (set in `.env.example`). `vite.config.ts` reads `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` from repo-root `.env`. Docker image serves both on `:8080` via `WEB_STATIC_ROOT=/app/web`.

**View scoping**: all ledger endpoints accept `?view=mine` to filter by `owner_user_id = current_user`. Default is `household` (full installation scope). This is a client-side filter, not an authorization boundary.

**OpenAPI**: generated via `utoipa`, served at `GET /openapi.json`. All handler structs annotated with `#[utoipa::path]`.

**CORS**: `CORS_ORIGINS` env var (comma-separated). Required — panics if empty. Default includes `:5173` (Vite fallback port) and `:8080`.

### Migrations
SQLx embed migrations in `apps/api/migrations/`. Run automatically on startup via `db::run_migrations`. Filenames: `YYYYMMDDHHMMSS_description.sql`. The migration runner has a checksum-repair loop for versions listed in `IDEMPOTENT_MIGRATION_REPAIR_VERSIONS` in `db.rs`.

## UI conventions (from Cursor rules)

- **Monetary amounts**: no decimals, currency symbol after the number (`1.234 €`). Use `formatCurrencyAmount` / `formatCurrencyNumber` — never `toString()` or manual concatenation.
- **Percentages**: exactly one decimal, suffix ` %` (`3,5 %`). Use `formatPercentAmount` / `formatPercentDisplay`. The function already includes the suffix.
- **MetricCard additional info**: always goes in the `parenthetical` prop, not `suffix`.
- **Copy**: minimal — prefer short labels, empty states in a few words (`Sin datos.`).

## Git / Docker workflow

- Active development on `dev` branch; `main` is releases only.
- To release: merge `dev` → `main`, push a tag `vX.Y.Z` → GitHub Action publishes the image automatically.
- Tags published: `:X.Y.Z`, `:X.Y`, `:X`, `:latest`. No `sha-*` auto-tags — versioning is strictly semver.
- Docker Hub: add `DOCKERHUB_USERNAME` + `DOCKERHUB_TOKEN` secrets to the repo to publish there in addition to GHCR.
- Before resuming work: `git pull --ff-only`. After push: pull again.
- After changing code, Dockerfile, or web assets: `make docker-rebuild` (or `docker compose up -d --build`) from repo root. Changes on the host do not enter a running container.
- `make docker-down` before `make docker-up` if containers were deleted manually to avoid orphan state.
