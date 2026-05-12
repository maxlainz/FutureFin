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
Add `FooResponse`, `CreateFooBody` to the `components(schemas(...))` list.

## 5. Add migration if needed
```bash
touch apps/api/migrations/$(date +%Y%m%d%H%M%S)_add_foo.sql
```

## Key patterns
- All monetary `Decimal` fields need `#[serde(with = "rust_decimal::serde::str")]` for JSON serialization as string
- Optional decimals: `#[serde(with = "rust_decimal::serde::str_option")]`
- `?view=mine` support: add `Query(q): Query<LedgerViewQuery>`, then `let view = q.resolve()`, filter on `owner_user_id` in SQL
- Never return `sqlx::Error` directly — it's covered by `impl From<sqlx::Error> for ApiError`
