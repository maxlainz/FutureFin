# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Reference docs (`.claude/`)

Extended reference — read these before working on the relevant area:

| File | Contents |
|------|----------|
| [`.claude/api-routes.md`](.claude/api-routes.md) | Full route map with auth patterns |
| [`.claude/data-model.md`](.claude/data-model.md) | DB schema, table invariants, FIRE JSONB shape |
| [`.claude/engine.md`](.claude/engine.md) | Projection engine public API and simulation loop |
| [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md) | Auth flow, roles, cookie, pending users |
| [`.claude/env-and-config.md`](.claude/env-and-config.md) | All env vars, `.env` loading order, Vite config |
| [`.claude/adding-handler.md`](.claude/adding-handler.md) | Step-by-step pattern for adding a new API handler |
| [`.claude/frontend-structure.md`](.claude/frontend-structure.md) | SPA layout post-refactor (lib/, api/, components/, views/, auth/) and where to put what |
| [`.claude/tests.md`](.claude/tests.md) | How to run + write backend integration tests (Postgres schemas) and frontend Vitest tests |

**Keep these files up to date** whenever the corresponding area changes (routes, schema, env vars, etc.).

## Commands

### Development (split-dev: API + Vite hot reload)
```bash
cp .env.example .env
# Uncomment the dev vars in .env (PORT, DATABASE_URL, RUST_LOG)
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

### Test local con Docker Desktop (sin publicar imagen)
Útil para validar el stack completo (API + frontend + DB) exactamente como en producción, sin esperar a que CI publique una imagen.

```bash
# 1. Construir la imagen localmente (tarda la primera vez; usa caché en rebuilds)
#    --load es obligatorio con BuildKit para que quede en el store local de Docker
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .

# 2. Asegúrate de que .env tiene:
#      FUTUREFIN_IMAGE=futurefin-local
#      FUTUREFIN_TAG=dev
#      POSTGRES_PASSWORD=<lo que sea>

# 3. Arrancar el stack con el override local (evita que Compose haga pull de la imagen local)
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d

# 4. Smoke test
curl -sf http://127.0.0.1:8080/v1/health

# 5. Rebuild tras cambios (la caché de Docker reutiliza las capas sin cambios)
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev . \
  && docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env \
     up -d --no-deps futurefin
```

> `docker-compose.local.yml` añade `pull_policy: never` al servicio `futurefin` para que Compose no intente hacer pull de una imagen que solo existe en local.

### Production stack (maintenance)
```bash
docker compose logs -f futurefin          # logs
docker compose down --remove-orphans      # stop
curl -sf http://127.0.0.1:8080/v1/health  # smoke test
```

### Rust
```bash
cd apps/api && cargo build
cargo test -p futurefin-engine           # engine unit tests only (no DB)
cargo test -p futurefin-engine -- <name> # single test

# Integration tests (require a running Postgres):
# 1) Start a dedicated test DB once (port 5433 to avoid clashing with dev):
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine
# 2) Run the full workspace test suite (each test gets its own schema, see .claude/tests.md):
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

### Frontend
```bash
npm run typecheck:web   # tsc --noEmit
npm run lint:web        # eslint
npm run build:web       # Vite production build → apps/web/dist/
npm test --workspace futurefin-web   # Vitest run (pure-function tests)
```

### Production deploy
```bash
docker compose --env-file .env up -d
```

## Architecture

### Workspace layout
```
Cargo workspace: apps/api + crates/domain + crates/engine
npm workspace:   apps/web (futurefin-web)
```

**crates/domain** — shared primitives: `UserId` (newtype over `Uuid`), re-exports `Decimal` and `Uuid`. No `f64` for monetary values anywhere in the domain.

**crates/engine** — pure projection math (`project_net_worth_series`, `first_month_per_asset_contribution_nominals`). No I/O, no DB; only `Decimal` arithmetic. Has unit tests.

**apps/api** — Axum HTTP server. Entry point: `main.rs` (bin), with shared crate modules in `lib.rs`. Key modules:
- `routes/mod.rs` — full route map; all routes under `/v1/` except `/health` and `/openapi.json`. `DefaultBodyLimit` caps requests at 1 MB globally, 16 MB on `/backup/user-import*`.
- `state.rs` — `AppState` (pool, cookie_secure, session_ttl_days, version)
- `error.rs` — `ApiError` → `(StatusCode, JSON {error, message})` via `IntoResponse`. `impl From<sqlx::Error>` detects SQLSTATE 23505 → `Conflict` (409), 23503 → `BadRequest`; handlers can just `?` any `sqlx::Error` without manual mapping.
- `auth/` — password hashing (Argon2id)
- `handlers/session.rs` — `require_session_user` reads cookie `ff_session` → validates against `sessions` table
- `handlers/installation.rs` — singleton installation, FIRE settings, `require_installation_member`
- `handlers/membership.rs` — roles: `owner`, `member`, `viewer`; `role_can_write` used by handlers
- `handlers/person_view.rs` — `LedgerView` enum (`Household` / `Mine`) **plus helpers** `scope_where(table_alias)`, `next_arg_index()`, `bind_scope_as`, `bind_scope_scalar`. Use them instead of duplicating `match view { Household | Mine }` blocks — they enforce consistent placeholder ordering across both branches.
- `db.rs` — pool setup (`max=10, min=1, idle_timeout=10min, max_lifetime=30min`) + `sqlx::migrate!` runner. No more auto-repair loop; if a checksum mismatches in dev, fix manually via `DELETE FROM _sqlx_migrations WHERE version = X` and rerun.
- **`tests/`** — integration tests against a real Postgres (schema-isolated per test). See [`.claude/tests.md`](.claude/tests.md).

**apps/web** — React 19 + TypeScript + Vite. `App.tsx` is the composition root (auth gate + global state + route → view dispatch). All views, components, helpers and types live in separate modules — see [`.claude/frontend-structure.md`](.claude/frontend-structure.md).

### Key design decisions

**Authentication**: cookie `ff_session` (UUID), `HttpOnly`, `SameSite=Lax`. Session stored in DB with expiry. First user to register becomes installation owner automatically (`bootstrap_installation_as_owner_if_empty`).

**Installation singleton**: one row in `installation` per deployment. All financial data belongs to it. Users who register but aren't in `installation_memberships` are "pending" — they see no data until the owner approves them.

**Money**: always `rust_decimal::Decimal`. API serializes amounts as decimal strings (`serde-with-str`). The frontend receives and sends strings, never floats. Never use `f64` for financial values.

**Dual-port dev**: Vite `:8080`, API `:8081` (set in `.env.example`). `vite.config.ts` reads `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` from repo-root `.env`. Docker image serves both on `:8080` via `WEB_STATIC_ROOT=/app/web`.

**View scoping**: all ledger endpoints accept `?view=mine` to filter by `owner_user_id = current_user`. Default is `household` (full installation scope). This is a client-side filter, not an authorization boundary. Handlers must use `LedgerView::scope_where` + `bind_scope_as/scalar` so the two branches stay in sync.

**Reads never mutate**: liabilities with `payment_end_date < today` are **filtered** out of `GET /v1/liabilities`, `/summary`, `/budget` (derived lines), `/assets`, `/projection` via `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. They are **not** physically deleted. The legacy `purge_expired_liabilities` function was removed in May 2026 — GET handlers were silently issuing `DELETE` statements, violating HTTP semantics and impeding caching.

**OpenAPI**: generated via `utoipa`, served at `GET /openapi.json`. All handler structs annotated with `#[utoipa::path]`.

**CORS**: `CORS_ORIGINS` env var (comma-separated). Not required — defaults to localhost origins. Set explicitly only for cross-origin API access.

### Migrations
SQLx embed migrations in `apps/api/migrations/`. Run automatically on startup via `db::run_migrations`. Filenames: `YYYYMMDDHHMMSS_description.sql`. No auto-repair: a checksum mismatch fails loud and must be resolved by hand (e.g. `psql -c "DELETE FROM _sqlx_migrations WHERE version = X"` if the change is genuinely idempotent).

## UI conventions

- **Monetary amounts**: no decimals, currency symbol after the number (`1.234 €`). Use `formatCurrencyAmount` / `formatCurrencyNumber` — never `toString()` or manual concatenation.
- **Percentages**: exactly one decimal, suffix ` %` (`3,5 %`). Use `formatPercentAmount` / `formatPercentDisplay`. The function already includes the suffix.
- **MetricCard additional info**: always goes in the `parenthetical` prop, not `suffix`.
- **Copy**: minimal — prefer short labels, empty states in a few words (`Sin datos.`).

## Git workflow

**Branches:**
- `dev` — desarrollo activo. Contiene todo: CLAUDE.md, .claude/, .github/, workflows de CI.
- `main` — rama de usuario final. Solo archivos que el usuario necesita (docker-compose.yml, README, .env.example, Cargo/package files). CLAUDE.md, .claude/ y .github/ están en .gitignore de main y no se deben subir allí.

**Releases:**
1. Desarrollar en `dev`, hacer commit y push.
2. Bumpar versión en `apps/api/Cargo.toml` y añadir entrada en `CHANGELOG.md`.
3. Actualizar archivos de usuario en `main` (docker-compose.yml, README, .env.example, CHANGELOG.md) copiando los cambios relevantes — **no hacer merge completo de dev a main**.
4. Push tag `vX.Y.Z` desde `dev` → el workflow `publish-image.yml` (que vive en `dev`) publica la imagen.

Tags published: `:X.Y.Z`, `:X.Y`, `:X`, `:latest`. Requiere secrets `DOCKERHUB_USERNAME` + `DOCKERHUB_TOKEN` en GitHub repo.

Before resuming work: `git pull --ff-only`. After push: pull again.
