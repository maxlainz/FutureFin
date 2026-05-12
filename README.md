# FutureFin

Self-hosted personal finance app: shared household budget, upcoming cash flows, net-worth projection, and FIRE / retirement planning.

- **API:** Rust + Axum — all endpoints under `/v1/`
- **UI:** React 19 + TypeScript + Vite — embedded in the Docker image
- **DB:** PostgreSQL — migrations run automatically on startup
- **Auth:** username + password (Argon2id), `HttpOnly` session cookie, no email required
- **Multi-user:** one installation per deployment; new users wait for owner approval

---

## Quick start (Docker)

1. Descarga `docker-compose.yml` de este repositorio.
2. Crea `.env` en el mismo directorio:
   ```env
   POSTGRES_PASSWORD=tu_contraseña_fuerte
   ```
3. Arranca:
   ```bash
   docker compose up -d
   ```

El primer usuario en registrarse se convierte en propietario de la instalación.

### Actualizar

Cambia `FUTUREFIN_TAG` en `.env` y ejecuta:
```bash
docker compose pull && docker compose up -d
```

Para volver a una versión anterior, cambia `FUTUREFIN_TAG` al valor deseado y repite.

---

## Backups

El API expone `GET /v1/backup/export.zip` (solo owner) — ZIP CSV con todos los datos de la instalación.

---

## Environment variables

| Variable | Required | Notes |
|----------|----------|-------|
| `POSTGRES_PASSWORD` | Yes | DB password |
| `POSTGRES_USER` | No | Default `futurefin` |
| `POSTGRES_DB` | No | Default `futurefin` |
| `FUTUREFIN_IMAGE` | No | Default `maxlainz/futurefin` |
| `FUTUREFIN_TAG` | No | Default `latest`. Pin to `vX.Y.Z` for stability. |
| `APP_PORT` | No | Host port. Default `8080`. |
| `DATABASE_URL` | No | Set automatically from Postgres vars. Override only for external DB. |
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
docker compose up -d futurefin-database   # Postgres only

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

---

## Stack

- **API:** Rust (`apps/api` — Axum, SQLx, utoipa)
- **Domain:** `crates/domain` — shared `UserId`, `Decimal` re-exports; no `f64` in financial code
- **Engine:** `crates/engine` — pure projection math, no I/O
- **UI:** React 19 + TypeScript + Vite (`apps/web`)
- **DB:** PostgreSQL 16 with pinned digest in Compose file
- **Build:** Cargo workspace + npm workspaces

OpenAPI spec available at `GET /openapi.json`.

---

## License

To be defined.
