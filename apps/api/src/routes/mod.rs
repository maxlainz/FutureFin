use crate::handlers::assets::assets_router;
use crate::handlers::auth::{login, logout, me, register};
use crate::handlers::backup::export_backup_zip;
use crate::handlers::budget::budget_router;
use crate::handlers::categories::categories_router;
use crate::handlers::fallback;
use crate::handlers::fire::fire_router;
use crate::handlers::health::{health_check, ready_check};
use crate::handlers::installation::{
    get_installation_session_context, get_my_installation, patch_my_installation,
    setup_installation,
};
use crate::handlers::liabilities::liabilities_router;
use crate::handlers::pending_users::pending_users_router;
use crate::handlers::persons::persons_router;
use crate::handlers::planning::planning_router;
use crate::handlers::projection::projection_router;
use crate::handlers::summary::summary_router;
use crate::openapi::openapi_json;
use axum::routing::{get, post};
use axum::Router;

pub fn app_router() -> Router {
    let v1 = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(ready_check))
        .nest(
            "/auth",
            Router::new()
                .route("/register", post(register))
                .route("/login", post(login))
                .route("/logout", post(logout))
                .route("/me", get(me)),
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
        .nest("/categories", categories_router())
        .nest("/assets", assets_router())
        .nest("/liabilities", liabilities_router())
        .nest("/summary", summary_router())
        .nest("/budget", budget_router())
        .nest("/planning", planning_router())
        .nest("/fire", fire_router())
        .nest("/projection", projection_router())
        .nest("/persons", persons_router())
        .route("/backup/export.zip", get(export_backup_zip))
        .fallback(fallback::v1_not_found);

    Router::new()
        .route("/health", get(health_check))
        .route("/openapi.json", get(openapi_json))
        .nest("/v1", v1)
}
