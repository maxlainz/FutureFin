# Environment Variables & Configuration

Template: `.env.example` (dev) and `.env.prod.example` (production NAS deploy).

## Key variables

| Variable | Default | Notes |
|----------|---------|-------|
| `DATABASE_URL` | `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin` | Required |
| `PORT` | `8080` | API listen port. Use `8081` in split-dev so Vite can use `8080`. |
| `RUST_LOG` | — | e.g. `futurefin_api=info,tower_http=info,sqlx=warn` |
| `CORS_ORIGINS` | 4 localhost entries | Comma-separated. Panics if empty. Include `:5173` for Vite fallback. |
| `COOKIE_SECURE` | `0` | Set to `1` behind HTTPS |
| `SESSION_TTL_DAYS` | `30` | 1–400 |
| `WEB_STATIC_ROOT` | — | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only. |
| `FUTUREFIN_API_PORT` | `8081` | Used by Vite proxy (`vite.config.ts`) |
| `WEB_DEV_PORT` | `8080` | Vite dev server port |

## Docker-specific (prod)
| Variable | Notes |
|----------|-------|
| `FUTUREFIN_IMAGE` | e.g. `ghcr.io/<user>/futurefin` |
| `FUTUREFIN_TAG` | `latest` or `vX.Y.Z` |
| `FUTUREFIN_DOMAIN` | Domain for Caddy TLS |
| `CADDY_EMAIL` | Let's Encrypt registration |
| `POSTGRES_PASSWORD` | DB password |

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
| `docker-compose.yml` | Local dev stack (DB + app, `--build` local image) |
| `docker-compose.watch.yml` | Hot-reload compose watch variant |
| `docker-compose.prod.yml` | Production: pulls from GHCR |
| `docker-compose.tls.yml` | Caddy reverse proxy + TLS (overlay for prod) |
