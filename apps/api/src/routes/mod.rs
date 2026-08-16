use crate::handlers::allocation_rules::allocation_rules_router;
use crate::handlers::api_tokens::api_tokens_router;
use crate::handlers::assets::assets_router;
use crate::handlers::auth::{login, logout, me, patch_me, register};
use crate::handlers::backup_user::{
    export_user_backup, import_user_backup_apply, import_user_backup_preview,
};
use crate::handlers::budget::budget_router;
use crate::handlers::categories::categories_router;
use crate::handlers::fallback;
use crate::handlers::health::{health_check, ready_check};
use crate::handlers::history::history_router;
use crate::handlers::installation::{
    get_installation_session_context, get_my_installation, patch_my_installation,
    setup_installation,
};
use crate::handlers::liabilities::liabilities_router;
use crate::handlers::pending_users::pending_users_router;
use crate::handlers::planning::planning_router;
use crate::handlers::projection::projection_router;
use crate::handlers::summary::summary_router;
use crate::handlers::transactions::transactions_router;
use crate::openapi::openapi_json;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

/// Tope global del body en endpoints normales. Endpoints que reciben backups crecen su tope.
const DEFAULT_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;
/// Tope para los endpoints de import de backup (`.ffbackup` en base64 → ~33% inflado).
/// Reutilizado por las rutas de import de transacciones (CSV en base64).
pub(crate) const BACKUP_IMPORT_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

pub fn app_router(state: &Arc<AppState>) -> Router {
    let v1 = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(ready_check))
        .nest(
            "/auth",
            Router::new()
                .route("/register", post(register))
                .route("/login", post(login))
                .route("/logout", post(logout))
                .route("/me", get(me).patch(patch_me)),
        )
        .route(
            "/installation/session-context",
            get(get_installation_session_context),
        )
        .route(
            "/installation",
            get(get_my_installation).patch(patch_my_installation),
        )
        .route("/installation/setup", post(setup_installation))
        .nest("/installation/pending-users", pending_users_router())
        .nest("/api-tokens", api_tokens_router())
        .nest("/categories", categories_router())
        .nest("/assets", assets_router())
        .nest("/allocation-rules", allocation_rules_router())
        .nest("/liabilities", liabilities_router())
        .nest("/summary", summary_router())
        .nest("/budget", budget_router())
        .nest("/planning", planning_router())
        .nest("/projection", projection_router())
        .nest("/history", history_router())
        .nest("/transactions", transactions_router())
        .route("/backup/user-export", post(export_user_backup))
        .route(
            "/backup/user-import/preview",
            post(import_user_backup_preview)
                .layer(DefaultBodyLimit::max(BACKUP_IMPORT_BODY_LIMIT_BYTES)),
        )
        .route(
            "/backup/user-import",
            post(import_user_backup_apply)
                .layer(DefaultBodyLimit::max(BACKUP_IMPORT_BODY_LIMIT_BYTES)),
        )
        .fallback(fallback::v1_not_found);

    // El endpoint MCP vive en el nivel raíz (como /health y /openapi.json), dentro del
    // router api → gana siempre al fallback SPA de main.rs. Con MCP deshabilitado el
    // router ni se monta y /mcp cae al fallback (404 o index.html según despliegue).
    let mcp = if state.mcp_enabled {
        crate::mcp::mcp_router(state.clone())
    } else {
        Router::new()
    };

    Router::new()
        .route("/health", get(health_check))
        .route("/openapi.json", get(openapi_json))
        .nest("/v1", v1)
        .merge(mcp)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES))
}
