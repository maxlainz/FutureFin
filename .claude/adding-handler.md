# How to Add a New API Handler

Step-by-step pattern used throughout the codebase.

## 1. Create handler file `apps/api/src/handlers/foo.rs`

```rust
use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, post, patch, delete};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// Response type
#[derive(Debug, Serialize, ToSchema)]
pub struct FooResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

// Request body
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFooBody {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

#[utoipa::path(
    post,
    path = "/v1/foo",
    tag = "foo",
    request_body = CreateFooBody,
    responses(
        (status = 201, description = "Created", body = FooResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No session"),
        (status = 403, description = "Insufficient role"),
    )
)]
pub async fn create_foo(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateFooBody>,
) -> Result<(axum::http::StatusCode, Json<FooResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    
    // DB insert...
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO foo (installation_id, owner_user_id, amount) VALUES ($1, $2, $3) RETURNING id"#
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(body.amount)
    .fetch_one(&state.pool)
    .await?;
    
    Ok((axum::http::StatusCode::CREATED, Json(FooResponse { id, amount: body.amount })))
}

pub fn foo_router() -> Router {
    Router::new()
        .route("/", get(list_foos).post(create_foo))
        .route("/{id}", patch(patch_foo).delete(delete_foo))
}
```

## 2. Register in `handlers/mod.rs`
```rust
pub mod foo;
```

## 3. Wire route in `routes/mod.rs`
```rust
use crate::handlers::foo::foo_router;
// in app_router():
.nest("/foo", foo_router())
```

## 4. Register types in `openapi.rs`
Add `FooResponse`, `CreateFooBody` to the `components(schemas(...))` list, and the handler fn to
`paths(...)`.

**Autenticación en la spec (4.0.0)**: `openapi.rs` declara `security(("ff_session" = []))` **global**,
así que un handler con sesión no necesita decir nada. Un endpoint **público** debe llevar
`security(())` en su `#[utoipa::path]` — hoy lo llevan exactamente cinco (`health_check`,
`ready_check`, `register`, `login` y, desde 4.3.0, `sso_login`: no lleva credencial de FutureFin
porque la credencial la pone el proxy de confianza en una cabecera). Sin esa declaración global la spec presentaba 81 operaciones con
sesión obligatoria como públicas y cualquier cliente generado nacía sin credencial.

**Dos trampas que `tests/openapi_contract.rs` ya vigila** — si tu handler las dispara, el test falla
y no hace falta que las recuerdes, pero conviene saber por qué existe:
- **Colisión de nombre de componente**: utoipa nombra el schema por el **último segmento del tipo**,
  así que dos structs `ImportPreviewResponse` en módulos distintos se machacan y ambos endpoints
  acaban apuntando al mismo `$ref`. Desambigua con `#[schema(as = OtroNombre)]`.
- **Path con plantilla sin `params(...)`**: `/v1/foo/{id}` sin declarar `id` produce un documento
  formalmente inválido.

## 5. Add migration if needed
```bash
touch apps/api/migrations/$(date +%Y%m%d%H%M%S)_add_foo.sql
```

## Key patterns
- **Split extractor+auth vs `*_core`**: keep session/membership/role checks in the handler fn and move everything else (validation, SQL, response building, cache invalidation post-commit) into a `pub(crate) async fn foo_core(state, iid, user_id, …)`. That core is what an MCP tool reuses — a handler written monolithically forces the extraction later (see `patch_liability_core`, extracted when `update_liability` shipped). Architecture rationale: `futurefin-architecture-contract` D14.
- All monetary `Decimal` fields need `#[serde(with = "rust_decimal::serde::str")]` for JSON serialization as string
- Optional decimals: `#[serde(with = "rust_decimal::serde::str_option")]`
- `?view=mine` support: add `Query(q): Query<LedgerViewQuery>`, then `let view = q.resolve()`, then build SQL via `view.scope_where("alias")` + bind via `view.bind_scope_as(query_as(&sql), iid, user.id.0)`. **Never** hand-write a `match view { Household => sql_a, Mine => sql_b }` block — the two branches drift (we already had a bug where the `Mine` branch had `$3` and `$2` swapped).
- Never return `sqlx::Error` directly — `impl From<sqlx::Error> for ApiError` maps 23505 → 409 and 23503 → 400 automatically. Just `?` it.
- For long horizons / large datasets in CPU-bound work, wrap in `tokio::task::spawn_blocking` so the Tokio runtime stays responsive.
- Add an integration test in `apps/api/tests/{topic}.rs` using `common::TestApp::spawn()` (see [`.claude/tests.md`](tests.md)). One test per surprising behavior is plenty.
- **Client side: la URL nueva pasa por `apiUrl`.** Si el frontend llama a tu endpoint con un
  `fetch(` directo, envuélvela en `apiUrl("/v1/foo")` (`apps/web/src/lib/basePath.ts`) — es lo que
  hace que la app funcione bajo un subpath (Ingress de Home Assistant). Los wrappers de
  `api/client.ts` ya lo aplican solos. Ver [`frontend-structure.md`](frontend-structure.md)
  §Subpath tras proxy.
- **Si tu handler emite o borra la cookie de sesión**, usa los helpers de `handlers/auth.rs`
  —`session_cookie_path(&state, &headers)`, `session_cookie(&state, sid, path)`,
  `session_cookie_removal(path)`— en vez de construir el `Cookie` a mano: el `Path` va acotado al
  prefijo público de la request, y un borrado con otro `Path` **no** casa con la cookie viva. Ver
  [`auth-and-membership.md`](auth-and-membership.md) §Cookie.

## 6. Add a regression test
Drop a file in `apps/api/tests/foo.rs` that exercises your handler end-to-end:
```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn create_foo_succeeds() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let resp = app.post_json_with_cookie(
        "/v1/foo",
        serde_json::json!({ "amount": "1234" }),
        &owner.cookie,
    ).await;
    assert_eq!(resp.status, http::StatusCode::CREATED);
}
```
Run with `TEST_DATABASE_URL=postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test cargo test --workspace`.

## 7. Evaluate the MCP surface (mandatory)

The MCP catalog (`apps/api/src/mcp/server.rs`) is a derived surface of this API: a new or
changed endpoint must end in exactly one of **tool added/updated**, **deliberate omission
recorded**, or **n/a** — never silence. The decision rubric, the omission register and the
add-a-tool recipe live in
[`futurefin-mcp-parity`](skills/futurefin-mcp-parity/SKILL.md); the gate is enforced by
`futurefin-change-control` §1 (class "API contract"). If the answer is "tool", the `*_core`
split from Key patterns above is the prerequisite.
