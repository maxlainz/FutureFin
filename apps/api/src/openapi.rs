use crate::error::{ErrorBody, ErrorCode};
#[allow(unused_imports)]
use crate::handlers::auth::{__path_login, __path_logout, __path_me, __path_register};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
#[allow(unused_imports)]
use crate::handlers::installation::{__path_get_my_installation, __path_setup_installation};
#[allow(unused_imports)]
use crate::handlers::pending_users::{__path_approve_pending_user, __path_list_pending_users};
#[allow(unused_imports)]
use crate::handlers::categories::{
    __path_create_category, __path_delete_category, __path_list_categories,
    __path_patch_category,
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
        get_my_installation,
        setup_installation,
        list_pending_users,
        approve_pending_user,
        list_categories,
        create_category,
        patch_category,
        delete_category,
    ),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::UserResponse,
        crate::handlers::installation::InstallationSnapshot,
        crate::handlers::installation::InstallationAccess,
        crate::handlers::installation::SetupInstallationBody,
        crate::handlers::membership::MembershipRole,
        crate::handlers::pending_users::ApprovePendingUserBody,
        crate::handlers::pending_users::ApproveMemberRole,
        crate::handlers::categories::CategoryScope,
        crate::handlers::categories::CategoryResponse,
        crate::handlers::categories::CreateCategoryBody,
        crate::handlers::categories::PatchCategoryBody,
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
            name = "installation",
            description = "Singleton installation, setup, and owner approvement of registered users — see `docs/spec/AUTH_MODEL.md`"
        ),
        (
            name = "categories",
            description = "Installation-scoped categories (asset, liability, income, expense); see parity checklist Settings — Categories"
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
