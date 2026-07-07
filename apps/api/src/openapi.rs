use crate::error::{ErrorBody, ErrorCode};
#[allow(unused_imports)]
use crate::handlers::auth::{
    __path_login, __path_logout, __path_me, __path_patch_me, __path_register,
};
#[allow(unused_imports)]
use crate::handlers::backup_user::export::__path_export_user_backup;
#[allow(unused_imports)]
use crate::handlers::backup_user::import::{
    __path_import_user_backup_apply, __path_import_user_backup_preview,
};
#[allow(unused_imports)]
use crate::handlers::health::{__path_health_check, __path_ready_check};
#[allow(unused_imports)]
use crate::handlers::history::{
    __path_capture_snapshots, __path_create_snapshot, __path_delete_snapshot,
    __path_get_history_series, __path_list_snapshots, __path_prefill_snapshot,
    __path_update_snapshot,
};
#[allow(unused_imports)]
use crate::handlers::installation::{
    __path_get_installation_session_context, __path_get_my_installation,
    __path_patch_my_installation, __path_setup_installation,
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
use crate::handlers::allocation_rules::{
    __path_create_allocation_rule, __path_delete_allocation_rule, __path_list_allocation_rules,
    __path_patch_allocation_rule, __path_reorder_allocation_rules,
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
use crate::handlers::planning::{
    __path_create_planning_flow, __path_delete_planning_flow, __path_list_planning_flows,
    __path_patch_planning_flow,
};
#[allow(unused_imports)]
use crate::handlers::categories::{
    __path_create_category, __path_delete_category, __path_list_categories,
    __path_patch_category,
};
#[allow(unused_imports)]
use crate::handlers::projection::__path_get_projection_series;
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
        patch_me,
        get_installation_session_context,
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
        list_allocation_rules,
        create_allocation_rule,
        patch_allocation_rule,
        delete_allocation_rule,
        reorder_allocation_rules,
        list_liabilities,
        create_liability,
        patch_liability,
        delete_liability,
        get_summary,
        get_budget_snapshot,
        create_budget_entry,
        patch_budget_entry,
        delete_budget_entry,
        list_planning_flows,
        create_planning_flow,
        patch_planning_flow,
        delete_planning_flow,
        get_projection_series,
        capture_snapshots,
        list_snapshots,
        create_snapshot,
        update_snapshot,
        delete_snapshot,
        prefill_snapshot,
        get_history_series,
        export_user_backup,
        import_user_backup_preview,
        import_user_backup_apply,
    ),
    components(schemas(
        crate::handlers::health::HealthBody,
        crate::handlers::auth::RegisterBody,
        crate::handlers::auth::LoginBody,
        crate::handlers::auth::PatchMeBody,
        crate::handlers::auth::UserResponse,
        crate::handlers::installation::FireNumberMode,
        crate::handlers::installation::FireSettings,
        crate::handlers::installation::TaxBracket,
        crate::handlers::installation::InstallationSnapshot,
        crate::handlers::installation::InstallationAccess,
        crate::handlers::installation::InstallationSessionContext,
        crate::handlers::installation::SetupInstallationBody,
        crate::handlers::installation::PatchInstallationBody,
        crate::handlers::membership::MembershipRole,
        crate::handlers::pending_users::ApprovePendingUserBody,
        crate::handlers::pending_users::ApproveMemberRole,
        crate::handlers::categories::CategoryScope,
        crate::handlers::categories::CategoryResponse,
        crate::handlers::categories::CreateCategoryBody,
        crate::handlers::categories::PatchCategoryBody,
        crate::handlers::categories::DeleteCategoryQuery,
        crate::handlers::assets::AssetResponse,
        crate::handlers::assets::CreateAssetBody,
        crate::handlers::assets::PatchAssetBody,
        crate::handlers::allocation_rules::AllocationRuleResponse,
        crate::handlers::allocation_rules::CreateAllocationRuleBody,
        crate::handlers::allocation_rules::PatchAllocationRuleBody,
        crate::handlers::allocation_rules::ReorderBody,
        crate::handlers::liabilities::LiabilityResponse,
        crate::handlers::liabilities::CreateLiabilityBody,
        crate::handlers::liabilities::PatchLiabilityBody,
        crate::handlers::liabilities::PaymentFrequency,
        crate::handlers::summary::SummaryResponse,
        crate::handlers::summary::FinancialHealthMetrics,
        crate::handlers::summary::CategoryBreakdownLine,
        crate::handlers::summary::TypeTagBreakdownLine,
        crate::handlers::budget::BudgetSnapshotResponse,
        crate::handlers::budget::BudgetEntryResponse,
        crate::handlers::budget::DerivedBudgetLineResponse,
        crate::handlers::budget::BudgetTotalsResponse,
        crate::handlers::budget::CreateBudgetEntryBody,
        crate::handlers::budget::PatchBudgetEntryBody,
        crate::handlers::budget::BudgetCategoryScope,
        crate::handlers::planning::PlanningFlowResponse,
        crate::handlers::planning::PlanningFlowDirection,
        crate::handlers::planning::CreatePlanningFlowBody,
        crate::handlers::planning::PatchPlanningFlowBody,
        crate::handlers::projection::ProjectionSeriesResponse,
        crate::handlers::projection::ProjectionPoint,
        crate::handlers::history::SnapshotResponse,
        crate::handlers::history::SnapshotItemResponse,
        crate::handlers::history::CaptureResponse,
        crate::handlers::history::CaptureBody,
        crate::handlers::history::CreateSnapshotBody,
        crate::handlers::history::UpdateSnapshotBody,
        crate::handlers::history::SnapshotItemBody,
        crate::handlers::history::HistorySeriesResponse,
        crate::handlers::history::HistorySeriesPoint,
        crate::handlers::history::HistoryAssetSeries,
        crate::handlers::history::HistoryMarker,
        crate::handlers::history::PrefillResponse,
        crate::handlers::history::PrefillItemResponse,
        crate::handlers::backup_user::ExportRequest,
        crate::handlers::backup_user::ImportRequest,
        crate::handlers::backup_user::ImportPreviewResponse,
        crate::handlers::backup_user::ImportApplyResponse,
        crate::handlers::backup_user::ImportCounts,
        crate::handlers::backup_user::schema::UiPreferences,
        ErrorBody,
        ErrorCode,
    )),
    tags(
        (name = "health", description = "Liveness and readiness probes"),
        (
            name = "auth",
            description = "Username/password sessions"
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
            name = "allocation-rules",
            description = "Cascade rules that route the monthly surplus into assets, in priority order. See projection engine docs for evaluation semantics."
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
            description = "Persisted monthly income/expense budget lines; liability-derived debt payments included in snapshot only"
        ),
        (
            name = "planning",
            description = "Upcoming / planned cash flows (income or expense categories); parity PlanningTabView list CRUD"
        ),
        (
            name = "projection",
            description = "MVP serie lineal de patrimonio; no sustituye el motor completo"
        ),
        (
            name = "backup",
            description = "Backup `.ffbackup` por usuario: formato propio cifrado (AES-256-GCM, KDF Argon2id derivado de la contraseña de cuenta). Solo datos del usuario actual; portable entre instalaciones."
        ),
        (
            name = "history",
            description = "Snapshots históricos manuales de patrimonio (per-user). Captura desde el ledger propio (upsert por día) o backfill editable. CRUD Decimal-as-string; no invalidan la cache de proyección."
        ),
    ),
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
