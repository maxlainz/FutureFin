use crate::error::{ErrorBody, ErrorCode};
#[allow(unused_imports)]
use crate::handlers::auth::{__path_login, __path_logout, __path_me, __path_register};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
#[allow(unused_imports)]
use crate::handlers::installation::{
    __path_get_my_installation, __path_patch_my_installation, __path_setup_installation,
};
#[allow(unused_imports)]
use crate::handlers::pending_users::{__path_approve_pending_user, __path_list_pending_users};
#[allow(unused_imports)]
use crate::handlers::summary::__path_get_summary;
#[allow(unused_imports)]
use crate::handlers::assets::{
    __path_create_asset, __path_delete_asset, __path_list_assets, __path_patch_asset,
};
#[allow(unused_imports)]
use crate::handlers::liabilities::{
    __path_create_liability, __path_delete_liability, __path_list_liabilities,
    __path_patch_liability,
};
#[allow(unused_imports)]
use crate::handlers::budget::{
    __path_create_budget_entry, __path_delete_budget_entry, __path_get_budget_snapshot,
    __path_patch_budget_entry,
};
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
        patch_my_installation,
        setup_installation,
        list_pending_users,
        approve_pending_user,
        list_categories,
        create_category,
        patch_category,
        delete_category,
        list_assets,
        create_asset,
        patch_asset,
        delete_asset,
        list_liabilities,
        create_liability,
        patch_liability,
        delete_liability,
        get_summary,
        get_budget_snapshot,
        create_budget_entry,
        patch_budget_entry,
        delete_budget_entry,
    ),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::UserResponse,
        crate::handlers::installation::InstallationSnapshot,
        crate::handlers::installation::InstallationAccess,
        crate::handlers::installation::SetupInstallationBody,
        crate::handlers::installation::PatchInstallationBody,
        crate::handlers::membership::MembershipRole,
        crate::handlers::pending_users::ApprovePendingUserBody,
        crate::handlers::pending_users::ApproveMemberRole,
        crate::handlers::categories::CategoryScope,
        crate::handlers::categories::CategoryResponse,
        crate::handlers::categories::CreateCategoryBody,
        crate::handlers::categories::PatchCategoryBody,
        crate::handlers::assets::AssetResponse,
        crate::handlers::assets::CreateAssetBody,
        crate::handlers::assets::PatchAssetBody,
        crate::handlers::liabilities::LiabilityResponse,
        crate::handlers::liabilities::CreateLiabilityBody,
        crate::handlers::liabilities::PatchLiabilityBody,
        crate::handlers::liabilities::PaymentFrequency,
        crate::handlers::summary::SummaryResponse,
        crate::handlers::budget::BudgetSnapshotResponse,
        crate::handlers::budget::BudgetEntryResponse,
        crate::handlers::budget::DerivedBudgetLineResponse,
        crate::handlers::budget::BudgetTotalsResponse,
        crate::handlers::budget::CreateBudgetEntryBody,
        crate::handlers::budget::PatchBudgetEntryBody,
        crate::handlers::budget::BudgetCategoryScope,
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
        (
            name = "assets",
            description = "Installation asset ledger (category must be asset scope)"
        ),
        (
            name = "liabilities",
            description = "Installation liabilities, optional APR and payment plan; optional derive_principal_from_plan (principal = payment × intervals from installation calendar_tz through payment_end_date); expired payment-end rows purged on GET list"
        ),
        (
            name = "summary",
            description = "Installation-wide KPI aggregates (purges expired liabilities before totals; parity SummaryMetricGrid basics)"
        ),
        (
            name = "budget",
            description = "Persisted income/expense budget lines (weekly→monthly ×52/12); liability-derived debt payments included in snapshot only (parity BudgetTabView)"
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
