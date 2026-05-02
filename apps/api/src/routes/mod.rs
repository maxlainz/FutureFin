use crate::handlers::auth::{login, logout, me, register};
use crate::handlers::fallback;
use crate::handlers::health::{health_check, ready_check};
use crate::openapi::openapi_json;
use axum::routing::{get, post};
use axum::Router;

pub fn app_router() -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/health", get(health_check))
        .route("/v1/ready", get(ready_check))
        .route("/openapi.json", get(openapi_json))
        .nest(
            "/v1/auth",
            Router::new()
                .route("/register", post(register))
                .route("/login", post(login))
                .route("/logout", post(logout))
                .route("/me", get(me)),
        )
        .fallback(fallback::not_found)
}
