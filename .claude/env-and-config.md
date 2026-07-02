# Environment Variables & Configuration

Template: `.env.example` (cubre producción y desarrollo split-dev).

## Key variables

| Variable | Default | Notes |
|----------|---------|-------|
| `DATABASE_URL` | — (sin default) | **Required** — `main.rs` hace panic al arrancar si falta (`DATABASE_URL must be set`). `.env.example` trae el valor típico de dev: `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`. |
| `PORT` | `8080` | API listen port. Use `8081` in split-dev so Vite can use `8080`. |
| `RUST_LOG` | — | e.g. `futurefin_api=info,tower_http=info,sqlx=warn` |
| `CORS_ORIGINS` | 4 localhost entries | Comma-separated. Not required — defaults to localhost. Set only for cross-origin API access. |
| `COOKIE_SECURE` | `false` | Bool env var parsed by `main.rs`. |
| `SESSION_TTL_DAYS` | `30` | Acepta 1–400; un valor fuera de rango o no numérico **cae silenciosamente al default 30** (filter-then-default, no clamp). |
| `WEB_STATIC_ROOT` | — | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only. |
| `FUTUREFIN_API_PORT` | `8081` | Used by Vite proxy (`vite.config.ts`) |
| `WEB_DEV_PORT` | `8080` | Vite dev server port |

## Docker-specific (prod)

| Variable | Notes |
|----------|-------|
| `FUTUREFIN_IMAGE` | e.g. `maxlainz/futurefin` (Docker Hub) |
| `FUTUREFIN_TAG` | `latest` or `vX.Y.Z` |
| `APP_PORT` | Host port. Default `8080`. |
| `POSTGRES_USER` | Default `futurefin` |
| `POSTGRES_DB` | Default `futurefin` |
| `POSTGRES_PASSWORD` | Required — no default |

## `.env` loading order (API)
`main.rs::load_env()` tries:
1. `{CARGO_MANIFEST_DIR}/../../.env` (repo root — works when running `cargo run` from `apps/api`)
2. `dotenvy::dotenv()` (CWD `.env`)

Env vars already set in the environment take precedence over `.env` files.

## Vite config
`apps/web/vite.config.ts` loads env from repo root (two levels up from `apps/web`). It reads `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` without the `VITE_` prefix (uses `loadEnv` with `""`).

## Docker Compose files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Producción: pulls from Docker Hub (`maxlainz/futurefin`), exposes `:8080`. La DB no mapea puerto al host. |
| `docker-compose.local.yml` | Override para usar imagen construida localmente (`futurefin-local:dev`). `pull_policy: never` en el servicio `futurefin`. Ver el bloque "Test local con Docker Desktop" del CLAUDE.md. |
| `docker-compose.split-dev.yml` | Override para split-dev (`cargo run` + `vite`). Solo expone Postgres en `127.0.0.1:5432` para que la API local pueda conectarse. Usar así: `docker compose -f docker-compose.yml -f docker-compose.split-dev.yml up -d futurefin-database`. **No usar en producción.** |
