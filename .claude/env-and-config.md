# Environment Variables & Configuration

Template: `.env.example` (cubre producción y desarrollo split-dev). Desde 3.0.0 **producción no
requiere ninguna variable** (`docker compose up -d` funciona con `.env` vacío o ausente).

## Key variables (binario API)

| Variable | Default | Notes |
|----------|---------|-------|
| `DATABASE_URL` | — (sin default) | El binario hace panic si falta (`DATABASE_URL must be set`) — pero **en el contenedor 3.x la fabrica el entrypoint** (socket Unix: `postgres:///futurefin?host=/var/run/postgresql&user=futurefin`). Definirla en el entorno del contenedor activa el **modo DB externa (deprecado, se elimina en 4.0.0)**. En split-dev sigue siendo la var normal: `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | Ventana total del retry de conexión (`db::connect_with_retry`, backoff 0,5→4 s). Acepta 1–600; fuera de rango cae al default. |
| `PORT` | `8080` | API listen port. Use `8081` in split-dev so Vite can use `8080`. |
| `RUST_LOG` | — | e.g. `futurefin_api=info,tower_http=info,sqlx=warn` |
| `CORS_ORIGINS` | 4 localhost entries | Comma-separated. Not required — defaults to localhost. Set only for cross-origin API access. |
| `COOKIE_SECURE` | `false` | Bool env var parsed by `main.rs`. |
| `SESSION_TTL_DAYS` | `30` | Acepta 1–400; un valor fuera de rango o no numérico **cae silenciosamente al default 30** (filter-then-default, no clamp). |
| `WEB_STATIC_ROOT` | — | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only. |
| `FUTUREFIN_API_PORT` | `8081` | Used by Vite proxy (`vite.config.ts`) |
| `WEB_DEV_PORT` | `8080` | Vite dev server port |

## Entrypoint del contenedor (3.0.0, `apps/api/docker-entrypoint.sh`)

| Variable | Default | Notes |
|----------|---------|-------|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded` \| `external`. `auto` decide por presencia de `DATABASE_URL` y volumen; `external` fuerza la DB externa (deprecado) y desactiva la automigración. |
| `FUTUREFIN_MODE` | `serve` | `serve` \| `db-only` (modo rescate: solo PostgreSQL, sin API; lo usa `scripts/restore-postgres.sh`). También como argv: `docker run … db-only`. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | Nº de backups automáticos pre-migración intocables (los más recientes). |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | Del resto, se borran los más viejos que esto. Poda extra bajo 256 MB libres (nunca los 3 últimos). |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | `off` desactiva el backup automático pre-migración (y su aborto-si-falla). |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` permite arrancar **sin volumen** en `PGDATA` (CI/demo). Sin esto, el contenedor aborta a propósito. |
| `FUTUREFIN_EXTERNAL_WAIT_SECS` | `60` | Espera a que la DB externa responda antes de automigrar/abortar. |
| `FUTUREFIN_API_STOP_TIMEOUT` / `FUTUREFIN_PG_STOP_TIMEOUT` | `15` / `30` | Timeouts del apagado ordenado (API: TERM; postmaster: SIGINT). La escalada está acotada: señal de escalada (KILL / QUIT) y, si 10 s después sigue vivo, KILL — nunca puede bloquear. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` | Volumen `ffdata`: backups, staging de pg_upgrade, estado (marcadores de reindex/automigración). Avanzada. |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | Avanzada. |
| `FUTUREFIN_PG_LISTEN` | vacío (sin TCP) | Solo depuración: `127.0.0.1` abre TCP dentro del contenedor. |
| `FUTUREFIN_PG_LOG_LEVEL` | — | Solo depuración: `log_min_messages` del PG embebido. |
| `POSTGRES_USER` / `POSTGRES_DB` | `futurefin` | Compat con instalaciones 2.x personalizadas. |
| `POSTGRES_PASSWORD` | — | **Ya no es obligatoria** (socket local, auth trust). Si viene, se aplica al rol y nada más. |
| `PGDATA` | `/var/lib/postgresql/data` | Avanzada; cambiarla rompe la compat con volúmenes 2.x. |

## Docker-specific (prod)

| Variable | Notes |
|----------|-------|
| `FUTUREFIN_IMAGE` | e.g. `maxlainz/futurefin` (Docker Hub) |
| `FUTUREFIN_TAG` | `latest` or `X.Y.Z` |
| `APP_PORT` | Host port. Default `8080`. |

## `.env` loading order (API)
`main.rs::load_env()` tries:
1. `{CARGO_MANIFEST_DIR}/../../.env` (repo root — works when running `cargo run` from `apps/api`)
2. `dotenvy::dotenv()` (CWD `.env`)

Env vars already set in the environment take precedence over `.env` files.

**Trampa 3.x**: un `.env` de desarrollo con `DATABASE_URL` descomentada junto al
`docker-compose.yml` de producción hace que compose se la pase al contenedor → la imagen
entra en modo DB externa (o aborta si el volumen está vacío y la externa no responde).
Mantén `.env` de dev y de prod separados.

## Vite config
`apps/web/vite.config.ts` loads env from repo root (two levels up from `apps/web`). It reads `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` without the `VITE_` prefix (uses `loadEnv` with `""`).

## Docker Compose files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Producción: **un solo servicio** `futurefin` (PostgreSQL embebido, socket-only — ningún puerto de DB, ni al host ni interno). Volúmenes `pgdata` (datos, mismo nombre que 2.x) y `ffdata` (backups/estado). Healthcheck `curl /v1/ready` **sin** fallback `/dev/tcp`; `stop_grace_period: 60s`. |
| `docker-compose.local.yml` | Override para usar imagen construida localmente (`futurefin-local:dev`). `pull_policy: never` en el servicio `futurefin`. Ver el bloque "Test local con Docker Desktop" del CLAUDE.md. |
| `docker-compose.dev.yml` | **Compose autónomo** (no override) para split-dev (`cargo run` + `vite`): project `futurefin-dev`, servicio `db` en `127.0.0.1:5432`, volumen `devdata` (el fichero incluye cómo reutilizar el volumen antiguo `futurefin_pgdata`). Usar así: `docker compose -f docker-compose.dev.yml up -d`. **No usar en producción.** Sustituye al antiguo `docker-compose.split-dev.yml`. |
