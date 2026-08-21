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
| `FUTUREFIN_MCP_ENABLED` | `true` | Bool (`parse_bool_env` de `main.rs`). Con `0`/`false` el router `/mcp` **ni se monta** (404 del fallback) **y desde 3.1.0 tampoco el protocolo OAuth**: las 7 rutas raíz (`/.well-known/oauth-*`, `/oauth/register|token|revoke`) ni se construyen, y con ellas caen `/v1/oauth/authorize-details` y `POST /v1/oauth/authorize`. **EXCEPCIÓN: `GET/DELETE /v1/oauth/connections[/{id}]` se montan siempre**, igual que `/v1/api-tokens` — apagar MCP no puede dejarte sin poder revocar credenciales ya concedidas. Default habilitado: el endpoint es inerte sin credenciales (todo 401) y producción sigue sin requerir env vars. |
| `FUTUREFIN_PUBLIC_URL` | — (derivado del request) | **Opcional** (3.1.0). Origen público canónico usado como `issuer` OAuth y para construir los endpoints de la metadata `.well-known`. Sin ella, `oauth/url.rs::public_base_url` lo **deriva por request**: `X-Forwarded-Proto` + `X-Forwarded-Host` (primer valor de cada uno) o el header `Host`, con charset estricto (`host[:puerto]`, IPv6 entre corchetes, ≤255 chars, sin `/`, `@`, espacios ni controles) → si no cuadra, **400 `invalid_request`**. **Fíjala solo si tu reverse proxy no manda esos headers** (o manda un `Host` interno): entonces claude.ai recibiría un issuer inalcanzable y la conexión falla. Formato: origen desnudo, `https://finanzas.example.com` — **sin path, query, fragmento ni barra final**. **Validación fail-loud** como `CORS_ORIGINS` (`main.rs::public_url()`): si está presente pero es inválida (no parseable, esquema ≠ http/https, sin host, o con path/query/fragmento) el arranque hace **panic** en vez de servir metadata OAuth rota en silencio. Se normaliza al origen ASCII. El log de arranque la imprime (`public_url=…` o `(derived from request)`). Irrelevante con `FUTUREFIN_MCP_ENABLED=0`. |
| `FUTUREFIN_RECONCILE_SWEEP_HOURS` | `24` | **Opcional** (3.8.1). Cada cuántas horas corre el barrido de conciliación de transferencias (`main.rs::spawn_reconcile_sweep` → `reconcile::sweep_all_owners`), la **única tarea periódica del binario**. `0` la desactiva. Se parsea como `u64` y se **descarta si supera 168** (una semana): valor no parseable, negativo o `>168` → default 24, sin avisar (`main.rs::reconcile_sweep_hours`). Es una **red de reintento**, no el mecanismo principal: el pase corre ya tras cada mutación del conjunto, y esos pases son best-effort (un fallo se loguea y no convierte una escritura persistida en 5xx), así que sin barrido un fallo puntual dejaba el par sin conciliar para siempre y en silencio. La primera pasada va **tras el primer intervalo**, no al arrancar. En modos B/C, una pasada que recupera pares **invalida la cache de proyección** (D12a) — si no recupera nada, no la toca. La tarea se aborta antes de cerrar el pool en el apagado ordenado. |
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

**Proxy en dev** — las claves de `server.proxy` son **prefijos**, y las rutas de protocolo se listan
**una a una** a propósito:

| Clave | Por qué |
|---|---|
| `/health`, `/openapi.json`, `/v1` | El API de siempre (`/v1` cubre también `/v1/oauth/*`). |
| `/.well-known` | Metadata OAuth (RFC 8414/9728) — vive en el API. |
| `/oauth/token`, `/oauth/register`, `/oauth/revoke` | Protocolo OAuth (3.1.0), endpoint por endpoint. |
| `/mcp` | Transporte MCP. |

- **PROHIBIDO proxyar `"/oauth"` a secas.** Al ser prefijo, se llevaría también `/oauth/authorize`
  — que es una **vista de la SPA**, no una ruta del backend — al servidor Rust, que no la tiene: en
  dev verías un 404 en lugar de la pantalla de consentimiento, y el authorization request moriría en
  el primer salto del navegador. Mismo motivo por el que el backend no registra la ruta (ver
  [`api-routes.md`](api-routes.md) §OAuth 2.1).

## Docker Compose files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Producción: **un solo servicio** `futurefin` (PostgreSQL embebido, socket-only — ningún puerto de DB, ni al host ni interno). Volúmenes `pgdata` (datos, mismo nombre que 2.x) y `ffdata` (backups/estado). Healthcheck `curl /v1/ready` **sin** fallback `/dev/tcp`; `stop_grace_period: 60s`. |
| `docker-compose.local.yml` | Override para usar imagen construida localmente (`futurefin-local:dev`). `pull_policy: never` en el servicio `futurefin`. Ver el bloque "Test local con Docker Desktop" del CLAUDE.md. |
| `docker-compose.dev.yml` | **Compose autónomo** (no override) para split-dev (`cargo run` + `vite`): project `futurefin-dev`, servicio `db` en `127.0.0.1:5432`, volumen `devdata` (el fichero incluye cómo reutilizar el volumen antiguo `futurefin_pgdata`). Usar así: `docker compose -f docker-compose.dev.yml up -d`. **No usar en producción.** Sustituye al antiguo `docker-compose.split-dev.yml`. |
