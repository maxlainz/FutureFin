# Auth & Membership Model

Este documento y el código (`apps/api/src/handlers/{session,installation,membership}.rs`, `apps/api/src/auth/`) son la spec completa — no existe documento externo.

## Flow

1. **Register** (`POST /v1/auth/register`): creates `User`. If no `installation` row exists, atomically creates it and sets caller as `owner`. Otherwise user is "pending" (no membership).
2. **Login** (`POST /v1/auth/login`): verifies Argon2id hash, inserts `sessions` row, sets `ff_session` cookie.
3. **Session check**: `require_session_user` reads cookie UUID → joins `sessions` + `users` → returns `SessionUser { id: UserId }`.
4. **Installation gate**: `GET /v1/installation/session-context` returns `{installation_initialized, access}`. Frontend uses this to decide between: setup screen, pending screen, or main app.

## Roles
| Role | Permissions |
|------|------------|
| `owner` | Full CRUD + approve/reject pending users + backup export |
| `member` | Full CRUD financial data |
| `viewer` | Read-only (GET endpoints) |

`role_can_write(role)` → true for `owner` and `member`.
`role_can_read(role)` → true for all three.

## Cookie
- Name: `ff_session`
- `HttpOnly`, `SameSite=Lax`
- `Secure` when `COOKIE_SECURE=1` (set behind HTTPS)
- TTL: `SESSION_TTL_DAYS` (default 30, max 400)

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

## OAuth 2.1 — tercer esquema de credencial (v3.1.0)

FutureFin es su **propio authorization server** (`apps/api/src/oauth/` + `handlers/oauth_consent.rs`):
emite access tokens delegados (`ffo_`) para que una app cliente — hoy solo el conector MCP de
claude.ai, que no acepta un Bearer pegado a mano — llegue a `/mcp` en nombre de un usuario. Contrato
de rutas y de tokens en [`api-routes.md`](api-routes.md) §OAuth 2.1; tablas en
[`data-model.md`](data-model.md) §OAuth.

**Esto NO es "login con OAuth".** Ese es el rol contrario y sigue rechazado: FutureFin como *cliente*
de un IdP externo se eliminó pre-1.0 (`auth/oauth.rs`, ver `futurefin-failure-archaeology` §2.10 y su
scope note). Una persona se autentica **solo** con usuario + contraseña Argon2id; OAuth se limita a
delegar acceso *después* de ese login. Los tres esquemas conviven así:

| Esquema | Credencial | Quién la usa | Cómo se corta |
|---|---|---|---|
| Sesión | cookie `ff_session` (UUID en DB) | la SPA, todo `/v1` | borrar la fila de `sessions` / logout |
| Token de API | Bearer `ffp_…` (`api_tokens`) | `/mcp` desde Claude Code/Desktop, pegado a mano | `DELETE /v1/api-tokens/{id}` (soft-revoke) |
| OAuth | Bearer `ffo_…` (`oauth_access_tokens` vía `oauth_grants`) | `/mcp` desde un cliente OAuth (claude.ai web) | `DELETE /v1/oauth/connections/{id}` → revoca el **grant** |

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
instante, sin esperar a que el token caduque. Y como todas las tools MCP son de solo lectura, un
`ffo_` nunca puede hacer más de lo que su dueño ya puede leer.

**Revocación — un solo punto de corte**: la query de auth hace JOIN con `oauth_grants` y exige
`g.revoked_at IS NULL`, así que marcar **una fila** (el grant) mata todos los access y refresh tokens
de esa app sin tocarlos, igual que borrar una sesión. Se revoca desde tres sitios, y los tres
escriben `revoked_reason`: el panel (Ajustes → Acceso → Conexiones, `user_panel`), el propio cliente
(`POST /oauth/revoke` con un `ffr_`, `rfc7009`) y **el servidor por su cuenta** al detectar reuso de
un code o de un refresh ya consumido (`code_reuse` / `refresh_token_reuse`, OAuth 2.1 §4.3.1/§7.5).
El panel se monta **siempre**, incluso con `FUTUREFIN_MCP_ENABLED=0`: apagar MCP no puede dejarte sin
poder revocar lo que ya concediste.

## Key functions
```rust
// handlers/session.rs
require_session_user(&jar, &pool) -> Result<SessionUser, ApiError>

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
