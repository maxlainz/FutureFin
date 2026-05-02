use crate::error::ApiError;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

/// Unknown routes under `/v1/*` return JSON (never the SPA shell).
pub async fn v1_not_found() -> impl IntoResponse {
    ApiError::NotFound
}
