# API Route Map

All routes in `apps/api/src/routes/mod.rs`. Routes under `/v1/` require valid session cookie `ff_session` unless noted.

## Top-level
| Method | Path | Handler | Auth |
|--------|------|---------|------|
| GET | `/health` | `health::health_check` | none |
| GET | `/openapi.json` | `openapi::openapi_json` | none |
| POST/GET/DELETE | `/mcp` | `mcp::mcp_router` (rmcp `StreamableHttpService`) | Bearer `ffp_…` (api_tokens) **o** `ffo_…` (OAuth) — ver sección MCP abajo |
| GET | `/.well-known/oauth-protected-resource` | `oauth::metadata::protected_resource` | none |
| GET | `/.well-known/oauth-protected-resource/mcp` | `oauth::metadata::protected_resource` (mismo handler; sufijo de path RFC 9728 §3.1) | none |
| GET | `/.well-known/oauth-authorization-server` | `oauth::metadata::authorization_server` | none |
| GET | `/.well-known/oauth-authorization-server/mcp` | `oauth::metadata::authorization_server` (mismo handler; sufijo RFC 8414 §3.1) | none |
| POST | `/oauth/register` | `oauth::register::register_client` (DCR, RFC 7591) | **ninguna — registro público** |
| POST | `/oauth/token` | `oauth::token::token` (`authorization_code`+PKCE / `refresh_token`) | client auth (`none` \| `client_secret_basic` \| `client_secret_post`) |
| POST | `/oauth/revoke` | `oauth::token::revoke` (RFC 7009) | client auth (idem) |

> `GET /oauth/authorize` **no tiene ruta backend**: la sirve el fallback SPA de `main.rs`. Ver la
> sección OAuth abajo — registrarla es un error que rompe la pantalla de consentimiento.

> **Las ocho filas de arriba se montan SIEMPRE, `FUTUREFIN_MCP_ENABLED` incluido.** El kill-switch
> cambia el *handler*, no la tabla de rutas: con `0`, `/mcp` y las siete rutas de protocolo OAuth
> responden **404 JSON `mcp_disabled`** (`ApiError::NotFoundWith`, mensaje compartido en
> `mcp::MCP_DISABLED_MESSAGE`), con cualquier método. Por qué, en §Kill-switch de la sección OAuth.
> La forma del router no depende del entorno — misma doctrina que `/v1/auth/sso` (D18).

### El `index.html` lo sirve un handler, no `ServeDir`

Todo lo que no es API ni un asset existente cae en `ServeDir(WEB_STATIC_ROOT)`, montado con
**`.append_index_html_on_directories(false)`** y con `.fallback(spa::serve_index)`. Ese `false` es
lo que hace que `GET /` **no** sea servido como fichero: cae al fallback, que es un handler
(`handlers/spa.rs::serve_index`) y no un `ServeFile`. Los assets hasheados sí los sigue sirviendo
`ServeDir` tal cual.

Por qué un handler: el prefijo público es **por request** (la misma imagen sirve compose en `/` y
el Ingress de Home Assistant bajo `/api/hassio_ingress/<token>` a la vez), así que ni un `base` de
Vite en build ni un placeholder reescrito al arrancar valen. `spa::load_index` lee el HTML del
disco **una vez** al arrancar (si no hay `index.html` legible se degrada a API-only con un `warn`);
por request, `spa::inject` reescribe los refs absolutos (`src="/…"`, `href="/…"` — las
protocol-relative `//…` y las absolutas con esquema no se tocan) e inserta, justo después de
`<head>`, un `<script>` con `window.__FF_BASE__`, `window.__FF_SSO__` y `window.__FF_HA_LOGIN__`
(los consume `apps/web/src/lib/basePath.ts`). Las dos banderas se calculan distinto **a propósito**
(`handlers/spa.rs::ShellFlags`):

| Bandera | Verdadera cuando | Por qué esa condición |
|---|---|---|
| `__FF_SSO__` | SSO por cabeceras habilitado **y** peer de confianza **y** la request trae `X-Remote-User-Id` | Es configuración **de la request**: el atajo solo existe si este mismo tráfico viene del Ingress. |
| `__FF_HA_LOGIN__` | `AppState.ha_sso.is_some()` — es decir, hay `FUTUREFIN_HA_SSO_URL` (`ha_idp::ha_login_available`) | Es configuración **del proceso**: el botón «Entrar con Home Assistant» existe precisamente donde NO hay proxy de confianza (el origen directo o un túnel), así que **no** puede depender del peer ni de ninguna cabecera. |

- **Invariante maestro**: sin prefijo, sin SSO y sin login de HA
  (`prefix.is_empty() && !sso && !ha_login`), `inject` devuelve `Cow::Borrowed` — los bytes
  **exactos** del fichero. El modo
  compose no cambia ni un carácter (unit test `ha_login_alone_injects_the_bootstrap` fija el otro
  lado: la bandera de HA sola ya basta para inyectar).
- **Cache headers**, atados a ese mismo invariante: respuesta sin modificar → `Cache-Control:
  no-cache` (el shell de una SPA se revalida siempre, los assets hasheados sí cachean); respuesta
  modificada → `Cache-Control: no-store` + `Vary: X-Ingress-Path, X-Forwarded-Prefix`, porque el
  HTML pasa a depender de cabeceras de proxy y ningún caché intermedio debe servir el shell de un
  despliegue con el prefijo de otro.
- Regresión: `apps/api/tests/base_path.rs` (+ los unit tests de `spa.rs`).

### CORS y topes de body — **dos superficies, dos privilegios** (4.4.0, issue #85)

`CORS_ORIGINS` sigue siendo **una sola lista** de orígenes, pero desde el issue #85 alimenta **dos
capas distintas**, y la diferencia es el privilegio que conceden:

| Capa | Dónde | `allow_credentials` | `allow_headers` | Se aplica a |
|---|---|---|---|---|
| API | `routes::app_router` → `api_cors_layer` | **`true`** — su credencial es la cookie `ff_session` | `Content-Type`, `Accept`, `Authorization` (esta última por `client_secret_basic` del protocolo OAuth) | `/v1/*`, `/health`, `/openapi.json`, `/oauth/*`, `/.well-known/*` |
| MCP | `mcp::mcp_cors_layer` | **ausente** — su credencial es el header `Authorization` | + `Mcp-Session-Id`, `MCP-Protocol-Version`, `Last-Event-ID`, `Mcp-Method`, `Mcp-Name`; expone además `WWW-Authenticate` | solo `/mcp` |

- **Por qué dos**: hasta 4.3.1 la capa era una sola sobre el router entero, con `allow_credentials(true)`.
  Añadir un origen para que funcionara un cliente MCP de navegador (el Inspector) concedía **de paso**
  acceso con cookie a `/v1/backup/user-export`, `/v1/api-tokens` y `/v1/installation`. Ahora la lista
  se comparte y el privilegio no.
- **El orden del `merge` es el que separa las capas** y no es cosmético: `Router::layer` solo envuelve
  las rutas **ya registradas**, así que `mcp` se mergea **después** del `.layer(api_cors_layer(...))`
  para quedar fuera de él. Si mueves ese `merge` arriba, `/mcp` hereda `allow_credentials(true)`.
- **`route_layer`, nunca `layer`, dentro del router de `/mcp`** (capa CORS y middleware Bearer):
  `Router::layer` envuelve también el *fallback* del router, y un `merge` arrastra ese fallback al
  router de destino — el resultado sería que **toda ruta desconocida**, `/oauth/authorize` incluida,
  pasaría por la auth Bearer del MCP y devolvería 401. Lo cazó
  `oauth_flow.rs::get_oauth_authorize_is_not_handled_by_the_api`, que por eso vigila también el 401
  y no solo el 405.
- **`WWW-Authenticate` va en `expose_headers`** porque no es una cabecera de respuesta *safelisted*:
  sin exponerla, un cliente de navegador no puede leer el `resource_metadata=` del 401 y **nunca
  descubre el authorization server** (RFC 9728 §5.1).
- **`Mcp-Param-*` (SEP-2243) NO está en la lista**: es un *prefijo*, no un nombre, y `allow_headers`
  solo admite nombres. Ningún cliente conocido los manda hoy; la alternativa
  (`AllowHeaders::mirror_request`) cambiaría una lista auditable por un espejo.

**Topes de body** (`routes::app_router` + `mcp::MCP_MAX_REQUEST_BODY_BYTES`):

| Superficie | Tope | Mecanismo |
|---|---|---|
| Todo lo que pasa por un extractor | **1 MiB** | `DefaultBodyLimit::max` global |
| `/v1/backup/user-import*` | **16 MiB** | `DefaultBodyLimit` propio de esas rutas |
| `/mcp` | **1 MiB** | `StreamableHttpServerConfig::with_max_request_body_bytes` — **explícito y obligatorio** |

> `/mcp` necesita su propia línea porque `DefaultBodyLimit` de axum actúa **a través de los
> extractores**, y `/mcp` es un `route_service`: el servicio de rmcp lee el body por su cuenta, con
> su propio default de **4 MiB**. El invariante «1 MiB global» que este documento y las skills
> afirmaban era falso justo en la única ruta del binario que no pasa por un extractor. Regresión:
> `body_limits.rs::oversized_mcp_body_returns_413` (2 MiB: por encima del global, por debajo del
> default del SDK — exactamente el hueco).

### OpenAPI (`GET /openapi.json`) — cómo declara la autenticación (4.0.0)

`apps/api/src/openapi.rs` genera la spec con `utoipa`. Hasta 4.0.0 **no declaraba ni un
`securityScheme`**: presentaba 81 operaciones con sesión obligatoria como si fueran públicas, y
cualquier cliente generado a partir de ella nacía sin enviar credencial ninguna.

- **`components.securitySchemes`** (añadidos por el `Modify` llamado `SecurityAddon`):
  `ff_session` (`apiKey`, `in: cookie`) y `bearer_token` (`http`, scheme `bearer`). Son dos
  credenciales **no intercambiables**: la cookie la usa la SPA en todo `/v1`; el Bearer (`ffp_…` /
  `ffo_…`) solo vale para `/mcp`, que deliberadamente **no** está en esta spec — se declara porque
  las 401 de la API lo mencionan y un lector necesita saber que existe.
- **`security` global**: `("ff_session" = [])` en el `#[openapi(...)]` raíz. La excepción se marca
  por operación con **`security(())`** (lista vacía = pública), y desde 4.3.1 la llevan exactamente
  **siete**: `health_check`, `ready_check`, `register`, `login`, `sso_login`, `ha_start` y
  `ha_callback` (`grep -rn 'security(())' apps/api/src`). Al añadir un handler público hay que
  ponerlo; al añadir uno privado no hay que hacer nada. Las tres últimas son «públicas» en el
  sentido de OpenAPI porque su credencial **no se puede expresar como `securityScheme`**: en
  `sso_login` es la palabra de un proxy de confianza (cabecera `X-Remote-User-Id` desde una IP
  autorizada); en `ha_start`/`ha_callback` es el **round-trip del navegador contra Home Assistant**
  más la cookie `ff_ha_state` de un solo uso — algo que ocurre *durante* la operación, no una
  credencial que el cliente presente al empezar. La política vive en la descripción de cada
  operación y en `handlers/{sso,ha_sso}.rs`. La lista está congelada en `PUBLIC_OPERATIONS`
  (`apps/api/tests/openapi_contract.rs`): añadir un público sin tocarla rompe el test.
- Otras tres deudas cerradas en el mismo cambio: dos structs distintos compartían el nombre de
  componente `ImportPreviewResponse` (preview de CSV y preview de backup) — utoipa nombra por el
  último segmento del tipo, así que uno machacaba al otro y **los dos endpoints apuntaban al mismo
  `$ref`**; se desambigua con `#[schema(as = TransactionImportPreviewResponse)]`. Un path con
  plantilla no declaraba su parámetro (documento formalmente inválido), y `?density` no estaba
  declarado mientras la descripción de `months` citaba `target_age`, eliminado en v1.0.6.
- **Gates**: `apps/api/tests/openapi_contract.rs` (cuatro tests sobre el propio documento —
  parámetros de path declarados, los dos previews con schema distinto, autenticación declarada y
  aplicada a toda operación privada, y cero `$ref` colgantes). No existía ningún test sobre la spec:
  por eso nada de lo anterior rompía nada.

## /v1 routes

### Health
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/health` | No auth |
| GET | `/v1/ready` | DB ping |

### Auth (`/v1/auth/`)
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/auth/register` | No prior session needed. First user auto-becomes installation owner. |
| POST | `/v1/auth/login` | Sets `ff_session` cookie |
| POST | `/v1/auth/logout` | Clears cookie + DB session |
| POST | `/v1/auth/password` | **4.0.0**. Sesión válida. Body `{current_password, new_password}` → **204**. Verifica la actual, aplica la política de longitud (12..=256 chars, `auth/password.rs`) y **revoca en la misma transacción** las demás sesiones, los tokens `ffp_` del usuario (`api_tokens.revoked_at`) y sus concesiones OAuth (`oauth_grants.revoked_at`, `revoked_reason = 'password_change'`). La sesión que llama **sobrevive** (se excluye por su propio `id`), para no echar al usuario de la app al terminar. |
| POST | `/v1/auth/sso` | **SSO por proxy de confianza**. Identidad delegada a un proxy de confianza (add-on de Home Assistant). Sin cuerpo; la credencial es la cabecera `X-Remote-User-Id` (UUID) desde un peer autorizado. Devuelve el mismo `UserResponse` que el login y pone la misma cookie `ff_session`. **Se monta siempre** (la forma del router no depende del entorno): con `FUTUREFIN_TRUSTED_PROXY_AUTH` apagado → 401 `sso_disabled`; peer fuera de `FUTUREFIN_TRUSTED_PROXY_IPS` → 401 `sso_untrusted_peer`; cabecera ausente o no-UUID → 400 `sso_bad_identity`. El primer usuario que entra por aquí crea el hogar y queda owner; los siguientes quedan pendientes. |
| GET | `/v1/auth/ha/start` | **4.3.1 — «Entrar con Home Assistant»**. Arranca el flujo de código de autorización contra HA como IdP. `?next=` opcional. **302** a `{FUTUREFIN_HA_SSO_URL}/auth/authorize?…` + cookie `ff_ha_state`. Se monta siempre; sin `FUTUREFIN_HA_SSO_URL` → **401 `ha_sso_disabled`** (este es el único error del flujo que sí sale como JSON, porque aquí el navegador todavía no está en mitad de una navegación venida de HA). |
| GET | `/v1/auth/ha/callback` | **4.3.1**. Vuelta del navegador desde HA (`?code=&state=` o `?error=`). Éxito → **302** a la app + cookie `ff_session`; fallo → **302** a `{prefijo}/?ha_error=<código>`. **Nunca** devuelve un cuerpo JSON de error. |
| GET | `/v1/auth/me` | Current user info |
| PATCH | `/v1/auth/me` | Update `birth_date` |

- **Las cuentas SSO no tienen contraseña** (`users.password_hash` NULL desde `20260827120000_users_trusted_header_identity.sql`). `POST /v1/auth/login`, `POST /v1/auth/password` y `POST /v1/backup/user-export` las rechazan con **401 `sso_account_no_password`** — un 401 hablado a propósito: sin él, la persona se queda probando una contraseña que nunca existió. El login sigue pagando el Argon2id de descarte antes de responder, así que el reloj no delata nada.

- **`current_password` incorrecta → 400 `current_password_invalid`, NO 401** (`handlers/auth.rs`).
  Es deliberado y load-bearing: la sesión es válida — lo que falla es un dato del formulario. Con un
  401, el handler global de no-autorizado de la SPA (`setUnauthorizedHandler`, ver
  [`frontend-structure.md`](frontend-structure.md)) echaría al usuario al login por escribir mal su
  propia contraseña.
- **Los `.ffbackup` ya exportados NO se recifran**: siguen atados a la contraseña con la que se
  generaron (su clave sale de la contraseña vía Argon2id, y el servidor no guarda la vieja). Aviso
  duplicado a propósito en `SECURITY.md` — un usuario que rota la contraseña porque sospecha un
  compromiso tiene que saber que su copia antigua sigue abriéndose con la contraseña filtrada.
- **Sin UI todavía**: la SPA no llama a este endpoint (`grep -rn 'auth/password' apps/web/src` está
  vacío). Se usa por API/`curl`.
- Argon2id corre en `spawn_blocking` en los cinco call sites (`auth/password.rs`): inline en un
  worker de Tokio, cuatro `/v1/auth/register` concurrentes —endpoint sin auth por diseño— paraban el
  proceso entero, `/v1/ready` incluido, y el healthcheck marcaba el contenedor unhealthy. El login
  verifica además SIEMPRE contra un hash constante aunque el usuario no exista (`dummy_hash`): sin
  eso, ~1 ms vs ~40-80 ms enumeraba quién tiene cuenta.

#### `POST /v1/auth/sso` — identidad delegada a un proxy de confianza (`handlers/sso.rs`)

**Request**: sin cuerpo y sin cookie. La credencial son cabeceras que pone el proxy:

| Header | Obligatorio | Uso |
|---|---|---|
| `X-Remote-User-Id` | **sí** | Identidad estable del proveedor. Debe parsear como UUID; se persiste en `users.external_user_id`. |
| `X-Remote-User-Display-Name` | no | Nombre para mostrar. **Gana** sobre el siguiente: es el que la persona reconoce como suyo. |
| `X-Remote-User-Name` | no | Nombre de cuenta del proveedor. Fallback del anterior. |

**200** → el mismo `UserResponse` que `login` (`{id, username, birth_date?}`) + `Set-Cookie:
ff_session` con el `Path` acotado al prefijo de la request (ver §Cookie en
[`auth-and-membership.md`](auth-and-membership.md)) + el mismo warm-up en background de la
proyección del hogar que hace `login` (se salta en silencio si el usuario está pending).

**Errores** — cinco códigos, todos con el prefijo `sso_`:

| Código | Status | Cuándo |
|---|---|---|
| `sso_disabled` | 401 | `FUTUREFIN_TRUSTED_PROXY_AUTH` apagado (el default). |
| `sso_untrusted_peer` | 401 | La IP del peer no está en `FUTUREFIN_TRUSTED_PROXY_IPS`. |
| `sso_bad_identity` | 400 | `X-Remote-User-Id` ausente o no parsea como UUID. |
| `sso_account_no_password` | 401 | *No lo devuelve este endpoint*: lo devuelven `login`, `password` y `user-export` cuando la cuenta es SSO. Se lista aquí porque es parte del mismo contrato. |
| `sso_username_unavailable` | 409 | Se agotaron los seis candidatos de nombre (slug, `-2`..`-5`, `ha-<8 hex del id externo>`) sin encontrar uno libre. |

- **Las dos primeras comprobaciones SON la frontera de seguridad entera.** Una cabecera de
  identidad es una afirmación sin prueba; solo vale la palabra de un peer que el operador nombró.
  La ruta se monta **siempre** (`routes/mod.rs`) para que la forma del router no dependa del
  entorno — lo que decide es el estado, no el montaje.
- **Provisión**: si no hay fila con ese `external_user_id`, se crea el usuario (`password_hash`
  NULL) y se ejecuta `bootstrap_installation_as_owner_if_empty` en la **misma transacción** — el
  primero crea el hogar y es owner, los siguientes quedan pendientes de aprobación, igual que
  `register`. Cada candidato de nombre abre su propia transacción (en Postgres una violación de
  unique aborta la transacción entera, así que reintentar dentro no es posible), y una colisión
  concurrente sobre `users_external_user_id_key` devuelve el usuario que ganó, no un error.
- **Omisión deliberada del catálogo MCP** (registro en `futurefin-mcp-parity`): es un mecanismo de
  sesión de navegador atado a cabeceras de un proxy, no una operación sobre datos. Ningún cliente
  MCP puede ni debe invocarlo — su credencial es el Bearer, no una cookie.
- Regresión: `apps/api/tests/sso_login.rs`.

#### `GET /v1/auth/ha/start` + `GET /v1/auth/ha/callback` — «Entrar con Home Assistant» (4.3.1, `handlers/ha_sso.rs` + `ha_idp/`)

El **segundo** camino de la misma identidad externa: el SSO por cabeceras solo funciona *dentro* del
Ingress del Supervisor (que ya autenticó a la persona); este funciona **donde no hay proxy de
confianza** — el add-on abierto por el puerto directo o por un túnel —, porque la prueba de
identidad no es una cabecera sino un round-trip del navegador contra el propio HA. Ambos caminos
convergen en la MISMA fila de `users` (ver [`auth-and-membership.md`](auth-and-membership.md) §SSO y
la decisión **D19** del contrato de arquitectura). Las dos rutas **se montan siempre**; lo que
decide es el estado (`AppState.ha_sso`, poblado solo si hay `FUTUREFIN_HA_SSO_URL` **y**
`FUTUREFIN_HA_ADDON=1` — ver [`env-and-config.md`](env-and-config.md)).

**Query params**

| Ruta | Param | Obligatorio | Uso |
|---|---|---|---|
| `/start` | `next` | no | Ruta de la app a la que volver. Se sanea con `ha_idp::sanitize_next` y viaja **dentro de la cookie**, nunca en el `state` (que es tamperable). |
| `/callback` | `code` | sí (salvo `error`) | Código de autorización de HA. Vacío o >512 chars ⇒ `ha_exchange_failed`. |
| `/callback` | `state` | sí | Debe casar con el nonce de la cookie (comparación en tiempo constante). |
| `/callback` | `error` | no | Lo manda HA si la persona rechaza el permiso (`access_denied`). No hay código que canjear ⇒ **no se llama al proveedor**. |

**Flujo, paso a paso** (el orden es contrato, no detalle de implementación):

1. `/start` resuelve el **origen público** (`oauth::url::public_base_url`) y lo **congela** en la
   cookie: HA compara el `client_id` del canje con el de la autorización **byte a byte**, y las
   cabeceras de la segunda petición podrían derivar otro (`Host` ausente o deforme ⇒ 400
   `missing or malformed Host header`).
2. Redirige (302) a `{ha}/auth/authorize` con exactamente tres parámetros: `client_id` = `{origen}/`
   (con barra final), `redirect_uri` = `{origen}/v1/auth/ha/callback` y `state` = nonce
   (`uuid4` en forma simple). **Nada de PKCE, `client_secret`, `scope` ni `response_type`**: HA no
   los soporta; la defensa es el mismo-origen exacto `client_id`↔`redirect_uri` (con eso HA no hace
   fetch de nuestra URL) más la cookie.
3. `/callback` **lee** la cookie, valida el `state` con `ct_eq`, y **la retira SIEMPRE** — pase lo
   que pase, éxito o fallo: es de un solo uso, y dejarla viva permitiría reintentar el mismo
   `state`. Solo **después** de validar el estado se mira si la instalación tiene HA configurado: un
   callback sin cookie es un callback ajeno y no merece saberlo.
4. Canje del código (`POST {ha}/auth/token`) con el `client_id` **byte-idéntico** derivado del
   origen de la cookie.
5. Identidad por **WebSocket** (`auth/current_user`): HA no la expone por REST.
6. **Revocación del refresh token ANTES de tocar la base de datos** (`POST {ha}/auth/revoke`,
   best-effort e infalible por firma). FutureFin no retiene ninguna credencial de la domótica; un
   fallo aquí se registra y el login sigue, porque ya está probado.
7. `resolve_or_provision` — **la misma función** que usa `POST /v1/auth/sso`, con el mismo
   `external_user_id`: el `result.id` de HA es `uuid4().hex` (32 hex **sin guiones**) y
   `Uuid::parse_str` lo normaliza al mismo UUID que la forma canónica de `X-Remote-User-Id`.
8. `establish_session` (fila en `sessions`, cookie `ff_session` acotada al prefijo, warm-up D7) y
   302 limpio a `{prefijo}{next}`.

**Cookie `ff_ha_state`** (`ha_idp::HA_STATE_COOKIE`), formato
`1.<nonce>.<b64url(origin)>.<b64url(next)>` — base64url **sin padding** (`=` no cabe en un
cookie-value sin comillas) y `1.` es la versión, para que un formato futuro pueda coexistir con las
cookies vivas en vez de reventarlas:

| Atributo | Valor | Por qué |
|---|---|---|
| `HttpOnly` | sí | El `state` es un secreto; ningún script tiene que leerlo. |
| `SameSite` | **`Lax` — obligatorio, no una preferencia** | El callback llega como navegación de nivel superior **cross-site** desde el dominio de HA. Con `Strict` el navegador no la manda y el flujo fallaría SIEMPRE con `ha_state_mismatch`. `None` exigiría `Secure`, que no se puede dar por hecho en una LAN por http. |
| `Max-Age` | `600` (10 min) | Lo que HA da a sus códigos; más tiempo solo alarga la ventana de un `state` robado. |
| `Path` | el mismo que `ff_session` (`session_cookie_path`) | Bajo el Ingress todos los add-ons comparten origen; con `Path=/` la cookie viajaría a los demás. Compartir el helper garantiza además que el `Set-Cookie` de borrado del callback **case** con la cookie viva (el navegador exige nombre **y** `Path` iguales). |
| `Secure` | sigue `COOKIE_SECURE` | Igual que la de sesión. |
| Tamaño | ≤ 2048 chars al decodificar; `next` ≤ 512 | Se rechaza antes de decodificar nada. |

**Los cinco errores llegan por REDIRECT, no en un cuerpo JSON.** El navegador está en mitad de una
navegación venida de HA: un 4xx con cuerpo dejaría a la persona mirando un JSON. Se vuelve a
`{prefijo}/?ha_error=<código>` (302, `Cache-Control: no-store`) y la SPA traduce el código con el
catálogo de `apps/web/src/lib/errorMessages.ts`. Los códigos existen **también** como mensajes de
`ApiError` para que `tests/error_codes_parity.rs` los recoja y ninguno se quede sin frase en
español:

| Código | Cuándo | Nota |
|---|---|---|
| `ha_sso_disabled` | La instalación no tiene login con HA configurado | Es el único que además se ve como **401 JSON**, y solo en `/start`. En el callback es un redirect, y se comprueba **después** del `state`. |
| `ha_state_mismatch` | Cookie ausente, ilegible, caducada, `state` ausente o distinto | Es la frontera de seguridad entera: sin él **no se llama al proveedor** (pin: el doble no registra ni una llamada). |
| `ha_exchange_failed` | HA no aceptó el código, o volvió con `?error=` (p.ej. `access_denied`), o el `code` falta / es desmesurado | Deliberadamente grueso: detallar más daría a un atacante una sonda sobre el HA de la víctima. |
| `ha_identity_failed` | El canje fue bien pero `auth/current_user` no dio identidad | |
| `sso_username_unavailable` | Se agotaron los seis candidatos de nombre | **Mismo código que emite `POST /v1/auth/sso`** para la misma situación: la persona se topa con lo mismo entre por donde entre, y una sola frase la explica. Cualquier otro error de `resolve_or_provision` (fallo de BD, carrera) sí sale como error de servidor — un redirect a la pantalla de login lo escondería. |

- **El 302 se construye a mano** (`http::StatusCode::FOUND` + `Location`), **no** con
  `axum::response::Redirect::to`, que emite **303 See Other**. 302 es lo que emiten los flujos OAuth
  y lo que este contrato fija; el 303 añade además la semántica «convierte el método en GET», que
  aquí sobra porque ya es GET. Las dos ramas (éxito y error) llevan `Cache-Control: no-store`: sin
  él un caché intermedio podría servir el redirect de un intento a otro — y el de éxito lleva un
  `Set-Cookie` de sesión.
- **Anti-open-redirect**: `sanitize_next` acepta por lista blanca de forma y cualquier duda cae a
  `/` — debe empezar por `/`, nunca `//`, ningún `\` en ninguna posición (varios navegadores lo
  tratan como `/`), ningún carácter de control (un `\r\n` partiría el `Location`), fragmento
  descartado, ≤512 chars. **`://` y `@` se prohíben solo en la parte de PATH** (antes del primer
  `?`): en la query se permiten a propósito, porque
  `/oauth/authorize?client_id=https://claude.ai&state=y` es un retorno legítimo de esta misma app y
  prohibirlos ahí rompería la pantalla de consentimiento OAuth — una query no puede cambiar el
  destino de un `Location` que ya empieza por `/`. El prefijo se aplica **una sola vez** e
  idempotentemente (la cookie guarda la forma canónica, sin prefijo).
- **Omisión deliberada del catálogo MCP** (registro fechado en `futurefin-mcp-parity` §3.1): es un
  mecanismo de redirect de navegador que termina en una cookie, no en un token; una tool MCP no
  puede conducirlo.
- Regresión: `apps/api/tests/ha_idp_login.rs` (18 tests) + los 11 unitarios de `ha_idp/mod.rs`.

### Installation
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/installation/session-context` | Returns `{installation_initialized, access}` — used for routing the UI gate |
| GET | `/v1/installation` | Own membership + installation snapshot |
| PATCH | `/v1/installation` | Owner only. Updates `base_currency`, `onboarding_completed`, tz, inflation, show_age_mode, fire_settings (incluye `savings_source: "budget" \| "transactions_avg" \| "budget_income_real_expense"` — fuente del ahorro de la simulación; no existe `target_age` — eliminado en v1.0.6) y `mcp_write_enabled` (bool, kill-switch vivo de las tools de escritura MCP — issue #3). **Dos de esos campos NUNCA son alcanzables por MCP** y es una decisión escrita, no un olvido: `mcp_write_enabled` (autorreferencia del kill-switch — uno reencendible desde la superficie que corta es decorativo) y `onboarding_completed` (**estado de la UI**: marca que la SPA ya no debe enseñar el asistente de alta; que un agente lo ponga a `true` no cambia un dato del hogar, solo le quita a una persona una pantalla que quizá no había visto). Los tres ejes de **presentación** —`base_currency`, `calendar_tz`, `show_age_mode`— sí lo son desde 4.4.0, vía `update_installation_settings` (allowlist estricta, §MCP) |
| POST | `/v1/installation/setup` | Creates singleton installation (409 if exists) |

### Pending users (`/v1/installation/pending-users/`)
Owner-only management of users awaiting approval.

### Members (`/v1/installation/members`) — handler `members.rs`, **4.0.0**
Gestión de las membresías **ya concedidas**. Hasta 4.0.0 `installation_memberships` solo recibía
`INSERT` (bootstrap del primer usuario, `setup`, aprobación de un pendiente): no había forma de
degradar ni de expulsar a nadie desde la aplicación. El mecanismo de corte sí existía —rol y
pertenencia se re-resuelven en cada request—, pero no la palanca: aprobar al usuario equivocado
concedía acceso permanente a todas las finanzas del hogar y el único remedio era un `DELETE` a mano
por `psql`. `SECURITY.md` y [`auth-and-membership.md`](auth-and-membership.md) prometían lo
contrario.

| Method | Path | Auth | Notes |
|--------|------|------|-------|
| GET | `/v1/installation/members` | **cualquier miembro** (viewer incluido) | `[{user_id, username, role, joined_at}]`, orden `owner → member → viewer` y después por `username`. Lectura abierta a propósito: todos los miembros comparten los mismos datos financieros, así que saber quién más tiene acceso no revela nada nuevo — y es justo lo que permite auditar el hogar. |
| PATCH | `/v1/installation/members/{user_id}` | **owner-only** (`user_is_installation_owner`) | Body `{role: "owner" \| "member" \| "viewer"}` → **204**. `owner` es asignable a propósito: un hogar debe poder traspasar la propiedad sin pasar por `psql`. No miembro → 404. |
| DELETE | `/v1/installation/members/{user_id}` | **owner-only** | Revoca el acceso → **204**. No miembro → 404. |

- **Guardia `last_owner`** (PATCH y DELETE): degradar o expulsar al último `owner` devuelve **400**
  con el prefijo `last_owner:`. El recuento (`owners_left_without`) va **dentro de la transacción y
  con `FOR UPDATE`** sobre las filas de la instalación — sin eso, dos owners degradándose a la vez
  dejarían el hogar sin ninguno.
- **El DELETE conserva los datos de la persona.** Sus movimientos, snapshots, activos y reglas
  siguen ligados a su `owner_user_id`; si se la vuelve a aprobar los recupera intactos. Lo que se
  corta es el **acceso**, y se corta entero y en la misma transacción: fila de
  `installation_memberships`, `sessions` del usuario, `api_tokens` (`revoked_at = now()`) y
  `oauth_grants` (`revoked_reason = 'membership_revoked'`). Sin ese corte de las cuatro credenciales
  la persona conservaría acceso durante días — la sesión dura `SESSION_TTL_DAYS` y un `ffp_` puede
  no caducar nunca.
- Post-commit, `state.invalidate_projection_by_user(target)`: sus entradas de la cache de proyección
  llevan SU demografía.
- Regresión: `apps/api/tests/account_and_members.rs`.
- **Sin UI todavía**: la SPA no expone estos endpoints (`Ajustes → Usuarios` sigue siendo solo la
  aprobación de pendientes). Se usan por API/`curl`.

### API tokens (`/v1/api-tokens`) — handler `api_tokens.rs`
Credencial Bearer del servidor MCP (`/mcp`). Gestión autenticada por cookie de sesión; cualquier
miembro (viewer incluido) gestiona SOLO los suyos — el token hereda identidad y rol vivo, no puede
hacer nada que su dueño no pueda ya.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/api-tokens` | Lista propios (`token_prefix`, `scope`, nunca el secreto ni el hash), orden `created_at DESC`. Incluye revocados (auditoría). |
| POST | `/v1/api-tokens` | Body `{label (1..64), expires_in_days? (1..=3650), scope? ("read_write"\|"read_only", default "read_write")}` → 201 con `token` (secreto `ffp_` + 43 chars base64url) **una única vez**. Máx. 10 activos por usuario → 400 `token_limit_reached`. `scope` desconocido → 400 `token_scope_invalid` (validado a mano, no por serde, para dar el código estable en vez del 422 genérico). |
| DELETE | `/v1/api-tokens/{id}` | Soft-revoke (`revoked_at = now()`). Id ajeno o ya revocado → 404. |

- **Solo se persiste el SHA-256 hex** del secreto (`token_hash` UNIQUE); lookup O(1), sin
  comparación de secretos en Rust.
- `require_api_token(pool, authorization)` (mismo archivo) valida `Bearer ffp_…` → 401 para todo
  fallo (ausente/malformado/revocado/expirado, sin distinguir). Actualiza `last_used_at` con
  throttle de 60 s (telemetría de auth, análoga a `sessions` — no viola reads-never-mutate).
- **`scope` (Fase 3, issue #84)**: `read_write` (default, preserva byte a byte los tokens
  emitidos antes de que la columna existiera) o `read_only`. Se lee vivo en el mismo SELECT que
  autentica y solo RESTA — nunca concede nada que el rol vivo de la persona no conceda ya. Es la
  segunda de las tres puertas de `require_mcp_write` (rol → **scope** → toggle de la instalación);
  ver §MCP y [`auth-and-membership.md`](auth-and-membership.md). El selector vive en Ajustes →
  Integraciones, junto a la columna «Permisos» del listado.

### Categories (`/v1/categories/`)
Scopes: `asset`, `liability`, `income`, `expense`. Per-installation.

### Assets (`/v1/assets/`)
Accepts `?view=mine` to filter by `owner_user_id`. The asset record no longer carries contribution fields — those live in `/v1/allocation-rules/`.

**Siembra del sumidero (4.11.0, #150)**: el `POST` (y la tool `create_asset`) del **PRIMER activo de un scope virgen** — cero activos Y cero `allocation_rules` del owner; las DOS condiciones, para no retro-sembrar en instalaciones antiguas — crea además la regla `remainder` sin tope apuntando al activo recién creado, vía la MISMA `create_allocation_rule_core` que valida la invariante del sumidero (cero SQL nuevo, cero invariante duplicada). La respuesta lo **declara** en `seeded_allocation_rule_id` (`skip_serializing_if`; solo viaja en ese create). Límite conocido: la secuencia activo→regla **no es atómica** (el módulo de reglas tiene un único punto de commit custodiado por test estructural) — si la regla falla queda el estado pre-#150, que `unallocated_savings_reason: "no_sink"` de la resolución delata (4.12.1 — sustituye a `surplus_destination`, retirado). Instalaciones existentes sin sumidero: en 4.11.0 fue «sin retro-siembra» + aviso; **desde 4.12.0 el owner REVIRTIÓ esa decisión** (2026-08-31) y la migración `20260901150000_allocation_rules_retro_seed_sink` siembra retroactivamente todo scope con activos y sin sumidero (destino: el LÍQUIDO de menor rentabilidad esperada, empate al de mayor saldo — sin `created_at` no hay «primer activo» recuperable), con la MISMA regla al importar un backup pre-siembra (import.rs, cross-referenciado). El aviso de la SPA queda para los estados residuales (sumidero deshabilitado, o borrado del activo del sumidero — #176). Test-cabecera: `smoke.rs::a_fresh_installation_can_create_an_asset_right_away`; criterios ejecutables en `backup_user_roundtrip.rs::importing_a_pre_seed_backup_seeds_the_sink_with_the_owner_criteria`.

**Sumidero INDESTRUCTIBLE (4.12.1, #176)**: con el scope teniendo activos vivos, el sumidero ya no
se puede perder. `DELETE /v1/assets/{id}` sobre el activo al que apunta el sumidero devuelve 400
`remainder_required` si quedan OTROS activos en el scope — borrarlo se llevaría la regla en
cascada (`target_asset_id` es `ON DELETE CASCADE`) y el sobrante se quedaría sin destino, que desde
4.12.1 ya no tiene caja donde caer. El ÚLTIMO activo del scope SÍ se puede borrar (sin activos no
hay cascada que proteger — la misma condición de la siembra). Guarda:
`assert_asset_delete_keeps_the_sink` (`handlers/allocation_rules.rs`, invocada desde
`handlers/assets.rs`); test: `allocation_sink_invariant.rs`. La misma indestructibilidad cubre
deshabilitar o degradar la última regla resto sin tope: el `PATCH` de `/v1/allocation-rules/{id}`
pasa la pre-guardia de `is_sink` a `is_sink && enabled` (un sumidero deshabilitado deja el sobrante
sin destino, igual que borrarlo). La migración
`20260901160000_allocation_rules_reenable_disabled_sinks.sql` reactivó retroactivamente los
sumideros que ya estaban apagados (con el mismo espejo en el import de backups).

**Plusvalía latente (4.4.0, Fase 6)** — `AssetResponse` gana cuatro campos, **todos sin `skip_serializing_if`: viajan siempre, con `null` explícito**: `unrealized_pnl` (Decimal-as-string), `unrealized_pnl_absent_reason`, `unrealized_pnl_pct` (Decimal-as-string, 1 decimal) y `unrealized_pnl_pct_absent_reason`. Base: la columna `assets.purchase_price` — **no** aportaciones, **no** snapshots, **no** coste medio ponderado. `pnl = current_value − purchase_price`, `pct = pnl/purchase_price × 100`. **Lo caro no es la resta, es la etiqueta**: no es rentabilidad (no anualiza ni descuenta las aportaciones posteriores a la compra, así que en un activo con reglas de reparto activas el número está inflado), y `purchase_price` es opcional, así que hay que distinguir tres estados — `NULL` ⇒ ambos `null` con `no_purchase_price`; `= 0` ⇒ el pnl es el valor entero y el porcentaje es `null` con `zero_purchase_price`; `> 0` ⇒ ambos. Presente en GET, POST y PATCH (los tres pasan por `row_to_response`); **no** aparece en `/v1/summary`. **Trampa conocida (issue #95)**: el `null` que el doc-comment del PATCH promete para **borrar** `purchase_price` es inalcanzable por HTTP —serde colapsa `null` presente y clave ausente en `None`, así que sale 400 `patch_empty`—; la vía viva es el flag `clear_purchase_price` de la tool MCP. Hay un test que fija ese 400 para que no se lea como descuido de esta feature.

**Objetivo resuelto**: `fetch_asset_resolved_targets` dejó su propio `match cap_kind` y llama a `allocation_rules::resolve_cap_ceiling_eur`, la misma función que el techo del ETA de `/v1/allocation-rules/goals` — el objetivo de la pantalla de activos y el de la ETA son el mismo número por construcción.

Each `AssetResponse` row carries `owner_user_id: Option<Uuid>` (`null`/absent = shared row). It is **display data only** (used by the frontend snapshot-prompt trigger to know which assets are "mine" in household view), never a security boundary — scoping still happens via `?view=mine`. Serialized as a uuid string, omitted when `None` (`skip_serializing_if`).

### Allocation rules (`/v1/allocation-rules/`)
Cascade rules that route the monthly surplus (`income − expense − debt_service`) into assets, in priority order. Accepts `?view=mine` to scope by `owner_user_id`.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/allocation-rules` | List ordered by `priority ASC`. |
| POST | `/v1/allocation-rules` | Body: `{target_asset_id, kind, amount?, cap_kind?, cap_value?, notes?, enabled?}`. Auto-assigns the next `priority` in the scope. |
| PATCH | `/v1/allocation-rules/{id}` | Updates fields. `amount` accepts a string, number, or `null` (only for `remainder`). `cap` accepts `{kind, value}` or `null`. Does **not** change `priority`. Rejects with 400 + `remainder_required` if it would orphan the scope. |
| DELETE | `/v1/allocation-rules/{id}` | Returns 400 `remainder_required` if deleting the last `remainder` rule in scope. |
| GET | `/v1/allocation-rules/goals` | **4.4.0 (Fase 6)** — ETA de cada tope de la cascada: para cada regla CON `cap`, su techo en euros y el **mes** en que el activo destino lo cruza. Acepta `?view=`. **Sin tabla `goals` a propósito**: el cap YA es el objetivo (`months_expense(N)` es literalmente un fondo de emergencia), y una tabla nueva duplicaría ese número — la lección de las contribuciones por activo (`futurefin-failure-archaeology` §3 fila 2). Corre bajo `heavy::run_projection_sim`: simula el horizonte completo y cruza el `per_asset_series` que devuelve el motor —**no** el `asset_series` de `GET /v1/projection/series`, que no toca— con el techo. Un tope inalcanzable dentro del horizonte se declara `not_within_horizon`, no un `null` mudo. Core `allocation_goals_core`; el techo lo resuelve `resolve_cap_ceiling_eur`, la MISMA función que `assets.rs::fetch_asset_resolved_targets`, y `allocation_goals.rs::goal_ceilings_match_the_engine_resolution` compara los tres tipos de cap contra el `cap_ceiling` del motor. **Por qué esa función existe fuera del motor**: en un mes sin sobrante el motor emite `cap_ceiling: null` para todas las reglas (issue #96), así que no hay una sola definición del techo — hay dos, y el test solo puede cruzarlas en el camino donde el motor sí lo publica. |
| POST | `/v1/allocation-rules/reorder` | Body: `{ids: [uuid,...]}`. Must list exactly the rules in the current scope; reassigns `priority` 1..N in the given order in one transaction. |

Rule kinds:
- `fixed` — €/mes; `amount` required, ≥ 0.
- `percent` — `amount` ∈ [0, 100], applied to the **surplus remaining at this step** (cascade pure).
- `remainder` — `amount` ignored (must be NULL). At least one per scope is enforced server-side.

Cap kinds (all optional; `cap_kind`/`cap_value` are paired):
- `amount` — absolute € target value for the destination asset.
- `months_expense` — N × (monthly expense + debt service); evaluated per-month against current state.
- `income_multiple` — N × monthly income.

**`GET /v1/allocation-rules/resolution` (3.8.0)** — la cascada **resuelta** para el mes en curso.
Endpoint nuevo y no un envelope sobre `GET /v1/allocation-rules`: convertir aquel array en objeto
habría roto el contrato. Construye su propio `ProjectionInput` con horizonte 1 (mismo coste que
`GET /v1/assets`, una tanda de SELECTs) y **no pasa por la cache de proyección**, coherente con
`assets_projection_context`.

Acepta `?view=mine` y **desde 4.4.0 lo ecoa en `view`** (raíz). Aquí el eco decide qué reglas y qué
activos entran en la cascada, así que dos resoluciones distintas pueden diferir **solo** en ese
campo.

Devuelve `base_cash` —lo que la cascada reparte de verdad— **desglosado** en `recurring_net`
(`income − expense − debt_service`, estable) y `planning_component` (el tramo del mes en curso de
los planning flows sin fecha — 90 días anclados al día 1 del mes desde 4.11.0/#126, así que es
**constante dentro del mes** y se agota en ~3 meses), con el flag `base_includes_transient`.
Hasta 4.11.0 (#150) publicaba además `surplus_destination` (`"asset"` ⟺ hay sumidero HABILITADO en
la vista; `"cash"` ⟺ el sobrante no absorbido se quedaba en caja al 0 % — la señal del aviso de la
SPA; un sumidero deshabilitado contaba como `"cash"`). **`surplus_destination` MURIÓ en 4.12.1
(#175, breaking §5)**: con `surplus_cash` retirado del motor ya no hay un destino «caja» que
declarar. Lo sustituye **`unallocated_savings_reason`** (`null` = no hay sobrante varado;
`"no_assets"` = el scope no tiene activos; `"no_sink"` = hay activos sin sumidero habilitado —
estado residual que la guarda de #176 hace inalcanzable por API con activos vivos). Ese
desglose sigue siendo el punto: un flag de «sobreasignación» a secas habría dicho «sí» y habría sido igual de
engañoso — la cascada **no** puede repartir de más (`take` se acota por intención, cap y caja, y el
bucle corta al agotarse), lo que pasaba es que la base incluye un término transitorio.

Por regla: `amount_intent` vs `amount_resolved` (si difieren sin `skipped_reason`, la regla fue
**recortada** por el cap — no saltada), `cap_ceiling`/`cap_room` y `skipped_reason` ∈ {`no_cash`,
`not_reached`, `cap_full`, `zero_amount`, `invalid_target`}. `no_cash` y
`not_reached` **no se colapsan**: «no te sobra dinero» y «las reglas de arriba se lo comieron»
tienen remedios distintos. **`in_retirement` MURIÓ en 4.12.1 (#175)**, junto con
`AllocationSkipReason::InRetirement`: hasta entonces, en jubilación HAY caja (se publica en
`base_cash`) pero el bucle la mandaba entera a `surplus_cash` sin ejecutar la cascada — antes la
resolución publicaba aportaciones que la simulación jamás hacía (H-cascada-1, auditoría 2026-08).
Desde 4.12.1 la cascada corre TAMBIÉN jubilado — la MISMA del usuario, sin rama especial —, así que
ese literal ya no tiene nada que reportar.
Las reglas posteriores al corte por caja se emiten con `not_reached` en vez de desaparecer del
informe. Cierra con `per_asset` y **`leftover_unallocated`** (4.12.1, breaking §5 — antes
`leftover_to_surplus_cash`: el sobrante sin destino ya NO entra en ningún balance, solo se
cuantifica), y la identidad `Σ per_asset + leftover_unallocated = base_cash` sigue pinneada en
`allocation_resolution.rs`.

**`debt_service` es nullable desde 4.3.1, y desde 4.8.0 es un número en los TRES modos** (#142,
opción 3 del owner). El contrato 4.3.1→4.7.x publicaba `null` +
`debt_service_absent_reason: "included_in_real_expense"` en B/C con base de gasto real (la cuota
vivía dentro del promedio y publicarla la contaba dos veces); con la opción 3 el gasto efectivo del
engine YA resta la cuota declarada del promedio, así que cobrarla como servicio de deuda es
contarla **una** vez y la cifra vuelve a ser medible siempre. `debt_service_absent_reason` es ahora
**siempre `null`** (el campo se conserva por forma; retirarlo sería un breaking §5 aparte) y un `0`
significa solo «no hay pasivos con cuota activa». La decide el único punto
`BuiltProjection::debt_service_absent_reason`, consumido por esta superficie y
`simulate_projection`. Pin (invertido en 4.8.0): `projection_number_semantics.rs`.

**Los tres campos de aportación de `/v1/assets` son cosas distintas** — la confusión entre ellos es
el defecto de contrato que abrió el auditoría MCP:

- `contribution_nominal_monthly`: aporte del **primer mes** resuelto por la cascada. No es un
  importe mensual estable pese al nombre — la cascada reparte `net_cash_month`, que incluye el tramo
  de los planning flows sin fecha del mes en curso (`importe/90` por día natural), así que el valor
  **decrece cada día** y **salta el día 1 de cada mes**. Un número «mensual» que cambia a diario es
  una trampa para cualquiera que haga aritmética con él, humano o modelo.
- `contribution_recurring_monthly` (**3.8.0**): la MISMA cascada evaluada sobre el neto
  **recurrente** (`income − expense − debt_service`, sin el tramo de planning). Estable y
  reproducible: es el número que una persona quiere decir cuando dice «mi aportación mensual», y el
  único con el que tiene sentido hacer cuentas. Se calcula con una segunda pasada del engine sobre
  el mismo input con `planning_monthly_cash_adjustment[0] = 0` — reutilizar la cascada en vez de
  aproximarla garantiza caps y precedencia idénticos, sin ningún SELECT extra.
- `contribution_target_amount`: **no es una aportación**, es el tope en euros del activo.

Los dos primeros se sirven redondeados a **4 decimales** (política monetaria de la casa; antes
salían los 28 dígitos de la división).

**Base de los caps en `/v1/assets` (v2.2.0)**: el `contribution_target_amount` que devuelven `GET/POST/PATCH /v1/assets` resuelve `months_expense` / `income_multiple` con los escalares **efectivos** del engine — o sea, en modo B/C con datos el gasto/income salen del promedio real 12m, no del presupuesto (antes se resolvían siempre con presupuesto y el objetivo no casaba ni con la aportación del mes 1 mostrada en la misma respuesta ni con la proyección). Un único `assets_projection_context` (`handlers/projection.rs`) devuelve `{nominals, income_monthly, expense_with_debt}` de **un solo** `build_installation_projection_input` por request; sustituye a `first_month_asset_contribution_nominals_map` + `monthly_income_expense_debt_for_view` (eliminados).

### Liabilities (`/v1/liabilities/`)
Accepts `?view=mine`. `principal_derived_from_plan` flag indicates auto-derived principal from planning flows.

**`expense_category_id` (3.4.0, API breaking interno en el create)**: categoría de GASTO donde vive la cuota — el presupuesto y la comparativa de Movimientos atribuyen ahí el equivalente mensual del plan. **Obligatoria en `POST /v1/liabilities`** (campo requerido, validado scope `expense` + instalación; también en la tool MCP `create_liability`); en PATCH es **set-only** (asignar/cambiar, nunca vaciar). Los pasivos anteriores a 3.4.0 conservan `NULL` («sin asignar»: se comportan como antes, sin atribución) hasta que el usuario la asigne; el import de `.ffbackup` viejos también deja `NULL`. FK `ON DELETE SET NULL` (no bloquea borrar la categoría; el `remap_to` de `/v1/categories` sí la arrastra cuando la categoría remapeada es de gasto).

**`GET /v1/liabilities/{id}/schedule` (4.4.0, Fase 6)** — calendario de amortización de UN pasivo **desde el saldo de hoy** (no desde el préstamo original), mes a mes y agregado por año civil. Query: `?view=`, `from_month_index` (≥1, def 1), `months` (1..480, def 12). **Los agregados salen del calendario COMPLETO, no de la ventana pedida** (`total_interest_remaining`, `payoff_month_index`…): pedir 12 meses no debe cambiar cuánto interés queda. Core `liability_schedule_core`, que envuelve `futurefin_engine::liability_amortization_schedule` — **cero matemática nueva**: publica el `closing_principal` que el motor ya derivaba hasta 840 veces por request y tiraba (`ProjectionOutput` nunca lo expuso), de ahí que «¿cuánto interés pago?» y «¿cuándo termino?» fueran incontestables. Tres cosas que hay que leer bien:
- **El interés es un RESIDUO**: `interest_accrued := payment − (opening − closing_tras_cuota)`. No se devenga aparte, así que `payment + extra == interest + principal_repaid` es exacto **por construcción** en los cuatro `repayment_model` (`schedule_payment_identity_holds_in_every_model`).
- **`principal_repaid` puede ser NEGATIVO** cuando la cuota no cubre el devengo — clamparlo a 0 escondería justo el caso que el modelo pre-4.2.0 no sabía representar. El que sí se clampa es el saldo (`closing_principal ≥ 0`).
- **Ausencia de payoff con motivo, no `null`**: `payoff_month_index` y `payoff_absent_reason` son mutuamente excluyentes (**ojo**: en el wire el campo es `payoff_absent_reason`; `payoff_absent` es el nombre del enum en el motor y no se serializa nunca), y las cuatro razones (`no_payment_plan`, `payment_plan_ends_before_payoff`, `payment_does_not_reduce_principal`, `not_within_horizon`) tienen remedios distintos.
Un pasivo vencido con SALDO VIVO devuelve desde 4.7.0 (#145) su calendario **congelado** (cero meses, interés 0, `payoff_absent_reason: no_payment_plan`); el vencido y saldado sigue siendo **404**. Con what-if de amortización, cada mes publica además `early_repayment_fee` (#151) — FUERA de la identidad cuota+extra = interés+amortizado — y los agregados `total_early_repayment_fee`/`total_cash_out` la incluyen. Regresión: `apps/api/tests/liability_schedule.rs` (7 tests; el caso ancla —100.000 € al 3 % con cuota 500 → extinción en el **mes 278**— es el MISMO número que el pin del engine `french_extinction_at_month_278`).

**Visibility predicate (4.7.0, #145 — sustituye al «expiration filter»)**: la fila se ve ⟺ plan vivo O saldo vivo — `WHERE (payment_end_date IS NULL OR payment_end_date >= $today OR principal > 0)`. El plan vencido con SALDO VIVO aparece en TODAS las lecturas (listado, summary, histórico, proyección — donde entra como resta constante congelada, sin devengo ni cuota), marcado **`plan_expired_with_balance: true`** en la respuesta; solo el vencido y saldado (`principal = 0`) se oculta. Las filas nunca se borran. EXCEPCIÓN deliberada: `/v1/budget` y la comparativa de Movimientos siguen filtrando por plan vivo — sus queries son de CUOTA, y un plan vencido no gira cuota. Use `installation.calendar_tz` to compute `today`.

**Modelo y campos 4.7.0 (#144)**: default `french` (columna + formulario; el body ausente en POST sigue siendo `fixed_payments`); `fixed_payments` = préstamo sin intereses y **rechaza** TIN (`apr_forbidden_for_model`); todos los demás exigen TIN > 0 y plan mensual; `revolving` exige y publica sus mínimos `min_payment_pct`/`min_payment_eur` (cuota real = `max(pct·saldo, suelo)`, la declarada solo alimenta el presupuesto). En PATCH, `apr_percent` es **tri-estado** (ausente conserva, `null` LIMPIA — patrón `purchase_price`; necesario para volver a `fixed_payments`), y al salir de `revolving` los mínimos se anulan solos. La derivación de principal es rama única: valor actual de las cuotas al TIN (sin TIN = Σ exacta), contada desde el **día ancla** (#123: `hoy + n meses`, el día 31 no se degrada tras febrero).

**`repayment_model` (4.2.0)**: cómo cobra el engine de proyección la cuota. Campo del wire en las
tres direcciones — **siempre presente** en `LiabilityResponse` (la columna es `NOT NULL DEFAULT
'fixed_payments'`, así que no hay pasivo sin modelo), **opcional** en `POST` (ausente ⇒
`fixed_payments`: un cliente que no sepa nada de 4.2.0 crea exactamente los mismos pasivos que
antes) y **set-only** en `PATCH` (`None` conserva el actual; no hay «volver a NULL», para deshacer
se manda `fixed_payments` explícito). Cuatro literales `snake_case`, idénticos a los del CHECK de
la columna: `fixed_payments` | `french` | `interest_only` | `revolving`. Un literal desconocido en
un body HTTP lo rechaza **serde** con un **422** (mismo comportamiento que `payment_frequency`);
por MCP, donde el parámetro llega como string suelto, es un **400 `repayment_model_invalid`**.
Semántica de cada modelo: [`engine.md`](engine.md) §ProjectionLiabilityInput.

Validación por modelo (`validate_repayment_model_state`, sobre el **estado resultante** — en PATCH,
tras mergear el body sobre la fila actual). `fixed_payments` no impone **nada** (el modelo histórico
sigue admitiendo pasivos sin plan, `weekly`, o con un TIN informativo que el engine ignora). Los
otros tres se comprueban en este orden fijo, para que el código de error sea predecible:

| # | Regla | Modelos | Código (400) |
|---|---|---|---|
| 1 | exige `payment_amount` + `payment_frequency` | `french`, `interest_only`, `revolving` | `payment_plan_required_for_model` |
| 2 | exige `apr_percent > 0` | `french`, `revolving` | `apr_required_for_model` |
| 3 | prohíbe `payment_frequency = weekly` | `french`, `interest_only`, `revolving` | `weekly_not_supported_for_model` |
| 4 | prohíbe `derive_principal_from_plan` | `interest_only`, `revolving` | `derive_not_supported_for_model` |

Por qué cada una: (1) sin cuota el engine no devenga (`liability_active` gatea también el interés),
así que un `french` sin plan sería un `fixed_payments` disfrazado que no mueve un número — se
rechaza en vez de mentir; (2) un TIN ausente o 0 hace degenerar el engine a `fixed_payments`, y
guardar eso sería ofrecer un «francés» que no cobra intereses (`interest_only` NO lo exige: su cuota
declarada YA es el interés); (3) la recurrencia del engine es **mensual** y `weekly` se convierte
×52/12 — exacto sin intereses, pero cambiaría el devengo; (4) derivar el principal solo tiene inversa
cerrada en `fixed_payments` (Σ cuotas) y `french` (valor actual).

**Cambio de comportamiento de POST/PATCH con `derive_principal_from_plan`**: la derivación deja de
ser una sola fórmula. En `fixed_payments` sigue siendo `cuota × nº de intervalos` **bit a bit** (no
pasa por el engine ni por `round_dp`); en `french` es el **valor actual** de esa renta al TIN
(`present_value_of_payments`), redondeado a 4 decimales en el handler. 200 cuotas de 500 € al 3 %
son 100.000 € de caja pero **78.618,1542 €** de deuda hoy. Además, en PATCH, cambiar
`repayment_model` o `apr_percent` con el derive activo **re-deriva** el principal con los valores
nuevos (el modelo se resuelve antes del bloque de derivación, a propósito).

### Summary (`/v1/summary/`)
Aggregated net worth, financial health metrics, category breakdowns. Accepts `?view=mine`. `total_liabilities` and breakdowns use the 4.7.0 visibility predicate (plan vivo o saldo vivo — see Liabilities note above); el `net_return` solo resta el TIN de lo que DEVENGA (#121).

**Campos de contexto (4.4.0, Fase 5, issue #86)** — ninguno cambia una cifra; todos declaran de dónde sale:
- **`view`** (raíz, `"household" | "mine"`) — eco de la vista aplicada (`LedgerView::as_str`). Reenviarlo como `?view=` reproduce la misma respuesta. Existe porque en una instalación de un solo usuario `?view=mine` y omitirlo devolvían payloads **byte a byte idénticos**.
- **`financial_health.basis`** (`"plan" | "actual" | "mixed"`) — función pura de los dos `savings_*_basis` (`financial_health_basis` en `summary.rs`): `plan` ⟺ los dos lados salieron del presupuesto, `actual` ⟺ los dos promediaron movimientos reales, `mixed` ⟺ uno de cada (lo normal en el modo C, y lo que pasa en el B cuando un lado se queda sin meses reales). **Regla de lectura**: si `basis != "plan"`, los cuatro equivalentes mensuales de aquí y sus homónimos de `GET /v1/budget` → `totals` (que son SIEMPRE el plan) **no son comparables uno a uno**. Es la misma familia de incidente que las tres cifras de ahorro de 3.9.0. **No se renombraron los cuatro homónimos**: los nombres son correctos en su contexto, renombrar era breaking sobre seis campos que lee la SPA, y no habría hecho la cifra más legible — seguirías sin saber en qué modo está el summary. Lo que faltaba era **declarar la base**.
- **`upcoming_flows_count`** (`i64`) y **`upcoming_last_due_date_ymd`** (`YYYY-MM-DD` u omitido) — los `upcoming_*_total` suman los `planning_flows` **PUNTUALES** (`amount_basis = one_off`) del scope, con y sin `due_date`, sin ventana temporal y sin anualizar: mezclaban un pago del mes que viene con uno de 2042 y las dos lecturas eran indistinguibles. El recuento (que cuenta TODO, recurrentes incluidos) hace que `0` signifique «no hay flujos» y no «se anulan»; la fecha máxima declara el horizonte que los totales no tienen. Salen del **mismo `GROUP BY`** que ya existía. Solo cuentan los scopes `income` y `expense`. **Desde 4.11.0 (#148)** los recurrentes van aparte — `upcoming_recurring_monthly_inflow` / `upcoming_recurring_monthly_outflow` (**€/MES**, sin mirar las ventanas: intensidad, no total) y `upcoming_recurring_count` — porque sumar el `expected_amount` de un `per_month` (€/mes) dentro de un total en € es un error de magnitud; `upcoming_coverage_ratio` conserva su base (solo puntuales). Regresión: `context_fields.rs::upcoming_totals_publish_the_horizon_they_are_summing`.
- **`liabilities_by_type_tag[].type_tag` pasa de `String` a `Option<String>`** (breaking en lectura): los pasivos sin etiquetar van con **`null`** en vez del literal español `"(sin etiqueta)"`, que era texto de interfaz dentro de un campo de datos —indistinguible de un usuario que hubiera etiquetado un pasivo con ese nombre, e imposible de reenviar como filtro—. Mismo criterio que `category_id` en `CategoryMonthlySeriesEntry`. La SPA **no consume** este desglose (solo `LiabilityApi.type_tag`), así que el impacto es de API/MCP. La dimensión se escribe con `type_tag` en `POST`/`PATCH /v1/liabilities` — desde 4.4.0 también desde MCP.
- **Unidades declaradas en el esquema, no en el nombre**: cada campo de `FinancialHealthMetrics` lleva la marca `**Unidad:**` en su doc-comment (y por tanto en OpenAPI y en la descripción de la tool). **No** se pusieron sufijos (`savings_rate_fraction`…): la unidad es propiedad del campo, constante en todas las respuestas, así que su sitio es el esquema y no 200 bytes repetidos en el endpoint más caliente de la app. Regla vigente: `_rate`/`_ratio` = **fracción** (`0.35` = 35 %); `_pct`/`_percent` = **porcentaje** (`3.5` = 3,5 %).

Contrato pinneado en `apps/api/tests/context_fields.rs` (11 tests, camino HTTP — los campos los pone la core, no la tool).

**`financial_health` sigue el toggle `fire_settings.savings_source`** (3 modos; gate `SavingsSource::uses_transactions()` = B o C). Con datos:
- **Modo B (`transactions_avg`)**: `income_monthly_equivalent`, `expense_regular_monthly_equivalent`, `net_monthly_equivalent` (= `income_avg − expense_avg`) y `savings_rate` salen del promedio real 12m **crudo** (reforma 3.4.0: las cuotas de pasivo ya viven dentro de los movimientos — sin resta híbrida ni debt service re-sumado), no del presupuesto.
- **Modo C (`budget_income_real_expense`)**: igual que B pero `income_monthly_equivalent` **conserva el income del presupuesto** (NO se sobreescribe); `expense_regular_monthly_equivalent = expense_avg` y `net_monthly_equivalent = income (presupuesto) − expense_avg`. El `match` sobre `savings_source` es exhaustivo (`Budget` es rama inalcanzable no-op, guardada por `uses_transactions()`).
- **Base de gasto total**: en modo A la cuota vive dentro de `expense_regular_monthly_equivalent` (fusión en el presupuesto, 3.7.0); en B/C, dentro del promedio real de gasto (reforma 3.4.0: los pasivos solo restan su principal en `net_worth` y en la proyección) y `expense_total_monthly_equivalent` = `expense_avg`. **3.9.0 RETIRÓ** `expense_derived_monthly_equivalent`, `monthly_net_excluding_derived_debt` y `savings_rate_excluding_derived_debt`: eran degenerados desde 3.7.0 (idénticos a sus hermanos) y solo servían para que el Resumen enseñara tres cifras de ahorro irreconciliables. Sigue valiendo, en los tres modos:
  - `net_monthly_equivalent = income_monthly_equivalent − expense_total_monthly_equivalent`
- **Fallback**: sin meses reales en B/C (`savings_*_basis.basis == "budget"`, `avg_months == 0`) → el bloque `financial_health` completo es **idéntico** al de modo A (runway incluido). El fallback se resuelve **por lado**: `savings_source` colapsa a `"budget"` ⟺ cayeron los DOS.

Campos de `financial_health` relacionados con el modo y el runway:
- `savings_source` (`"budget" | "transactions_avg" | "budget_income_real_expense"`) — modo **efectivo** tras el fallback (B o C con `months_with_data == 0` → devuelve `"budget"`).
- `savings_income_basis` / `savings_expense_basis` (`SavingsAvgBasis`) — de qué meses sale cada lado del promedio. **Sustituyen a `savings_source_months_with_data` desde 3.9.0**: con ventanas configurables por lado, un solo número ya no podía describir las dos. Campos: `basis` (`"budget" | "average"`), **`avg_months`**, `window_months`, `window_mode` (`"data" | "calendar"`, omitido si el lado no promedia), `first_month`, `last_month`, `has_gaps`.
- **API breaking 4.0.0 — `SavingsAvgBasis.months_with_data` → `avg_months`** (en `/v1/summary`, `/v1/projection/series` y la tool `simulate_projection`). El campo era **el denominador realmente usado**, mientras que en `GET /v1/transactions/summary` `months_with_data` es lo contrario: los meses que HAY en el tramo, y el denominador allí ya se llamaba `avg_months`. Mismo nombre con significados opuestos, y el mismo concepto con dos nombres: un consumidor que preguntara «¿sobre cuántos meses está calculada mi media?» citaba 9 (los que hay) cuando el motor promedió 6 (los reales), y con esa cifra justificaba un ahorro proyectado que no cuadraba. Ahora **`avg_months` = denominador en las dos familias** y `months_with_data` se queda **solo** donde significa «lo que hay» — es decir, en `/v1/transactions/summary`, donde **no cambia**.

**Las DOS cifras de ahorro de `financial_health`** — sobreviven a la limpieza de 3.9.0 y son distintas a propósito. Confundirlas desplaza una respuesta ~14 % (auditoría MCP §1):
- `net_monthly_equivalent` — el ahorro **real del modo activo**. Es el que usa el motor: cuadra con `monthly_delta_assumption` de `/v1/projection/series`, con `baseline.net_monthly` de `simulate_projection` y con `recurring_net` de `get_allocation_resolution`, y es el numerador de `savings_rate`.
- `savings_expected_monthly_equivalent` — el neto del **presupuesto**, siempre, sin seguir al modo: se captura en `summary.rs` **antes** del override B/C (el orden es load-bearing) y existe solo para el delta «real vs plan» que la SPA pinta como flecha de tendencia en la tarjeta de ahorro. En modo A coincide con el anterior por construcción — por eso la SPA suprime la flecha; en B y C difiere. Fijado por `summary_savings_source.rs::mode_b_expected_is_budget_net_not_override`.

3.9.0 **retiró** `savings_actual_monthly_avg_12m` y `savings_actual_months_with_data` (la comparativa que sobraba), y con ellos la llamada incondicional a `transactions_avg`: hoy está gateada por `source.uses_transactions()`, así que el modo A (default) **no toca el ledger** en el endpoint más caliente de la app. `/v1/summary` no tiene cache → sin contrato de invalidación; esto **no** convierte las transacciones en input del engine (D12a intacto — en modo A la proyección sigue ignorándolas).
- `runway_months` (Decimal-string, opcional) — meses que los activos **líquidos** cubren `expense_total_monthly_equivalent`, **drenándolos secuencialmente** en el mismo orden que la simulación (menor rentabilidad esperada primero, cada saldo restante componiendo la suya — 4.8.0, #128; hasta 4.7.x era una media ponderada por valor, sistemáticamente más corta en carteras mixtas) y con la inflación del gasto (`installation.annual_inflation_assumption_percent`, rango [−2, 50] desde 4.9.0/#146 — con deflación el gasto DECRECE y el runway se alarga). Lo calcula `futurefin_engine::liquid_runway_months` (ver [`engine.md`](engine.md) §Runway). Desde 4.10.0 el bucle vende **BRUTO** (gemelo de #140, misma escala de tramos que el objetivo), y desde 4.12.0 (#178) la `g` de cada líquido CON coste declarado se DERIVA de su base viva dentro del bucle (los sin coste usan el escalar; el UMBRAL SWR sigue con el escalar — perpetuidad), así que **no** es `liquid_assets_total / expense_total` ni siquiera sin rentabilidad — la división exacta solo sobrevive con impuestos apagados (o g=0 en todos los tramos). Como sigue `expense_total`, en B/C se calcula sobre la base de gasto real. Se **omite** del JSON (`skip_serializing_if`) cuando es `None`: sin base de gasto (`expense_total == 0`) o runway indefinido. El valor `1200` (`MAX_RUNWAY_MONTHS`) **no** es un centinela de infinito sino un **suelo**: significa «al menos 100 años» (el bucle agotó el tope sin cumplir el umbral SWR) y la UI lo pinta «+100 años».
- **Precisión de salida (3.8.0)** — los ratios se sirven **redondeados** (`round_ratio`, en la core;
  nunca en la capa MCP, que devuelve la struct intacta). `savings_rate`,
  `savings_rate_excluding_derived_debt`, `upcoming_coverage_ratio` y `debt_to_assets_ratio` a **6
  decimales de fracción** (= 4 decimales de porcentaje, muy por encima del único decimal que pinta
  la UI); `runway_months` a **1 decimal**, alineado con `simulate_projection`, que ya redondeaba
  así. Antes salían los hasta 28 dígitos que produce cada división de `rust_decimal`
  (`"0.2435991666666666666666666667"`). Es un cambio de **presentación**: el gross-up, el umbral SWR
  y el propio runway se calculan con la precisión completa y solo el resultado publicado se recorta,
  así que ninguna cifra derivada se mueve. Los importes monetarios (4 decimales) no cambian.
- **Precisión de salida, segunda mitad (4.0.0)** — 3.8.0 arregló los ratios y **dejó fuera los importes de proyección y FIRE**, que seguían saturando la escala: `simulate_projection.final_net_worth` salía con 21 decimales y `fire_target_base` / `get_projection.jubilacion_target_net_worth` con 22 (auditoría MCP §7). El origen: `annual_factor.powd(1/12)` es una raíz duodécima irracional y `gross / (swr/100)` una división, y ninguna se redondeaba. **La escala de los importes es 4** (`money_out`, **punto único en `apps/api/src/money.rs` desde 4.0.0** — antes había dos copias, en `projection.rs` y en `transactions/summary.rs`, y solo la segunda mataba el cero negativo: la duplicación ERA el bug, porque el arreglo se aplicó a una copia y no a la otra. La consumen `summary.rs`, `projection.rs` y `transactions/summary.rs`), aplicada a la **copia que se serializa, nunca a la que entra al motor** — `fire_target_base` ES `FireTarget.base_amount`, y redondear esa movería el cruce FIRE. Con ella se van dos rarezas más del serializador: el `-0` de las categorías sin movimientos (`impl Neg for Decimal` voltea el bit de signo también sobre el cero) y los hitos con formato mixto (`"25000.0"` junto a `"50000"`: `2.5 × 10⁴` heredaba la escala del literal). 4.0.0 la añade además donde faltaba: `/v1/summary`, `savings_expected_monthly_equivalent` y `monthly_delta_assumption`, que nacen de divisiones (`cuota × 52 / 12`, `suma / meses reales`) y viajaban con ~25 decimales. Excepciones **deliberadas** a la escala 4: ratios a 6, `runway_months` a 1, y los KPI del cash-flow a 2 (`money()` en `history.rs`, con su propio motivo escrito). Regresión estructural — barre TODO string decimal del payload, no una lista de campos: `mcp_simulate.rs::no_money_string_carries_more_than_four_decimals_or_negative_zero`.
- `runway_is_indefinite` (`bool`) — desde **v2.3.0** lo decide el **umbral SWR**, no sobrevivir el cap, y desde **4.8.0 (#128)** exige además la **puerta de rentabilidad**: `true` ⟺ la retirada anual bruta no supera el SWR sobre el saldo líquido — `gross_up(expense_total × 12) × 100 ≤ liquid_assets_total × swr_pct`, con `swr_pct`/`tax_brackets`/`taxes_enabled` de `installation.fire_settings` (pestaña Jubilación) y el **mismo** `gross_up_net_annual_fire` del target FIRE — **y** la rentabilidad esperada ponderada de los líquidos es > 0 (la regla del SWR se validó para carteras invertidas; el dinero parado al 0 % siempre se agota y ahora se dice en meses: 300.000 € al 0 % con 875 €/mes cumplen el umbral por igualdad y publican 342,9). Entonces `runway_months` no viaja. Con `swr_pct ≤ 0` nunca es `true`. Con `expense_total == 0` es `false` (no hay base de gasto, no es que esté cubierto). La inflación sigue sin mirar el disparador (gobierna solo el caso finito). La UI muestra «Infinito (dentro del SWR 3,5 %)» en el primer caso y oculta la tarjeta en el segundo. **API no breaking**: tipo y nullabilidad de ambos campos son los de v2.2.0.

- `net_return_nominal_annual_pct` / `net_return_real_annual_pct` (Decimal-string, opcionales) — **rendimiento anual esperado del patrimonio neto**, en **porcentaje** (no en fracción como `savings_rate`: `"3.5556"` = 3,5556 %/año), a **4 decimales** (`PCT_DP`, la misma resolución que `RATIO_DP` expresada en otra unidad). Numerador: `Σ (current_value × expected_annual_return_percent)/100` de **TODOS** los activos del scope − `Σ (principal × apr_percent)/100` de los pasivos que **devengan** (desde 4.7.0/#121: `liability_interest_accrues`, el MISMO predicado del engine — modelo con intereses + TIN > 0 + plan de pago vivo); denominador: `net_worth`, con TODAS las filas visibles (plan vivo o saldo vivo, #145) — el pasivo que no devenga pesa en el denominador a coste 0 (el caller pasa su `apr` como `None`). Una rentabilidad sin configurar (`NULL`) cuenta **0 %** y la fila **sigue pesando en el denominador** — se diluye, no se excluye. Lo calcula `futurefin_engine::net_return_percentages` (ver [`engine.md`](engine.md) §Rendimiento neto); el redondeo es de publicación, el engine devuelve el valor exacto. El **real** se obtiene **dividiendo factores** —`100·((1+n/100)/(1+i/100) − 1)` con `installation.annual_inflation_assumption_percent`—, nunca restando puntos. Ambos se **omiten** (`skip_serializing_if`) ⟺ `net_worth ≤ 0`: con denominador no positivo el cociente cambia de signo y se leería al revés. **No sigue `savings_source`** (no depende de ingreso ni de gasto). Desde 4.8.0 (#142) los modos B/C **ya no anulan el TIN en el engine** — la deuda amortiza igual en los tres modos y este KPI comparte base con la curva en todos ellos (la anulación de 3.4.0 y su divergencia residual quedaron revertidas). Regresión: `summary_net_return.rs`.

### Budget (`/v1/budget/`)
Partidas de ingreso/gasto en **una sola lista** (`entries`). Acepta `?view=mine`, y **desde 4.4.0 lo ecoa en `view`** (raíz) igual que `/v1/summary`.

**`totals.basis` es SIEMPRE `"plan"`** (constante `BUDGET_TOTALS_BASIS`, no una cadena suelta: el mismo literal lo consumen el test de contrato y la descripción de la tool MCP). Declara que estos totales son el presupuesto —lo que el usuario dijo que ingresaría y gastaría— y **nunca** lo medido, en ningún modo de `savings_source`. Cuatro de ellos (`income_monthly_equivalent`, `expense_regular_monthly_equivalent`, `expense_total_monthly_equivalent`, `net_monthly_equivalent`) se llaman **exactamente igual** que cuatro de `GET /v1/summary` → `financial_health`, que sí siguen el modo. Ver la regla de lectura en la sección Summary.

**Fusión de las cuotas de pasivo (3.7.0, API breaking).** Hasta la 3.6.0 las cuotas vivían en un array aparte (`derived_from_liabilities`) que se sumaba por debajo del presupuesto en `totals.expense_derived_monthly_equivalent`. Ahora son **una partida más de `entries`**:

- `source`: `"manual"` (fila de `budget_entries`, editable) | `"liability"` (cuota derivada del plan de pago, **solo lectura**). `PATCH`/`DELETE /v1/budget/entries/{id}` sobre el id de una cuota derivada → **422 `budget_entry_is_liability_derived`** (Fase 3, issue #84; antes 404). El id que se lee en `GET /v1/budget` para una cuota **es el del pasivo** (`source = "liability"`), así que un cliente que lo copia de ahí y lo pasa al PATCH/DELETE de una partida estaba recibiendo «no existe» sobre un recurso que el propio servidor le acababa de enseñar — el error decía lo contrario de la verdad. El 422 nombra el destino correcto (`update_liability` / `delete_liability`, `PATCH`/`DELETE /v1/liabilities/{id}`); un id que de verdad no existe en ninguna tabla sigue siendo 404 (`missing_budget_entry_error`, `handlers/budget.rs`, un SELECT extra solo en el camino de error).
- En una cuota: `id` = id del pasivo (los UUID no colisionan entre tablas) y `liability_id` lo repite; `label` = etiqueta del pasivo; `category_id` = su **`expense_category_id`** (la misma categoría de GASTO con la que la comparativa de Movimientos empareja los recibos reales); `amount` = **equivalente mensual** del plan (`weekly` → ×52/12; el importe y la frecuencia crudos siguen en `/v1/liabilities`); `expense_end_date` = fin del plan (`null` = indefinido).
- `category_id` es **opcional** (`skip_serializing_if`): se omite solo en cuotas de pasivos sin `expense_category_id` (anteriores a 3.4.0, y los importados de `.ffbackup` viejos). Esas cuotas **siguen sumando** en los totales — descartarlas bajaría el gasto presupuestado en silencio.
- Totales: `expense_regular_monthly_equivalent` incluye las cuotas y es exactamente la suma de los `entries` de scope `expense`; `expense_total_monthly_equivalent` vale lo mismo (se mantiene por compatibilidad). **`expense_derived_monthly_equivalent` y `derived_from_liabilities` ya no existen.**
- `expense_retirement_monthly_equivalent` cuenta **solo partidas manuales**: una cuota termina con su plan, así que no es gasto post-jubilación. Es el campo que consume la previa FIRE (incidente v1.3.0, divergencia 2–3×).

**Cuota «activa»**: pasivo con plan de pago (`payment_amount` + `payment_frequency`) y `payment_end_date IS NULL OR payment_end_date >= today` — mismo predicado que `/v1/liabilities` y `/v1/summary` (unificado en 3.4.0; antes exigía fecha fin NOT NULL y `>` estricto, y un pasivo sin fecha fin no derivaba línea).

> **La base de gasto del engine NO cambia.** `ledger_regular_monthly_income_and_expense` sigue devolviendo solo lo persistido: el engine cobra la cuota por su lado (`ProjectionLiabilityInput::monthly_payment`, con amortización y fecha fin), así que fundirla también ahí la contaría dos veces en el modo A. `monthly_delta_assumption` de `/v1/projection/series` sigue siendo `income − gasto persistido`. Clavado por `budget_liability_quotas.rs::liability_quota_stays_out_of_the_engine_expense_base`.

### Planning (`/v1/planning/`)
Upcoming cash flows: puntuales (`amount_basis = one_off`, importe TOTAL con `due_date` opcional) y, desde 4.11.0 (#148), **recurrentes con ventana** (`amount_basis = per_month`: `expected_amount` son **€/MES** durante `[window_start_date, window_end_date]`, fin NULL = sin fin, sin `due_date`). `amount_basis` viaja SIEMPRE en la respuesta. El vector mensual de la proyección carga un `per_month` **mes civil completo** en cada mes que su ventana toca (sin prorrateo de meses frontera — coherente con presupuesto y servicio de deuda; prorratear reintroduciría la dependencia del día que #126 retiró). Un `per_month` no produce `events[]` (es una rampa, no un escalón) ni admite `show_in_chart`. **El objetivo FIRE NO ve los Próximos** (ni puntuales ni recurrentes — alimentan la caja de la proyección, no el target; decisión del owner en #148, declarada también en financial-contracts §2.4). El PATCH valida el estado RESULTANTE (cambiar de base exige dejar coherentes fecha y ventana en el mismo request; tri-estado también en las dos fechas de ventana); códigos nuevos: `amount_basis_invalid`, `window_requires_per_month`, `per_month_excludes_due_date`, `window_start_required`, `window_end_before_start`, `window_date_type`, `window_date_format`, `window_date_out_of_range` (cota de 100 años hermana de `due_date_out_of_range`).

**Cota superior de `due_date` (issue #82)** — `due_date_out_of_range` (400) si la fecha cae más de
**100 años** por delante del hoy civil de la instalación. Existe porque `"9999-12-31"` se aceptaba:
se validaba el FORMATO de la fecha, nunca su rango, y ese flujo entraba tal cual en
`upcoming_inflows_total` / `upcoming_outflows_total` / `upcoming_coverage_ratio` de `GET /v1/summary`
— una cifra de portada movida por un evento a ocho mil años vista, sin aviso. La cota es
deliberadamente generosa: el horizonte tope del motor son 1.200 meses, así que nada por encima puede
afectar a ninguna serie, solo a los agregados. En el PATCH **solo se valida lo que el patch
introduce**, para que una fila antigua fuera de cota se pueda seguir editando (y arreglando) en vez
de quedar intocable. Misma cota, mismo criterio y código hermano `payment_end_date_out_of_range` en
`/v1/liabilities`; el bound compartido vive en `handlers::max_user_settable_future_date`, pero **los
códigos se emiten como literales completos en cada sitio** porque `error_codes_parity` los extrae del
fuente y uno compuesto con `format!` sería invisible para el catálogo.

### Projection (`/v1/projection/`)
Net-worth series via `futurefin-engine`. Accepts `?view=mine` and `?months=N`. `N` fuera de
**12–840** es **400 `months_out_of_range`** (desde 4.4.0; antes se clampaba en silencio y la
respuesta afirmaba `horizon_basis: "months_override"` como si hubiera hecho caso al valor pedido).

Response (`ProjectionSeriesResponse`) includes:
- `view` (4.4.0) — eco de la vista aplicada, `"household" | "mine"`. Aquí importa además porque el horizonte y la demografía (`viewer_birth_date`, `jubilacion_age`) son SIEMPRE del solicitante, también en `household`: sin este campo, dos respuestas con el mismo horizonte y distinto scope de patrimonio se leen igual.
- `points[]` — `{month_index, net_worth, contributed_capital}` for months 0..=N. **`net_worth` y `contributed_capital` se serializan como `f64`** (no Decimal-as-string) por rendimiento: ~30 KB menos en JSON y evita ~5.000 `parseDisplayDecimal` cliente. Precisión <1 € en horizontes de 70 años.
- `months`, `horizon_years`, `horizon_basis` — effective horizon (`lifespan_age` — hasta 4.8.0 `lifespan_90` — | `fallback_no_demographics` | `months_override`); al lado viaja **`horizon_lifespan_age`** (la edad límite configurada, `fire_settings.horizon_lifespan_age`, 85..=105 default 90 — 4.9.0/#149) y **`final_net_worth_real`** (Decimal-string: patrimonio del último mes en euros de HOY, paridad con `simulate_projection` — el «margen al final» nominal es `points[último].net_worth`, sin campo nuevo; «no llegó» ⟺ `assets_depleted_month_index != null` o `uncovered_deficit_total > 0`)
- `starting_net_worth`, `monthly_delta_assumption` — snapshot values at month 0 (Decimal-as-string para totales)
- `drawdown_gain_basis` + `taxable_gain_ratio_today` (4.12.0, #178) — qué rigió la fiscalidad del DRENAJE (`cost_basis` | `declared_ratio` | `mixed`, según qué activos declaran `purchase_price`) y la `g₀` informativa de la cartera de hoy (`Σ max(0, v−coste)/Σ v` SOLO sobre los declarados; `null` sin ninguno). El objetivo y el umbral de Autonomía usan siempre el escalar — reparto de regímenes en financial-contracts §2.4. Dos-casos pineados en `context_fields.rs::projection_declares_what_governs_the_drawdown_taxation`.
- `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date` — UI axis helpers
- `jubilacion_month_index`, `jubilacion_date_ymd`, `jubilacion_age` — el cruce con el target FIRE en las tres lecturas. **Desde 4.8.0 el cruce compara la riqueza LÍQUIDA** (`points[].net_worth_liquid`: Σ activos vendibles, BRUTA — #143; `surplus_cash` retirado del término en 4.12.1/#175 — teorema: el cruce solo pudo irse MÁS TARDE con el cambio, nunca adelantarse, y en producción es invariante) contra un objetivo que, desde 4.10.0 (#170), se evalúa **MES A MES sobre la necesidad real** — `gross_up(need(k), tramos, g)/SWR + término_deuda(k)`, con la pensión plana restada DESPUÉS de inflar y el fiscal drag capturado (los tramos son nominales); `jubilacion_target_net_worth` sigue siendo la base a k=0 (inmóvil) y el objetivo **no es monótono por partida doble** (término de deuda decreciente + base súper-inflada con pensión): cruce por escaneo lineal. La jubilación es un **estado absorbente** (#141). La **fecha civil** (issue #6) es `anchor_date_ymd + N` meses **conservando el día del ancla** con recorte a fin de mes — exactamente `addMonthsCivil` (`apps/web/src/lib/dates.ts`), de modo que `jubilacion_age` (años cumplidos) coincide con la etiqueta «N a» del chart. Anclar al día 1 restaría un año cuando el cruce cae en el mes de cumpleaños. Los tres viajan como **`null` explícito** cuando no hay cruce (4.0.0: con `skip_serializing_if` no se distinguía «no se alcanza» de «esta versión no publica el campo»); `jubilacion_age` es además `null` sin fecha de nacimiento resuelta (independiente de `show_age_mode`). Pin: `jubilacion_civil_tests` en `handlers/projection.rs`.
- `jubilacion_series_position`, `jubilacion_target_net_worth_nominal` (issue #82) — **`jubilacion_month_index` no indexa ninguna serie devuelta**, y durante versiones se documentó como si lo hiciera. Es un número de MES; `points` es de densidad híbrida (mensual 0..12, anual después) y `fire_target_series` es paralela **por posición** a `points`, así que con `density=hybrid` —la que fuerza la tool MCP `get_projection`— hay ~42 posiciones para 361 meses: indexar con el mes se salía del array, y caer en `[0]` presentaba el objetivo de HOY como el de dentro de décadas. Los dos campos nuevos cierran el agujero y son `null` ⟺ no hay cruce, como sus hermanos.
  - `jubilacion_series_position`: la posición (base 0) dentro de `points` / `fire_target_series` / `asset_series[].values`. **Convención: el punto servido inmediatamente ANTERIOR o igual** — la última `p` con `points[p].month_index <= jubilacion_month_index`, así que el cruce cae en el segmento `[p, p+1)` (donde un chart pinta el marcador) y `points[p].month_index == jubilacion_month_index` ⟺ el mes es un punto servido (siempre con `density=monthly`). Se eligió «anterior» y no «siguiente» porque su error es **conservador**: infravalora el patrimonio del cruce en vez de inflarlo.
  - `jubilacion_target_net_worth_nominal`: el objetivo FIRE **del mes del cruce, en euros nominales** — el número que hasta 4.3.1 era inobtenible desde la respuesta (solo viajaba la base en euros de hoy, y con inflación las dos difieren por factores de 1,5× y más). Se evalúa **exacto** con `fire_target_at_month_index(ft, jubilacion_month_index)`, el mismo helper del motor: no se interpola entre puntos ni se lee de `fire_target_series`. Pin: `projection_number_semantics.rs`.
- `milestones[]` — next 3 net-worth milestones (1/2.5/5×10ⁿ thresholds), each with `target`, `reached_month_index`, `reached_date_ymd`. **Ojo**: `reached_date_ymd` ancla al **día 1** del mes (contrato ya publicado, se deja como está), a diferencia de `jubilacion_date_ymd`. Ambas coinciden siempre en año y mes; solo difieren en el día.
- `compound_outpaces_true_savings_month_index` — primer **MES** (misma base que `points[].month_index`, **no** una posición de array) en que el rendimiento del patrimonio supera el ahorro mensual base — sin Próximos ni plan de amortización, que son puntuales o decrecientes y harían depender el cruce de un pago suelto. `null` = no cruza dentro del horizonte, **no** «no calculado». No tiene `*_series_position` porque la cifra no se lee de la serie.
- `events[]` + `events_truncated` (4.4.0, Fase 5) — los **saltos** de la curva: un elemento `{month_index, date_ymd, title, amount, direction, overdue}` por cada Próximo **con `due_date`** — el VENCIDO incluido (4.11.0, #126): carga íntegro en el mes 0 con `overdue: true` y su `date_ymd` REAL (pasada), así que el mes señalado y la fecha mostrada dejan de coincidir a propósito y el flag es lo que lo declara —, `amount` como magnitud ≥ 0 y el signo en `direction` (`inflow` scope income | `outflow` scope expense), orden mes ASC + importe DESC, tope `PROJECTION_EVENTS_MAX = 100` (`events_truncated` marca el recorte, que se lleva los meses más lejanos). **Sin query nueva**: sale de los `planning_rows` ya cargados y **comparte la regla de mapeo fecha→mes** con `planning_monthly_cash_adjustments_from_flows`, así que el mes que señala es exactamente aquel en el que la curva salta. **No entran** los Próximos sin fecha (se reparten sobre 90 días naturales desde el día 1 del mes ancla — #126: producen una rampa idéntica todos los días del mes, no un escalón), ni los pasivos, ni las partidas de presupuesto con fecha de fin (cambian la PENDIENTE, no producen un escalón). Existe porque con `density=hybrid` —la que sirve la tool MCP— entre dos puntos consecutivos caben doce meses y una caída de decenas de miles de euros no tenía en la respuesta **nada** que la explicara. **Se descartó exponer `density` como parámetro de la tool**: `monthly` multiplica el payload por ~5 (841 puntos) y sigue sin decir POR QUÉ cayó, solo dónde; un evento son ~90 bytes y contesta la pregunta entera.
- `fire_target_series: f64[]`, `asset_series[].values: f64[]` — arrays grandes paralelos a `points` (también `f64`).
- `points[].net_worth_liquid` (4.8.0, #143) — la riqueza **líquida** de cada punto (Σ activos `is_liquid`, BRUTA, sin restar principal — `surplus_cash` retirado del término en 4.12.1/#175), escalar por punto serializado como `f64` como sus vecinos. Es **la serie contra la que se decide el cruce FIRE** — `net_worth` (total) se sigue publicando y pintando, pero cruzar con él contaba la vivienda como si pudiera pagar la compra del mes. Emparejada con el término de deuda del objetivo (#142): quien no resta el principal en la base debe cubrir TODAS las cuotas pendientes en el objetivo (algebraicamente equivalente al par «NW neto vs base + interés restante»).
- `fire_target_debt_component` (4.8.0, #142; Decimal-string, opcional) — el término de deuda del objetivo **en el mes 0**: Σ de todos los pagos de cuota pendientes (cuota + extra + comisión) + cola residual. `fire_target_series[p]` ya lo lleva dentro (la serie es base inflada + término decreciente); este escalar existe para que la vista Jubilación sume al objetivo del formulario el componente que la forma cerrada del cliente no modela. Con deuda viva **el objetivo deja de ser monótono** (base creciente + término decreciente; con inflación 0, estrictamente decreciente).
- `net_recurring_monthly` / `net_cash_monthly` (semántica 4.8.0, #127) — convergen al **primer paso real del motor** (`first_month_allocation`): el servicio de deuda es el que se paga de verdad el mes 1 (`min(cuota, payoff)` + extra + comisión, no la cuota nominal) y `net_cash_monthly` incluye el tramo de Próximos del mes 1. Hasta 4.7.x se recalculaban aparte con la cuota nominal y las dos superficies publicaban dos «cajas del mes» distintas (300 € de brecha en el escenario del issue). Sin activos ya no hay atajo a ceros (el engine calcula la caja igual). Fallback a la fórmula nominal solo si el engine devuelve error.
- `points[].net_worth_real` + `deflation_annual_inflation_percent` (4.4.0, Fase 6) — el patrimonio de cada punto **en euros de hoy**, servido en vez de rehecho por cada cliente, y la base con la que se calculó (Decimal-as-string; la asunción de la instalación, rango [−2, 50] desde 4.9.0 — con deflación el deflactor es > 1 y lo real queda POR ENCIMA de lo nominal). `net_worth_real` es un escalar por punto serializado como `f64` (`serialize_decimal_as_f64`), misma excepción chart-only que sus vecinos `net_worth`/`contributed_capital` — **no** un array nuevo. (En `GET /v1/projection/deflate`, en cambio, todo viaja como Decimal-as-string: ahí no hay chart que alimentar.) **Fórmula**: `net_worth / (1 + i/100)^(month_index/12)` vía `deflator_at_month_index`, con el exponente sacado del **`month_index`, jamás de la posición del array** — es literalmente el bug de v1.4.2, y con `density=hybrid` (la que fuerza la tool MCP) las dos lecturas divergen de verdad. **Se publica SIEMPRE, también con inflación 0** (donde el deflactor es exactamente `1` y el par sale como el mismo número): omitirlo dejaría al consumidor sin distinguir «no hay inflación» de «esta versión no publica el campo». Contraste deliberado: `milestones_real` sí queda vacío con inflación EXACTAMENTE 0, contrato previo intacto — con inflación negativa (#146) se publica, y los hitos reales llegan ANTES que los nominales. **Esto NO reabre el motor «real puro»** rechazado en v1.2.0 (`futurefin-failure-archaeology` §1 fila 3): el motor sigue simulando 100 % en nominal y esto es capa de presentación — la forma testable de esa afirmación es la igualdad de arriba, o sea cero información que el motor no haya producido ya.
- `savings_source` + `savings_income_basis` / `savings_expense_basis` (v2.2.0; los `*_basis` sustituyen al escalar `savings_source_months_with_data` desde 3.9.0; su denominador se llama **`avg_months`** desde 4.0.0 — ver la nota de renombrado en §Summary) — fuente del ahorro **efectiva** (tras el fallback) que produjo `monthly_delta_assumption`, y de qué meses sale cada lado del promedio; mismo naming y semántica que en `/v1/summary`. Aditivos: los sirve `BuiltProjection` sin queries extra, para que el chart etiquete la base del Δ mensual sin pedir `/v1/summary`.

**Estados de fallo (4.6.0, #119)** — la superficie HTTP publica lo que el motor ya calculaba: `assets_depleted_month_index` (primer mes cuyo déficit iguala o supera TODO lo drenable — la cartera se vacía ese mes; `null` = no se agota en el horizonte), `uncovered_deficit_total` (déficit acumulado no cubierto, Decimal-string; ya se restaba de `net_worth`), **`unallocated_savings_total`** (4.12.1, #175 — Decimal-string; ahorro que ninguna regla de la cascada absorbió, acumulado; NO entra en `net_worth` ni en `contributed_capital` — el modelo se niega a simular un euro sin destino declarado; `"0.0000"` es el caso normal: en producción es inalcanzable con activos vivos, sumidero indestructible #176) + **`unallocated_savings_reason`** (`null` = no hay sobrante varado; `"no_assets"` = el scope no tiene activos; `"no_sink"` = hay activos sin sumidero habilitado — residual, mismo vocabulario que `/v1/allocation-rules/resolution`), `liabilities_negative_amortization[]` (pasivos cuya cuota no cubre el devengo — la deuda CRECE; más estrecho que `payment_does_not_reduce_principal`: un `interest_only` congelado NO aparece) y `fire_target_absent_reason` (`manual_amount_missing` | `net_need_not_positive` | `swr_not_positive`, los mismos literales que `simulate_projection` — nota: el primero no tiene camino vivo por la API, la escritura lo rechaza antes). `simulate_projection` gana además `assets_depleted_month_index` en ambos lados y su delta.

> La misma excepción f64 cubre los arrays por punto de `GET /v1/history/series` (`points[].net_worth/assets_total/liabilities_total`, `asset_series[].values`, `markers[].total`) y la curva fina de `/v1/history/cashflow` — misma justificación chart-only. **Desde 4.4.0 los del histórico usan su propio serializador** (`serialize_decimal_as_chart_f64` / `serialize_opt_decimal_as_chart_f64`, privados en `handlers/history.rs`), que además **recorta a 2 decimales** (`CHART_DP`); `handlers/projection.rs` conserva `serialize_decimal_as_f64` (`pub(crate)`) sin recorte para la proyección. El motivo del recorte: los valores del histórico nacen de una interpolación (`(v1 − v0) · días/días` para activos, amortización francesa para pasivos), así que arrastraban la escala completa de `rust_decimal` al JSON — `78012.333333333333333333333` son 25 caracteres por punto × ~290 puntos × 4 series. Ningún consumidor los usa a esa precisión (el chart posiciona píxeles, un agente cita euros) y los trece decimales sobrantes solo sugerían una exactitud que la interpolación no tiene. Es redondeo de **publicación**, como `money_out` y `round_ratio`: la interpolación y el anclaje siguen exactos.

**`GET /v1/projection/deflate` (4.4.0, Fase 6)** — convierte un importe entre euros nominales de un mes futuro y euros de hoy, **en las dos direcciones a la vez** (`deflator`, `amount_in_today_euros`, `amount_in_month_euros`). Query: `amount` (con signo) y **exactamente uno** de `month_index` (0..840) o `date` — la guardia es literalmente `month_index.is_some() == date.is_some()`, así que **mandar los dos Y no mandar ninguno** dan el mismo `deflate_timing_ambiguous`; una fecha pasada `deflate_date_in_past`, un mes fuera de rango `deflate_month_out_of_range` (por cualquiera de las dos vías, también desde una `date` lejana). **No acepta `?view=`**: la inflación asumida es de la **instalación**, no de una persona, así que un scope aquí sería un parámetro que no significa nada. No simula: reusa el mismo `deflator_at_month_index` que produce `net_worth_real` y `milestones_real` (un solo deflactor para las tres superficies, pineado en `projection_deflation.rs::the_served_deflator_is_the_one_behind_milestones_real`). Core `deflate_amount_core`.

**Cache server-side**: `AppState` mantiene un cache in-memory por `(installation_id, view, owner_user_id, density)` (`ProjectionCacheKey`, `state.rs`) con sliding TTL de 60 min. **`owner_user_id` es SIEMPRE `Some(_)`, también en `household` (4.0.0)**: hasta entonces solo viajaba en `mine`, y la respuesta de `household` lleva demografía del **solicitante** (`viewer_birth_date`, el horizonte derivado de su edad, `jubilacion_age`, el eje de edades), así que el primer miembro que pedía la proyección dejaba la suya cacheada para todo el hogar — un miembro recibía la fecha de nacimiento de otro y, con el orden inverso, un horizonte de 360 meses en vez de 648: si su cruce FIRE caía en el mes 400, la app le decía «no llegas a jubilarte». Regresión: `apps/api/tests/projection_cache.rs`. Hits sub-ms; el GET sin cache hace el cómputo full (~500 ms). Invalidación automática: cualquier mutación en assets/liabilities/budget/planning/allocation/installation/user.birth_date llama `state.invalidate_projection_by_installation(iid)`; las mutaciones de **transactions** invalidan **solo en los modos que usan transacciones** (`fire_settings.savings_source ∈ {transactions_avg, budget_income_real_expense}`, i.e. `SavingsSource::uses_transactions()`; ver sección Transactions). Logout llama `state.invalidate_projection_by_user(user_id)` (solo `view=mine`). Warm-up: `tokio::spawn` tras `POST /v1/auth/login` recomputa `view=household` para que el primer GET sea hit. Sin warm-up tras mutación (evita race condition de warm-ups concurrentes).

**Compresión**: todos los endpoints pasan por `tower_http::compression::CompressionLayer::new().gzip(true)`. `/v1/projection/series` baja de ~260 KB a ~30 KB con `Content-Encoding: gzip`.

**Densidad (`?density=hybrid`)**: con `?density=hybrid` el response decima los arrays grandes (`points`, `fire_target_series`, `asset_series[].values`) a un patrón mixto — mes 0..12 mensual + mes 24, 36, … **y siempre el último mes del horizonte** (`density_month_indices`, `handlers/projection.rs`). Ese último empujón es de 4.0.0 y no es cosmético: el bucle anual solo emitía múltiplos de 12, así que con un horizonte que no lo fuera la serie se cortaba antes de tiempo sin decir nada — con `?months=100&density=hybrid` el último punto era el mes 96 y los meses 97–100 no existían en `points`, ni en `fire_target_series`, ni en `asset_series[].values`, y desaparecía el punto que cualquiera lee como «patrimonio al final»; con `?months=19` se perdía el 32 % del horizonte pedido. Invisible desde la web (el horizonte derivado siempre es años × 12) pero alcanzable por `?months=N` y por la tool MCP `get_projection`, que **fuerza** `hybrid` — o sea, era el camino por defecto de un consumidor conversacional. Pin: `hybrid_density_always_includes_the_last_month_of_the_horizon`. Total ~82 puntos en lugar de ~841. JSON ~5 KB. El compute interno del engine es idéntico (840 meses); solo cambia la serialización. Cada densidad tiene su propia entry en el cache (`ProjectionCacheKey.density`). Milestones, FIRE crossover y compound marker se calculan sobre el array full (no decimado) para no perder precisión. El campo `density: "monthly" | "hybrid"` viaja en el response para que el cliente sepa qué tiene.

**Two-phase loading en el cliente**: `App.tsx` dispara `?density=hybrid` y `?density=monthly` en paralelo. El hybrid suele llegar primero (JSON más pequeño) → se renderiza el chart con menos puntos. Cuando llega el monthly, se reemplaza dentro de `startTransition()` (sin bloquear inputs). Si ambos son cache hit, ambos llegan en <10 ms → el hybrid no añade latencia perceptible.

### Backup
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/backup/user-export` | Returns `.ffbackup` binary for the **current user only**. Body: `{password, ui_preferences?}`. Encrypted with the user's account password (Argon2id KDF → AES-256-GCM). Any role. |
| POST | `/v1/backup/user-import/preview` | Body: `{file_b64, password}`. Returns counts of what would be imported. Write role required. |
| POST | `/v1/backup/user-import` | Body: `{file_b64, password, confirm_replace: true}`. **Destructive**: replaces **all** `owner_user_id = current_user` user-scoped rows (`assets/liabilities/budget_entries/planning_flows/allocation_rules/history_snapshots/transactions/transaction_imports/categorization_rules/recurring_transaction_rules`) in a single transaction, then invalidates the projection cache. Write role required. Table order + re-link details: [`data-model.md`](data-model.md) §Per-user `.ffbackup`. |

The `.ffbackup` format is a versioned, encrypted binary container — see [`backup_user/crypto.rs`](../apps/api/src/handlers/backup_user/crypto.rs) for the frame layout and [`backup_user/schema.rs`](../apps/api/src/handlers/backup_user/schema.rs) for the payload schema + migration layer (`schema_version`).

### History snapshots (`/v1/history/snapshots/`)
Snapshots manuales, **per-user**, del patrimonio en un día civil (`installation.calendar_tz`), de los que el servidor reconstruye la serie histórica de net worth. **Ninguna mutación de `/v1/history/*` llama a `refresh_projection_after_mutation`**: los snapshots no son inputs del engine de proyección y jamás invalidan su cache (regresión: `snapshot_mutations_do_not_touch_projection_cache`, `apps/api/tests/history_snapshots.rs`). Dos `kind` independientes: `asset` | `liability` (singular, como el CHECK de DB). Siempre **own-data** (`owner_user_id = usuario`); estos endpoints **no** aceptan `?view=mine` (no aplican los helpers `LedgerView`). CRUD Decimal-as-string; `total` = Σ items calculado en Rust (nunca almacenado). Auth: cualquier miembro puede leer; mutaciones requieren `role_can_write` (owner/member) o `403`.

| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/history/snapshots/capture` | Body `{kinds?: ["asset","liability"]}` (omitido → ambos; `[]` → 400 `kinds_empty`; valor desconocido → 400 `invalid_kind`). Por kind, en una transacción: fecha = hoy civil; **upsert** de cabecera (`ON CONFLICT ... DO UPDATE SET source='capture', updated_at=now()`) → la captura del mismo día sobrescribe silenciosamente; borra items y los recopia del **ledger propio** (assets: `id→source_item_id`, `name→label`, `current_value→value`, sin términos; liabilities no expiradas: además copia `apr_percent`/`payment_amount`/`payment_frequency`). Filas compartidas (`owner_user_id IS NULL`) excluidas por construcción; 0 filas propias → snapshot válido con 0 items. Respuesta **200** `{snapshots:[SnapshotResponse]}`. **No** invalida la cache de proyección (los snapshots no son inputs del engine). |
| GET | `/v1/history/snapshots?year=YYYY&kind=` | Siempre own-user. `year` opcional (1900..=3000) filtrado como **rango de fechas** (`>= YYYY-01-01 AND < (YYYY+1)-01-01`); `kind` opcional. Orden `snapshot_date DESC, kind ASC, id ASC` (el desempate por `id` es de 4.4.0: sin orden total, la paginación de la tool MCP repetía u omitía filas entre páginas). **El GET HTTP no pagina** — devuelve el conjunto entero con el detalle incluido y sin `COUNT`, contrato REST intacto; la paginación (`limit` 1..200 def 50 + `offset`) y la supresión de items viven en la core y las usa solo la tool MCP, mismo patrón que `list_transactions_query`. Solo lectura (nunca muta). → **200** `[SnapshotResponse]` (array plano, como los demás GET de listado). |
| POST | `/v1/history/snapshots` | Backfill. Body `{kind, snapshot_date, items:[{item_id?, label, value, apr_percent?, payment_amount?, payment_frequency?}]}`. Códigos 400 estables: `snapshot_date_in_future`, `snapshot_date_too_old` (<1900-01-01), `too_many_items` (>500), `duplicate_item_id`, `terms_only_for_liabilities`; bounds de `value`/términos copiados de assets/liabilities. `item_id` ausente → UUID de servidor (devuelto). Fecha (usuario,kind,fecha) ocupada → **409**. `source='backfill'`. → **201** `SnapshotResponse`. |
| PUT | `/v1/history/snapshots/{id}` | Body `{snapshot_date?, items?}` — `items` omitido → conserva los items (solo actualiza cabecera/fecha); `items` presente (incluso `[]`) → reemplazo completo. `kind` inmutable. Guardia `id + installation + owner` → **404** si no es tuyo (no revela existencia). Mover a fecha ocupada → **409**. `source` intacto, `updated_at=now()`. → **200** `SnapshotResponse`. |
| DELETE | `/v1/history/snapshots/{id}` | **204**; misma guardia 404; items en cascada. |
| GET | `/v1/history/snapshots/prefill?kind=&date=` | Pre-rellena el panel de backfill. **Siempre own-user** (sin `?view`); solo lectura (viewer incluido, nunca muta). `kind` requerido ∈ {asset, liability} (ausente/inválido → 400 `invalid_kind`); `date` requerida `YYYY-MM-DD` (ausente → 400; futura → `snapshot_date_in_future`; <1900-01-01 → `snapshot_date_too_old`). Reconstruye el MISMO timeline own-user de `/history/series` (snapshots del kind + obs virtual «hoy» de las filas vivas no expiradas salvo que el último snapshot real sea hoy) y evalúa cada item en `date`. Sin timeline (0 snapshots del kind) → universo = filas vivas, `basis="live"`, valor = current_value/principal. Con timeline: antes del primer snapshot → valor del primer snapshot (`basis="first_snapshot"`) para los items presentes allí, resto `not_owned`; dentro de `[first,last]` interpola vía el motor (`amortized_segment_value`; activos lineal en días, pasivos amortización francesa) → `basis="interpolated"`; item sin obs ≤ `date` (posterior) o cuya última obs precede `date` y ausente después (borrado/vendido) → `value 0, existed false, basis="not_owned"`. Términos (pasivos) desde la obs de inicio de segmento, si no la obs con términos más cercana, si no la fila viva. Orden: `existed=true` primero (`label ASC`), luego `not_owned` (`label ASC`). Universo vacío → **200** items `[]`. → **200** `PrefillResponse`. |

`SnapshotResponse`: `{id, kind, snapshot_date_ymd, source, total (Decimal-string), item_count, items_included, items:[{item_id (=source_item_id), label, value, apr_percent?, payment_amount?, payment_frequency?}] orden label ASC, created_at, updated_at}`. **`item_count` / `items_included` (4.4.0, Fase 5)**: `item_count` es el nº de items **según la BD**, viaje o no el detalle; `items_included = false` significa que `items` llega **vacío por supresión**, no que el snapshot esté vacío. Hasta 4.4.0 la supresión (la que hace la tool MCP `list_snapshots` sin `include_items`) dejaba `items: []`, exactamente el mismo JSON que un snapshot sin ítems — con un `total` de 12.000 € al lado para rematar la contradicción. La supresión vive ahora **dentro de la core** (`build_response_with_items`), que es donde se puede declarar; el GET HTTP nunca suprime. `total` e `item_count` se calculan SIEMPRE sobre los ítems reales.

`PrefillResponse`: `{date_ymd, kind, items:[{item_id (=source_item_id), label, value, existed, basis, apr_percent?, payment_amount?, payment_frequency?}]}`. `value` es Decimal-string **redondeado a 2 decimales** (sugerencia display-grade que el usuario edita); los términos se ecoan sin redondear (son observaciones copiadas, no computadas); `existed` es bool; `basis` es string ∈ {`interpolated`, `first_snapshot`, `live`, `not_owned`}.

### History series (`GET /v1/history/series`)
Serie histórica de net worth **interpolada server-side** desde los snapshots (el cliente no interpola). **Desde 4.0.0 el punto `month_index = 0` se evalúa en el hoy civil**, no en su primero-de-mes: los meses pasados se evalúan el día 1 y el mes en curso —que está a medias— en hoy, así el último punto empalma con el patrimonio vivo y cuadra con `GET /v1/summary`. `anchor_month_first_ymd` sigue siendo la ETIQUETA de mes del punto 0 (clave de alineación con la rejilla de la proyección; moverla rompe el empalme del chart) y `anchor_date_ymd` es su fecha de evaluación. La observación virtual «hoy» (`append_virtual = last_real < today`) pasa de conveniente a **imprescindible**: es el ancla de ese punto — sin ella `e > dates[m-1]` cae en la rama «tras el último snapshot: 0» y el punto 0 valdría cero. Antes, con dos snapshots del mes en curso, la curva terminaba por debajo de fotos reales del propio usuario y un activo cuya primera foto era la más reciente valía 0 en toda la ventana (auditoría MCP §2; regresión: `history_series.rs::series_reaches_snapshots_taken_this_month`). **`liabilities_snapshotted` manda sobre `net_worth` (issue #82, F1 — cambio breaking: `net_worth` pasa de siempre-número a nullable)**: vale `true` ⇔ el pasivo del scope está fotografiado **entero** — hay algún snapshot y **todos** los usuarios que aportan serie tienen alguna cabecera de kind `liability` (helper `liabilities_fully_snapshotted`). Es un `all`, no un `any`: en hogar, con Alice fotografiando su hipoteca y Bob no, un `any` publicaba `activos − media deuda`, un número que ya no coincide con `assets_total` y por eso *parece* correcto. Con el flag en `false`, `points[].net_worth` es **`null` en toda la serie** (nunca omitido: `null` explícito); `assets_total` y `liabilities_total` se publican igual que siempre. Antes se publicaba como número y, sin snapshots de pasivo, era **idéntico a `assets_total`**: la serie decía un patrimonio y `GET /v1/summary` otro, en la misma conversación, sin nada que dijera cuál mirar (auditoría MCP F1). Un usuario **sin deuda** declara la ausencia capturando un snapshot de pasivo (la cabecera se escribe aunque no haya ni una fila viva) — hecho afirmado, no ausencia interpretada. Regresiones: `history_series.rs::liabilities_snapshotted_tells_missing_data_from_no_debt` y `::household_net_worth_needs_every_member_to_have_snapshotted_liabilities`. Solo lectura, cualquier miembro (viewer incluido). Acepta `?view=mine` vía helpers `LedgerView`: household = TODOS los snapshots de la instalación (todos tienen `owner_user_id NOT NULL`), agregados per-user en Rust; mine = solo los del usuario. Sin `?density`, sin cache y sin `spawn_blocking` — deliberado: el cómputo es sub-ms (decenas de snapshots × decenas de meses).

**Ventana por defecto acotada (4.4.0, Fase 5)**: omitir `window_months` ya **NO** devuelve todo el histórico — devuelve los **últimos 120 meses** (`DEFAULT_HISTORY_WINDOW_MONTHS`, 10 años). Para todo, `window_months=1200` (= `MAX_HISTORY_WINDOW_MONTHS`; nada puede haber más atrás porque el tope de la ventana ES el tope del producto). El default anterior era literalmente el peor caso: un hogar que hubiera anclado su histórico muy atrás —hasta su fecha de nacimiento— recibía ~290 puntos, los primeros doscientos interpolando entre 0 € y unos cientos, a 15 decimales cada uno; **53,6 KB → 16,1 KB (−70 %)** en la medición de la auditoría. 10 años y no 5: es el tramo en el que un patrimonio real ya tiene forma (una hipoteca, un cambio de trabajo, un mercado bajista) y sigue cabiendo en ~121 puntos. La SPA pide `window_months=1200` **explícitamente** (`App.tsx`), porque el chart quiere la serie entera. Fuera de `1..=1200` sigue siendo **400 `window_months_out_of_range`**, nunca un clamp silencioso.

Response (`HistorySeriesResponse`) — los numéricos por punto en **f64 recortado a 2 decimales** (`CHART_DP`; ver nota en §Projection):
- `anchor_date_ymd` (hoy civil de la instalación), `anchor_month_first_ymd` (fecha del punto `month_index = 0`), `view` (`household` | `mine`)
- `window_months` (u32) — **ventana efectivamente emitida** (`points.len()` es como mucho `window_months + 1`): eco del valor pedido o del default. Se resuelve **antes** del early-return de «0 snapshots», para que una respuesta vacía declare igualmente qué ventana aplicó: si no, «no hay datos» y «no hay datos EN ESTA VENTANA» volverían a ser indistinguibles.
- `window_truncated` (bool) — `true` ⟺ hay snapshots **anteriores** a la ventana emitida. Recortar sin decirlo sería exactamente el fallo que 4.3.1 arregló en el `window_months` fuera de rango.
- `first_snapshot_date_ymd` / `first_snapshot_month_index` (opcionales) — el snapshot **más antiguo del scope**, esté dentro o fuera de la ventana (`month_index` menor que `-window_months` cuando `window_truncated`). Responden «¿desde cuándo hay datos?» sin obligar a repetir la llamada con la ventana máxima.
- `points[]` — `{month_index: i32 ≤ 0 (contiguos k_min..=0, incluye el mes 0), net_worth: f64 | null, assets_total, liabilities_total}`; `net_worth = A − L` **solo si `liabilities_snapshotted`**, si no `null` en todos los puntos (`net_worth === null` ⇔ flag `false`, un único invariante). El consumidor que quiera un neto histórico tiene que mirar el flag; el que quiera activos ya los tiene en `assets_total`.
- `asset_series[]` — `{asset_id (= source_item_id), asset_name, values: f64[] paralelo a points}`. Agrupado por `source_item_id` **entre usuarios** (valores sumados); nombre = el asset vivo si el id coincide, si no el label del snapshot más reciente que lo contiene; orden `asset_name ASC, asset_id ASC`. Solo los assets tienen serie por item (paridad con projection).
- `markers[]` — uno por snapshot en scope: `{date_ymd, month_index, month_fraction = month_index + (día−1)/días_del_mes **redondeado a 4 decimales** (`MONTH_FRACTION_DP`; 1/10.000 de mes ≈ 4 minutos, y la rejilla más fina que existe es diaria ≈ 0,032 — lo que sobra es ruido de la división en f64), kind, source, owner_user_id, total (Σ items)}`. **`source` (4.4.0)**: `capture` (foto que la app tomó de los activos/pasivos vivos ese día) | `backfill` (valores tecleados a posteriori para una fecha pasada). No es cosmético: un backfill puede estar en CUALQUIER fecha, y sin este campo el ancla remota de un hogar se presenta igual que una foto real — a «¿cuándo empecé a ahorrar?» la serie contestaba con la fecha del ancla. Con `source` y `total` a la vista, un backfill de importe ~0 en una fecha remota se reconoce por lo que es.
- 0 snapshots en scope → **200** con los tres arrays vacíos.

Algoritmo: ancla = primero-de-mes del hoy civil; timelines por `(owner_user_id, kind)` (fechas ascendentes + vectores de observación paralelos por `source_item_id`); a cada timeline se le añade la observación virtual «hoy» con las filas vivas del owner (assets y liabilities no expiradas del scope, ambas con conjunto extra `owner_user_id IS NOT NULL` — las filas compartidas nunca participan), salvo que el último snapshot real sea de hoy. La interpolación vive en `crates/engine/src/history.rs` (`evaluate_timeline`): assets lineal en días civiles, liabilities amortización francesa corregida por residuo (exacta en ambos extremos; cuota `weekly → ×52/12`). Usuarios sin snapshots de un kind no tienen timeline → no aportan (household = suma de los usuarios que snapshotean). Como todo GET: nunca muta.

### History cash-flow (`GET /v1/history/cashflow`) — v1.6.0
Cash-flow histórico de las transacciones (tier-2 sobre los snapshots). Solo lectura, cualquier miembro. Acepta `?view=mine` vía `LedgerView`. **Nunca invalida la cache de proyección** (las transacciones no son inputs del engine). Sin cache; `spawn_blocking` solo en `resolution=daily`. Dos capas independientes en el mismo response:
1. **`months[]`** — agregado mensual **firmado** por kind, **Decimal-string** (son KPIs, escala 2dp). Solo un `GROUP BY (mes, kind)` sobre la ventana, independiente de los snapshots: `expense`/`savings` conservan su signo real (≤0), `income` ≥0. **Dos netos desde 4.0.0** (auditoría MCP §6): `cash_delta = expense + income + savings` (variación de caja, **incluye** los traspasos a ahorro — un mes excelente con una aportación grande sale negativo) e `income_minus_expense = income + expense` (sin ahorro; **misma cifra** que `totals.net_actual` de `/v1/transactions/summary`, allí con magnitudes ≥0). El campo se llamaba `net` y colisionaba de nombre con `net_actual` significando otra cosa: un abril con 3.710,97 € movidos a inversión salía `net: -3075.26` y se leía como una pérdida. Regresión que ata las dos tools: `history_cashflow.rs::the_two_nets_differ_by_savings_and_one_matches_the_monthly_summary`. Contiguo `-window_months..=0` (incluye el mes 0 en curso), ascendente.
2. **`fine`** (opcional) — la **curva fina** de patrimonio (`weekly`/`daily`) donde los deltas de cash-flow **moldean** los assets vinculados sin contradecir los snapshots (curva anclada, `crates/engine`). Presente **solo** si hay transacciones vinculadas a algún asset **y** snapshots que anclar; cuando falta (o el cálculo falla), `fine_absent_reason` dice cuál de los cuatro motivos fue y **el pasado queda idéntico a la serie de snapshots** de `/v1/history/series` — el tier-2 solo añade, nunca sustituye. Patas de cash-flow: pata cuenta (batch con `account_asset_id` → `delta = +amount`) y pata destino de ahorro (`kind='savings'` con `linked_asset_id` → `delta = −amount`); una savings importada aparece en ambas (partida doble). `fine` = `{resolution, grid:[{date_ymd, month_index, month_fraction}], asset_series:[{asset_id, asset_name, values: f64[]}], net_worth: f64[] | null}`, todo paralelo a `grid`; la rejilla fina termina **exacta en hoy** (empalma con el vivo). `month_fraction` es el mismo helper que los `markers[]` de `/history/series` (fuente única → la escala mes→px no puede divergir). **`fine.net_worth` es `null` cuando `liabilities_snapshotted` (issue #82, F1 — mismo invariante que `/v1/history/series`, cambio breaking) es `false`**: sin el pasivo del scope fotografiado entero (helper `liabilities_fully_snapshotted`, `all` por usuario) la resta no es un patrimonio neto, y antes de 4.4.0 este campo era exactamente `Σ activos` con nombre de neto — **peor** que la serie mensual, porque `CashflowResponse` ni siquiera publicaba el flag con el que sospechar de la cifra. `liabilities_snapshotted` (raíz, aditivo) se resuelve dentro de la rama `fine` (es donde se carga el scope de snapshots); sin capa fina vale `false` por defecto — lectura honesta, no hay serie de neto que cualificar. En ese caso `fine.asset_series` sigue disponible: es lo que el chart pinta cuando el pasado es «solo activos».

Params: `view` (`mine` | omitido → household); `window_months` (i64, default 24, **rango 1..=120; fuera de rango es 400 `window_months_out_of_range`, NO se clampa** — `validate_window_months`, unificado en 4.3.1); `resolution` (`weekly` default | `daily`). **Gating de daily**: `resolution=daily` exige `window_months <= 6` → si no, **400** `daily_window_too_large`. Response `CashflowResponse {anchor_date_ymd, anchor_month_first_ymd, view, months, liabilities_snapshotted, fine?, fine_absent_reason}`; numéricos de `fine` en **f64 recortado a 2 decimales** (`CHART_DP`, misma excepción chart-only que projection/history-series) y `grid[].month_fraction` a 4 (`MONTH_FRACTION_DP`), `months[]` en Decimal-string.

**`fine_absent_reason` (4.4.0, Fase 5) — por qué falta `fine`**; `null` ⟺ `fine` viaja. Se decide en la MISMA expresión que produce `fine`, así que no pueden desincronizarse (nunca un `Some(fine)` con razón ni un `None` sin ella). Cuatro valores: `not_requested` (el llamante no la pidió — `include_curve` de la tool MCP; el GET HTTP la pide siempre), `window_too_large_for_curve`, `no_asset_linked_transactions` (ninguna transacción ligada a un activo, ni por cuenta de import ni por destino de ahorro: no hay nada que moldee la curva), `no_snapshots_to_anchor` (hay movimientos pero ningún snapshot al que anclar: sería una curva de deltas flotando en el vacío). Hasta 4.4.0 las tres últimas producían **exactamente la misma respuesta** —el campo simplemente no estaba— y «no tengo datos», «no me lo has pedido» y «te lo he recortado por tamaño» eran indistinguibles.

**Cota de la curva fina (4.4.0, Fase 5): `MAX_FINE_CURVE_WINDOW_MONTHS = 36`.** Es el peor caso del catálogo — la rejilla weekly avanza de 7 en 7 días, así que 120 meses son ~522 puntos **por activo**, y un hogar con cinco activos vinculados se lleva ~2.600 números en una sola respuesta; con 36 meses son ~157 por activo (**64 KB → 20 KB, −69 %** en la medición de la auditoría) y la curva sigue contando lo que un overlay de patrimonio necesita — el propio chart de la app pide 24 semanales y 6 diarios. **Pasarse NO es un 400**: el agregado mensual `months[]` llega igual hasta 120 meses, `fine` llega `null` y `fine_absent_reason = "window_too_large_for_curve"`. Un error habría obligado a reintentar para conseguir unos `months[]` que ya eran servibles — es la misma doctrina que `fine_absent_reason`: degradar con motivo publicado antes que fallar.

### Transactions (`/v1/transactions/`) — v1.6.0
Histórico de gasto mensual **per-user**: import de CSV bancario (MyInvestor/N26) o efectivo a mano, categorización con reglas aprendidas, y comparativa mes real vs presupuesto vs promedio. Decimal-as-string (importes firmados: negativo = cargo). **Invalidación de la cache de proyección condicionada al modo** (`fire_settings.savings_source`): en modo A (`budget`, default) las transacciones **no son inputs del engine** → ninguna mutación invalida; en los modos que usan transacciones (B `transactions_avg` y C `budget_income_real_expense` → `SavingsSource::uses_transactions()`) el ahorro de la simulación deriva del promedio real 12m → las mutaciones que cambian el conjunto (create/batch/patch/delete, delete import, import confirm, `recurring/materialize`, y desde 3.5.0 conciliar/desconciliar) invalidan la cache vía `invalidate_projection_if_savings_uses_transactions` (best-effort post-commit, jamás convierte una mutación exitosa en 5xx). `rules.rs`, previews y un pase de conciliación sin pares nuevos nunca invalidan. **El REPLAY de un `create` con `idempotency_key` (Fase 3, issue #84) tampoco invalida**: no inserta nada, así que el conjunto no cambia — desalojar una cache caliente por una petición que no movió un número sería pagar un recompute de ~500 ms por nada. Regresión: `transactions_projection_cache.rs::mode_b_replay_of_an_idempotent_create_does_not_invalidate`. **Corrección 4.0.0 — borrar una regla recurrente SÍ invalida (COND)**: el contrato decía «no cambia el conjunto de transacciones», y es cierto, pero cambia su **CLASIFICACIÓN**, que es lo que cuenta. `real_txns` filtra `recurring_rule_id IS NULL` y el mes de origen está exento de la poda, así que puede haber una instancia en un mes sin ningún movimiento real: un mes que el promedio ignora entero. Al borrar la plantilla, el `ON DELETE SET NULL` lo convierte en mes real y entra en numerador **y** denominador. `delete_recurring_rule_core` recibía un `&PgPool` y era **estructuralmente incapaz** de invalidar; ahora recibe el `&Arc<AppState>`. El test que congelaba lo contrario queda corregido. Regresión (A/B/C + flip + reconcile): `transactions_projection_cache.rs`.

> **`op_date` es la fecha que manda; `value_date` es informativa** (documentado en 4.4.0, sin cambio
> de comportamiento). TODOS los cortes por mes y por ventana —la comparativa, el promedio ponderado,
> las series por categoría, el agregado de cash-flow, la lista de meses— agrupan por `op_date`.
> `value_date` es la fecha VALOR del banco cuando el extracto la trae y el preset la mapea (ausente
> en manuales y en bancos que no la publican) y **ningún agregado la usa**: un cargo con `op_date`
> el 31 de enero y `value_date` el 1 de febrero cuenta entero en enero. Se conserva porque es lo que
> el usuario ve en su banco y sirve para casar un movimiento con su extracto — si una cifra mensual
> no le cuadra con el banco, la explicación suele estar ahí.

**Promedio real que alimenta el engine** (`transactions_avg`, distinto del summary de Movimientos): ventana `[first-of-month(today) − 12m, first-of-month(today))`. El denominador `months_with_data` y las sumas por kind cuentan solo **meses reales** — meses del tramo con ≥1 transacción `recurring_rule_id IS NULL` **y `kind` clasificado** (4.8.0, #125: un mes cuyo único contenido son importaciones sin clasificar sumaba 0 € al numerador y 1 al denominador — seis meses así partían la media por la mitad, y de ahí un objetivo FIRE 300 k€ más bajo en modo B). Un mes vacío o «pseudovacío» (solo instancias recurrentes materializadas, p. ej. tras un backfill) se excluye **por completo** (ni numerador ni denominador); un mes real cuenta entero, incluidas sus recurrentes. Desde 3.5.0 las **transferencias conciliadas** (`transfer_counterpart_id IS NOT NULL`) quedan igualmente fuera de numerador Y denominador (un mes cuyo único contenido son patas conciliadas es un mes vacío). Desde el auditoría MCP el `GET /v1/transactions/summary` de la pestaña Movimientos aplica **el mismo predicado de mes real**, y desde 4.8.0 (#125) también **el mismo ancla de ventana** (HOY): las dos «medias de N meses» de la app describen por fin el mismo tramo (hasta 4.7.x el summary anclaba en el mes seleccionado y quedaban desplazadas un mes bajo el mismo rótulo). Siguen sin ser idénticos: `transactions_avg` tiene ventanas por lado configurables (`AvgWindowSpec`, modos `data`/`calendar`), mientras que el summary usa siempre ventana de calendario. Los importes se promedian en euros **nominales de su fecha**, sin deflactar (declarado en la ayuda; con lookback `data` de hasta 120 meses, un histórico viejo pesa igual que el reciente — deuda declarada de #125, no arreglada). Ambos excluyen conciliadas de todos sus buckets. Lecturas: cualquier miembro (`?view=mine` vía `LedgerView` en los GET de listado/comparativa/imports; las **reglas** son siempre own-user, sin `?view`); escrituras siempre `owner_user_id = usuario` y exigen `role_can_write` o **403**. Import limit 16 MiB (`BACKUP_IMPORT_BODY_LIMIT_BYTES`, reutilizado). Códigos 400 estables entre comillas. **Signo↔kind (4.0.0)**: `assert_amount_sign_matches_kind` (`transactions/schema.rs`) exige `income > 0`, `expense`/`savings < 0` en el **alta manual** y en el PATCH **cuando éste fija `amount`**. Reclasificar no valida —ni el PATCH de solo `kind`, ni el lote (que no admite `amount`), ni `apply_categorization_rule`— porque un `expense` positivo es contabilidad correcta (devolución que netea) y porque el lote y el individual tienen que seguir siendo equivalentes (`batch_patch_matches_individual_patches_and_rejects_rewrites`). Import de CSV y restore de `.ffbackup` **exentos**: traen el signo del banco. Igual de acotado el guard de `op_date` futura (`op_date_in_future`): alta manual + PATCH, nunca import/restore. Regresión: `transactions_crud.rs`. **`list_categorization_rules` pagina en MCP desde 4.0.0** (`limit` 1–200 default 50, `offset`, sobre `{total_count, offset, truncated, rules}`): es la única lista del catálogo que crece con el uso normal —`learn_rule` inserta una por concepto distinto en cada import— y devolvía ~11 KB de una tacada. El GET HTTP sigue sirviendo el array entero (`limit = None`, mismo contrato que `list_transactions_query`), así que la tool ya **no** es byte-idéntica al endpoint y sale del bucle `new_read_tools_match_http_endpoints`; su paridad de contenido la cubre `list_categorization_rules_paginates_without_changing_the_http_contract`.

| Method | Path | Rol | Notas |
|--------|------|-----|-------|
| GET | `/v1/transactions?view=&month=&kind=&category_id=&uncategorized=&import_id=` | lectura | Listado, orden `op_date DESC`. `month` = `YYYY-MM` (inválido → 400). **`uncategorized=true` (4.4.0, Fase 6)** filtra `category_id IS NULL`: hasta ahora `category_id` solo hacía igualdad de UUID, así que «enséñame lo que falta por clasificar» —la pregunta que abre cualquier sesión de limpieza— obligaba a paginar el ledger entero detectando la **ausencia** de una clave. Excluyente con `category_id` → 400 `category_filter_exclusive`. Excluye `savings` salvo `kind` explícito (un movimiento de ahorro sin categoría no es un hueco: es que la categoría no aplica). → **200** `[TransactionResponse]`. |
| POST | `/v1/transactions` | write | Alta manual (efectivo, `import_id NULL`, `source='manual'`). Body `{op_date, value_date?, concept, amount, kind, category_id?, linked_asset_id?, linked_liability_id?, notes?, recurrence?, idempotency_key?}` (`CreateTransactionRequest = CreateTransactionBody` aplanado con `#[serde(flatten)]` + el campo de idempotencia — el JSON no cambia de forma). **`recurrence: {}`** (opcional, marcador sin campos desde 3.2.0): crea además una regla recurrente-plantilla y deja esta transacción enlazada como instancia de origen (`recurring_rule_id`). Las reglas tienen **resolución mensual** — el legacy `day_of_month` (≤3.1.0) se **ignora** si un cliente viejo lo envía (breaking documentado en CHANGELOG 3.2.0). **Un alta con `op_date` pasada backfillea las instancias de todos los meses CERRADOS intermedios en el MISMO commit** (el mes en curso jamás; ya no depende de una llamada posterior a `/materialize`); `op_date` a más de 10 años atrás → **422** `recurrence_too_old`. **`idempotency_key` (Fase 3, issue #84, opt-in)**: 1..200 chars; misma clave + mismo cuerpo (huella del cuerpo YA VALIDADO, así que `"10"` y `"10.00"` son el mismo reintento) → devuelve la fila original **sin crear nada**, mismo `id`, mismo 201; misma clave + cuerpo distinto → **409** `idempotency_key_conflict` (gana el primero); clave reclamada DENTRO de la misma transacción que el INSERT (dos reintentos simultáneos: el perdedor deshace su INSERT y reproduce el del ganador). Caduca a las 24 h, poda perezosa en el propio POST. Ver [`data-model.md`](data-model.md) §`transaction_idempotency_keys`. 400: `invalid_kind`, `amount_zero`, `savings_no_category`, `category_scope_mismatch`, `linked_asset_not_found`, `linked_liability_not_found`, `idempotency_key_invalid`. Huella duplicada → **409**. → **201** `TransactionResponse` (incluye `recurring_rule_id?`). |
| POST | `/v1/transactions/batch` | write | Alta manual multifila (1..=1000). Body `{transactions:[CreateTransactionRequest]}`. Cada item acepta `recurrence` (misma semántica que el alta simple, backfill de meses intermedios incluido; item con `op_date` a >10 años → **422** `recurrence_too_old`). Ordinal de huella se avanza dentro del batch. **`idempotency_key` por ítem se RECHAZA, no se ignora** (Fase 3, issue #84): cualquier ítem con la clave → **400** `idempotency_key_batch_unsupported` **antes** de tocar la BD (todo el lote, cero filas). **Pero desde 4.4.0 (Fase 6) el LOTE sí acepta una clave, en la RAÍZ del body** — la Fase 3 rechazaba toda idempotencia de lote porque «reproducir parcialmente» no tiene semántica, y el razonamiento que la reabre es que **el lote es UNA unidad atómica y por tanto lleva UNA clave**. `idempotency_key` en la raíz: 1..180 chars (20 menos que los 200 del alta individual, para el sufijo derivado). **Sin tabla ni columna nueva**: como `transaction_idempotency_keys` guarda UN `transaction_id` por fila y un lote crea N, se escribe **una fila por ítem** con la clave derivada `{clave}#b{i}` y, en todas, el hash del **lote entero** (marcador `batch-v1` + nº de ítems + los ítems ya validados y en orden). Las tres salidas: misma clave + mismo cuerpo → **los N movimientos originales, mismos ids, mismo orden, mismo 201**, sin insertar nada; misma clave + cuerpo distinto (un importe, el orden, el nº de ítems) → **409** `idempotency_key_conflict`, gana el primero; clave por ítem → sigue siendo 400. Las N claves se reclaman **en la misma transacción** que los N INSERT, así que «3 de 5» no puede ocurrir; si al reproducir falta alguna fila (un movimiento borrado — la FK es `ON DELETE CASCADE`) el replay es un 409 ruidoso, no medio lote. Ventana 24 h, poda perezosa en el propio POST (nunca en un GET, D5). Regresión: `apps/api/tests/transactions_batch_idempotency.rs`. → **201** `[TransactionResponse]`. |
| GET | `/v1/transactions/months?view=` | lectura | Meses con datos (`GROUP BY YYYY-MM`), orden DESC; `is_complete=false` **solo** para el mes civil en curso de la instalación (no significa «faltan datos»: el mes no ha terminado). **Desde 4.4.0 el mes en curso viaja SIEMPRE**, aunque el `GROUP BY` no lo devuelva por estar vacío, con `txn_count: 0` — es el único mes en el que ese 0 puede darse. Antes desaparecía justo en el caso en que importa: la única rama que produce `is_complete = false` no se materializaba nunca en esa instalación, la descripción de la tool prometía un caso inalcanzable y, peor, el mes **sí** consumía su hueco en las series (`/v1/transactions/category-series` y `/v1/history/cashflow` lo cuentan igual) — una lista de meses que omite el mes que las series incluyen no sirve para orientar consultas, que es justo para lo que existe. Se inserta al frente (`insert(0)`), que conserva el orden porque en este agregado no hay fechas futuras. → **200** `[MonthEntry]`. |
| GET | `/v1/transactions/category-series?view=&kind=&category_id=&window_months=` | lectura | Serie mensual por categoría (issue #2): para cada categoría del `kind` (`expense`\|`income`, obligatorio) con ≥1 movimiento en la ventana, un punto por mes **cero-relleno** (`{month: "YYYY-MM", total}`; magnitudes ≥ 0 Decimal-string escala 2, misma convención de signos que el summary). `window_months` default 12, clamp 1..=60; el último mes es el actual (parcial). Orden: nombre ASC, pseudo-categoría `null` (sin categorizar) al final. **Fase 1 (issue #82)**: cada punto lleva `has_data` (¿ese mes tiene ALGÚN movimiento en el scope, de cualquier `kind`?) y la raíz publica `first_month_with_data` (`YYYY-MM` del primer movimiento de toda la historia, omitido si no hay ninguno) — sin ellos, el cero-relleno hacía indistinguibles «no gastaste en esta categoría» y «de ese mes no hay datos». Y un `category_id` del scope equivocado ya no devuelve `{series: []}` con 200: **400 `category_scope_mismatch`** si el scope no casa con el `kind`, **400 `category_not_found`** si el UUID no existe (400 y no 404, igual que `assert_transaction_category` y `budget.rs`: el recurso existe, lo que está mal es un parámetro). 400 `category_series_kind_invalid`. → **200** `CategoryMonthlySeriesResponse`. Espejo MCP: `get_category_monthly_series`. |
| GET | `/v1/transactions/aggregate?view=&month=&kind=&category_id=&uncategorized=&import_id=&concept_contains=&min_amount=&max_amount=&date_from=&date_to=&top=` | lectura | **4.4.0 (Fase 6)** — suma y cuenta **dentro de SQL**, con los MISMOS filtros que el listado (`PreparedFilters::prepare` compartida, así que no puede haber deriva entre listar y agregar). Responde `total_signed` (Σ con signo), `total` (magnitud ≥0) + `total_absent_reason` (`no_transactions` \| `mixed_kinds` \| `kind_unset_rows` — sumar magnitudes de kinds distintos no significa nada, así que se dice en vez de devolver un número), `kind_basis`, `first_op_date`/`last_op_date` — **estos cuatro se OMITEN cuando son `None`** (`skip_serializing_if`), a diferencia de la doctrina de `null` explícito del resto de la Fase 6: aquí la ausencia de la clave ES el «no aplica» — y los desgloses `by_kind[]` (orden **fijo** expense→income→savings→sin kind: es taxonomía, no ranking), `by_month[]`, `by_category[]` (con `share_pct`, 1 decimal, dentro de su propio kind) y `top[]` (`top` 0..=50, def 5; fuera → 400 `limit_out_of_range`). **La razón de existir**: «¿cuánto llevo gastado en X este año?» obligaba a bajar hasta 500 filas al contexto y sumarlas con un modelo que **no aplicará** `transfer_counterpart_id IS NULL` — número plausible y falso. Ese predicado vive **dentro de la core**, y lo excluido se publica en `reconciled_excluded_count` para que la exclusión sea **auditable** en vez de silenciosa. Paridad mes a mes con `/summary` pineada en `aggregate_matches_get_transactions_summary_month_by_month`. Cache NONE. → **200** `AggregateResponse`. |
| GET | `/v1/transactions/duplicates?view=&month=&kind=&import_id=&concept_contains=&date_from=&date_to=&limit=` | lectura | **4.4.0 (Fase 6)** — grupos de ≥2 movimientos que comparten la **huella canónica de dedup**, la misma que ya usaba el preview de import (`basis` la publica literal: `owner + source + op_date + amount(4dp) + normalized_concept`). Agrupa por **`(owner_user_id, fingerprint)`**, el ámbito exacto de la constraint `transactions_unique_fingerprint` — agrupar solo por huella metería el mismo recibo de dos personas del hogar. Son **candidatos, no veredicto**: el discriminante es `spans_multiple_imports`/`distinct_import_count` (dentro de un lote suelen ser reales; entre lotes es el patrón del re-import). `limit` 1..=100 def 20 (`limit_out_of_range`); no acepta `category_id`/`uncategorized`/`min_amount`/`max_amount`. Cache NONE. → **200** `DuplicatesResponse`. |
| GET | `/v1/transactions/summary?view=&year=&month=&avg_window=&avg_months=` | lectura | Comparativa del mes (default: último mes **completo**). **Ventana del promedio** con `avg_window` ∈ {`3`,`6`,`12`,`ytd`,`all`} (default `6`; trim + case-insensitive; inválido → 400 `avg_window must be one of 3, 6, 12, ytd, all`), **siempre meses de calendario** — no confundir con el `window_mode` (`data`|`calendar`) del promedio configurable del engine (§Summary); `avg_months` (1..24) es **alias legado** y `avg_window` gana si vienen ambos. Promedio **ponderado**: denominador = `avg_months` = meses **reales** del tramo `[window_start, first_of_month(hoy))` — desde 4.8.0 (#125) la ventana se ancla en **HOY**, no en el mes seleccionado: es el MISMO tramo que promedia `transactions_avg` (la media de la proyección), dos selecciones distintas comparan contra la misma media, y el mes seleccionado **entra en su propio promedio** si cae dentro. Mes real = ≥1 transacción del scope con `recurring_rule_id IS NULL` **y `kind` clasificado** (#125 — un mes solo de importaciones sin clasificar no divide), **no** el nº de meses del tramo ni todos los que tienen algo. Un mes no real queda fuera del **numerador Y del denominador** — excluirlo solo del denominador dejaría su importe arriba y dispararía las categorías presentes en él. Denominador **único** para todas las líneas (no por categoría), así que `Σ avg de categorías == totals.expense_avg` y la tasa de ahorro promedio no se infla; la contrapartida aceptada es que un mes real sin movimientos de una categoría concreta sí cuenta como cero para ella. YTD = meses **completos del año en curso** (en enero → tramo vacío, `empty_window`); ALL = desde el mes del primer movimiento hasta el mes en curso (exclusive). Los importes se promedian en euros nominales, sin deflactar (declarado en la ayuda). Magnitudes ≥0 para comparar con budget (gasto = `−Σ`, ingreso = `+Σ`, ahorro = `−Σ`). **Cuotas atribuidas por categoría (3.4.0)**: cada pasivo activo EN EL MES seleccionado (`payment_end_date IS NULL OR >= primer día del mes`) con `expense_category_id` asignada suma su equivalente mensual al lado **budget** de esa categoría — se empareja con los recibos reales (que ya viven categorizados) y `totals.expense_budget` = Σ budget de categorías de gasto **+ cuotas atribuidas**. Una categoría solo-cuota materializa su fila (budget = plan, actual = 0). Pasivos sin asignar (NULL, pre-3.4.0): sin atribución (comportamiento previo). Sigue **sin** `derived_debt_line` (la fila sintética sin pareja de la v1.6-1.8 no vuelve). Response añade `avg_window: string`, `window_months: u32`, `months_with_data: u32` (meses con ≥1 movimiento de **cualquier** tipo, recurrentes incluidos — describe lo que hay, **no** es el denominador), `avg_months: u32` (**el denominador**), `avg_basis: {months, first_month, last_month, has_gaps}` (omitido ⟺ `avg_months == 0`; `has_gaps` impide etiquetar «abr–jun» una media de abril y junio) y `avg_unavailable_reason: "empty_window" \| "only_recurring_months"` (omitido cuando sí hay promedio). **API breaking, Fase 1 (issue #82) — un cero ya no puede confundirse con un hueco**: (a) la respuesta añade `actual_txn_count: i64` y `has_actual_data: bool` (movimientos del mes seleccionado, conciliadas fuera) porque `is_partial` dice si el mes civil ha terminado, **no** si tiene datos; (b) sin meses reales las medias ya **no son 0 sino `null`** — `CategoryComparisonLine.avg`, `BlockActualAvg.avg` y `totals.{expense,income,savings}_avg` pasan a `string \| null`, con `null ⟺ avg_months == 0`; (c) `delta_vs_budget` es `null ⟺ has_actual_data == false` y `delta_vs_avg` es `null` si falla cualquiera de los dos operandos. Los `actual` **no** se anulan: son mediciones (Σ∅ = 0); lo que no puede existir sin base es la comparación, que era la que afirmaba «vas muy por debajo de tu media» a quien no había importado nada. `avg_months` como campo de query sigue siendo el **alias legado** de `avg_window`; no confundir con el campo homónimo del response. Ya **no** trae `derived_debt_line`. 400: `year`/`month` fuera de rango o desapareados, `avg_window`/`avg_months` inválidos. → **200** `TransactionsSummaryResponse`. |
| POST | `/v1/transactions/import/preview` | write | **Stateless**, sin escrituras. Body `{source (auto\|myinvestor\|n26), file_b64, account_asset_id?}`. Autodetección por cabecera; dedup por huella (estado `new`/`already_imported`), heurísticas de transferencia y savings, matching de reglas. Devuelve `file_sha256` (a reenviar en confirm). 400: `csv_preset_unrecognized`, `csv_date_invalid`, `csv_amount_invalid`, base64 inválido. → **200** `ImportPreviewResponse`. |
| POST | `/v1/transactions/import/confirm` | write | Aplica el import. Body `{source, file_b64, file_sha256, decisions:[ImportDecision] (paralelo por índice a las filas), learn_rules=true, account_asset_id?, original_filename?}`. `file_sha256`/nº de filas deben coincidir con el preview → si no, 400 `preview_confirm_mismatch`. `decision.discard`/`force` por fila; solo la divisa base del hogar (`currency_mismatch`; configurable desde 3.10.0). `learn_rules` hace upsert de una regla por decisión categorizada. Lote vacío → cabecera borrada, `import_id: null`. Doble-confirm concurrente → **409**. Post-commit corre el **pase de auto-conciliación** sobre todo el dataset del owner (la contrapartida puede venir de un lote anterior) — best-effort, reportado en `reconciled_pairs` (0 si falló). → **200** `ImportConfirmResponse {import_id?, imported, skipped_already_imported, discarded, rules_learned, reconciled_pairs}`. |
| GET | `/v1/transactions/imports?view=` | lectura | Lotes de import (orden `created_at DESC, id DESC`), con `txn_count`, nombre de cuenta origen y **`possible_duplicate_of` (4.4.0, Fase 5)**: los otros lotes del mismo scope con el **mismo `original_filename` y la misma `account_asset_id`** — vacío en el caso normal, relación **simétrica** (si A lista a B, B lista a A). El doble import es el accidente clásico de esta pantalla: se sube el mismo extracto dos veces, la dedup por huella canónica salva los movimientos idénticos pero no los que difieren en un byte. El dato ya estaba en la respuesta —dos filas con el mismo nombre de fichero— pero exigía que el consumidor lo cruzara, y ninguno lo hacía. Es una **sospecha, no un veredicto** (`original_filename` NULL no agrupa; el mismo fichero en dos cuentas distintas no aparece; un extracto corregido sí). El cruce se hace **en Rust sobre la página ya cargada, sin query extra**, así que por la tool MCP paginada solo ve gemelos **dentro de la misma página** — precio deliberado de no meter un self-join en un listado. → **200** `[ImportBatchResponse]`. |
| DELETE | `/v1/transactions/imports/{id}?confirm=true` | write | Deshace un import (transacciones en cascada). `confirm` debe ser `true` → si no, 400 `confirm_required`. Guardia id+installation+owner → **404** si no es tuyo. → **204**. |
| PATCH | `/v1/transactions/{id}` | write | Edita una transacción (guardia owner → **404**). `op_date`/`amount`/`concept` son **editables en manuales e importadas** (ya no hay `immutable_field`). La diferencia está en la huella de dedup: en **manuales** se recomputa al cambiarlos (tomando un ordinal libre, liberando el anterior); en **importadas** queda **anclada** a la del CSV original y nunca se recomputa → un re-import del mismo archivo sigue detectando el duplicado pese a la edición. Campos `clear_*` para borrar opcionales. **Fase 1 (issue #82), API breaking**: poner y borrar el MISMO campo en la misma llamada es **400**, no un 200 con el `clear` ganando en silencio — `value_date_set_and_clear`, `category_set_and_clear`, `linked_asset_set_and_clear`, `linked_liability_set_and_clear`, `notes_set_and_clear` (mismo estilo por campo que el lote y que `cap_set_and_clear`, no el `rule_patch_conflict` de un código único). La guardia ya existía en el camino de lote y en el de reglas; el hueco era el PATCH individual, que comparten HTTP y la tool MCP `update_transaction`: un agente que armaba el patch desde plantilla creía recategorizar y dejaba el movimiento **sin categoría**, con los totales cuadrando y la atribución mintiendo. Huella duplicada tras recomputar (solo manuales) → **409**. **4.0.0 — mover la `op_date` de una INSTANCIA recurrente la DESVINCULA de su plantilla** (`recurring_rule_id = NULL`): antes el PATCH persistía y acto seguido la convergencia podaba la fila (su mes nuevo no era el de origen ni un mes activo, y el mes en curso nunca lo es), `load_txn` no la encontraba y la respuesta era un **500 sobre una mutación que sí había ocurrido**, con la edición reapareciendo revertida y con id nuevo. Desvincular es lo que la acción significa: deja de describir la recurrencia. → **200** `TransactionResponse`. |
| DELETE | `/v1/transactions/{id}` | write | Borra (guardia owner → **404**). → **204**. |
| GET | `/v1/transactions/rules` | lectura | Reglas de categorización del usuario (orden `updated_at DESC`). → **200** `[RuleResponse]`. |
| POST | `/v1/transactions/rules` | write | Crea regla. Body `{match_kind? (substring\|prefix\|exact), pattern, source?, assign_kind (requerido), assign_category_id?}`. `(source, pattern)` duplicado → **409 `rule_duplicate`**, tratando `source` ausente y `source` vacío como el mismo valor. Hasta 4.3.1 la promesa era falsa **sin `source`** (la constraint UNIQUE no atrapa `NULL`), que es el caso por defecto y el del reintento tras un timeout: dos llamadas idénticas creaban dos reglas contradictorias que luego «ganan por precedencia, no por acierto». Respaldo en BD: índice parcial `categorization_rules_unique_agnostic` (ver data-model.md). → **201** `RuleResponse`. |
| PATCH | `/v1/transactions/rules/{id}` | write | Edita (guardia owner → **404**). `clear_source`/`clear_assign_kind`/`clear_assign_category`. Colisión `(source, pattern)` → **409**. Desde 4.0.0 cuerpo vacío → **400** `rule_patch_empty`, y poner+borrar el mismo campo → **400** `rule_patch_conflict` (antes ganaba el `clear` en silencio). → **200** `RuleResponse`. Espejo MCP: `update_categorization_rule` (`patch_rule_core`). |
| DELETE | `/v1/transactions/rules/{id}` | write | Borra (guardia owner → **404**). **No descategoriza nada**: los movimientos conservan su categoría; la regla deja de aplicarse a imports futuros. → **204**. Espejo MCP: `delete_categorization_rule` (`delete_rule_core`, con preview/confirm). |
| GET | `/v1/transactions/recurring` | lectura | Reglas recurrentes del usuario (**plantillas**), orden `created_at DESC`. **Siempre own-user** (sin `?view`), como las reglas de categorización. Cada regla trae `category_name`. → **200** `[RecurringRuleResponse]`. |
| POST | `/v1/transactions/recurring/materialize` | write | **Pasada de convergencia bajo demanda** (3.9.0). Lleva las instancias recurrentes de la INSTALACIÓN al estado que definen las reglas: existen exactamente en los meses **activos** (mes civil cerrado con ≥1 movimiento real no conciliado) desde `origin_month`, y en ningún otro — creando lo que falta y **podando** lo que sobra. Cada instancia va fechada el **último día de su mes**; el mes en curso jamás se materializa. Idempotente **por existencia** (índice UNIQUE parcial), no por cursor: un CSV de un mes antiguo importado hoy sí lo materializa. La convergencia corre además post-commit tras cada mutación de transacciones, así que en régimen estacionario este endpoint es un no-op — sobrevive como red de auto-reparación. Huella `manual` + ordinal siguiente → **nunca 409**. Invalida la cache de proyección solo en modos B/C. Body vacío. → **200** `{rules_processed, materialized, pruned}` (`MaterializeResponse`). **El ámbito es la INSTALACIÓN entera**, no el usuario del token, y la convergencia **PODA** (de ahí `pruned`): por eso la tool MCP homónima declara `destructive_hint = true` desde 4.0.0. |
| DELETE | `/v1/transactions/recurring/{id}` | write | Borra la plantilla (guardia id+installation+owner → **404**). Las instancias ya materializadas **se conservan** (`transactions.recurring_rule_id` es `ON DELETE SET NULL` → quedan como movimientos manuales sueltos). → **204**. |
| POST | `/v1/transactions/reconcile` | write | **Pase explícito de auto-conciliación** (3.5.0) sobre TODO el dataset del owner: empareja importes exactamente opuestos, misma divisa, mismo owner, `\|Δop_date\| ≤ 5 días`, determinista (greedy por Δfecha con orden total) y de **punto fijo** (repetirlo → 0). Nunca re-empareja pares rechazados (`transfer_match_rejections`). Own-user, sin `?view`. Invalida cache COND solo si enlazó algo. → **200** `ReconcileRunResponse {pairs_created, transactions_reconciled}`. |
| POST | `/v1/transactions/{id}/reconcile` | write | **Conciliación manual de un par**: body `{counterpart_id}`. Exige importes exactamente opuestos y misma divisa (conciliar jamás altera el neto) pero **sin** ventana de fecha (SEPA lento, traspaso a caballo de dos meses). Borra un rechazo previo del par; idempotente si ya están conciliadas entre sí. Guardia owner → **404**. 400: `already_reconciled`, `transfer_amounts_not_opposite`, `transfer_currency_mismatch`, `transfer_same_transaction`. → **200** `ReconcilePairResponse {transaction, counterpart}`. |
| GET | `/v1/transactions/transfer-matches?window_days=&limit=` | lectura | **4.4.0 (Fase 6)** — pares **candidatos** a transferencia interna, **sin escribir nada**: es el preview del pase de conciliación. Hasta ahora la única forma de ver un par candidato era **ejecutar** el pase. **Regla dura: GET aparte, nunca un `?dry_run` sobre el POST** — un GET que muta ya costó caro en este repo (`purge_expired_liabilities`, §Reads never mutate) y un `dry_run` sobre el verbo que escribe es la misma puerta con otra etiqueta. **Sin `?view`** (la conciliación es own-user por construcción: las dos patas tienen que ser del mismo usuario, así que un `view` aquí inventaría un scope que la ruta no tiene). `window_days` 1..=365 def **30** — deliberadamente **más ancha** que los 5 del pase automático, porque el pase es de punto fijo y en una instalación sana no queda ni un par dentro de sus 5 días: lo que esta ruta vale son justo los pares que el pase **no** puede hacer solo (SEPA lento, traspaso a caballo de dos meses). Se **valida, no se clampa** (400 `window_days_out_of_range`); `limit` 1..=100 def 20. Cada sugerencia trae `match_id` (24 hex), `outgoing`/`incoming`, `day_gap`, `within_auto_window` y `ambiguous` (calculado **antes** del greedy, que es justo lo que lo esconde). La raíz publica `candidate_pair_count` (pre-greedy) y `rejected_pairs_excluded` (pares que cumplirían el criterio pero están en `transfer_match_rejections`). Rol: lectura (viewer incluido). Cache NONE. → **200** `TransferMatchSuggestionsResponse`. |
| POST | `/v1/transactions/transfer-matches/{match_id}` | write | **4.4.0 (Fase 6)** — concilia **un** par por el `match_id` que emitió el GET. Sin body y sin query. **El `match_id` no se persiste**: es `sha256("ffm1\|{installation}\|{owner}\|{id_menor}\|{id_mayor}")[..24]`, determinista sobre los ids ordenados y **deliberadamente no un UUID** (nombra una PROPUESTA del servidor, no una fila). El core lo resuelve **re-derivando el hash** sobre todos los candidatos de la ventana MÁXIMA (365 d, superconjunto de cualquier ventana pedible) y, si no aparece ahí, sobre los pares **ya conciliados entre sí** — de ahí que un reintento tras timeout devuelva 200 y no 404. **Esto es lo que cierra la omisión deliberada de `reconcile_pair` manual**: un par arbitrario **no es expresable en el esquema**, así que no hay barrera que saltarse — el input no existe. 404 `transfer_match_not_found`; 400 heredados de `reconcile_pair_core`: `already_reconciled`, `transfer_amounts_not_opposite`, `transfer_currency_mismatch` — **no** `transfer_same_transaction`, inalcanzable por esta vía (el `match_id` solo resuelve contra pares de dos filas distintas con importes opuestos), y por eso su `#[utoipa::path]` declara tres donde el `POST /{id}/reconcile` declara cuatro. Cache COND. → **200** `ReconcilePairResponse`. |
| DELETE | `/v1/transactions/{id}/reconcile` | write | **Desconcilia** el par de `{id}` (cualquiera de las dos patas) y **persiste el rechazo** — el pase automático no lo resucita. Ambas patas vuelven a contar en los agregados. Guardia owner → **404**. 400 `not_reconciled`. → **200** `ReconcilePairResponse` (ambas ya sueltas). |

**`PATCH /v1/transactions/batch` (3.8.0)** — reclasificación en lote, 1..=200 ids **propios**.
Conjunto de campos **cerrado**: `kind`, `category_id`/`clear_category`, `notes`/`clear_notes`. No
admite `amount`, `op_date`, `concept` ni `value_date`, y ese es justo el punto: ninguno de los
campos admitidos entra en la huella de dedup (`source · op_date · amount · concept`) ni en el
emparejado de transferencias (`op_date`, `amount`), así que el lote no recomputa huellas, no rompe
pares y no dispara el pase de auto-conciliación. El lote **clasifica**; para reescribir está el
PATCH de uno en uno. **Todo o nada** en una única transacción: un id ajeno o inexistente ⇒ 404
nombrando hasta 5 culpables y cero filas tocadas (un resultado parcial obligaría al llamante a
reconciliar estado, que es lo que un lote viene a evitar). **Una sola invalidación COND** al final,
fuera del bucle: 16 recategorizaciones seguidas en modo C tiraban la cache 16 veces. El 404 con
mensaje usa `ApiError::NotFoundWith`, variante nueva que solo nombra ids que el llamante ya envió.

**Backfill de reglas de categorización (3.8.0)** — `POST /v1/transactions/rules/{id}/apply`, y el
eje `apply_to_existing` (`none` default | `uncategorized` | `all`) + `from_month` + `confirm` en
`POST /v1/transactions/rules`. Crear una regla sigue afectando **solo a imports futuros**; aplicarla
al pasado es esta ruta.

- **Precedencia completa, no la regla suelta**: el backfill evalúa `match_rule` sobre el conjunto
  ENTERO de reglas y solo escribe las filas cuya ganadora es `{id}` — el pasado queda como habría
  quedado importando hoy. Las filas donde esta regla casa pero pierde salen en
  `matched_by_other_rule`.
- **`source` se respeta** (una regla de MyInvestor no toca movimientos manuales, igual que en el
  import) y las filas afectadas se reportan en `skipped_by_source`. Sin ese contador, un
  `matched: 0` se leería como «no hay nada que hacer» cuando en realidad es «esta regla no aplica a
  este origen» — el no-op invisible es el modo de fallo caro de este repo.
- **Las patas de transferencia conciliadas se excluyen** (`skipped_reconciled`): están fuera de
  todos los agregados de flujo, recategorizarlas no significa nada.
- **Una regla SIN `assign_kind` se puede PREVISUALIZAR, no aplicar** (Fase 1, issue #82). Ese estado
  es alcanzable desde el propio catálogo (`clear_assign_kind`), y hasta 4.3.1 el `dry_run` moría
  antes de mirar `dry_run` con `rule_not_applicable`: el preview de `delete_categorization_rule`
  **fallaba** mientras el borrado con `confirm: true` **funcionaba** — lo destructivo pasaba y lo
  seguro no. Ahora `dry_run` responde y `ApplyRuleOutcome` gana tres campos:
  `assigns_nothing: bool`, `shadowed_transactions: i64` y `note: string?`. La huella de una regla
  que no asigna nada **no es «cero movimientos»**: la regla sigue participando en la precedencia de
  `match_rule`, así que puede TAPAR a otra que sí asignaría (`suggest_kind_category` cae al default
  por signo). `shadowed_transactions` cuenta exactamente las filas donde esta regla gana Y otra se
  las habría llevado; retirarla deja que esas reglas actúen en los imports futuros. Aplicarla de
  verdad (`dry_run = false`) sigue siendo **400 `rule_not_applicable`**: no hay nada que escribir.
- **Cache COND, y solo si escribe**: cambiar el `kind` de filas históricas cambia
  `transactions_avg`, input del engine en B/C → `invalidate_projection_if_savings_uses_transactions`
  dentro de la core, condicionada a que haya filas afectadas. **Crear** la regla sigue siendo NONE.
  Los tres casos (crear / preview / backfill, en los tres modos) están en
  `applying_a_rule_invalidates_cond_but_creating_it_still_does_not`. `would_change_kind` en el
  preview es la señal explícita de que la proyección se moverá.
- Por HTTP, `apply_to_existing != "none"` sin `confirm: true` es un **400** (la SPA ya enseña el
  impacto antes de llamar); por MCP la tool devuelve el **preview**, patrón de la casa.
- No recalcula huellas ni toca la conciliación: `kind` y `category_id` no entran en la huella de
  dedup (`source · op_date · amount · concept`) ni en el emparejado (`op_date`, `amount`).

**Filtros de búsqueda de `GET /v1/transactions` (3.8.0)** — aditivos: omitidos, el comportamiento es
el de siempre byte a byte. Viven en `list_transactions_query`, así que HTTP y MCP devuelven los
mismos 400.

- `concept_contains` (1–200): subcadena del concepto, insensible a mayúsculas **y a tildes** —
  `cafe` encuentra `CAFÉ` y viceversa, la misma semántica que el matching de reglas de
  categorización. El plegado se replica en SQL con `translate()` sobre una tabla que incluye
  también `a-z → A-Z`, **no** con `upper()`: `upper()` depende de la collation del cluster (bajo `C`
  no toca los no-ASCII) y esta imagen ya cambió de collation una vez. Como el `concept` se almacena
  sin normalizar, la expresión colapsa además los runs de whitespace con `regexp_replace`. Las dos
  tablas —Rust y SQL— están pinneadas carácter a carácter por `sql_fold_tables_mirror_the_rust_fold`,
  que además barre el latín extendido comprobando que nada que Rust pliegue falte en la tabla SQL.
  Los comodines `%` y `_` del usuario se escapan (`LIKE … ESCAPE '\'`): sin eso, buscar `%`
  devolvería el conjunto entero. Nada de `unaccent`/`pg_trgm` — son extensiones y el Postgres va
  embebido en la imagen.
- `min_amount` / `max_amount`: sobre el importe **con signo**, que es la trampa más probable para un
  cliente. `max_amount=-50` son los gastos de 50 € o más; `min_amount=0`, solo entradas de dinero.
  Banda invertida → 400 explícito en vez de un conjunto vacío silencioso.
- `date_from` / `date_to`: `YYYY-MM-DD` **inclusivos en los dos extremos** («hasta el 31» incluye el
  31; un `<` exclusivo es el off-by-one-day clásico). **Excluyentes con `month`** → 400 si vienen
  juntos: son dos formas de decir lo mismo y cualquier precedencia implícita sería una trampa.
- **Índices**: ninguno nuevo. El scope entra por `transactions_installation_op_date_idx` (household,
  el default) o `transactions_owner_op_date_idx` (`view=mine`), y el `LIKE` se evalúa sobre el
  subconjunto ya acotado. Para el volumen de un hogar es irrelevante; si algún día duele, un GIN
  `pg_trgm` — que hoy no está instalado.

**Conciliación de transferencias — notas (3.5.0)**: un movimiento conciliado (`transfer_counterpart_id` presente en `TransactionResponse`, junto a `transfer_reconciled_at/source` y los denormalizados `transfer_counterpart_concept/op_date`) sigue **visible** en `GET /v1/transactions` y cuenta en `/months`, pero queda **excluido de todos los agregados de flujo**: totales/promedios del summary, `MIN(op_date)` de la ventana «Todo», serie por categoría, promedio real 12m del engine (modos B/C) y `months[]` de `/v1/history/cashflow`. **Asimetría deliberada del cashflow**: la curva `fine` **SÍ** incluye conciliadas — modela el saldo real de cada cuenta y excluirlas la haría divergir de los snapshots anclados (test `reconciled_excluded_from_months_but_not_from_fine_curve`). El pase automático corre post-commit tras toda mutación del conjunto (create/batch/patch de `amount`/`op_date`, delete, delete import, materialize, import confirm, import de backup). Un PATCH que cambia `amount`/`op_date` **rompe el par sin crear rechazo** (revertir el valor re-empareja); borrar una pata desconcilia la otra vía `ON DELETE SET NULL`. El flag `suggested_transfer` del preview de import queda como **hint informativo** (ya no implica descarte).

**Recurrencia — notas**: no hay `PATCH` de plantilla (para cambiarla, bórrala y recréala). Las copias mensuales se crean por dos vías, ambas transaccionales y con la misma semántica de **mes cerrado + fin de mes** (loop compartido `materialize_rule`): (a) el **backfill del alta** con `recurrence` (`POST /v1/transactions` o `/batch`), que rellena en el mismo commit los meses cerrados entre la `op_date` y el mes actual (cota 10 años → 422 `recurrence_too_old`); y (b) `POST /recurring/materialize`, para el avance de mes posterior (lo dispara el frontend al montar Movimientos; no hay cron). Ningún GET muta (los listados nunca generan instancias). El alta con `recurrence` además crea la regla-plantilla y deja enlazada la transacción de origen, que **conserva su `op_date` real** (solo las copias materializadas van a fin de mes).

### Changes (`GET /v1/changes`) — 4.4.0, Fase 6

Qué se ha creado o editado desde una fecha, leyendo los `updated_at` que **ya** se mantienen en
ocho tablas del ledger (`assets`, `liabilities`, `budget_entries`, `planning_flows`,
`transactions`, `recurring_transaction_rules`, `categorization_rules`, `history_snapshots`), con un
`UNION ALL` y orden `updated_at DESC, id ASC` — el desempate por `id` no es adorno: dos filas de la
misma transacción comparten `now()` al microsegundo. Handler `get_recent_changes`, core
`list_recent_changes_core` (`handlers/changes.rs`), montado con `.nest("/changes", changes_router())`.

Query: `?view=`, `since` (**obligatorio**; RFC 3339 o `YYYY-MM-DD` = medianoche UTC — ausente/vacío
→ 400 `date_required`, ilegible → 400 `date_invalid`) y `limit` (1..=500, def 100). Rol: lectura
(viewer incluido). Cache NONE. Sin tabla de auditoría y **sin migración**: es una vista sobre
columnas que ya estaban.

**Es deliberadamente la mitad honesta de una auditoría, y las tres carencias son campos de la
respuesta, no omisiones**:

- `covers_deletions: false` + `deletions_absent_reason: "no_tombstones"` — **no hay tombstones**, así
  que un borrado es indistinguible de «nunca existió». Venderlo como auditoría sin esta nota sería
  mentir.
- `tables_missing_updated_at: ["categories", "allocation_rules"]` — quedan fuera **porque no tienen
  la columna**, no por criterio.
- `tables_covered` (las 8), `item_count` / `items_included` / `truncated`, y `now` — el `since` del
  siguiente sondeo, servido para que el cliente no tenga que fabricarlo con su propio reloj.

Cada `RecentChange`: `entity` (`asset` | `liability` | `budget_entry` | `planning_flow` |
`transaction` | `recurring_rule` | `categorization_rule` | `snapshot`), `id`, `label`, `change`
(`created` si `created_at >= since`, si no `updated`), `created_at`, `updated_at` y `owner_user_id`
(omitido si `None`). La etiqueta de `budget_entries` sale de su categoría por subquery: esa tabla no
tiene columna de nombre. **Ningún campo es un `Decimal`** — es un índice de cambios, no un extracto.

Regresión: `apps/api/tests/recent_changes.rs` (4 tests, uno de ellos —
`an_edit_reads_as_updated_and_a_deletion_leaves_no_trace` — dedicado a que la carencia declarada sea
exactamente la carencia real). Paridad MCP **ignorando `now`**, que cambia entre dos llamadas por
diseño: `mcp_http.rs::recent_changes_tool_matches_the_endpoint_except_for_now`.


## Auth pattern in handlers

Every protected handler calls:
```rust
let user = require_session_user(&jar, &state.pool).await?;
let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
// For write ops:
if !role_can_write(role.as_str()) { return Err(ApiError::Forbidden); }
```

## View-scoping pattern

For any endpoint that accepts `?view=mine`, **do not** write two `match view { Household => sqlx::query_as("…installation_id = $1…"), Mine => sqlx::query_as("…installation_id = $1 AND owner_user_id = $2…") }` branches. Use the helpers in `handlers/person_view.rs`:

**Desde 4.0.0 `resolve()` es falible.** Valores aceptados: `mine`, `household`, ausente o vacío. Cualquier otro → `400 invalid_view`. Antes el brazo comodín devolvía **household** (el hogar entero) en silencio, así que un cliente que escribiera `"MINE"` recibía datos de otros miembros creyendo haber pedido los suyos (auditoría MCP §4). No era un fallo de autorización — D2 sigue vigente: cualquier miembro puede pedir `household` a la cara — pero sí una respuesta sobre otra población que la pedida. **No reimplementes el parseo**: `projection.rs` tenía su propia copia del `match` y por eso se le escapó el arreglo; ahora delega como todos. Misma clase, arreglados a la vez: `resolution` de `/v1/history/cashflow` (`invalid_resolution`) y `density` de `/v1/projection/series` (`invalid_density`). Regresión: `apps/api/tests/query_param_validation.rs`.

```rust
let view = q.resolve()?; // Query<LedgerViewQuery> — falible desde 4.0.0
let scope = view.scope_where("a"); // table alias optional; "" = no prefix
let today_ph = view.next_arg_index(); // 2 (Household) or 3 (Mine)
let sql = format!(
    "SELECT ... FROM assets a WHERE {scope} AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph}) ORDER BY ...",
);
let rows: Vec<MyRow> = view
    .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
    .bind(today)
    .fetch_all(pool)
    .await?;
```

For `sqlx::query_scalar`, use `bind_scope_scalar` instead. The helpers guarantee placeholder order ($1=iid, optional $2=owner_user_id) so the two branches can never drift.

**Y ecoa la vista aplicada (4.4.0, Fase 5).** Toda respuesta cuyo contenido dependa del scope lleva
un campo `view` con `"household"` | `"mine"`. El eco se escribe **siempre** con
`LedgerView::as_str()`, nunca a mano: antes vivía como `if view == LedgerView::Mine { "mine" } else
{ "household" }` copiado en cuatro handlers, y ese brazo `else` habría convertido cualquier variante
nueva del enum en `"household"` en silencio — la misma forma del comodín que `resolve()` eliminó en
4.0.0. `resolve(as_str(v)) == v` para las dos variantes (unit test
`as_str_round_trips_through_resolve`), así que reenviar el valor ecoado como `?view=` reproduce
exactamente la misma respuesta.

Lo pone la **core** (y por tanto lo tienen el GET HTTP y la tool MCP a la vez) en `/v1/summary`,
`/v1/budget`, `/v1/projection/series`, `/v1/allocation-rules/resolution`, `/v1/history/series`,
`/v1/history/cashflow`, `/v1/transactions/summary` y `/v1/transactions/category-series`. Lo pone la
**tool MCP**, en un sobre, en los listados, porque su GET devuelve un array desnudo a propósito
(ver §MCP → «Paridad de los listados»). El incidente que lo forzó: en una instalación de un solo
usuario, `?view=mine` y omitirlo devolvían payloads **byte a byte idénticos** — imposible
distinguir «mine coincide con el hogar» de «el parámetro se ignoró», y en un hogar de dos personas
ésa es exactamente la pregunta que decide si la cifra que estás citando es la del hogar o la tuya.
Nada de esto es una frontera de autorización: sigue siendo un filtro, como dice el párrafo de
arriba.

## Error mapping

### Wire shape (3.10.0)

Every non-2xx response carries three fields, not two:

```json
{ "error": "conflict", "code": "username_taken",
  "message": "username_taken: that username is already registered" }
```

- **`error`** — HTTP class (`bad_request`, `unprocessable`, `unauthorized`, `forbidden`,
  `not_found`, `conflict`, `unavailable`, `internal`). Published since 1.0.0, unchanged.
- **`code`** — *stable, granular* identifier. `derive_error_code` takes it from the message's
  `snake_code: ` prefix (3–64 chars, `[a-z][a-z0-9_]*`); without a valid prefix it falls back to
  the HTTP class. **This is what clients branch on.**
- **`message`** — English technical detail, for developers. The SPA never shows it as the primary
  sentence: it translates `code` via `apps/web/src/lib/errorMessages.ts` and keeps `message` for
  the console / a folded «Detalles técnicos».

Adding a coded error = prefix the message. `ApiError::BadRequest("swr_out_of_range: swr_pct must
be between 0 and 4".into())`. Two variants exist purely to carry a code where the plain one could
not: `NotFoundWith` (404 with body) and `ConflictWith` (409 with body — the bare `Conflict` comes
from the automatic 23505 mapping and cannot know *what* collided).

**Gate**: `apps/api/tests/error_codes_parity.rs` extracts every code from the source into
`tests/fixtures/error-codes.json`, and `apps/web/src/lib/errorMessages.test.ts` fails if any lacks
Spanish copy. Regenerate with
`UPDATE_ERROR_CODES=1 cargo test -p futurefin-api --test error_codes_parity`.

The OAuth endpoints (`/oauth/*`) do **not** use this shape: they emit RFC 6749 §5.2
`{error, error_description}` with the protocol's own codes.

### SQLSTATE

`impl From<sqlx::Error> for ApiError` (in `error.rs`) auto-detects:
- `23505` (unique_violation) → `ApiError::Conflict` (409)
- `23503` (foreign_key_violation) → `ApiError::BadRequest("referenced record missing")`
- `22003` (numeric_value_out_of_range, 4.4.0 — issue #82) → `ApiError::BadRequest` con código
  `amount_out_of_range`. Las columnas de dinero son `NUMERIC(18,4)` (14 dígitos enteros + 4
  decimales); un importe que las desborda en el INSERT caía antes a `Db(_)` → 500 «internal error»
  pelado, el ÚNICO error de toda la superficie que un cliente no podía clasificar por código — y
  justo el que dispara las políticas de retry-on-5xx contra una entrada que nunca va a ser válida
  (un agente desatendido entraba en bucle). Es 400 porque el problema es la entrada, no el
  servidor; no deja escritura parcial (el fallo ocurre en el propio INSERT). Un valor cabe en
  `rust_decimal::Decimal` (así que el parseo del body lo acepta) y aun así desbordar la columna —
  el fallo es de BD, no de deserialización. Test: `unique_violation.rs::absurd_amount_returns_400_amount_out_of_range_not_500`.

Handlers should just `?` any `sqlx::Error`; never write per-call `.map_err(...)` to translate codes
— **except** when the handler knows which unique index collided and can say so
(`handlers/auth.rs` register → `username_taken`).

## MCP (`/mcp`, Streamable HTTP)

**Movida a [`mcp-catalog.md`](mcp-catalog.md)** (2026-08-30, consolidación): el catálogo por tool, los sobres de listado y el transporte de `/mcp` viven allí; aquí solo quedan las rutas HTTP.

## OAuth 2.1 (v3.1.0)

Authorization server **embebido** en el mismo binario y puerto, módulo `apps/api/src/oauth/`
(protocolo) + `apps/api/src/handlers/oauth_consent.rs` (pantalla de consentimiento y panel). Existe
para una sola cosa: que el conector de claude.ai web pueda hablar con `/mcp`, que exige OAuth 2.1 y
no acepta un Bearer pegado a mano. FutureFin es a la vez **authorization server y resource server**
— no hay IdP externo, ni claves de firma, ni JWT. Regresión completa: `apps/api/tests/oauth_flow.rs`.

### Rutas de protocolo (nivel raíz, **fuera de OpenAPI**)

| Method | Path | Notas |
|--------|------|-------|
| GET | `/.well-known/oauth-protected-resource[/mcp]` | RFC 9728. `{resource: "{base}/mcp", authorization_servers: [base], bearer_methods_supported: ["header"]}`. Sin SELECT y sin mutación: solo refleja la URL pública. **`Cache-Control: no-store` + `Vary: X-Forwarded-Proto, X-Forwarded-Host`** (4.4.0 — ver §Envenenamiento del issuer). |
| GET | `/.well-known/oauth-authorization-server[/mcp]` | RFC 8414. `issuer`, `authorization_endpoint` (`{base}/oauth/authorize`), `token_endpoint`, `registration_endpoint`, `revocation_endpoint`, `code_challenge_methods_supported: ["S256"]` (único), `grant_types_supported: [authorization_code, refresh_token]`, `authorization_response_iss_parameter_supported: true`. Mismas cabeceras de caché que la anterior (`no-store` + `Vary`). **Sin `scopes_supported`, y sigue así tras la Fase 3 (issue #84)** — aunque el argumento original («no hay scopes con función») ya no es literal: los tokens de API sí tienen `scope` (`api_tokens.scope`, ver §API tokens y `auth-and-membership.md`). El motivo por el que NO se extiende a OAuth es distinto: un scope solo restringe **si lo elige la persona** — en un token de API lo elige ella misma con cookie de sesión; en OAuth el `scope` del authorization request lo elige la **aplicación cliente**, así que anunciarlo sin una pantalla de consentimiento que lo recorte no restringiría nada, solo mentiría en la metadata. El techo de una conexión OAuth sigue siendo el rol vivo del usuario + el toggle `installation.mcp_write_enabled`, comprobados por request. |
| POST | `/oauth/register` | DCR (RFC 7591), **público y sin autenticación**. Body `{redirect_uris (1..5, requerido), client_name?, client_uri?, token_endpoint_auth_method?, grant_types?, response_types?}` → **201** `{client_id ("ffc_…"), client_id_issued_at, client_secret? ("ffcs_…"), client_secret_expires_at? (0 = no caduca), …}`. `token_endpoint_auth_method` omitido ⇒ `client_secret_basic` (default RFC 7591 §2) y se emite secreto; `none` ⇒ cliente público sin secreto (el caso de claude.ai). Errores `invalid_client_metadata` / `invalid_redirect_uri`. |
| POST | `/oauth/token` | `grant_type=authorization_code` (PKCE **S256 obligatorio**) o `grant_type=refresh_token` (rotación). Form-urlencoded. → `{access_token ("ffo_…"), token_type: "Bearer", expires_in: 3600, refresh_token ("ffr_…"), scope?}` + `Cache-Control: no-store`. |
| POST | `/oauth/revoke` | RFC 7009. Un `ffr_…` revoca el **grant entero** (§2.1: "desconectar" en claude corta todo); un `ffo_…` revoca solo su fila. Token desconocido → **200** igualmente (§2.2). |

**`GET /oauth/authorize` NO se registra en el backend — prohibido.** La sirve el fallback SPA
(`ServeDir(...).fallback(ServeFile(index.html))` de `main.rs`), porque la pantalla de consentimiento
es React. Si registraras cualquier método en ese path, axum devolvería **405** en los demás y un
method-mismatch **no cae al fallback**: mataría la pantalla en producción. Fijado por el test
`get_oauth_authorize_is_not_handled_by_the_api` y por el comentario de cabecera de `oauth/mod.rs`.

### Endpoints de la SPA (`/v1/oauth/*`, **sí en OpenAPI**) — handler `oauth_consent.rs`

| Method | Path | Auth | Notas |
|--------|------|------|-------|
| GET | `/v1/oauth/authorize-details` | **pública** (cookie opcional) | Valida los parámetros del authorization request y devuelve qué pintar. **Sin sesión a propósito**: solo devuelve metadata que el propio cliente registró (`client_name` — texto NO verificado —, `client_uri`, `redirect_host` — el único dato verificado —, `resource`), nada del usuario; a cambio, un `redirect_uri` que no cuadra se ve **antes** de teclear la contraseña. Con cookie válida añade `already_connected` / `connected_at`. `status` ∈ `consent` \| `invalid_request` (fatal: pintar el error, **jamás** redirigir) \| `redirect_error` (navegar a `redirect_to`). |
| POST | `/v1/oauth/authorize` | cookie + `require_installation_member` | Body `{approve: bool, …params del authorize (flatten)}` → **200** `{redirect_to}`, la URL a la que la SPA navega. Approve → `code` + `state` (eco literal) + `iss` (RFC 9207); deny → `error=access_denied` al redirect registrado (no dejar al cliente colgado). Error fatal → **400** `authorize_error: <code>`. 401 sin sesión, 403 si pending. |
| GET | `/v1/oauth/connections` | cookie + membership | Conexiones activas **del caller** (`oauth_grants` no revocados), orden `created_at DESC`: `{id, client_name, client_uri?, redirect_host?, created_at, last_used_at?}`. |
| DELETE | `/v1/oauth/connections/{id}` | cookie + membership | Soft-revoke (`revoked_at = now()`, `revoked_reason = 'user_panel'`) → **204**; corte inmediato. Solo grants propios: un id ajeno devuelve el mismo **404** que uno inexistente (no revela existencia). |

- **CSRF del POST, por partida doble**: la cookie es `SameSite=Lax` (un POST cross-site no la lleva)
  y el body es JSON, que no es un "simple request" → exige preflight, que la lista blanca CORS
  bloquea. **No cambies el body a form-urlencoded** (perderías la segunda mitad).
- **La validación del authorize vive UNA vez**, en `oauth::authorize::validate_authorize_params`, y
  la consumen los dos endpoints. Nunca dupliques esas reglas en un handler. La distinción crítica
  (OAuth 2.1 §7.12.2) es `AuthorizeParamError::Fatal` (client_id desconocido o `redirect_uri` sin
  match exacto → **no se puede redirigir**, sería un open redirect) vs `Redirectable`
  (`response_type`/PKCE/`resource` malos con cliente y redirect ya validados → error al
  `redirect_uri` registrado). El match del `redirect_uri` es de **string completa**, ni prefijo ni
  solo host.

### Contrato de tokens

| Credencial | Prefijo | Persistencia | TTL |
|---|---|---|---|
| `client_id` | `ffc_` | claro (no es secreto) | — |
| client secret | `ffcs_` | **solo** SHA-256 hex (`oauth_clients.client_secret_hash`) | no caduca (`client_secret_expires_at: 0`) |
| authorization code | *(sin prefijo)* | **solo** SHA-256 hex (PK `oauth_authorization_codes.code_hash`) | **2 min**, un solo uso |
| access token | `ffo_` | **solo** SHA-256 hex (`token_hash` UNIQUE) | **1 h** (`expires_in: 3600`) |
| refresh token | `ffr_` | **solo** SHA-256 hex (`token_hash` UNIQUE) | **90 días sin uso** (sliding: cada rotación emite uno nuevo con 90 días) |

- **Todos opacos y hash-only** — mismo contrato que `api_tokens` (`auth/secret.rs`:
  `generate_opaque_secret` = prefijo + 43 chars base64url de 32 bytes `OsRng`, `sha256_hex`,
  `generate_opaque_id` para `client_id`). Lookup O(1) por hash exacto, cero comparación de secretos
  en Rust. Nada se congela en el token: rol e installation se re-resuelven vivos en cada request.
- **Las expiries las calcula Postgres**, nunca Rust (`now() + $n::interval`).
- **El grant es la unidad de todo** (`oauth_grants`: una fila por app+usuario). Es lo que ve y
  revoca el panel, y lo que los `access.rs`/`token.rs` exigen vivo por JOIN → revocar una fila corta
  todos los tokens de esa app sin tocarlos, igual que borrar una sesión.
- **Rotación + reuse-detection**: cada canje de refresh consume el actual (`consumed_at`), emite uno
  nuevo y los encadena (`replaced_by`, auditoría de la rotación). Presentar un code o un refresh **ya
  consumido** es la señal de robo → se revoca el **grant entero** (OAuth 2.1 §4.3.1/§7.5), con
  `revoked_reason ∈ {code_reuse, refresh_token_reuse}`. Los dos grant types corren en una
  transacción con `FOR UPDATE` sobre la fila de la credencial.
- **Anti-flood del registro abierto**: `POST /oauth/register` hace GC perezoso (borra clientes de
  >24 h **sin ningún grant** — jamás uno consentido) y corta con `503 temporarily_unavailable` si
  quedan ≥1000 clientes. El GC vive en el POST y no en un GET (D5, reads never mutate).
- **GC de credenciales caducadas (4.4.0, issue #85)**: hasta 4.3.1 **no existía ni un `DELETE`**
  sobre `oauth_access_tokens`, `oauth_refresh_tokens` ni `oauth_authorization_codes`; cada rotación
  de refresh insertaba dos filas que no se borraban jamás. No tumba un self-host, pero engorda para
  siempre los dumps pre-migración y los índices `UNIQUE` de `token_hash`. `token::gc_expired_tokens`
  poda en `POST /oauth/token` —el camino que **hace crecer** las tablas, autorregulado: quien no
  emite tokens no crece— y **jamás en un GET** (D5), mismo patrón que `gc_orphan_clients`. Es
  best-effort y las tres sentencias son independientes: un fallo se loguea y no convierte un token
  ya emitido en un 5xx. **Las gracias no son «por si acaso»**: 1 día para codes y access (TTL 2 min
  y 1 h), **30 días para refresh** (TTL 90 días) — la reuse-detection mira `consumed_at` **antes**
  que la expiración, así que un refresh consumido sigue matando el grant aunque haya caducado y
  necesita la fila viva. `replaced_by` es `ON DELETE SET NULL`: podar un eslabón viejo de la cadena
  de rotación no rompe el que lo sucede. `oauth_refresh_tokens` no tiene índice por `expires_at`
  (las otras dos sí) — es un seq scan sobre una tabla que este mismo GC mantiene pequeña, y el
  índice costaría una migración que no compra nada a escala de self-host. Regresión:
  `oauth_flow.rs::expired_oauth_credentials_are_collected_on_the_next_token_request`.

### Formato de error — **no es `ApiError`**

Las rutas de protocolo devuelven `OAuthError` (`oauth/error.rs`): JSON
`{"error": "...", "error_description": "..."}` de RFC 6749 §5.2, no el `{error, code, message}` del API
propio, porque el body y los códigos (`invalid_request`, `invalid_client`, `invalid_grant`,
`invalid_target`, `unsupported_grant_type`, `invalid_client_metadata`, `invalid_redirect_uri`,
`server_error`, `temporarily_unavailable`) los fija la RFC. Toda respuesta lleva
`Cache-Control: no-store`. `invalid_client` es **siempre 401** (nunca 400): es la señal exacta con la
que claude.ai re-registra el cliente vía DCR — gracias a ella un restore de backup sin tablas OAuth
se auto-recupera sin intervención; ese 401 añade `WWW-Authenticate: Basic realm="FutureFin"`. Los
`/v1/oauth/*` sí hablan `ApiError` normal (son API propio). `oauth::access::require_oauth_access_token`
devuelve `ApiError` a propósito: alimenta al middleware de `/mcp`.

### Por qué el protocolo está fuera de OpenAPI

Igual que `/mcp`: su contrato lo fijan las RFC (8414/9728/7591/7009 + la spec de autorización MCP) y
los clientes lo descubren por los documentos `.well-known`, no por nuestro esquema. Duplicarlo en
`utoipa` solo crearía deriva. Los **cuatro** endpoints de la SPA sí están anotados
(`__path_authorize_details`, `__path_authorize_decision`, `__path_list_connections`,
`__path_revoke_connection`, tag `oauth`, en `openapi.rs`) porque son API propio.

### Kill-switch — el switch cambia el handler, no la tabla de rutas (4.4.0, issue #85)

`FUTUREFIN_MCP_ENABLED=0` apaga `/mcp` **y** el protocolo OAuth entero — hoy OAuth no sirve a nada
más que a MCP, de ahí el interruptor compartido. Lo que cambió en 4.4.0 es **cómo**:

| Superficie | Con el switch echado | Dónde |
|---|---|---|
| `/mcp` | ruta montada, **404 JSON `mcp_disabled`**, cualquier método | `mcp::mcp_router` → `mcp_disabled` |
| Las **7** rutas de protocolo (`/.well-known/oauth-{protected-resource,authorization-server}[/mcp]`, `/oauth/{register,token,revoke}`) | rutas montadas, **404 JSON `mcp_disabled`**, cualquier método | `oauth::oauth_protocol_router(false)` → `oauth_disabled` |
| `/v1/oauth/authorize-details`, `POST /v1/oauth/authorize` | **no se montan** → 404 JSON `not_found` del fallback de `/v1` | `oauth_consent_router(mcp_enabled)` |
| `GET/DELETE /v1/oauth/connections[/{id}]` | **se montan SIEMPRE** | idem |

- **Por qué las rutas ya no se desmontan.** Hasta 4.3.1 `oauth_protocol_router()` y `mcp_router()`
  simplemente no se construían, y las rutas caían al fallback. Eso solo se veía mal **en la imagen
  publicada**, no en los tests: el fallback final es un `ServeDir` con fallback al `index.html`, y
  **`ServeDir` no llama a su fallback para métodos distintos de GET/HEAD**. El resultado real era
  `POST /mcp` → **405 con cuerpo vacío** y `GET /.well-known/oauth-authorization-server` →
  **200 `text/html`** con el shell de la SPA. El conector fallaba al parsear JSON y enseñaba
  «connection failed» sin causa: **un control de seguridad que, al activarse, se diagnostica como
  avería**. El test antiguo montaba el router *sin* SPA, así que confirmaba un 404 que en producción
  no ocurría — describía un binario de laboratorio.
- **Doctrina**: la forma del router no depende del entorno (D18, la misma por la que
  `POST /v1/auth/sso` se monta siempre y responde `sso_disabled`). Un apagado se anuncia; no se
  disfraza de ruta inexistente.
- **Un solo código para un solo interruptor**: `mcp_disabled`, literal completo en
  `mcp::MCP_DISABLED_MESSAGE` y reutilizado por `oauth/mod.rs`. La causa es literalmente la misma
  variable; dos códigos obligarían a traducir dos frases que dicen lo mismo. Está en
  `errorMessages.ts` y en `fixtures/error-codes.json`.
- **`/v1/oauth/connections` sigue montado siempre** — precedente de `/v1/api-tokens`: con MCP
  apagado sigues pudiendo *ver y revocar* credenciales que ya existen. Sus dos compañeros de flujo
  sí se desmontan, y ahí está bien: viven bajo `/v1`, cuyo fallback **ya** devolvía JSON.
- Regresión: `mcp_http.rs::mcp_disabled_answers_json_even_with_the_spa_mounted` y
  `oauth_flow.rs::oauth_protocol_disabled_with_mcp_but_connections_panel_survives`, los dos ahora
  con `WEB_STATIC_ROOT` montado por `spa::mount_static_spa` —la **misma** función que llama
  `main.rs`— y con un assert previo de que el `ServeDir` está de verdad ahí.

### El challenge del 401 de `/mcp` — solo el 401

Cuando `/mcp` rechaza por credencial (**401**), el middleware añade
`WWW-Authenticate: Bearer realm="FutureFin", resource_metadata="{base}/.well-known/oauth-protected-resource/mcp"`
(RFC 9728 §5.1) para que un cliente OAuth descubra el authorization server. **Un 403 nunca lo
lleva**: un usuario pending o con membership revocada recibiría el challenge, se re-autenticaría,
obtendría un token nuevo y volvería a comer el mismo 403 — bucle infinito. Si la URL pública no se
puede derivar, el header degrada a `Bearer` a secas.

### URL pública (issuer)

`oauth/url.rs::public_base_url` — `FUTUREFIN_PUBLIC_URL` si está fijada; si no, se **deriva del
request**: `X-Forwarded-Proto`/`X-Forwarded-Host` (primer valor de cada uno) o el header `Host`, con
un charset estricto (`host[:puerto]`, IPv6 entre corchetes; sin `/`, `@`, espacios ni controles;
≤255 chars) → si no cuadra, **400 `invalid_request`**. Sin `Host` tampoco hay issuer. Así producción
sigue sin requerir ninguna env var (promesa 3.0.0). Ver [`env-and-config.md`](env-and-config.md).
Los redirects se construyen **siempre** con `oauth::url::append_query` (escaping de `url::Url`) —
concatenar a mano es donde nacen los open redirect.

#### Envenenamiento del issuer: `X-Forwarded-Host` se honra sin peer de confianza, a propósito

Esto **contradice en apariencia** la asimetría de D17/D18, que exigen peer de confianza para relajar
el anti-clickjacking y para aceptar identidad. No es una excepción, es la misma regla: aquellas
cabeceras **conceden autoridad**; esta solo se **refleja**. Un `X-Forwarded-Host` falsificado deforma
la respuesta del propio atacante y de nadie más — el argumento literal que `prefix.rs` da para no
exigirle peer al prefijo.

Lo único que convertía esa reflexión en algo más era la **cacheabilidad**: un caché intermedio podía
servirle a otro el issuer derivado de un `X-Forwarded-Host` inyectado. Desde 4.4.0 las dos metadatas
salen con **`Cache-Control: no-store`** y **`Vary: X-Forwarded-Proto, X-Forwarded-Host`**
(`oauth/metadata.rs::discovery_response`), y con eso el vector se cierra. `Host` no entra en el
`Vary`: los cachés ya distinguen por host de forma implícita.

> Ojo con la afirmación que este documento hacía antes: «toda respuesta OAuth lleva
> `Cache-Control: no-store`». Era cierta para `OAuthError`, para el token endpoint y para la
> revocación, y **falsa justo para la metadata** — la única que depende de cabeceras de proxy. Ahora
> es cierta, y la fija `oauth_flow.rs::discovery_metadata_is_never_cached_and_varies_on_the_forwarded_headers`.

Exigir peer de confianza aquí, en cambio, sería fail-closed contra el caso mayoritario: rompería
cualquier despliegue corriente (Cloudflare Tunnel, nginx) en cuanto el operador no configurase
además `FUTUREFIN_TRUSTED_PROXY_IPS`, una variable que nació para el add-on de Home Assistant.

#### El issuer NO se deriva del prefijo del request — se **declara** (4.4.0, issue #85)

`public_base_url` **no** consulta `crate::prefix` ni `AppState::base_path`. Bajo un proxy con
subpath (`location /futurefin/` en nginx) el issuer tiene que llevar el prefijo, o el cliente
descubre URLs que el proxy no enruta; hasta 4.3.1 **ninguna configuración lo conseguía** —
`FUTUREFIN_PUBLIC_URL` hacía `panic!` con cualquier path, y el prefijo del request no entraba en el
issuer. Era un despliegue que `prefix.rs` documenta como soportado, con el OAuth roto sin salida.

**La salida es declararlo**: `FUTUREFIN_PUBLIC_URL=https://ejemplo.com/futurefin`. De ahí cuelgan el
`issuer`, el `resource` de RFC 8707 y los cuatro endpoints anunciados; el path se valida con
`prefix::normalize_prefix`, la **misma** función ya probada que valida `FUTUREFIN_BASE_PATH`.
Regresión: `oauth_flow.rs::public_url_with_a_subpath_prefixes_every_advertised_url`, que recorre el
flujo entero (metadata → challenge del 401 → `resource` aceptado y el sin prefijo rechazado con
`invalid_target` → canje → `initialize` con el access token).

**Por qué NO se compone con `prefix::request_prefix`**, que era la otra opción:

1. **El issuer es una identidad, no una decoración.** `prefix.rs` no le exige peer al prefijo porque
   un `X-Forwarded-Prefix` falsificado solo deforma los assets de la respuesta del atacante; en
   cuanto ese mismo texto entra en un **documento de descubrimiento**, deja de ser inocuo por la
   misma razón por la que existe la sección anterior. Un valor de operador (fail-loud al arrancar)
   no lo puede mover una cabecera.
2. **Bajo el Ingress de Home Assistant el prefijo es `/api/hassio_ingress/<token>`, un token
   efímero de sesión**: componerlo lo hornearía dentro del issuer. Y el Ingress no es este caso —
   el add-on documenta que MCP/OAuth van por el **puerto directo**, no por la URL de ingress.

Con prefijo en la request y sin `FUTUREFIN_PUBLIC_URL` el issuer sale sin prefijo; eso no se
adivina, **se avisa**: `warn_missing_public_url_for_prefix` emite un `warn` **una vez por proceso**
diciendo exactamente qué variable falta. Sin esa línea el síntoma era un 404 mudo en `/oauth/token`
que no dice de dónde viene.

> La SPA y toda la API `/v1` funcionan bajo subpath sin declarar nada: lo que resuelve el navegador
> pasa por `apiUrl` (§Prefijo público). Lo que necesita la declaración es **solo** el issuer OAuth.

### Prefijo público de la request (`apps/api/src/prefix.rs`)

El servidor monta **todas** sus rutas en la raíz, siempre. Los proxies con subpath (el Ingress de
Home Assistant, un `location /futurefin/` de nginx) **quitan el prefijo** antes de entregar la
petición, así que el router no lo ve nunca. Lo que sí depende del prefijo es lo que resuelve el
**navegador**: los refs del HTML, las URLs de `fetch`/`pushState` y el `Path` de la cookie de
sesión.

`prefix::request_prefix(base_path, headers)` decide el prefijo efectivo de cada request, con esta
**precedencia**:

1. **`X-Ingress-Path`** — lo pone el Supervisor de Home Assistant (`/api/hassio_ingress/<token>`).
2. **`X-Forwarded-Prefix`** — el header genérico de nginx / Traefik / Caddy.
3. **`FUTUREFIN_BASE_PATH`** — prefijo fijo del despliegue, validado al arrancar.
4. **`""`** — raíz. El caso de siempre.

Un header **presente pero inválido no aborta**: se ignora (con un `warn` deduplicado, tope de 8
valores distintos para que nadie convierta el log en un canal de flood) y se sigue con la fuente
siguiente. `normalize_prefix` acepta `/` o vacío (⇒ `""`), o un path que empieza por `/`, sin `//`,
sin segmentos `.`/`..`, charset `[A-Za-z0-9._~/-]`, ≤128 chars, tolerando una barra final que
recorta. Ese charset es también lo que hace seguro interpolarlo en atributos HTML y en JS
(`spa::inject`): no hay comillas, ángulos ni backslash que escapar.

**La detección NO exige peer de confianza, a propósito**: un `X-Forwarded-Prefix` falsificado solo
deforma la respuesta del propio atacante (assets que no cargan). Lo que **sí** exige peer de
confianza es relajar el anti-clickjacking y aceptar identidad por cabeceras.

`PeerPolicy` (`FUTUREFIN_TRUSTED_PROXY_IPS`) es esa política: `Disabled` (sin definir — nadie es de
confianza, el default seguro), `Any` (`any`: todo peer, para tests y redes privadas donde el proxy
es el único camino al proceso) o `List` (IPs separadas por comas; el add-on usa `172.30.32.2`). La
IP del peer llega por el extractor infalible `PeerIp`, que la lee de `ConnectInfo<SocketAddr>`
— por eso `main.rs` sirve con `into_make_service_with_connect_info`. Un peer desconocido (`None`,
p.ej. los tests con `oneshot`) solo pasa con `Any`.

Regresión: `apps/api/tests/base_path.rs`, `frame_options.rs`, `session_cookie_path.rs` + los unit
tests de `prefix.rs`. Variables y bounds: [`env-and-config.md`](env-and-config.md).

### Anti-clickjacking — condicionado al peer (`handlers/frame.rs`)

La invariante histórica era absoluta: nada de FutureFin se embebe en iframes, implementada como un
`SetResponseHeaderLayer::overriding(X_FRAME_OPTIONS, "DENY")` fijo. La enmienda: el **Ingress de
Home Assistant pinta el add-on dentro de un iframe del mismo origen** que HA, y con `DENY` la app
sale en blanco.

El layer suelto se sustituye por `frame::with_frame_policy(router, state)` — un middleware
`from_fn_with_state` que envuelve el **router final** (API + fallback SPA), no el sub-router `api`:
la pantalla de consentimiento OAuth la sirve el fallback y es justo la que había que proteger. Se
expone como «envuelve este router» y no como un `Layer` suelto porque el tipo que devuelve
`from_fn_with_state` no es nombrable; así `main.rs` y el `TestApp` montan exactamente lo mismo.

La regla, exacta:

| Peer de confianza | `X-Ingress-Path` presente | Respuesta |
|---|---|---|
| no | — | `X-Frame-Options: DENY` |
| sí | no | `X-Frame-Options: DENY` |
| sí | sí | `Content-Security-Policy: frame-ancestors 'self'`, **y `X-Frame-Options` eliminado** |

- **La cabecera sola no basta**: sin el gate del peer, mandar `X-Ingress-Path` a mano desde fuera
  bastaría para desactivar la protección. Con peer no confiable —el default— la respuesta lleva
  `DENY` aunque el header venga.
- **`frame-ancestors 'self'`, no `DENY` relajado a medias**: sigue prohibiendo el embebido
  cross-origin, que es el vector real del clickjacking, y permite el same-origin que el Ingress
  necesita.
- **El `X-Frame-Options` hay que quitarlo, no dejarlo**: `DENY` gana sobre la CSP en los
  navegadores que miran los dos, y el add-on saldría en blanco igualmente.
- Se usa `insert` (no `append`), igual que hacía el `SetResponseHeaderLayer::overriding` anterior.
- Regresión: `apps/api/tests/frame_options.rs` — las cuatro filas de la tabla, una por test.
