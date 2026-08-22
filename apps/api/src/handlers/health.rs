use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use serde::Serialize;
use std::sync::Arc;
use tracing::error;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct HealthBody {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
}

#[utoipa::path(
    get,
    path = "/v1/health",
    tag = "health",
    responses(
        (status = 200, description = "Process up", body = HealthBody),
    )
)]
pub async fn health_check(Extension(state): Extension<Arc<AppState>>) -> Json<HealthBody> {
    Json(HealthBody {
        status: "ok",
        service: "futurefin",
        version: state.version,
    })
}

#[utoipa::path(
    get,
    path = "/v1/ready",
    tag = "health",
    responses(
        (status = 200, description = "Dependencies reachable", body = HealthBody),
        (status = 503, description = "Dependency unavailable (e.g. database)"),
    )
)]
pub async fn ready_check(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<HealthBody>, ApiError> {
    let ok: i32 = match sqlx::query_scalar(r#"SELECT 1"#)
        .fetch_one(&state.pool)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            error!(?e, "readiness database check failed");
            return Err(ApiError::Unavailable);
        }
    };
    if ok != 1 {
        return Err(ApiError::BadRequest("readiness_probe_unexpected: unexpected readiness probe".into()));
    }
    Ok(Json(HealthBody {
        status: "ok",
        service: "futurefin",
        version: state.version,
    }))
}
