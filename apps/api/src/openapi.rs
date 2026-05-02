use crate::error::{ErrorBody, ErrorCode};
// Required so utoipa-generated `__path_*` types resolve for `#[derive(OpenApi)]`.
#[allow(unused_imports)]
use crate::handlers::auth::{__path_login, __path_logout, __path_me, __path_register};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
#[allow(unused_imports)]
use crate::handlers::households::{__path_create_household, __path_list_households};
#[allow(unused_imports)]
use crate::handlers::persons::{
    __path_create_person, __path_delete_person, __path_list_persons, __path_update_person,
};
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
        list_persons,
        create_person,
        update_person,
        delete_person,
    ),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::UserResponse,
        crate::handlers::households::HouseholdSummary,
        crate::handlers::households::CreateHouseholdBody,
        crate::handlers::households::HouseholdRole,
        crate::handlers::persons::PersonResponse,
        crate::handlers::persons::CreatePersonBody,
        crate::handlers::persons::UpdatePersonBody,
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
            description = "Household singleton per installation and memberships — see `docs/spec/AUTH_MODEL.md`"
        ),
        (
            name = "persons",
            description = "Domain persons in the installation household (Person vs User — `AUTH_MODEL.md`)"
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
