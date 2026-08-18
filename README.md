# FutureFin

Self-hosted personal finance app: shared household budget, upcoming cash flows, net-worth projection, and FIRE / retirement planning.

- **API:** Rust + Axum — all endpoints under `/v1/`
- **UI:** React 19 + TypeScript + Vite — embedded in the Docker image
- **DB:** PostgreSQL — **incluido en la propia imagen** desde 3.0.0; migrations run automatically on startup
- **Auth:** username + password (Argon2id), `HttpOnly` session cookie, no email required
- **Multi-user:** one installation per deployment; new users wait for owner approval
- **MCP:** embedded MCP server (`/mcp`) with per-user API tokens — Claude can read, simulate what-ifs and (if you allow it) record and edit your finances (see [Conectar Claude](#conectar-claude-mcp))

---

## Quick start (Docker)

Un solo contenedor: la imagen incluye PostgreSQL. No hace falta `.env`.

1. Descarga `docker-compose.yml` de este repositorio.
2. Arranca:
   ```bash
   docker compose up -d
   ```

El primer usuario en registrarse se convierte en propietario de la instalación. Los datos viven en dos volúmenes Docker: `pgdata` (la base de datos) y `ffdata` (backups automáticos).

También puedes prescindir de compose:

```bash
docker run -d --name futurefin --restart unless-stopped -p 8080:8080 \
  -v futurefin_pgdata:/var/lib/postgresql/data \
  -v futurefin_ffdata:/var/lib/futurefin \
  maxlainz/futurefin:latest
```

> Sin un volumen montado en `/var/lib/postgresql/data` el contenedor se niega a arrancar
> (tus datos morirían con él). Es deliberado.

### Actualizar

Cambia `FUTUREFIN_TAG` en `.env` (o usa `latest`) y ejecuta:
```bash
docker compose pull && docker compose up -d
```

Antes de aplicar migraciones nuevas, el contenedor escribe **automáticamente** un backup
`pre-migration-*.sql.gz` en el volumen `ffdata` (retención configurable). Aun así, para
upgrades importantes: exporta tu `.ffbackup` y ejecuta `scripts/backup-postgres.sh` antes.

Con **watchtower** funciona sin intervención; configura `WATCHTOWER_TIMEOUT=60s` para que
respete el apagado ordenado de PostgreSQL (el compose ya trae `stop_grace_period: 60s`).

Para volver a una versión anterior, cambia `FUTUREFIN_TAG` y repite. Ten en cuenta que las
migraciones SQL solo avanzan: una versión antigua no arranca sobre una base ya migrada por
una más nueva (se detiene con un error claro, sin tocar datos).

### Actualizar desde 2.x (multi-contenedor) a 3.x

Sin pérdida de datos: el volumen `pgdata` de 2.x se reutiliza tal cual.

1. Recomendado: exporta tu `.ffbackup` (Ajustes → Datos y sistema) y haz un `pg_dump`.
2. Sustituye tu `docker-compose.yml` por el de 3.x (este repositorio).
3. ```bash
   docker compose pull && docker compose up -d --remove-orphans
   ```
   El `--remove-orphans` retira el antiguo contenedor `futurefin-database`.

El **primer arranque tarda más de lo normal una única vez**: ajusta permisos del volumen,
reconstruye los índices de texto (`REINDEX`, necesario al pasar de la imagen Alpine a la
Debian) y escribe el backup pre-migración. Los logs (`docker compose logs -f futurefin`)
lo cuentan paso a paso; espera a que `/v1/ready` responda 200.

- `POSTGRES_PASSWORD` **ya no es necesaria**: la base es local al contenedor, vía socket
  Unix, sin ningún puerto TCP. Si personalizaste `POSTGRES_USER`/`POSTGRES_DB` en 2.x,
  consérvalos en el `.env`.
- Si actualizaste **sin tocar el compose** (p.ej. watchtower con `:latest`): la imagen 3.x
  detecta tu topología 2.x y sigue funcionando contra el contenedor de base de datos
  antiguo, con un aviso de deprecación en los logs. Migra al compose nuevo cuando quieras;
  ese modo desaparece en 4.0.0.
- **Base de datos externa de verdad** (gestionada aparte): al arrancar la 3.x con
  `DATABASE_URL` definida y un volumen vacío montado, copia tus datos a la base embebida
  una única vez (automigración verificada; la externa solo se lee). Para quedarte en la
  externa: `FUTUREFIN_DB_MODE=external` (deprecado).

**Rollback a 2.x**: `docker compose down`, restaura tu `docker-compose.yml` y `.env` de
2.x (con `POSTGRES_PASSWORD`), `docker compose up -d`. El volumen no cambió de forma. No
borres el volumen `ffdata` si quieres conservar los backups automáticos.

---

## Backups

Tres capas complementarias:

- **Aplicación — `.ffbackup` por usuario**: desde `Ajustes → Datos y sistema` cada usuario exporta sus propios datos (activos, pasivos, presupuesto, próximos) en un contenedor binario cifrado con su contraseña (Argon2id → AES-256-GCM). Endpoints: `POST /v1/backup/user-export`, `POST /v1/backup/user-import/preview` y `POST /v1/backup/user-import` (import = reemplazo total de tus filas, transaccional).
- **Automática — pre-migración**: antes de aplicar migraciones nuevas (upgrade de versión), el contenedor escribe `pre-migration-*.sql.gz` en el volumen `ffdata`. Retención: las `FUTUREFIN_BACKUP_KEEP` (10) más recientes siempre se conservan; del resto se borran las de más de `FUTUREFIN_BACKUP_KEEP_DAYS` (90) días. Extraer al host: `docker compose cp futurefin:/var/lib/futurefin/backups ./backups-auto`.
- **Manual — dump de Postgres**: `scripts/backup-postgres.sh` hace `pg_dump` dentro del contenedor a `./backups/` (gzip, retención configurable). Restaurar: `scripts/restore-postgres.sh backups/<fichero>.sql.gz` (usa el modo rescate `db-only` del contenedor).

---

## Conectar Claude (MCP)

FutureFin incluye un **servidor MCP** en `/mcp` (mismo puerto que la app): Claude puede
consultar tu resumen, proyección FIRE, presupuesto, movimientos, histórico, activos y pasivos,
simular escenarios («¿y si gasto 200 € más al mes?») sin tocar nada, y — si tu rol lo permite y
el interruptor «Permitir escritura vía MCP» (Ajustes → MCP) está activado — registrar
movimientos, capturar snapshots y mantener tu plan al día. Las operaciones destructivas siempre
piden confirmación (sin ella devuelven un preview). Hay dos maneras de conectar:

### claude.ai (web / móvil / Desktop) — conector personalizado con OAuth (3.1.0)

1. Expón tu instalación por **HTTPS público** (p. ej. Cloudflare Tunnel): las conexiones de
   claude.ai salen de la infraestructura de Anthropic, no de tu navegador — `localhost` no sirve.
2. En claude.ai: `Configuración → Conectores → Añadir conector personalizado` y pega
   `https://tu-host/mcp`. No hay que rellenar nada más: el registro de cliente es automático (DCR).
3. Claude te llevará a la **pantalla de autorización de FutureFin**: inicia sesión con tu usuario
   de siempre y pulsa **Autorizar** (el acceso hereda tu rol; la escritura se puede apagar en
   Ajustes → MCP).
4. Revocar: `Ajustes → MCP → Conexiones` → **Revocar** (corte inmediato; claude tendrá que
   volver a pedir permiso).

### Claude Code / clientes MCP genéricos — token de API

1. En la app: `Ajustes → MCP → Tokens de API (MCP)` → **Crear token**. Copia el secreto
   (`ffp_…`): solo se muestra una vez. El token hereda tu usuario y rol, y puedes revocarlo cuando
   quieras (corte inmediato).
2. En Claude Code:

   ```bash
   claude mcp add --transport http futurefin https://tu-host/mcp \
     --header "Authorization: Bearer ffp_..."
   ```

   (Claude Code también puede conectar sin token, vía el mismo flujo OAuth: `claude mcp add
   --transport http futurefin https://tu-host/mcp` y autorizar en el navegador.)
3. Cualquier otro cliente MCP genérico funciona igual: transporte **Streamable HTTP** + header
   `Authorization: Bearer <token>`, o el flujo OAuth 2.1 estándar del protocolo MCP.

Notas:

- **Acceso remoto**: FutureFin no gestiona TLS ni conectividad (Cloudflare Tunnel, reverse proxy…
  a tu elección). Si tu proxy no manda `X-Forwarded-Proto`/`Host` correctos, fija
  `FUTUREFIN_PUBLIC_URL=https://tu-host`.
- **Apagarlo del todo**: `FUTUREFIN_MCP_ENABLED=0` (desmonta `/mcp` y todo el protocolo OAuth; el
  panel de conexiones sigue disponible para revocar). Sin tokens ni conexiones, `/mcp` responde
  401 a todo.

---

## Environment variables

Desde 3.0.0 **ninguna variable es obligatoria**.

| Variable | Required | Notes |
|----------|----------|-------|
| `FUTUREFIN_IMAGE` | No | Default `maxlainz/futurefin` |
| `FUTUREFIN_TAG` | No | Default `latest`. Pin to `X.Y.Z` for stability. |
| `APP_PORT` | No | Host port. Default `8080`. |
| `FUTUREFIN_BACKUP_KEEP` | No | Backups automáticos que se conservan siempre. Default `10`. |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | No | Edad máxima del resto de backups automáticos. Default `90`. |
| `FUTUREFIN_PREMIGRATION_BACKUP` | No | `off` desactiva el backup automático pre-migración. Default `on`. |
| `POSTGRES_USER` | No | Default `futurefin`. Solo si lo personalizaste en 2.x. |
| `POSTGRES_DB` | No | Default `futurefin`. Solo si lo personalizaste en 2.x. |
| `POSTGRES_PASSWORD` | No | Ya no es necesaria (socket local). Si viene, se aplica al rol. |
| `DATABASE_URL` | No | **Deprecado** (se elimina en 4.0.0): base de datos externa. Ver «Actualizar desde 2.x». |
| `FUTUREFIN_DB_MODE` | No | `auto` (default) \| `embedded` \| `external`. |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | No | `1` permite arrancar sin volumen (solo pruebas desechables). |
| `FUTUREFIN_MCP_ENABLED` | No | `0` desmonta el servidor MCP (`/mcp`) y el protocolo OAuth (el panel de conexiones sigue). Default `true`. |
| `FUTUREFIN_PUBLIC_URL` | No | Origen público (`https://tu-host`) para OAuth. Solo si tu proxy no manda `X-Forwarded-Proto`/`Host`. |
| `PORT` | No | Internal container port. Default `8080`. |
| `WEB_STATIC_ROOT` | No | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only mode. |
| `SESSION_TTL_DAYS` | No | Default `30`, max `400` |
| `RUST_LOG` | No | e.g. `futurefin_api=info,tower_http=info` |

---

## Development

**Requirements:** Rust (stable), Node.js 20+, npm 10+, Docker (for Postgres).

```bash
cp .env.example .env
# Uncomment the dev vars in .env (PORT, DATABASE_URL, RUST_LOG)
# Postgres de desarrollo (compose autónomo, expone 127.0.0.1:5432 — imprescindible para cargo run):
docker compose -f docker-compose.dev.yml up -d

# Terminal 1 — API at :8081 (auto-migrates on startup)
cd apps/api && cargo run

# Terminal 2 — UI at :8080 with proxy to the API
npm install
npm run dev:web
```

Open `http://127.0.0.1:8080`. The Vite proxy routes `/v1`, `/health`, and `/openapi.json` to the API port.

---

## Docker image versioning

Images are published to Docker Hub and GHCR on every `vX.Y.Z` tag pushed to `main`:

| Tag pushed | Images published |
|------------|-----------------|
| `v1.2.3` | `:1.2.3`, `:1.2`, `:1`, `:latest` |

Quien siga `:2` no recibirá la 3.x automáticamente — es la vía conservadora.

---

## Stack

- **API:** Rust (`apps/api` — Axum, SQLx, utoipa)
- **Domain:** `crates/domain` — shared `UserId`, `Decimal` re-exports; no `f64` in financial code
- **Engine:** `crates/engine` — pure projection math, no I/O
- **UI:** React 19 + TypeScript + Vite (`apps/web`)
- **DB:** PostgreSQL 16 **embebido en la imagen** (binarios oficiales digest-pinned; PG 15 incluido solo para auto-`pg_upgrade` de volúmenes antiguos), socket-only
- **Build:** Cargo workspace + npm workspaces

OpenAPI spec available at `GET /openapi.json`.

---

## License

To be defined.
