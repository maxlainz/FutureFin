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

## Key functions
```rust
// handlers/session.rs
require_session_user(&jar, &pool) -> Result<SessionUser, ApiError>

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
