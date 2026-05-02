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
- **Una instalación por despliegue:** un único contexto de datos por base de datos; nuevos usuarios entran solo tras **invitación aprobada por el owner**. Contrato en [`docs/spec/AUTH_MODEL.md`](docs/spec/AUTH_MODEL.md).
- **Web:** React + TypeScript + Vite (`apps/web`).
- **Monorepo:** workspace Cargo + npm workspaces.
- **Ramas:** desarrollo en `**dev`**; `git push origin dev` (evitar subir código nuevo solo a `main` hasta merge explícito).

### Docker: versiones e imágenes de terceros

Las etiquetas `**latest`** no se usan para servicios críticos: en `docker-compose.yml` y en `apps/api/Dockerfile` las bases (**PostgreSQL**, **Node**, **Rust**, **Debian**) van **fijadas por etiqueta acotada + digest** para que un pull arbitrario no rompa instalaciones self-hosted. La imagen de compilación Rust usa `rust:bookworm` (cadena **stable** oficial) con digest concreto — al subir de versión de dependencias que eleven el MSRV, hay que **regenerar `Cargo.lock`** con ese toolchain y, si hace falta, actualizar el digest del builder en el Dockerfile.

Convención al publicar en un registro (Docker Hub u otro):


| Artefacto                          | Ejemplo de nombre publicado                                  |
| ---------------------------------- | ------------------------------------------------------------ |
| API + UI estática embebida         | `futurefin/futurefin-api:1.0.0`                              |
| (solo referencia) Postgres oficial | `postgres:16.4-alpine@sha256:…` — ya referenciado en Compose |


La aplicación construida en Compose usa `**image: futurefin/futurefin-api:dev**` junto con `build:` para etiquetar la imagen local de forma reconocible antes de `docker push`.

### Desarrollo local

**Requisitos:** Rust (stable), Node.js 20+, npm 10+, Docker opcional para Postgres.

Convención **split-dev** (interfaz con hot reload): **Vite en `8080`**, **API en `8081`** para no pisarse (`.env.example` ya trae `PORT=8081` y el proxy de Vite apunta a ese puerto).

```bash
cp .env.example .env
# Contenedor de BD con nombre fijo `futurefin-database` (véase `docker ps`).
docker compose up -d futurefin-database

# Terminal 1 — API en :8081 (migra la BD al iniciar). Carga `.env` de la raíz del repo automáticamente.
cd apps/api && cargo run

# Terminal 2 — interfaz en :8080; proxy a la API en /v1, /health y /openapi.json
npm install
npm run dev:web
```

Abre `**http://127.0.0.1:8080**` para la UI en desarrollo.

Comprueba la API: `curl -s http://127.0.0.1:8081/v1/health` · OpenAPI: `curl -s http://127.0.0.1:8081/openapi.json | head`

Si `**8080**` está ocupado, Vite intentará el siguiente puerto libre; añade ese origen (p. ej. `http://127.0.0.1:5173`) a `**CORS_ORIGINS**` en `.env`. Si `**8081**` está ocupado, cambia `**PORT**` en `.env` y `**FUTUREFIN_API_PORT**` al mismo valor.

**Solo API local** (sin Vite): pon `**PORT=8080`** en `.env` y ejecuta `cargo run`; las rutas quedan en `http://127.0.0.1:8080`.

**Todo el stack con Compose:** el proyecto se llama `futurefin`; los contenedores aparecen como `**futurefin-database`** (PostgreSQL) y `**futurefin-api`** (API Rust **y** interfaz web en el mismo puerto **8080**).

```bash
docker compose up -d --build
```

Abre `**http://127.0.0.1:8080**` para la UI embebida en la imagen; la API sigue en las mismas rutas (`/v1/…`, `/openapi.json`). Sin `WEB_STATIC_ROOT`, `cargo run` sirve solo la API (útil junto a Vite).

Reverse proxy con TLS sigue recomendado para entornos expuestos.

## Publicar en GitHub

Si aún no has enlazado el remoto, sigue [docs/GITHUB_SETUP.md](docs/GITHUB_SETUP.md).

## Licencia

Por definir.