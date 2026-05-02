use crate::handlers::auth::{login, logout, me, register};
use crate::handlers::fallback;
use crate::handlers::health::{health_check, ready_check};
use crate::handlers::households::{create_household, list_households};
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
        .nest(
            "/households",
            Router::new()
                .route("/", get(list_households))
                .route("/", post(create_household)),
        )
        .fallback(fallback::v1_not_found);

    Router::new()
        .route("/health", get(health_check))
        .route("/openapi.json", get(openapi_json))
        .nest("/v1", v1)
}
