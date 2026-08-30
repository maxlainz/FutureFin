# Environment Variables & Configuration

Template: `.env.example` (cubre producción y desarrollo split-dev). Desde 3.0.0 **producción no
requiere ninguna variable** (`docker compose up -d` funciona con `.env` vacío o ausente).

## Key variables (binario API)

| Variable | Default | Notes |
|----------|---------|-------|
| `DATABASE_URL` | — (sin default) | El binario hace panic si falta (`DATABASE_URL must be set`) — pero **en el contenedor la fabrica el entrypoint** (socket Unix: `postgres:///futurefin?host=/var/run/postgresql&user=futurefin`), que la **exporta pisando** lo que hubiera. **4.0.0 retiró el modo DB externa**: una `DATABASE_URL` que no contenga `/var/run/postgresql` ya no conecta con nada — con cluster embebido presente se **ignora** con un `warn` («quítala de tu compose»), y **sin** cluster el entrypoint **aborta** (`refuse_external_database`, exit 1) indicando arrancar una vez la 3.9.0 con esa misma URL y ese mismo volumen. En split-dev sigue siendo la var normal y necesaria: `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`. |
| `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS` | `30` | Ventana total del retry de conexión (`db::connect_with_retry`, backoff 0,5→4 s). Acepta 1–600; fuera de rango cae al default. |
| `PORT` | `8080` | API listen port. Use `8081` in split-dev so Vite can use `8080`. |
| `RUST_LOG` | — | e.g. `futurefin_api=info,tower_http=info,sqlx=warn` |
| `CORS_ORIGINS` | 4 localhost entries | Comma-separated (fail-loud: entrada no parseable o lista vacía ⇒ panic al arrancar). Parseada una vez en `routes::cors_origins()`. **Desde 4.4.0 alimenta dos capas con privilegios distintos** (issue #85, hallazgo 4): `api_cors_layer` sobre `/v1/*`, `/health`, `/openapi.json` y el protocolo OAuth, **con `allow_credentials(true)`** (su credencial es la cookie `ff_session`); y `mcp::mcp_cors_layer` sobre `/mcp` **sin credenciales** (la suya es el header `Authorization`). Antes había una sola capa con `allow_credentials(true)` sobre todo el router: añadir un origen para que funcionara un cliente MCP de navegador concedía de paso acceso **con cookie** a `/v1/backup/user-export`, `/v1/api-tokens` e `/v1/installation`. La misma lista alimenta además la validación de `Origin` de `/mcp` (`StreamableHttpServerConfig::with_allowed_origins`). Tabla completa de headers/métodos por capa en [`api-routes.md`](api-routes.md) §"CORS y topes de body". |
| `COOKIE_SECURE` | `false` | Bool env var parsed by `main.rs`. |
| `SESSION_TTL_DAYS` | `30` | Acepta 1–400; un valor fuera de rango o no numérico **cae silenciosamente al default 30** (filter-then-default, no clamp). |
| `FUTUREFIN_MCP_ENABLED` | `true` | Bool (`parse_bool_env` de `main.rs`). **Desde 4.4.0 el switch ya NO desmonta rutas** (issue #85, doctrina D18): con `0`/`false`, `/mcp` y las 7 rutas raíz del protocolo OAuth (`/.well-known/oauth-*`, `/oauth/register\|token\|revoke`) se **montan igual** y responden **404 JSON `{code: "mcp_disabled"}`** para cualquier método, en vez de desaparecer. **Por qué cambió — el incidente**: hasta 4.3.1 desmontarlas solo se veía mal en la imagen publicada: el fallback final es un `ServeDir` que **no llama a su fallback para métodos distintos de GET/HEAD**, así que `POST /mcp` daba **405 con cuerpo vacío** y `GET /.well-known/oauth-authorization-server` daba **200 `text/html`** (el shell de la SPA). El conector de claude.ai fallaba al parsear JSON y mostraba «connection failed» sin causa — un control de seguridad que, al activarse, se diagnostica como avería. El test antiguo montaba el router sin la SPA, así que confirmaba un 404 que en producción no ocurría. **Doctrina D18**: la forma del router no depende del entorno — misma razón por la que `POST /v1/auth/sso` se monta siempre y responde `sso_disabled` con el SSO apagado. **Sigue sin cambios**: `/v1/oauth/authorize-details` y `POST /v1/oauth/authorize` **no se montan** (viven bajo `/v1`, cuyo fallback ya devolvía JSON), y `GET/DELETE /v1/oauth/connections[/{id}]` se montan **siempre**, igual que `/v1/api-tokens` — apagar MCP no puede dejarte sin poder revocar credenciales ya concedidas. Default habilitado: el endpoint es inerte sin credenciales (todo 401) y producción sigue sin requerir env vars. |
| `FUTUREFIN_PUBLIC_URL` | — (derivado del request) | **Opcional** (3.1.0; **admite subpath desde 4.4.0**, issue #85). Origen público canónico usado como `issuer` OAuth y para construir los endpoints de la metadata `.well-known`, el `resource` RFC 8707 (`{issuer}/mcp`) y el `resource_metadata` del 401 de `/mcp`. Sin ella, `oauth/url.rs::public_base_url` lo **deriva por request**: `X-Forwarded-Proto` + `X-Forwarded-Host` (primer valor de cada uno) o el header `Host`, con charset estricto (`host[:puerto]`, IPv6 entre corchetes, ≤255 chars, sin `/`, `@`, espacios ni controles) → si no cuadra, **400 `invalid_request`**; el prefijo de la request **nunca** se usa para componer el path (ver el porqué más abajo). **Fíjala si tu reverse proxy no manda esos headers** (o manda un `Host` interno), **o si sirve la app bajo un subpath**: en ambos casos claude.ai recibiría un issuer inalcanzable/sin prefijo y la conexión falla. Formato: `https://finanzas.example.com` o, con subpath, `https://finanzas.example.com/futurefin` — el path se valida con `prefix::normalize_prefix` (la MISMA función, ya probada, que valida `FUTUREFIN_BASE_PATH`): empieza por `/`, sin `//`, sin segmentos `.`/`..`, charset `[A-Za-z0-9._~/-]` (el `%` prohibido a propósito), ≤128 chars; barra final se recorta; `/` a secas ⇒ raíz. **Query y fragmento siguen prohibidos**. **Por qué no se compone del prefijo del request** (`prefix::request_prefix`): el issuer es una **identidad**, no una decoración — un `X-Forwarded-Prefix` falsificado solo deforma la respuesta del propio atacante mientras es un asset, pero en cuanto ese texto entra en un documento de descubrimiento deja de ser inocuo, y un valor de operador (fail-loud) no lo puede mover una cabecera; además, bajo el Ingress de Home Assistant el prefijo es un token efímero de sesión (`/api/hassio_ingress/<token>`) que no debe hornearse en un issuer — irrelevante en la práctica porque MCP/OAuth documentan ir por el puerto directo, no por el Ingress. **Validación fail-loud** como `CORS_ORIGINS` (`main.rs::public_url()`): si está presente pero es inválida (no parseable, esquema ≠ http/https, sin host, con path inválido, o con path/query/fragmento donde no toca) el arranque hace **panic** en vez de servir metadata OAuth rota en silencio. Se normaliza al origen ASCII + prefijo. Con prefijo de proxy en la request (`X-Ingress-Path`/`X-Forwarded-Prefix`) y esta var sin definir, `warn_missing_public_url_for_prefix` emite un **`warn` una vez por proceso** — el síntoma sin esa línea era un 404 mudo en `/oauth/token`. El log de arranque la imprime (`public_url=…` o `(derived from request)`). Irrelevante con `FUTUREFIN_MCP_ENABLED=0`. |
| `FUTUREFIN_RECONCILE_SWEEP_HOURS` | `24` | **Opcional** (3.8.1). Cada cuántas horas corre el barrido de conciliación de transferencias (`main.rs::spawn_reconcile_sweep` → `reconcile::sweep_all_owners`), la **única tarea periódica del binario**. `0` la desactiva. Se parsea como `u64` y se **descarta si supera 168** (una semana): valor no parseable, negativo o `>168` → default 24, sin avisar (`main.rs::reconcile_sweep_hours`). Es una **red de reintento**, no el mecanismo principal: el pase corre ya tras cada mutación del conjunto, y esos pases son best-effort (un fallo se loguea y no convierte una escritura persistida en 5xx), así que sin barrido un fallo puntual dejaba el par sin conciliar para siempre y en silencio. La primera pasada va **tras el primer intervalo**, no al arrancar. En modos B/C, una pasada que recupera pares **invalida la cache de proyección** (D12a) — si no recupera nada, no la toca. La tarea se aborta antes de cerrar el pool en el apagado ordenado. |
| `FUTUREFIN_BASE_PATH` | — (`""` = raíz) | **Opcional** (add-on HA). Prefijo público fijo para despliegues tras un proxy con subpath que **no** manda `X-Forwarded-Prefix`. Parseada en `main.rs::base_path()` → `prefix::validate_base_path_env`. Es la **fuente de menor precedencia**: `X-Ingress-Path` > `X-Forwarded-Prefix` > esta var > `""` (`prefix::request_prefix`). Bounds de `normalize_prefix`: empieza por `/`, sin `//`, sin segmentos `.`/`..`, charset `[A-Za-z0-9._~/-]`, ≤128 chars; `/` o vacío ⇒ raíz; una barra final se recorta. **Fail-loud** como `FUTUREFIN_PUBLIC_URL`: presente pero inválida ⇒ **panic** al arrancar, en vez de servir HTML con refs rotos. Solo afecta a lo que resuelve el navegador (refs del `index.html`, `fetch`/`pushState`, `Path` de la cookie): el router sigue montado en la raíz. **No arregla MCP/OAuth bajo subpath por sí sola** — desde 4.4.0 el arreglo es **declarar `FUTUREFIN_PUBLIC_URL` con el prefijo** (esa variable admite path desde 4.4.0, ver su fila). Separación deliberada: el issuer OAuth es una **identidad**, y una identidad no la puede mover una cabecera ni un valor de menor precedencia como este — solo un valor de operador fail-loud (ver [`api-routes.md`](api-routes.md) §URL pública). El log de arranque la imprime (`base_path=… ` o `(root)`). |
| `FUTUREFIN_TRUSTED_PROXY_IPS` | — (`Disabled`) | **Opcional** (add-on HA). Peers cuya palabra sobre identidad y embebido en iframe se acepta. Parseada en `main.rs::trusted_peers()` → `prefix::PeerPolicy::from_env_value`. Valores: sin definir o vacía ⇒ `Disabled` (**nadie** es de confianza, el default seguro); `any` (case-insensitive) ⇒ `Any`, todo peer — **solo** para tests y para relajar el frame tras un proxy en red privada; **incompatible con `FUTUREFIN_TRUSTED_PROXY_AUTH=1`** (el arranque hace panic: sería un «entra como quien digas» para cualquiera que alcance el puerto); lista de IPs separadas por comas ⇒ `List` (el add-on de HA usa `172.30.32.2`, el ingress del Supervisor). **Fail-loud** estilo `CORS_ORIGINS`: una entrada que no parsea como `IpAddr` hace **panic**, y una lista que resuelve vacía también. La IP la aporta `PeerIp` desde `ConnectInfo<SocketAddr>` (`main.rs` sirve con `into_make_service_with_connect_info`); un peer desconocido solo pasa con `any`. Habilita dos cosas y **nada más**: relajar el anti-clickjacking a `frame-ancestors 'self'` cuando además llega `X-Ingress-Path` (`handlers/frame.rs`), y aceptar identidad por cabeceras si `FUTUREFIN_TRUSTED_PROXY_AUTH=1`. La detección del prefijo **no** la usa a propósito. |
| `FUTUREFIN_TRUSTED_PROXY_AUTH` | `false` | **Opcional** (add-on HA). Bool (`parse_bool_env`). Con `1`/`true`, `POST /v1/auth/sso` acepta la identidad de `X-Remote-User-Id` desde un peer de confianza; apagada, ese endpoint —que **se monta siempre**— responde 401 `sso_disabled`. **Combinación fail-loud** (`main.rs`): `FUTUREFIN_TRUSTED_PROXY_AUTH=1` con `FUTUREFIN_TRUSTED_PROXY_IPS` sin definir hace **panic** al arrancar, porque aceptaría `X-Remote-User-Id` de cualquiera. El log de arranque imprime `trusted_header_auth`. Contrato completo: [`auth-and-membership.md`](auth-and-membership.md) §SSO. |
| `FUTUREFIN_HA_SSO_URL` | — (login con HA apagado) | **Opcional** (4.3.1, **exclusiva del add-on**). Origen público de Home Assistant, el que tecleas en el navegador (`https://ha.midominio.com`, `http://homeassistant.local:8123`). De ella cuelgan la URL de autorización (`{base}/auth/authorize`), la del canje (`/auth/token`), la de revocación (`/auth/revoke`) y la del WebSocket de identidad (`ws(s)://…/api/websocket`). Parseada en `main.rs::ha_sso_url()` con las **mismas reglas y el mismo fail-loud que `FUTUREFIN_PUBLIC_URL`**: presente pero no parseable, esquema ≠ http/https, sin host, o con path/query/fragmento ⇒ **panic al arrancar** (una URL deforme se manifestaría como un login que redirige a ninguna parte, no como un error). Se normaliza al origen ASCII; vacía o solo blancos = ausente. Con ella puesta, `AppState.ha_sso` existe y eso —y **solo** eso, sin depender del peer ni de ninguna cabecera— es lo que enciende `window.__FF_HA_LOGIN__` y el botón «Entrar con Home Assistant». El log de arranque la imprime (`ha_sso_url=…` o `(disabled)`). Contrato del flujo: [`api-routes.md`](api-routes.md) §`/v1/auth/ha/start`. |
| `FUTUREFIN_HA_ADDON` | `false` | **Interna del add-on** (4.3.1). Bool (`parse_bool_env`, mismo quirk que `COOKIE_SECURE`: solo `1/true/TRUE/yes/YES`). La exporta **el entrypoint** cuando detecta `/data/options.json`; no la pongas a mano. Su única función hoy es autorizar la anterior: **`FUTUREFIN_HA_SSO_URL` sin `FUTUREFIN_HA_ADDON=1` hace panic al arrancar** (`main.rs`), en vez de ignorarse. Es deliberado — una instalación compose que la configurara creería tener un login que no puede funcionar: el `client_id` que HA acepta es el origen de ESTA app, y HA solo lo acepta cuando ambos comparten el mismo origen a través de su propio Ingress. Al revés (flag sin URL) es normal: es lo que corre en cualquier add-on sin la opción rellenada. |
| `WEB_STATIC_ROOT` | — | Path to Vite `dist/`. Docker sets `/app/web`. Omit for API-only. |
| `FUTUREFIN_API_PORT` | `8081` | Used by Vite proxy (`vite.config.ts`) |
| `WEB_DEV_PORT` | `8080` | Vite dev server port |

## Entrypoint del contenedor (3.0.0, `apps/api/docker-entrypoint.sh`)

| Variable | Default | Notes |
|----------|---------|-------|
| `FUTUREFIN_DB_MODE` | `auto` | `auto` \| `embedded` — desde 4.0.0 **son sinónimos** (siempre la embebida). `external` se sigue reconociendo **solo para abortar con un mensaje útil** a quien lo arrastre de un compose 3.x; cualquier otro valor aborta con `invalid FUTUREFIN_DB_MODE`. |
| `FUTUREFIN_MODE` | `serve` | `serve` \| `db-only` (modo rescate: solo PostgreSQL, sin API; lo usa `scripts/restore-postgres.sh`). También como argv: `docker run … db-only`. |
| `FUTUREFIN_BACKUP_KEEP` | `10` | Nº de backups automáticos pre-migración intocables (los más recientes). |
| `FUTUREFIN_BACKUP_KEEP_DAYS` | `90` | Del resto, se borran los más viejos que esto. Poda extra bajo 256 MB libres (nunca los 3 últimos). |
| `FUTUREFIN_PREMIGRATION_BACKUP` | `on` | `off` desactiva el backup automático pre-migración (y su aborto-si-falla). |
| `FUTUREFIN_ALLOW_EPHEMERAL_DB` | `0` | `1` permite arrancar **sin volumen** en `PGDATA` (CI/demo). Sin esto, el contenedor aborta a propósito. |
| `FUTUREFIN_API_STOP_TIMEOUT` / `FUTUREFIN_PG_STOP_TIMEOUT` | `15` / `30` | Timeouts del apagado ordenado (API: TERM; postmaster: SIGINT). La escalada está acotada: señal de escalada (KILL / QUIT) y, si 10 s después sigue vivo, KILL — nunca puede bloquear. |
| `FUTUREFIN_STATE_DIR` | `/var/lib/futurefin` | Volumen `ffdata`: backups, staging de pg_upgrade, estado (`state/cluster.env` con `REINDEXED_SYSID`, `state/pgupgrade.env`, `state/last-version`). Avanzada. |
| `FUTUREFIN_BACKUP_DIR` | `$FUTUREFIN_STATE_DIR/backups` | Avanzada. |
| `FUTUREFIN_PG_LISTEN` | vacío (sin TCP) | Solo depuración: `127.0.0.1` abre TCP dentro del contenedor. |
| `FUTUREFIN_PG_LOG_LEVEL` | — | Solo depuración: `log_min_messages` del PG embebido. |
| `POSTGRES_USER` / `POSTGRES_DB` | `futurefin` | Compat con instalaciones 2.x personalizadas. |
| `POSTGRES_PASSWORD` | — | **Ya no es obligatoria** (socket local, auth trust). Si viene, se aplica al rol y nada más. |
| `PGDATA` | `/var/lib/postgresql/data` | Avanzada; cambiarla rompe la compat con volúmenes 2.x. **Bajo Home Assistant el entrypoint la pisa** con `/data/pgdata` (ver §Modo add-on). |
| `PG_MAJOR` | `16` | Major de PostgreSQL que arranca el entrypoint (resuelve `/usr/lib/postgresql/$PG_MAJOR/bin` y gobierna el auto-`pg_upgrade`). **No la toques**: la imagen solo empaqueta los binarios 15 y 16, y apuntar a un major no empaquetado aborta el arranque con error explícito en `maybe_pg_upgrade`. Documentada porque es un override real (`${PG_MAJOR:-16}`, `docker-entrypoint.sh`), no una constante. |

### Modo add-on de Home Assistant (entrypoint)

**La detección es la presencia de `/data/options.json`** — el fichero que el Supervisor escribe con
las opciones del add-on. No hay ninguna otra señal fiable desde dentro del contenedor. La variable
interna `HA_ADDON` (0/1) se imprime en la línea de arranque (`ha_addon=…`).

**Overrides de rutas, explícitos y ANTES de la sección de Configuración**: el Supervisor monta **un
único** bind persistente en `/data`, así que

| Variable | Valor bajo HA | Por qué el override es explícito |
|---|---|---|
| `PGDATA` | `/data/pgdata` | El Dockerfile las exporta como `ENV`, así que los `${VAR:-default}` de la sección de Configuración **nunca** verían un valor de HA: verían el del ENV, que apunta fuera del único volumen persistente y perdería la base al recrear el contenedor. |
| `FUTUREFIN_STATE_DIR` | `/data/state` | Idem — ahí van los backups pre-migración (`/data/state/backups`). |

**Mapeo `options.json` → env** (helper `ha_opt`, que lee con `jq` comprobando `has($k)` en vez de
`// empty`: el `//` de jq trata `false` como vacío, así que un booleano puesto a `false` se leería
como ausente y el toggle no se aplicaría nunca):

| Opción | Efecto |
|---|---|
| `log_level` | `trace`/`debug` → `RUST_LOG=futurefin_api=debug,tower_http=debug,sqlx=warn`; `warn`/`error` → `…=warn,…=warn,sqlx=error`. `info`/`notice`/vacío: se respeta el `RUST_LOG` que ya hubiera. |
| `sso` = `true` | `FUTUREFIN_TRUSTED_PROXY_AUTH=1`. La lista `FUTUREFIN_TRUSTED_PROXY_IPS` (default `172.30.32.2`, el ingress del Supervisor — el único peer que alcanza al add-on) se exporta **aparte y siempre**, ver la nota bajo la tabla; así la combinación fail-loud «auth sin lista» es inalcanzable aquí. |
| `mcp` = `false` | `FUTUREFIN_MCP_ENABLED=0` — desde 4.4.0 **no desmonta** `/mcp` ni el protocolo OAuth, solo cambia su handler a 404 JSON `mcp_disabled` (ver la fila de esa variable arriba). |
| `cors_origins` | `CORS_ORIGINS`, si no está vacía — alimenta las dos capas CORS (API con cookie, `/mcp` sin ella) más la validación de `Origin` de `/mcp` (ver la fila de esa variable arriba). |
| `public_url` | `FUTUREFIN_PUBLIC_URL`, si no está vacía — desde 4.4.0 admite subpath. |
| `ha_sso_url` (4.3.1) | `FUTUREFIN_HA_SSO_URL`, si no está vacía → enciende «Entrar con Home Assistant». Es la URL **pública** de HA (la que usa el navegador para el redirect y el propio add-on para canjear el código y leer la identidad); **no** requiere `hassio_api` ni `homeassistant_api`. Vacía (el default) = botón apagado, y todo queda byte-idéntico a la 4.3.0. Fail-loud: un valor deforme hace panic al arrancar. |

Además, en modo add-on el entrypoint exporta **siempre** `FUTUREFIN_HA_ADDON=1` (la señal de que
corre bajo el Supervisor) y `FUTUREFIN_TRUSTED_PROXY_IPS` (default `172.30.32.2`) — este último con
`sso` o sin él, porque el iframe del Ingress necesita el peer de confianza para que
`handlers/frame.rs` relaje el anti-clickjacking; sin la lista la respuesta sale con
`X-Frame-Options: DENY` y **el panel se ve en blanco** aunque el add-on funcione. Lo que sí depende
del toggle `sso` es aceptar identidad por cabeceras, que es la frontera de seguridad de verdad.

**Guarda de volumen: `is_persisted`, no `is_mounted`.** La comprobación de «hay volumen montado en
`PGDATA`» sube por los ancestros preguntando `is_mounted` y **para antes de `/`**: en cualquier
contenedor `/` es un mountpoint (el rootfs del overlay), así que aceptarlo convertiría la guarda en
decorativa. Sigue mordiendo igual en compose sin volumen (`/var/lib/postgresql/data`,
`/var/lib/postgresql`, `/var/lib`, `/var` — ninguno es mountpoint ⇒ aborta), y bajo HA acepta
`/data/pgdata` porque `/data` sí es el bind del Supervisor. Con el `is_mounted` a secas ese caso
moría, porque el mountpoint es el padre y no `$PGDATA`. `ensure_runtime_dirs` crea además `$PGDATA`
si falta (bajo HA no existe en el primer arranque) — **solo si falta**: tocar el directorio de un
cluster existente cambiaría el uid que inspecciona `adopt_cluster` y se saltaría la adopción de un
volumen 2.x.

> `.env.example` lista las tres comentadas en el bloque Producción (verificar con
> `grep -n TRUSTED_PROXY .env.example`): son variables del despliegue tras proxy — bajo Home
> Assistant no hacen falta porque las fija el propio entrypoint desde `options.json`.

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

**Trampa (corregida 2026-08-22)**: la versión larga de esta advertencia decía que un `.env` de
desarrollo con `DATABASE_URL` junto al `docker-compose.yml` de producción «se la pasa al
contenedor». **No es cierto con los compose de este repo**: ninguno declara `env_file:` ni lista
`DATABASE_URL` en `environment:` (`grep -n 'env_file\|DATABASE_URL' docker-compose*.yml` → vacío),
y Compose no inyecta el `.env` en el contenedor. La variable solo llega si el compose la declara
(el de 2.x lo hace) o vía `docker run -e DATABASE_URL=…`. Cuando llega y apunta fuera del socket,
4.0.0 la ignora (con cluster) o **aborta** (sin cluster). Aun así, mantén `.env` de dev y de prod
separados.

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

## Secrets de GitHub Actions (no son env vars del binario)

| Secret | Usado por | Qué es |
|---|---|---|
| `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` | `publish-image.yml`, `dockerhub-description.yml` | Publicación en Docker Hub |
| `DEPENDABOT_ALERTS_TOKEN` | `dependabot-alerts-mirror.yml` (solo el paso de LECTURA) | Token con acceso de lectura a las alertas Dependabot — el `GITHUB_TOKEN` de Actions no puede leerlas. **TODO(higiene)**: hoy es un token clásico con scope `repo`; sustituir por un PAT fine-grained con solo «Dependabot alerts: Read» sobre este repo. |
