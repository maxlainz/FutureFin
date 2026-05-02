use crate::error::{ErrorBody, ErrorCode};
// Required so utoipa-generated `__path_*` types resolve for `#[derive(OpenApi)]`.
#[allow(unused_imports)]
use crate::handlers::auth::{__path_login, __path_logout, __path_me, __path_register};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(health_check, ready_check, register, login, logout, me),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::UserResponse,
        ErrorBody,
        ErrorCode,
    )),
    tags(
        (name = "health", description = "Liveness and readiness probes"),
        (
            name = "auth",
            description = "Username/password sessions (OAuth/OIDC traits reserved in `crate::auth::oauth`)"
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
