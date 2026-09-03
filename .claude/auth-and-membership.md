# Auth & Membership Model

Este documento y el código (`apps/api/src/handlers/{session,installation,membership}.rs`, `apps/api/src/auth/`) son la spec completa — no existe documento externo.

## Flow

1. **Register** (`POST /v1/auth/register`): creates `User`. If no `installation` row exists, atomically creates it and sets caller as `owner`. Otherwise user is "pending" (no membership).
2. **Login** (`POST /v1/auth/login`): verifies Argon2id hash, inserts `sessions` row, sets `ff_session` cookie.
3. **Session check**: `require_session_user` reads cookie UUID → joins `sessions` + `users` → returns `SessionUser { id: UserId }`.
4. **Installation gate**: `GET /v1/installation/session-context` returns `{installation_initialized, access}`. Frontend uses this to decide between: setup screen, pending screen, or main app.
5. **Cambio de contraseña** (`POST /v1/auth/password`, 4.0.0): ver §Rotar la contraseña abajo.
6. **SSO por proxy de confianza** (`POST /v1/auth/sso`): ver §Identidad externa de Home Assistant, abajo.
7. **«Entrar con Home Assistant»** (`GET /v1/auth/ha/start` → `/callback`, 4.3.1): el mismo destino
   —una sesión normal— por el otro camino. Ver la misma sección.

## Rotar la contraseña (`POST /v1/auth/password`, 4.0.0)

Hasta 4.0.0 `hash_password` **solo se llamaba en `register`**: no existía forma de rotar la
contraseña. Una cookie esnifada (`COOKIE_SECURE=false` es el default, y tras un proxy es fácil
quedarse ahí), una sesión abierta en un equipo compartido o una filtración en otro servicio daban
`SESSION_TTL_DAYS` de acceso completo sin que la víctima pudiera hacer nada. `SECURITY.md`
describía qué pasa con los `.ffbackup` «si cambias la contraseña después» — documentando un flujo
que no existía.

Body `{current_password, new_password}` → **204**. Qué hace, en una sola transacción
(`handlers/auth.rs::change_password`):

| Credencial | Efecto |
|---|---|
| `users.password_hash` | se reescribe con el hash Argon2id nuevo (política de longitud: 12..=256 caracteres, `auth/password.rs::validate_password_strength`) |
| `sessions` | se borran **todas menos la que llama** (`id <> $current_sid`) — no echar al usuario de la app al terminar |
| `api_tokens` (`ffp_…`) | `revoked_at = now()` en todos los activos del usuario |
| `oauth_grants` | `revoked_at = now()`, `revoked_reason = 'password_change'` — y como la query de auth exige `g.revoked_at IS NULL`, eso mata de golpe todos sus access y refresh tokens |

- **Revocar las otras tres credenciales es el default seguro, no un extra**: si el motivo del cambio
  es un compromiso, dejar viva una credencial que no caduca haría el cambio decorativo.
- **`current_password` incorrecta → 400 `current_password_invalid`, NO 401.** La sesión es válida;
  lo que falla es un dato del formulario. Con un 401, el handler global de la SPA
  (`setUnauthorizedHandler`) echaría al usuario al login por escribir mal su propia contraseña.
- **Los `.ffbackup` ya exportados NO se recifran** y siguen abriéndose **solo** con la contraseña
  con la que se generaron: su clave se deriva de ella con Argon2id y el servidor no guarda la
  antigua. Quien rota la contraseña por sospecha de compromiso tiene que saberlo — está también en
  `SECURITY.md`.
- **Sin UI todavía**: la SPA no llama a este endpoint. Se usa por API/`curl`.
- Regresión: `apps/api/tests/account_and_members.rs`.

## Revocar y degradar membresías (`/v1/installation/members`, 4.0.0)

El mecanismo de corte llevaba aquí desde el principio —rol y pertenencia se **re-resuelven en cada
request**, así que quitar la fila corta el acceso al instante—, pero **no había palanca que lo
accionara**: `installation_memberships` solo recibía `INSERT` (bootstrap, `setup`, aprobación de un
pendiente). Aprobar al usuario equivocado concedía acceso permanente a todas las finanzas del hogar
y el único remedio era un `DELETE` a mano por `psql`. Este documento y `SECURITY.md` prometían lo
contrario: la promesa existía, la implementación no.

- `GET /v1/installation/members` — **cualquier miembro**, viewer incluido. Todos comparten los
  mismos datos financieros, así que saber quién más tiene acceso no revela nada nuevo — y es lo que
  permite auditar el hogar.
- `PATCH /v1/installation/members/{user_id}` `{role}` y `DELETE /v1/installation/members/{user_id}`
  — **owner-only**. `owner` es asignable a propósito: un hogar debe poder traspasar la propiedad sin
  pasar por `psql`.
- **Guardia `last_owner`**: degradar o expulsar al último owner → **400** `last_owner: …`. El
  recuento va dentro de la transacción y con `FOR UPDATE`, o dos owners degradándose a la vez
  dejarían el hogar sin ninguno.
- **Revocar NO borra los datos de la persona**: movimientos, snapshots, activos y reglas siguen
  ligados a su `owner_user_id` y los recupera intactos si se la vuelve a aprobar. Lo que se corta es
  el acceso, y se corta entero y a la vez: membresía + `sessions` + `api_tokens` +
  `oauth_grants` (`revoked_reason = 'membership_revoked'`). Sin ese corte, la persona conservaría
  acceso durante días — la sesión dura `SESSION_TTL_DAYS` y un `ffp_` puede no caducar nunca.
- Contrato completo (cuerpos, códigos, orden) en [`api-routes.md`](api-routes.md) §Members.
  **Sin UI todavía**: `Ajustes → Usuarios` sigue siendo solo la aprobación de pendientes.

## Roles
| Role | Permissions |
|------|------------|
| `owner` | Full CRUD + approve/reject pending users + **gestionar membresías** (`PATCH`/`DELETE /v1/installation/members/{user_id}`) + backup export |
| `member` | Full CRUD financial data |
| `viewer` | Read-only (GET endpoints) + **su propio perfil de jubilación** (ver abajo) |

`role_can_write(role)` → true for `owner` and `member`.
`role_can_read(role)` → true for all three.

### El rol no es lo único que decide una escritura (5.0.0)

Desde 5.0.0 hay **dos ejes** sobre la tabla de arriba, y ninguno la sustituye:

- **Dueño de la fila (D21)**. Toda mutación del ledger —`assets`, `liabilities`, `budget_entries`,
  `planning_flows`, `allocation_rules`— exige además que `owner_user_id` sea el usuario de la
  sesión: la fila de otro miembro devuelve **403 `not_row_owner`**, y **el rol `owner` tampoco la
  salta**. Ser dueño de la instalación no es ser dueño de la fila; con proyecciones independientes
  por miembro (D9) cada fila pertenece a la simulación de UNA persona. La LECTURA no cambia:
  `?view=household` sigue enseñando el hogar entero. Detalle y códigos:
  [`api-routes.md`](api-routes.md) §Dueño de la fila en las mutaciones.
- **Dato personal: cualquier rol edita el SUYO.** `PATCH /v1/auth/me/retirement-profile` (y la tool
  MCP `update_retirement_profile`) es la **única escritura del API que un `viewer` puede hacer**, y
  no es una excepción arbitraria: el perfil de jubilación no es configuración del hogar sino de esa
  persona, y sin poder fijar su edad de jubilación un viewer no podría ver su propia proyección —
  que es exactamente lo que un viewer sí puede hacer. Nadie puede editar el de otro: no hay
  parámetro para pedirlo, ni por HTTP ni por MCP.

La configuración COMPARTIDA del hogar sigue siendo owner-only (`PATCH /v1/installation`,
`update_fire_settings`, `update_installation_settings`) — desde 5.0.0 sin los cuatro ejes FIRE
personales, que se mudaron al perfil.

## Cookie
- Name: `ff_session`
- `HttpOnly`, `SameSite=Lax`
- `Secure` when `COOKIE_SECURE=1` (set behind HTTPS)
- TTL: `SESSION_TTL_DAYS` (default 30, max 400)
- **`Path` = el prefijo público de la request, o `/` si no hay ninguno.** Los tres puntos que la
  emiten o la borran pasan por los mismos helpers de `handlers/auth.rs`:
  `session_cookie_path(state, headers)` (→ `prefix::request_prefix`, o `"/"` si el prefijo es
  vacío), `session_cookie(state, sid, path)` y `session_cookie_removal(path)`. Los usan `login`,
  `logout` y `sso_login`.
  - **Por qué acotarla**: bajo el Ingress de Home Assistant **todos los add-ons comparten origen**
    (`http://homeassistant.local:8123`), así que un `Path=/` emitiría `ff_session` también hacia
    `/api/hassio_ingress/<token-de-otro-add-on>`.
  - **El borrado también** (`session_cookie_removal`): el navegador solo casa un `Set-Cookie` de
    borrado con la cookie viva si **nombre y `Path` coinciden**. Con el `Path=/` fijo de antes, un
    logout bajo Ingress dejaba viva la cookie acotada y el usuario seguía «dentro».
  - **Invariante maestro**: sin cabeceras de proxy el prefijo es `""` y la cookie sale con
    `Path=/`, **byte a byte** como siempre — el modo compose no cambia.
  - Regresión: `apps/api/tests/session_cookie_path.rs`.

## Pending users
Users who registered but have no `installation_memberships` row. Owner sees them via `/v1/installation/pending-users/`. Until approved they get `403 Forbidden` on any installation-scoped endpoint.

## API tokens (Bearer) — segundo esquema de auth (v3.0.0)
Per-user Bearer tokens (`ffp_` + 43 chars base64url de 32 bytes `OsRng`) para acceso programático —
hoy, el servidor MCP embebido (`/mcp`). Diseño espejo de las sesiones-en-DB (no JWT):

- **Solo se persiste el SHA-256 hex** del secreto (`api_tokens.token_hash` UNIQUE); el secreto viaja
  una única vez en el 201 del `POST /v1/api-tokens`.
- **El token NO congela rol ni installation**: `require_api_token` devuelve solo `{user_id, token_id}`
  y el caller encadena `require_installation_member` — revocar la membership (o el token:
  `revoked_at`) corta el acceso al instante, igual que borrar una sesión.
- **Cualquier miembro (viewer incluido) crea/lista/revoca los SUYOS** por cookie de sesión: un token
  no puede hacer nada que su dueño no pueda ya. Usuario pending → 403 (el mismo gate de siempre).
- Todo Bearer inválido (ausente, malformado, revocado, expirado, inexistente) es el mismo **401** —
  no se filtra qué tokens existen. Desde v3.1.0 el `WWW-Authenticate` lo pone el middleware de
  `/mcp` **solo en el 401** y anuncia la metadata OAuth
  (`Bearer realm="FutureFin", resource_metadata="…"`); ya no viaja en el 403 (ver §OAuth abajo y
  `api-routes.md` §MCP).
- Máx. 10 tokens activos por usuario; `last_used_at` con throttle de 60 s.
- **Scope, desde la Fase 3 (issue #84)** (migración `20260828140100_api_tokens_scope.sql`):
  `api_tokens.scope ∈ {read_write, read_only}`, default `read_write` (preserva byte a byte el
  comportamiento de todos los tokens emitidos antes de que existiera la columna). Se elige al
  crear el token (`POST /v1/api-tokens {scope}`, selector en Ajustes → Integraciones) y se lee
  **vivo** en el mismo `SELECT` que autentica (`require_api_token` → `TokenScope::from_db`,
  `handlers/api_tokens.rs`) — no hay nada congelado, igual que el resto de D14. Un valor
  desconocido en la columna (no debería poder existir: hay `CHECK`) falla **cerrado** a
  `read_only`. El scope **solo resta**: nunca concede nada que el rol vivo de la persona no
  conceda ya — un token `read_write` de un `viewer` sigue sin poder escribir.
- **OAuth NO tiene scope propio**: `mcp/auth.rs::authenticate` asigna `TokenScope::ReadWrite` a
  todo access token `ffo_…`, sin negociarlo. Ver la nota de `oauth/metadata.rs` (y el bloque OAuth
  más abajo) sobre por qué `scopes_supported` sigue ausente de la metadata RFC 8414: en un token
  de API el scope lo elige la persona con cookie de sesión; en OAuth el `scope` del authorization
  request lo elige la aplicación cliente, así que anunciarlo sin una pantalla de consentimiento
  que lo recorte no restringiría nada — solo mentiría en la metadata.

## OAuth 2.1 — tercer esquema de credencial (v3.1.0)

FutureFin es su **propio authorization server** (`apps/api/src/oauth/` + `handlers/oauth_consent.rs`):
emite access tokens delegados (`ffo_`) para que una app cliente — hoy solo el conector MCP de
claude.ai, que no acepta un Bearer pegado a mano — llegue a `/mcp` en nombre de un usuario. Contrato
de rutas y de tokens en [`api-routes.md`](api-routes.md) §OAuth 2.1; tablas en
[`data-model.md`](data-model.md) §OAuth.

**Esto NO es "login con OAuth".** Ese es el rol contrario y sigue rechazado: FutureFin como *cliente*
de un IdP externo se eliminó pre-1.0 (`auth/oauth.rs`, ver `futurefin-failure-archaeology` §2.10 y su
scope note). OAuth se limita a delegar acceso *después* del login. Los cuatro esquemas conviven así:

| Esquema | Credencial | Quién la usa | Cómo se corta |
|---|---|---|---|
| Sesión | cookie `ff_session` (UUID en DB) | la SPA, todo `/v1` | borrar la fila de `sessions` / logout |
| Token de API | Bearer `ffp_…` (`api_tokens`) | `/mcp` desde Claude Code/Desktop, pegado a mano | `DELETE /v1/api-tokens/{id}` (soft-revoke) |
| OAuth | Bearer `ffo_…` (`oauth_access_tokens` vía `oauth_grants`) | `/mcp` desde un cliente OAuth (claude.ai web) | `DELETE /v1/oauth/connections/{id}` → revoca el **grant** |
| Identidad externa de HA | **dos mecanismos, una sola identidad**: cabecera `X-Remote-User-Id` desde un peer de confianza (`POST /v1/auth/sso`), o el round-trip del navegador contra HA + cookie `ff_ha_state` de un solo uso (`GET /v1/auth/ha/start` → `/callback`, 4.3.1) | el navegador del add-on: el primero **tras el Ingress**, el segundo en el **origen directo** | apagar `FUTUREFIN_TRUSTED_PROXY_AUTH` / vaciar la opción `ha_sso_url` respectivamente, o revocar la membership del usuario (corta los dos a la vez) |

Los dos primeros producen una credencial propia; los dos últimos **no**: terminan en la misma fila
de `sessions` y la misma cookie `ff_session` de siempre.

**Matiz sobre «solo usuario + contraseña»**: hasta 4.2.x era literal. Desde el SSO por cabeceras hay
otras formas de autenticarse ante FutureFin. El SSO por cabeceras no es login-con-IdP: FutureFin no
habla con ningún proveedor, se limita a creerle a un proceso que el operador ha nombrado por IP y que
corre en la misma red privada. **«Entrar con Home Assistant» (4.3.1) sí lo es**, y reabre la fila
«OAuth login» de la arqueología de forma **estrecha y consciente**: solo para Home Assistant, solo
dentro del add-on, y tomando de HA **identidad, nunca autorización** (roles, membership, aprobación
de pendientes y bootstrap del owner siguen siendo 100 % de FutureFin). El login-con-IdP **genérico**
sigue rechazado — ver la decisión **D19** del contrato de arquitectura y el scope note de la
arqueología.

### Flujo completo

1. **El cliente se registra** (DCR, `POST /oauth/register`, público): obtiene `client_id` `ffc_…` y,
   si declara un método confidencial, un secreto `ffcs_…`. Una fila en `oauth_clients` **no da
   acceso a nada** — es solo identidad declarada.
2. **El cliente manda al navegador a `GET /oauth/authorize?…`**, que sirve la **SPA** (no el backend).
   La vista pide `GET /v1/oauth/authorize-details` — **sin sesión a propósito** — para validar el
   request y pintar de quién se trata: un `redirect_uri` que no cuadra se ve **antes** de teclear la
   contraseña. Un error fatal (cliente desconocido, redirect sin match exacto) se pinta y ahí muere:
   redirigir sería un open redirect.
3. **Login si hace falta**: sin cookie válida la vista muestra su propio panel de login
   (`auth/LoginPanel.tsx`) → `POST /v1/auth/login`, el mismo Argon2id de siempre. Sin login no hay
   consentimiento posible.
4. **Consentimiento explícito**: `POST /v1/oauth/authorize` con `{approve, …params}` exige cookie
   **y** `require_installation_member` (un usuario pending recibe **403**, igual que en todo lo
   demás). Approve → se crea (o se refresca) el grant y se emite un authorization code de 2 min;
   deny → `error=access_denied`. El servidor devuelve la URL a la que navegar; **la SPA nunca
   construye un redirect**.
5. **Canje** (`POST /oauth/token`, `grant_type=authorization_code` + PKCE S256): access token `ffo_`
   (1 h) + refresh token `ffr_` (90 días sin uso, con rotación y reuse-detection).
6. **Uso**: `Authorization: Bearer ffo_…` contra `/mcp`. El middleware `mcp/auth.rs` despacha por
   prefijo y encadena `require_installation_member`.

### Rol vivo por request — el mismo contrato que las sesiones

`oauth::access::require_oauth_access_token` devuelve solo `{user_id, grant_id, token_id}`: **el token
no congela nada**. Membership y rol se re-resuelven en cada request (`require_installation_member`),
así que degradar a `viewer`, revocar la membership o expulsar al usuario corta o recorta el acceso al
instante, sin esperar a que el token caduque. Un `ffo_` nunca puede hacer más de lo que su dueño
ya puede hacer: las lecturas siguen su rol, y las tools de escritura re-comprueban por request las
**tres puertas de `require_mcp_write`** (`mcp/auth.rs`, Fase 3, issue #84), de la más fundamental a
la más circunstancial:

1. **Rol vivo** — `role_can_write(role)`: `viewer` nunca escribe → `forbidden`.
2. **Scope de la credencial** — un token de API `read_only` corta aquí aunque el rol escriba →
   `mcp_token_read_only`. Los `ffo_…` de OAuth siempre llevan `read_write` (no negocian scope, ver
   arriba), así que esta puerta nunca los frena — solo la 1 y la 3.
3. **Toggle de la instalación** — `installation.mcp_write_enabled` → `mcp_write_disabled`. Apagarlo
   en Ajustes → Integraciones corta la escritura de TODAS las credenciales (tokens de API y OAuth)
   en la siguiente llamada, sin reinicio.

Las dos últimas responden `BadRequest` (no `Forbidden`): es la única variante que propaga el
mensaje al wire, y un cliente MCP necesita leer el motivo para explicárselo al usuario en vez de
reintentar a ciegas. Cada llamada a `require_mcp_write` —gane o pierda el gate— deja una fila en
`mcp_write_audit` (`outcome ∈ {denied, attempted}` en el momento del gate, cerrada después a
`ok`/`failed` por la propia tool): quién, con qué credencial, con qué **rol vivo**, qué tool, qué
desenlace y qué UUIDs mutó — nunca los argumentos. Retención 365 días, poda perezosa en el propio
camino de escritura (D5: nunca en un GET). Esquema y contrato completos:
[`data-model.md`](data-model.md) §`mcp_write_audit`.

**Revocación — un solo punto de corte**: la query de auth hace JOIN con `oauth_grants` y exige
`g.revoked_at IS NULL`, así que marcar **una fila** (el grant) mata todos los access y refresh tokens
de esa app sin tocarlos, igual que borrar una sesión. Se revoca desde tres sitios, y los tres
escriben `revoked_reason`: el panel (Ajustes → Integraciones → Conexiones, `user_panel`), el propio cliente
(`POST /oauth/revoke` con un `ffr_`, `rfc7009`) y **el servidor por su cuenta** al detectar reuso de
un code o de un refresh ya consumido (`code_reuse` / `refresh_token_reuse`, OAuth 2.1 §4.3.1/§7.5).
El panel se monta **siempre**, incluso con `FUTUREFIN_MCP_ENABLED=0`: apagar MCP no puede dejarte sin
poder revocar lo que ya concediste.

## Identidad externa de Home Assistant — dos caminos, una sola identidad

Desde 4.3.1 hay **dos** mecanismos que canjean la identidad de una persona en Home Assistant por una
sesión normal de FutureFin. No son dos identidades ni dos tipos de cuenta: son **dos caras de la
misma**, y esa es la propiedad load-bearing.

| | SSO por cabeceras (4.3.0) | «Entrar con HA» / HA-IdP (4.3.1) |
|---|---|---|
| Ruta | `POST /v1/auth/sso` (`handlers/sso.rs`) | `GET /v1/auth/ha/start` → `/callback` (`handlers/ha_sso.rs` + `ha_idp/`) |
| Dónde aplica | **Dentro del Ingress** del Supervisor (el panel de la barra lateral) | Donde **no** hay proxy de confianza: puerto directo, túnel, `homeassistant.local:8123` en una pestaña normal |
| La prueba de identidad es | Una cabecera que pone un peer que el operador nombró por IP | Un **round-trip del navegador** contra el propio HA + cookie `ff_ha_state` de un solo uso |
| Se habilita con | `FUTUREFIN_TRUSTED_PROXY_AUTH` + `FUTUREFIN_TRUSTED_PROXY_IPS` (opción `sso` del add-on) | `FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON=1` (opción `ha_sso_url` del add-on) |
| Lo anuncia a la SPA | `window.__FF_SSO__` (depende de la request: peer + cabecera) | `window.__FF_HA_LOGIN__` (depende **solo** del proceso) |
| Errores | JSON `sso_*` | **Redirect** `?ha_error=…` (ver [`api-routes.md`](api-routes.md)) |

**Lo que comparten es lo que importa**: el mismo `external_user_id` (el `User.id` de HA — el
Supervisor lo manda en `X-Remote-User-Id` en forma canónica y `auth/current_user` lo devuelve como
`uuid4().hex`, 32 hex sin guiones; `Uuid::parse_str` normaliza ambas al MISMO UUID), la misma
`resolve_or_provision` (`pub(crate)` desde 4.3.1, con **dos** callers y ni una línea duplicada), las
mismas cuentas sin contraseña y el mismo `establish_session`. Entrar por un camino o por el otro
lleva a la **misma fila de `users`**; si alguien toca una de las dos normalizaciones, la persona se
duplica en silencio y su hogar se parte en dos (invariante **I16**, pin
`ha_idp_login.rs::header_sso_and_ha_login_resolve_to_the_same_user`).

**El callback, resumido** (contrato de orden, no detalle): leer la cookie → comparar el `state` en
tiempo constante → **borrar la cookie SIEMPRE** (un solo uso, éxito o fallo) → canjear el código con
el `client_id` byte-idéntico congelado en la cookie → leer la identidad por WebSocket → **revocar el
refresh token de HA ANTES de tocar la base de datos** (FutureFin no retiene credenciales de la
domótica) → `resolve_or_provision` → `establish_session` → 302 limpio. Paso a paso, atributos de la
cookie y los cinco códigos de error: [`api-routes.md`](api-routes.md)
§`/v1/auth/ha/start`+`/callback`. El **porqué** de cada pilar (identidad y nunca autorización, IdP
puro, modelo CSRF por cookie, solo add-on): decisión **D19** de
[`futurefin-architecture-contract`](skills/futurefin-architecture-contract/SKILL.md).

### SSO por cabeceras de un proxy de confianza

`POST /v1/auth/sso`, `apps/api/src/handlers/sso.rs`. Para el add-on de Home Assistant: el Ingress
del Supervisor ya autenticó a la persona antes de que la petición llegue aquí y añade
`X-Remote-User-Id` (stripeando el que venga del cliente). El endpoint canjea esa identidad por una
**sesión normal** — la misma fila en `sessions`, la misma cookie `ff_session`, el mismo gate de
instalación. A partir del 200 no hay nada especial en el usuario salvo que su `password_hash` es
NULL. Contrato de request/respuesta y códigos de error: [`api-routes.md`](api-routes.md)
§`POST /v1/auth/sso`.

#### Modelo de confianza — doble puerta, y es la frontera entera

Una cabecera de identidad es **una afirmación sin prueba**: cualquiera puede escribir
`X-Remote-User-Id: <uuid del owner>`. Lo único que la convierte en credencial es de quién viene.
De ahí dos gates independientes, ambos por request:

| Gate | Fuente | Fallo |
|---|---|---|
| ¿Está el mecanismo habilitado? | `FUTUREFIN_TRUSTED_PROXY_AUTH` (`AppState.trusted_header_auth`) | 401 `sso_disabled` |
| ¿Viene de un peer nombrado? | `FUTUREFIN_TRUSTED_PROXY_IPS` (`PeerPolicy::allows(PeerIp)`) | 401 `sso_untrusted_peer` |

- **La ruta se monta SIEMPRE** (`routes/mod.rs`): la forma del router no puede depender del
  entorno, o los tests dejan de describir el binario que se despliega. Lo que decide es el estado.
- **Combinación imposible por construcción**: `FUTUREFIN_TRUSTED_PROXY_AUTH=1` sin
  `FUTUREFIN_TRUSTED_PROXY_IPS` hace **panic al arrancar** (`main.rs`), porque aceptaría la
  identidad de cualquiera. En el add-on, el entrypoint fija las dos juntas desde `options.json`.
- El default sigue siendo **apagado**: una instalación normal responde 401 `sso_disabled` a un
  `POST /v1/auth/sso` perfecto (primer test de `sso_login.rs`).

### Provisión de la cuenta — compartida por los dos caminos

`handlers/sso.rs::resolve_or_provision` (`pub(crate)`; la llaman `sso_login` y `ha_callback`). En el
camino HA-IdP el «nombre para mostrar» es el `result.name` de `auth/current_user`; el resto es
idéntico:

1. Se busca por `users.external_user_id` (UNIQUE parcial). Si existe → ese usuario, sin más. **La
   identidad, no el nombre, es la clave**: renombrarse en Home Assistant no crea una cuenta nueva.
2. Si no existe, se deriva un `username` de `X-Remote-User-Display-Name` (o, en su defecto,
   `X-Remote-User-Name`) con `username_slug`: pliega los diacríticos del español a ASCII —sin
   añadir un crate de normalización Unicode por «José»— y manda todo lo demás a `-`, colapsando
   rachas; sin caracteres utilizables cae a `ha-user`, y menos de 3 caracteres se rellena
   (`Al` → `al-ha`). El resultado siempre cumple `^[a-z0-9._-]{3,64}$`.
3. Se prueban seis candidatos en orden — el slug, `-2`..`-5`, y `ha-<8 hex del id externo>`, que es
   único por construcción y cierra el bucle. Agotados los seis: 409 `sso_username_unavailable`.
4. El alta va en **una transacción por intento** (en Postgres una violación de unique aborta la
   transacción entera, así que reintentar dentro no es posible) e incluye
   `bootstrap_installation_as_owner_if_empty`: **el primero que entra crea el hogar y queda owner;
   los siguientes quedan pendientes de aprobación**, exactamente el mismo camino que `register`.
5. Carrera con otra petición sobre la MISMA identidad externa (`users_external_user_id_key`): no es
   un error — se devuelve el usuario que ganó, que es lo que quien llama pedía.

Tras el 200, `sso_login` hace el **mismo warm-up en background** que `login` — y desde 5.0.0 lo que
calienta es la proyección **del propio usuario** (`warm_up_mine_projection`, `view=mine`, las dos
densidades), no la del hogar: con el default de `?view` invertido, el hogar sería una entrada que
nadie consulta, y además cuesta N simulaciones en vez de una (D7: no se espera al recompute);
usuario pending ⇒ no hay nada que calentar y se salta. El
camino HA-IdP lo hereda: `ha_callback` termina en el mismo `establish_session`.

**Un matiz del camino HA-IdP**: el 409 `sso_username_unavailable` del punto 3 no puede salir como
JSON (el navegador viene de HA), así que se traduce al redirect `?ha_error=sso_username_unavailable`
— **mismo código**, misma frase. Cualquier OTRO error de `resolve_or_provision` (fallo de BD, carrera
de instalación) sí sale como error de servidor: esconderlo detrás de la pantalla de login haría
indistinguible «tu base de datos está caída» de «vuelve a intentarlo».

### Cuentas sin contraseña — vale para los dos caminos

`password_hash` es NULL y no hay ninguna contraseña que probar. Los tres flujos que la exigen
devuelven **401 `sso_account_no_password`** (`handlers/auth.rs::sso_account_no_password`, un único
constructor compartido):

| Endpoint | Por qué |
|---|---|
| `POST /v1/auth/login` | No hay hash contra el que verificar. |
| `POST /v1/auth/password` | **Fijar** una contraseña desde aquí crearía una segunda vía de acceso a una cuenta cuya autenticación pertenece al proveedor. Fuera de alcance en esta release. |
| `POST /v1/backup/user-export` | La clave del `.ffbackup` se deriva de la contraseña de cuenta con Argon2id — sin contraseña no hay clave. |

- **Es un 401 hablado a propósito** (`ApiError::UnauthorizedWith`, no el `Unauthorized` mudo).
  Decirlo revela que ese nombre existe como cuenta SSO, y es un intercambio buscado: sin el
  mensaje, el único usuario del add-on se queda encallado tecleando una contraseña que nunca se
  fijó. El login **sigue pagando** el Argon2id de descarte antes de responder, así que el reloj no
  delata nada de más.
- Las cuentas de contraseña quedan **intactas**: la columna es aditiva y su `external_user_id` es
  NULL (test `password_accounts_are_untouched_by_the_sso_column`).

### Notas

- **Omisión deliberada del catálogo MCP** (registrada en `futurefin-mcp-parity` §3.1, las **tres**
  rutas): `POST /v1/auth/sso` es un mecanismo de sesión de navegador atado a cabeceras de proxy, y
  `/v1/auth/ha/{start,callback}` un mecanismo de redirect de navegador que termina en una cookie, no
  en un token — una tool MCP no puede conducirlo. Ninguna es una operación sobre datos.
- La SPA intenta el SSO por cabeceras **una sola vez** y solo si el shell le inyectó
  `window.__FF_SSO__` (ver [`frontend-structure.md`](frontend-structure.md) §`lib/basePath.ts`): un
  401 de `/v1/auth/me` con ese flag dispara un `POST /v1/auth/sso`, y cualquier fallo cae al
  formulario de acceso de siempre — el SSO es un atajo, nunca la única puerta. El botón «Entrar con
  Home Assistant» es lo contrario: **nunca es automático**, lo pulsa la persona
  (`haLoginHref(next)` → navegación completa a `/v1/auth/ha/start`), y aparece con
  `window.__FF_HA_LOGIN__` tanto en el login como en la pantalla de consentimiento OAuth — que es
  el caso que lo motiva: sin él, una cuenta sin contraseña no podía autorizar el conector MCP en el
  origen directo.
- Regresión: `apps/api/tests/sso_login.rs` (12 tests) y `apps/api/tests/ha_idp_login.rs` (18), más
  los 11 unitarios puros de `apps/api/src/ha_idp/mod.rs`.

## Key functions
```rust
// handlers/session.rs
require_session_user(&jar, &pool) -> Result<SessionUser, ApiError>

// handlers/sso.rs — UN solo camino de alta para la identidad externa de HA.
// pub(crate) desde 4.3.1: la llaman sso_login (cabeceras) y ha_callback (HA-IdP).
resolve_or_provision(&state, external_user_id: Uuid, raw_name: &str) -> Result<UserRow, ApiError>

// handlers/api_tokens.rs
require_api_token(pool, authorization: Option<&HeaderValue>) -> Result<ApiTokenIdentity, ApiError>

// oauth/access.rs — el espejo OAuth del anterior; devuelve ApiError (alimenta al middleware de /mcp)
require_oauth_access_token(pool, authorization: Option<&HeaderValue>) -> Result<OAuthIdentity, ApiError>

// auth/secret.rs — compartido por api_tokens (ffp_) y oauth (ffc_/ffcs_/ffo_/ffr_)
sha256_hex(bytes: &[u8]) -> String
generate_opaque_secret(prefix: &str) -> String  // prefijo + 43 chars base64url de 32 bytes OsRng
generate_opaque_id(prefix: &str) -> String      // prefijo + 22 chars (identificador público)

// handlers/installation.rs
require_installation_member(pool, user_id) -> Result<(Uuid, MembershipRole), ApiError>
bootstrap_installation_as_owner_if_empty(tx, user_id) -> Result<(), ApiError>
singleton_installation_id(pool) -> Result<Option<Uuid>, ApiError>
installation_naive_today(pool, installation_id) -> Result<NaiveDate, ApiError>
naive_date_in_calendar_tz(tz_name) -> Result<NaiveDate, ApiError>  // standalone version of "today" — useful when you already have calendar_tz from a joined query

// handlers/membership.rs
role_can_write(role: &str) -> bool  // owner | member
```

## Duplicate username / category / membership
Don't write per-handler 23505 detection. `impl From<sqlx::Error> for ApiError` (`error.rs`) returns `ApiError::Conflict` (409) for any unique-violation that bubbles up from an `INSERT`. Old code had four hand-rolled mappers (`map_unique_violation`, `insert_conflict`, `is_unique_violation`, `db_conflict`) — all deleted.
