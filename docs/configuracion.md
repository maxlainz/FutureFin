# Configuración

Todas las variables de entorno de FutureFin, con su valor por defecto y quién las lee. Y, al final,
los ajustes que **no** son variables de entorno sino opciones dentro de la app.

> **Desde la 3.0.0 ninguna variable es obligatoria.** `docker compose up -d` funciona con un `.env`
> vacío o sin ningún `.env`. Todo lo que hay en esta página es opcional.

## Dónde se ponen y quién gana

En producción, en un `.env` **junto a tu `docker-compose.yml`**. Compose lo lee solo.

Hay tres procesos distintos que leen configuración, y confundirlos es la causa habitual de "mi
variable no hace nada":

| Quién la lee | Qué variables | Cuándo se aplica |
|---|---|---|
| **Compose** | `FUTUREFIN_IMAGE`, `FUTUREFIN_TAG`, `APP_PORT` | Sustituidas en el YAML antes de que arranque ningún contenedor. El contenedor nunca las ve. |
| **El entrypoint del contenedor** | Casi todas las `FUTUREFIN_*` y las `POSTGRES_*` | Al arrancar el contenedor, antes de la API. Solo existen dentro de la imagen Docker. |
| **El binario de la API** | `PORT`, `SESSION_TTL_DAYS`, `COOKIE_SECURE`, `CORS_ORIGINS`… | Al arrancar el proceso de la API. |

Y una regla de precedencia: **el entorno real gana al `.env`**. Si una variable ya está exportada
en tu shell o inyectada por Compose, lo que ponga el fichero da igual. Cuando un cambio en `.env`
"no hace efecto", esto es lo primero que hay que mirar.

## Despliegue (las lee Compose)

| Variable | Por defecto | Qué hace |
|---|---|---|
| `FUTUREFIN_IMAGE` | `maxlainz/futurefin` | Imagen a usar. Cámbiala a `ghcr.io/maxlainz/futurefin` para tirar de GHCR, o a un nombre local para probar una imagen construida por ti. |
| `FUTUREFIN_TAG` | `latest` | Etiqueta de la imagen. Fíjala a `X.Y.Z` para que no salte de versión sola. Ver [actualizar.md](actualizar.md). |
| `APP_PORT` | `8080` | Puerto **del host**. El de dentro del contenedor es siempre 8080. |

## Contenedor: base de datos, backups y ciclo de vida

Las lee `docker-entrypoint.sh`, el proceso PID 1 de la imagen. Ninguna es obligatoria: los valores
por defecto son exactamente lo que corre una instalación normal.

| Variable | Por defecto | Qué hace |
|---|---|---|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded`. Desde la 4.0.0 los dos significan lo mismo: la base embebida. `external` **aborta** el arranque con instrucciones (ver abajo), y cualquier otro valor también. |
| `FUTUREFIN_MODE` | `serve` | `serve` \| `db-only`. `db-only` es el **modo rescate**: levanta PostgreSQL sin la API, para restaurar o inspeccionar. Lo usa `scripts/restore-postgres.sh`. |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | Cualquier otro valor desactiva el backup automático pre-migración. Si el backup falla, el arranque se aborta a propósito. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | Cuántos backups automáticos son intocables, por recientes. |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | Más allá de los anteriores, se borran los de más días que este número. |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` permite arrancar **sin volumen** montado. Los datos mueren con el contenedor: solo para pruebas de usar y tirar. |
| `FUTUREFIN_API_STOP_TIMEOUT` | `15` | Segundos de gracia para que la API cierre tras el SIGTERM, antes del SIGKILL. |
| `FUTUREFIN_PG_STOP_TIMEOUT` | `30` | Segundos de gracia para que PostgreSQL cierre tras el SIGINT (apagado *fast*), antes del SIGQUIT. Mantén el `stop_grace_period` del compose por encima de la suma de este y el anterior. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` | Dónde vive el volumen `ffdata`: estado del entrypoint, backups y área de `pg_upgrade`. Avanzado. |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | Directorio de los backups automáticos. Avanzado. |
| `FUTUREFIN_PG_LISTEN` | vacío (solo socket) | **Solo depuración.** Abre PostgreSQL a TCP dentro del contenedor. En producción es socket-only por diseño. |
| `FUTUREFIN_PG_LOG_LEVEL` | sin definir | **Solo depuración.** Equivale a `log_min_messages` de PostgreSQL. |
| `POSTGRES_USER` | `futurefin` | Nombre del rol. Ponlo **solo** si lo personalizaste en una instalación 2.x: el superusuario del cluster adoptado es ese, y sin el valor correcto el arranque muere con un mensaje claro. |
| `POSTGRES_DB` | `futurefin` | Nombre de la base de datos. Mismo motivo de compatibilidad. |
| `POSTGRES_PASSWORD` | sin definir | **Ya no hace falta**: la base es local y se accede por socket Unix con `trust`. Si viene, se aplica al rol y nada más. |

Hay además unas cuantas variables que el `Dockerfile` fija y que **no deberías sobrescribir**:
`PGDATA=/var/lib/postgresql/data`, `PG_MAJOR=16`, `WEB_STATIC_ROOT=/app/web` y `PORT=8080`.

## API: red, sesiones y logs

Las lee el binario de Rust. Sirven igual en Docker y en desarrollo.

| Variable | Por defecto | Qué hace |
|---|---|---|
| `PORT` | `8080` | Puerto en el que escucha la API (bind a `0.0.0.0`). En el contenedor vale siempre 8080; lo que cambias fuera es `APP_PORT`. Un valor no numérico cae al 8080 sin avisar. |
| `SESSION_TTL_DAYS` | `30` | Vida de la sesión (cookie y fila en base de datos). Entero entre **1 y 400**; fuera de rango o ilegible, vuelve a 30 en silencio. |
| `COOKIE_SECURE` | `false` | Marca la cookie `ff_session` como `Secure`. **Ponlo a `true` si sirves por HTTPS.** Cuidado con el parseo: solo `1`, `true`, `TRUE`, `yes` y `YES` cuentan como verdadero — `True`, `on` o `Yes` se leen como falso. |
| `CORS_ORIGINS` | `http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080` | Orígenes permitidos, separados por comas. Solo hace falta si accedes a la API desde otro origen; el despliegue normal es mismo-origen y nunca envía preflight. **Una entrada inválida hace fallar el arranque** a propósito, y una lista que quede vacía también. |
| `WEB_STATIC_ROOT` | sin definir (la imagen fija `/app/web`) | Carpeta del SPA compilado. Si el path no existe, la API avisa y arranca en modo solo-API (la interfaz dará 404). |
| `RUST_LOG` | `futurefin_api=info,tower_http=info,sqlx=warn` | Verbosidad, sintaxis de `EnvFilter`. Para depurar: `futurefin_api=debug,tower_http=debug,sqlx=info`. Un filtro inválido cae al de por defecto. |
| `FUTUREFIN_MCP_ENABLED` | `true` | `0` desmonta el servidor MCP (`/mcp`) **y** todo el protocolo OAuth. El panel de Conexiones sigue montado, para que apagar MCP nunca te quite la capacidad de revocar lo que ya concediste. Ver [mcp.md](mcp.md). Mismo parseo estricto que `COOKIE_SECURE`, salvo que **sin definir vale `true`**. |
| `FUTUREFIN_PUBLIC_URL` | se deriva de cada petición | Origen público (`https://tu-host`) para el OAuth del conector de claude.ai. Solo hace falta si tu proxy no manda `X-Forwarded-Proto`/`Host` correctos. Tiene que ser un origen pelado, sin path ni query: **si está y es inválido, el arranque falla**. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | Presupuesto total de reintentos al conectar con la base de datos (backoff 0,5 s → 1 → 2 → 4…). Entre 1 y 600; fuera de rango, 30. Dentro del contenedor casi nunca importa —el entrypoint ya espera a que PostgreSQL conteste antes de lanzar la API—; en desarrollo sí, si arrancas `cargo run` antes que el PostgreSQL de `docker-compose.dev.yml`. |
| `FUTUREFIN_RECONCILE_SWEEP_HOURS` | `24` | Horas entre barridos de conciliación de transferencias. **`0` lo desactiva.** No es el mecanismo principal —la conciliación automática ya corre tras cada mutación—, sino su red de reintento. Fuera de 0–168 vuelve a 24. |

## Bases de datos externas: retiradas en la 4.0.0

Hasta la 3.x el contenedor sabía hablar con un PostgreSQL de fuera. **Ya no.** PostgreSQL vive
dentro de la imagen y no hay forma de apuntarlo a otro sitio. Estaba anunciado desde la 3.0.0 —en
el README, en `.env.example` y en un aviso de deprecación en cada arranque—, y aquí está.

Qué hace hoy cada resto de aquella época:

| Resto de la 3.x | Qué hace la 4.0.0 |
|---|---|
| `DATABASE_URL` apuntando fuera, **con** base embebida ya en el volumen | La **ignora**, con un aviso en los logs: tus datos ya están dentro. Quítala del compose. |
| `DATABASE_URL` apuntando fuera, **sin** base embebida en el volumen | El contenedor **se niega a arrancar** y explica cómo migrar. No toca nada: ni tu base externa, ni el volumen. Tus datos siguen intactos donde están. |
| `FUTUREFIN_DB_MODE=external` | **Aborta** el arranque diciendo qué quitar. Ese valor se sigue reconociendo *solo* para poder dar esa explicación en vez de un error críptico. |
| `FUTUREFIN_EXTERNAL_WAIT_SECS` | Ya no existe: nadie la lee. |
| `POSTGRES_PASSWORD` | Sigue siendo opcional e inofensiva (ver la tabla de arriba). |

**Si todavía no has migrado**, la ruta es pasar una vez por la 3.9.0, la última versión que sabía
hacerlo. Pasos exactos en [actualizar.md](actualizar.md#vengo-de-2x-o-tengo-una-base-de-datos-externa).

`DATABASE_URL` **sigue existiendo y sigue siendo necesaria en desarrollo** (`cargo run` contra el
PostgreSQL de `docker-compose.dev.yml`). Lo que se ha retirado es el modo externo *del contenedor
de producción*.

**Cuidado con este pie**: si te dejas descomentada la `DATABASE_URL` de desarrollo en el `.env` que
le pasas al compose de producción, el contenedor la verá. Con el volumen ya poblado te llevas el
aviso; con el volumen vacío **no arranca**. Mantén ficheros `.env` separados y pásalos con
`--env-file`.

## Solo desarrollo

| Variable | Por defecto | Qué hace |
|---|---|---|
| `FUTUREFIN_API_PORT` | `8081` | Puerto al que el proxy de Vite manda `/v1`, `/health`, `/openapi.json`, `/.well-known`, `/oauth/token`, `/oauth/register`, `/oauth/revoke` y `/mcp`. Si cambias `PORT`, cambia esta también. |
| `WEB_DEV_PORT` | `8080` | Puerto del servidor de desarrollo de Vite. Si está ocupado, Vite coge otro sin avisar: mira el banner de la terminal. |
| `TEST_DATABASE_URL` | `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test` | PostgreSQL para los tests de integración. Ver [desarrollo.md](desarrollo.md). |

Estas dos primeras se leen **sin prefijo `VITE_`** y desde el `.env` de la **raíz del repositorio**,
no desde `apps/web/`.

## Lo que no se configura por entorno

Son constantes de código; cambiarlas es cambiar el programa:

- **Pool de conexiones**: máximo 10, mínimo 1, timeout de adquisición 5 s, `idle_timeout` 600 s,
  `max_lifetime` 1800 s.
- **Cache de proyección**: TTL deslizante de 60 minutos, en memoria. Se pierde en cada reinicio y
  se reconstruye sola.
- **Límite de cuerpo de petición**: 1 MiB global, 16 MiB para importar un `.ffbackup`.
- **Compresión gzip** de respuestas de más de 1 KB.

## Ajustes de la instalación (dentro de la app, no por entorno)

Esto no va en el `.env`: vive en la base de datos, se cambia desde la interfaz y afecta a toda la
instalación. Los toca el propietario.

| Ajuste | Dónde | Notas |
|---|---|---|
| **Divisa base** | `Ajustes → General → Divisa` (solo el propietario), y también en el paso 1 del asistente de bienvenida | `EUR`, `USD` o `GBP`. **Una sola por instalación**: FutureFin no convierte ni mezcla divisas. Decide también qué filas acepta el importador de CSV. Cambiarla más tarde **no reconvierte** los importes ya guardados: solo cambia el símbolo. Un código de tres letras que no sea uno de esos tres se rechaza con un 400. |
| **Zona horaria** | `Ajustes → General` | Define qué es "hoy" para los cálculos con fecha. El asistente propone la del navegador. |
| **Inflación anual asumida** | `Ajustes → Plan` | Entre 0 y 50 %. Hace crecer el objetivo FIRE y permite ver las cifras en euros de hoy. |
| **Modo de edad** | `Ajustes → Plan` | Enseñar fechas o edades en la proyección. |
| **Supuestos FIRE** | `Ajustes → Plan` | Tasa de retirada segura (SWR), tramos de IRPF del ahorro para el *gross-up*, y de dónde sale el ahorro mensual de la simulación (presupuesto, promedio real de movimientos, o mezcla de ambos). |
| **Permitir escritura vía MCP** | `Ajustes → Integraciones` | Interruptor vivo: al apagarlo, las herramientas de escritura de Claude se cortan al instante. Solo el propietario. Ver [mcp.md](mcp.md). |
| **Asistente de primera vez** | `Ajustes → General` | Se puede reabrir cuando quieras. No borra nada. |

## Ver también

- [Instalación](instalacion.md) · [Actualizar](actualizar.md) · [Copias de seguridad](backups.md)
- [Conectar Claude](mcp.md) — `FUTUREFIN_MCP_ENABLED` y `FUTUREFIN_PUBLIC_URL` en contexto
- [Desarrollo](desarrollo.md) — el bloque de variables de `split-dev`
