# Desarrollo

Cómo levantar FutureFin en local, ejecutar las pruebas y construir la imagen Docker sin publicarla.

## Requisitos

| Herramienta | Versión | Notas |
|---|---|---|
| Rust | stable | Lo fija `rust-toolchain.toml`; rustup lo coge solo. |
| Node.js | 24 recomendado (20+ funciona) | CI usa Node 24 y la imagen se construye con 24: usa 24 para parecerte a lo que se publica. |
| npm | 10+ | Hace falta el soporte de *workspaces*. |
| Docker + Compose v2 | reciente | Para el PostgreSQL de desarrollo y para construir la imagen. |

**PostgreSQL no se instala en la máquina.** En desarrollo corre como su propio contenedor; en
producción va dentro de la imagen de FutureFin.

Estructura del repositorio:

```
Cargo workspace: apps/api + crates/domain + crates/engine
npm workspace:   apps/web  (paquete futurefin-web)
```

## El entorno normal: `split-dev`

Dos procesos: la API de Rust en el puerto **8081** y el servidor de desarrollo de Vite en el
**8080**, que hace de proxy hacia la API. La imagen con PostgreSQL embebido **no se usa en
desarrollo**.

### 1. Configura el `.env`

```bash
cp .env.example .env
```

Desde la 3.0.0 **todas las líneas del ejemplo vienen comentadas** (producción no necesita ninguna
variable). Para `split-dev`, descomenta las tres del bloque de desarrollo:

```env
PORT=8081
DATABASE_URL=postgres://futurefin:futurefin@127.0.0.1:5432/futurefin
RUST_LOG=futurefin_api=info,tower_http=info
```

> **Cuidado con esa `DATABASE_URL` si en la misma máquina levantas el compose de producción.**
> La imagen interpreta cualquier `DATABASE_URL` que no apunte a su socket local como una base de
> datos externa, y desde la 4.0.0 esas ya no se soportan: con el volumen poblado la ignora con un
> aviso, y con el volumen vacío **se niega a arrancar**. El `docker-compose.yml` de este repositorio
> no se la pasa al contenedor (ni la lista en `environment:` ni usa `env_file:`), así que dejarla en
> el `.env` no basta para colártela; sí llega si la declaras a mano en tu compose o si lanzas la
> imagen con `docker run -e DATABASE_URL=…`. Aun así, ten ficheros `.env` separados y pásalos con
> `--env-file`.

### 2. Levanta el PostgreSQL de desarrollo

Es un compose **autónomo**, no un override:

```bash
docker compose -f docker-compose.dev.yml up -d
```

Publica `127.0.0.1:5432`, que es justo lo que necesita tu `cargo run`. Proyecto `futurefin-dev`,
servicio `db`, volumen `devdata`. **En producción no se usa nunca** (expone el puerto de la base de
datos en el host).

### 3. Arranca las dos mitades

```bash
# Terminal 1 — API en :8081 (aplica las migraciones al arrancar)
cd apps/api && cargo run

# Terminal 2 — interfaz en :8080, desde la raíz del repositorio
npm install
npm run dev:web
```

Abre `http://127.0.0.1:8080`. El proxy de Vite manda a la API estas rutas: `/v1`, `/health`,
`/openapi.json`, `/.well-known`, `/oauth/token`, `/oauth/register`, `/oauth/revoke` y `/mcp`. Todo
lo demás lo sirve el SPA.

Regístrate: **el primer usuario se convierte en propietario** de la instalación, igual que en
producción.

### Modo solo API (sin Vite)

Pon `PORT=8080` en el `.env` y `cd apps/api && cargo run`. Tendrás la API y `/openapi.json` en
`http://127.0.0.1:8080`, sin interfaz. Útil para trabajar a base de `curl`.

### Las migraciones se aplican solas

Los ficheros de `apps/api/migrations/` se **empotran en el binario al compilar** y se aplican en
cada arranque. No hay un paso de migración aparte.

Esto tiene una consecuencia al cambiar de rama: **arrancar la API muta el esquema de tu base de
datos de desarrollo**. Si la rama A añadió una migración y te vas a la rama B, que no la lleva, el
binario de B **no arranca**. Lo mismo pasa si el mismo número de versión tiene ficheros distintos:
ahí el error es de checksum.

Salida limpia — recrear la base de desarrollo:

```bash
docker compose -f docker-compose.dev.yml down
docker volume rm futurefin-dev_devdata
docker compose -f docker-compose.dev.yml up -d
```

Ojo con el nombre: `futurefin-dev_devdata` es el volumen de **desarrollo**. `futurefin_pgdata` es
el de **producción** — nunca lo borres para arreglar un problema de desarrollo.

Y si de verdad la migración era equivalente e idempotente, la salida quirúrgica es borrar su fila a
mano:

```bash
psql postgres://futurefin:futurefin@127.0.0.1:5432/futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>"
```

Es manual a propósito: hubo un mecanismo de auto-reparación y se quitó porque tapaba la deriva en
silencio. Un checksum que no cuadra tiene que fallar a gritos.

## Comandos de build y verificación

Todos desde la raíz del repositorio.

| Qué | Comando | ¿Necesita base de datos? |
|---|---|---|
| Compilar la API | `cd apps/api && cargo build` | No |
| Tests del engine (matemática pura) | `cargo test -p futurefin-engine` | No |
| Un test concreto del engine | `cargo test -p futurefin-engine -- <nombre>` | No |
| Suite completa (incluye integración) | ver abajo | **Sí** |
| Typecheck del frontend | `npm run typecheck:web` | No |
| Lint del frontend | `npm run lint:web` | No |
| Build de producción del frontend | `npm run build:web` → `apps/web/dist/` | No |
| Tests del frontend (Vitest) | `npm test --workspace futurefin-web` | No |

### Tests de integración

Necesitan su propio PostgreSQL, en el puerto **5433** para no chocar con el de desarrollo. Se
arranca una vez y se reutiliza siempre:

```bash
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine

TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

Cada test crea su propio esquema `ff_test_<uuid>` dentro de `futurefin_test`, le aplica todas las
migraciones y corre contra el router de verdad. Los esquemas **se dejan ahí a propósito**, para
poder inspeccionar un fallo después. Cuando molesten, lo más rápido es recrear el contenedor:

```bash
docker rm -f ff-test-db
```

### Qué corre la integración continua

`.github/workflows/ci.yml`, seis trabajos:

- **`secrets-scan`** — ningún dato personal en ficheros trackeados. Bloqueante y el primero.
- **`rust`** — el CHANGELOG cubre la versión de `Cargo.toml`, build de la API y tests del engine.
- **`web`** — typecheck, ESLint, Vitest y build.
- **`integration`** — `cargo test --workspace` contra un PostgreSQL de servicio. Es la mayor parte
  de la suite.
- **`docker-stack`** — construye la imagen y ejercita los caminos críticos del contenedor:
  instalación desde cero, recreación estilo watchtower, apagado ordenado, actualización real desde
  un stack 2.x, el rechazo de una `DATABASE_URL` externa heredada (aborta sin inicializar nada) y
  `pg_upgrade` 15→16.
Y aparte de `ci.yml`, tres workflows más:

- **`codeql.yml`** — análisis estático del código propio (`rust`, `javascript-typescript` y
  `actions`), en su propio workflow y **no** como check obligatorio.
- **`publish-image.yml`** — publica la imagen al empujar un tag `vX.Y.Z`, o por
  `workflow_dispatch` (con la casilla `create_tag` crea él mismo el tag sobre `main`). Dos
  guardas propias: CI verde sobre el commit del tag, y **orden estricto** (una versión no
  construye hasta que la anterior tiene su GitHub Release).
- **`dependabot-alerts-mirror.yml`** — vuelca las alertas Dependabot abiertas en el issue con
  label `dependabot-mirror` (diario + dispatch + push a manifiestos). Es la fuente de lectura
  de la rutina de dependencias; necesita el secret `DEPENDABOT_ALERTS_TOKEN`.

El propio `ci.yml` valida todos los workflows con **actionlint** (job `docker-stack`).

`cargo clippy` y `cargo fmt --check` están preparados pero **comentados**: el repositorio todavía
no está limpio para ellos y meterlos en rojo sería peor que no tenerlos. Los números medidos están
en el propio fichero.

## Construir la imagen en local

Sirve para validar el artefacto de producción completo (API + frontend + PostgreSQL embebido) sin
esperar a que CI publique nada.

```bash
# 1. Construir. La primera vez tarda; después reutiliza caché.
#    --load es obligatorio con BuildKit: sin él la imagen se queda en la caché del builder
#    y Compose intentará hacer pull de algo que no existe en ninguna registry.
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .

# 2. En el .env:
#      FUTUREFIN_IMAGE=futurefin-local
#      FUTUREFIN_TAG=dev
#    …y SIN una DATABASE_URL descomentada.

# 3. Arrancar con el override local
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d

# 4. Smoke test — /v1/ready valida también el PostgreSQL embebido
curl -sf http://127.0.0.1:8080/v1/ready

# 5. Tras cada cambio
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev . \
  && docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env \
     up -d --no-deps futurefin
```

`docker-compose.local.yml` solo añade `pull_policy: never` al servicio, para que Compose use la
imagen que únicamente existe en tu máquina.

**Verifica siempre qué versión sirve la imagen**, no solo que el contenedor arranque:

```bash
curl -s http://127.0.0.1:8080/v1/health
```

### Tres reglas del `Dockerfile` que no se pueden "simplificar"

- **La base de runtime no puede ser `postgres:*`.** Su `VOLUME` heredado crea volúmenes anónimos en
  un `docker run` sin volumen explícito, y un actualizador automático los pierde al recrear el
  contenedor: pérdida de datos silenciosa.
- **No se declara `VOLUME`** en el `Dockerfile`, por lo mismo. El entrypoint detecta el montaje real
  con `mountpoint` y se niega a arrancar sin él; un `VOLUME` desactivaría esa protección.
- **La comprobación con `ldd`** recorre todos los binarios y `.so` de PostgreSQL y **rompe el build**
  si alguno dice "not found". Si añades una extensión o quitas una librería del `apt-get install`,
  esto es lo que te avisa: lee la lista que imprime y añade el paquete que falte.

## Problemas frecuentes

| Síntoma | Causa | Solución |
|---|---|---|
| `Connection refused` en `127.0.0.1:5432` | No has levantado el PostgreSQL de desarrollo. El compose de producción **no publica** ningún puerto de base de datos. | `docker compose -f docker-compose.dev.yml up -d` |
| Vite dice «Port 8080 is in use» | Dejaste `PORT` comentado y `cargo run` cogió el 8080. | Pon `PORT=8081` en el `.env`. |
| Cambias el `.env` y no pasa nada | El entorno real gana al fichero. | `env \| grep -E 'DATABASE_URL\|PORT'` y `unset` lo que estorbe. |
| `VersionMismatch` / «previously applied but has been modified» | Una migración cambió después de aplicarse. | Recrea la base de desarrollo, o borra su fila de `_sqlx_migrations`. |
| El navegador enseña una interfaz vieja | `WEB_STATIC_ROOT` apunta a un `dist/` rancio, o estás en el puerto de la API. | En `split-dev` deja `WEB_STATIC_ROOT` sin definir y usa el 8080. |
| El contenedor local muere con `no persistent volume is mounted` | Protección anti-pérdida de datos: falta el volumen. | Arráncalo por Compose, o añade `-v` a mano. |
| Compose intenta hacer pull de `futurefin-local:dev` | Faltó `--load` en el build, o el override local. | Reconstruye con `--load` y añade `-f docker-compose.local.yml`. |

## Ver también

- [Configuración](configuracion.md) — todas las variables, incluidas las de desarrollo
- [Instalación](instalacion.md) · [Actualizar](actualizar.md) · [Copias de seguridad](backups.md)
