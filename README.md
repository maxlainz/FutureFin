# FutureFin

Aplicación de **finanzas personales** pensada para **self-hosting** (Docker): hogar compartido, presupuesto mensual, flujos próximos, proyección de patrimonio neto y planificación **FIRE / jubilación**. Es la **línea principal** del producto; sustituye el prototipo macOS histórico.

## Estado del repositorio

- Rama `main`: base estable y documentación de producto.
- Rama `dev`: desarrollo activo del servidor y del cliente web.

## Documentación de producto

La especificación MVP (paridad de capacidades respecto al cliente Swift de referencia, modelo multi-usuario, backups, oráculos de tests) vive en `[docs/README.md](docs/README.md)`.

## Stack de implementación (rama `dev`)

- **API:** Rust (`futurefin-api`, Axum), prefijo HTTP `/v1` para contratos estables.
- **Web:** React + TypeScript + Vite (`apps/web`).
- **Monorepo:** raíz con workspace Cargo + npm workspaces.
- **Datos:** PostgreSQL vía Docker Compose (servicio `postgres`); la API aún no persiste — siguiente iteración.

### Desarrollo local

**Requisitos:** Rust (stable), Node.js 20+, npm 10+, Docker opcional para Postgres.

```bash
cp .env.example .env
docker compose up -d postgres   # opcional: solo base de datos

# Terminal 1 — API (puerto 8080 por defecto)
cd apps/api && cargo run

# Terminal 2 — web (puerto 5173; proxy a la API en /v1 y /health)
npm install
npm run dev:web
```

Comprueba la API: `curl -s http://127.0.0.1:8080/v1/health`

**Todo el stack con Compose (API + Postgres):**

```bash
docker compose up --build api
```

Reverse proxy con TLS sigue recomendado para entornos expuestos.

## Publicar en GitHub

Si aún no has enlazado el remoto, sigue [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md).

## Licencia

Por definir.