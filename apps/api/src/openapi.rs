use crate::error::{ErrorBody, ErrorCode};
// Required so utoipa-generated `__path_*` types resolve for `#[derive(OpenApi)]`.
#[allow(unused_imports)]
use crate::handlers::auth::{__path_login, __path_logout, __path_me, __path_register};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
#[allow(unused_imports)]
use crate::handlers::households::{__path_create_household, __path_list_households};
use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        health_check,
        ready_check,
        register,
        login,
        logout,
        me,
        list_households,
        create_household,
    ),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::UserResponse,
        crate::handlers::households::HouseholdSummary,
        crate::handlers::households::CreateHouseholdBody,
        crate::handlers::households::HouseholdRole,
        ErrorBody,
        ErrorCode,
    )),
    tags(
        (name = "health", description = "Liveness and readiness probes"),
        (
            name = "auth",
            description = "Username/password sessions (OAuth/OIDC traits reserved in `crate::auth::oauth`)"
        ),
        (
            name = "households",
            description = "Multi-tenant households and memberships (`AUTH_MODEL.md`)"
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
