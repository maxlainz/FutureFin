# FutureFin

Self-hosted personal finance app: shared household budget, upcoming cash flows, net-worth projection, and FIRE / retirement planning.

- **API:** Rust + Axum — all endpoints under `/v1/`
- **UI:** React 19 + TypeScript + Vite — embedded in the Docker image
- **DB:** PostgreSQL — migrations run automatically on startup
- **Auth:** username + password (Argon2id), `HttpOnly` session cookie, no email required
- **Multi-user:** one installation per deployment; new users wait for owner approval

---

## Quick start (Docker)

```bash
# Pull and run with the bundled compose file
curl -fsSL https://raw.githubusercontent.com/maxlainz/FutureFin/main/docker-compose.prod.yml -o docker-compose.prod.yml
curl -fsSL https://raw.githubusercontent.com/maxlainz/FutureFin/main/.env.prod.example -o .env.prod
```

Edit `.env.prod`:

```env
FUTUREFIN_IMAGE=ghcr.io/maxlainz/futurefin
FUTUREFIN_TAG=latest          # or a specific version: v1.0.0

FUTUREFIN_DOMAIN=finance.example.com
CADDY_EMAIL=you@example.com

COOKIE_SECURE=1
SESSION_TTL_DAYS=30
CORS_ORIGINS=https://finance.example.com

POSTGRES_USER=futurefin
POSTGRES_DB=futurefin
POSTGRES_PASSWORD=change_me_strong_password
```

Then start the stack:

```bash
docker compose --env-file .env.prod \
  -f docker-compose.prod.yml \
  -f docker-compose.tls.yml \
  up -d

# Verify
curl -sS https://finance.example.com/v1/health
```

The first user to register becomes the installation owner automatically.

---

## Compose files reference

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Local development (builds image locally, hardcoded dev credentials) |
| `docker-compose.prod.yml` | Production — pulls image from registry |
| `docker-compose.tls.yml` | Caddy reverse proxy overlay (HTTPS + automatic TLS via Let's Encrypt) |

### `docker-compose.prod.yml`

```yaml
name: futurefin

services:
  futurefin-database:
    image: postgres:16.4-alpine@sha256:5660c2cbfea50c7a9127d17dc4e48543eedd3d7a41a595a2dfa572471e37e64c
    container_name: futurefin-database
    restart: unless-stopped
    environment:
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: ${POSTGRES_DB}
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER} -d ${POSTGRES_DB}"]
      interval: 5s
      timeout: 5s
      retries: 5
    networks:
      - futurefin-net

  futurefin:
    image: ${FUTUREFIN_IMAGE}:${FUTUREFIN_TAG}
    container_name: futurefin
    restart: unless-stopped
    environment:
      PORT: "8080"
      WEB_STATIC_ROOT: /app/web
      DATABASE_URL: postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@futurefin-database:5432/${POSTGRES_DB}
      RUST_LOG: futurefin_api=info,tower_http=info
      COOKIE_SECURE: "${COOKIE_SECURE}"
      SESSION_TTL_DAYS: "${SESSION_TTL_DAYS}"
      CORS_ORIGINS: ${CORS_ORIGINS}
    depends_on:
      futurefin-database:
        condition: service_healthy
    networks:
      - futurefin-net

volumes:
  pgdata:

networks:
  futurefin-net:
```

### `docker-compose.tls.yml` (Caddy overlay)

```yaml
services:
  caddy:
    image: caddy:2-alpine
    container_name: futurefin-caddy
    restart: unless-stopped
    depends_on:
      - futurefin
    environment:
      FUTUREFIN_DOMAIN: ${FUTUREFIN_DOMAIN}
      CADDY_EMAIL: ${CADDY_EMAIL}
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./deploy/Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    networks:
      - futurefin-net

volumes:
  caddy_data:
  caddy_config:
```

`deploy/Caddyfile`:

```caddyfile
{
  email {$CADDY_EMAIL}
}

{$FUTUREFIN_DOMAIN} {
  reverse_proxy futurefin:8080
}
```

---

## Update & rollback

Change `FUTUREFIN_TAG` in `.env.prod` and re-run:

```bash
docker compose --env-file .env.prod \
  -f docker-compose.prod.yml \
  -f docker-compose.tls.yml \
  pull

docker compose --env-file .env.prod \
  -f docker-compose.prod.yml \
  -f docker-compose.tls.yml \
  up -d
```

Rollback: set `FUTUREFIN_TAG` to a previous `vX.Y.Z` and run the same `pull + up -d`.

---

## Backups

```bash
./scripts/backup-postgres.sh
```

Creates a gzip'd `pg_dump` in `./backups/`. Keeps the last `KEEP_BACKUPS` (default 30) dumps.

The API also exposes `GET /v1/backup/export.zip` (owner only) — CSV ZIP of all installation data.

---

## Environment variables

| Variable | Required | Notes |
|----------|----------|-------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `PORT` | No | Default `8080` |
| `WEB_STATIC_ROOT` | No | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only mode. |
| `CORS_ORIGINS` | Yes | Comma-separated. Panics on startup if empty. |
| `COOKIE_SECURE` | No | Set to `1` behind HTTPS |
| `SESSION_TTL_DAYS` | No | Default `30`, max `400` |
| `RUST_LOG` | No | e.g. `futurefin_api=info,tower_http=info` |

---

## Development

**Requirements:** Rust (stable), Node.js 20+, npm 10+, Docker (for Postgres).

```bash
cp .env.example .env
docker compose up -d futurefin-database   # Postgres only

# Terminal 1 — API at :8081 (auto-migrates on startup)
cd apps/api && cargo run

# Terminal 2 — UI at :8080 with proxy to the API
npm install
npm run dev:web
```

Open `http://127.0.0.1:8080`. The Vite proxy routes `/v1`, `/health`, and `/openapi.json` to the API port.

**Full stack via Docker Compose (local build):**

```bash
docker compose up -d --build
```

Serves the UI + API together at `http://127.0.0.1:8080`.

---

## Docker image versioning

Images are published to GHCR on every `vX.Y.Z` tag pushed to `main`:

| Tag pushed | Images published |
|------------|-----------------|
| `v1.2.3` | `:1.2.3`, `:1.2`, `:1`, `:latest` |

No `sha-*` or branch-based tags are published — versioning is strictly semver.

To publish to Docker Hub in addition to GHCR, add `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` as GitHub Actions secrets.

---

## Stack

- **API:** Rust (`apps/api` — Axum, SQLx, utoipa)
- **Domain:** `crates/domain` — shared `UserId`, `Decimal` re-exports; no `f64` in financial code
- **Engine:** `crates/engine` — pure projection math, no I/O
- **UI:** React 19 + TypeScript + Vite (`apps/web`)
- **DB:** PostgreSQL 16 with pinned digest in Compose files
- **Build:** Cargo workspace + npm workspaces

OpenAPI spec available at `GET /openapi.json`.

---

## License

To be defined.
