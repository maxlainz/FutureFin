# FutureFin

Aplicación de **finanzas personales** pensada para **self-hosting** (Docker): hogar compartido, presupuesto mensual, flujos próximos, proyección de patrimonio neto y planificación **FIRE / jubilación**. Es la **línea principal** del producto; sustituye el prototipo macOS histórico.

## Estado del repositorio

- Rama `main`: base estable y documentación de producto.
- Rama `dev`: desarrollo activo del servidor y del cliente web.

## Documentación de producto

La especificación MVP (paridad de capacidades respecto al cliente Swift de referencia, modelo multi-usuario, backups, oráculos de tests) vive en `[docs/README.md](docs/README.md)`.

## Stack de implementación (rama `dev`)

- **API:** Rust (`futurefin-api`, Axum), prefijo HTTP `/v1` para contratos estables.
- **Contrato:** OpenAPI generado en Rust (`utoipa`), expuesto en `GET /openapi.json`.
- **Persistencia:** PostgreSQL + **SQLx** (consultas parametrizadas; migraciones en `apps/api/migrations`).
- **Dinero / dominio:** crate `futurefin-domain` con `Decimal` para cantidades (sin `f64` en el modelo financiero).
- **Auth MVP:** usuario + contraseña (**sin email**), Argon2id (crate `argon2`), sesión en cookie `HttpOnly` (`ff_session`). Traits OAuth/OIDC reservados en `apps/api/src/auth/oauth.rs`.
- **Web:** React + TypeScript + Vite (`apps/web`).
- **Monorepo:** workspace Cargo + npm workspaces.
- **Ramas:** desarrollo en **`dev`**; `git push origin dev` (evitar subir código nuevo solo a `main` hasta merge explícito).

### Desarrollo local

**Requisitos:** Rust (stable), Node.js 20+, npm 10+, Docker opcional para Postgres.

```bash
cp .env.example .env
# Contenedor de BD con nombre fijo `futurefin-database` (véase `docker ps`).
docker compose up -d futurefin-database

# Terminal 1 — API (puerto 8080; migra la BD al iniciar). Carga `.env` de la raíz del repo automáticamente.
cd apps/api && cargo run

# Terminal 2 — web (puerto 5173; proxy a la API en /v1, /health y /openapi.json)
npm install
npm run dev:web
```

Comprueba la API: `curl -s http://127.0.0.1:8080/v1/health` · OpenAPI: `curl -s http://127.0.0.1:8080/openapi.json | head`

**Todo el stack con Compose:** el proyecto se llama `futurefin`; los contenedores aparecen como **`futurefin-database`** (PostgreSQL) y **`futurefin-api`** (servidor HTTP).

```bash
docker compose up -d --build
```

Reverse proxy con TLS sigue recomendado para entornos expuestos.

## Publicar en GitHub

Si aún no has enlazado el remoto, sigue [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md).

## Licencia

Por definir.