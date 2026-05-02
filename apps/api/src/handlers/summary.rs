use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::liabilities::purge_expired_liabilities;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryResponse {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_assets: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_liabilities: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_worth: Decimal,
    /// Liabilities ÷ assets when assets > 0; omitted otherwise (same quotient as Mac debt/assets).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub debt_to_assets_ratio: Option<Decimal>,
}

#[utoipa::path(
    get,
    path = "/v1/summary",
    tag = "summary",
    responses(
        (status = 200, description = "Installation aggregates (purges expired liability payment plans first, same as liabilities list)", body = SummaryResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_summary(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<SummaryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    purge_expired_liabilities(&state.pool, iid).await?;

    let total_assets: Decimal =
        sqlx::query_scalar(r#"SELECT COALESCE(SUM(current_value), 0) FROM assets WHERE installation_id = $1"#)
            .bind(iid)
            .fetch_one(&state.pool)
            .await?;

    let total_liabilities: Decimal =
        sqlx::query_scalar(r#"SELECT COALESCE(SUM(principal), 0) FROM liabilities WHERE installation_id = $1"#)
            .bind(iid)
            .fetch_one(&state.pool)
            .await?;

    let net_worth = total_assets - total_liabilities;

    let debt_to_assets_ratio = if total_assets > Decimal::ZERO {
        Some(total_liabilities / total_assets)
    } else {
        None
    };

    Ok(Json(SummaryResponse {
        total_assets,
        total_liabilities,
        net_worth,
        debt_to_assets_ratio,
    }))
}

pub fn summary_router() -> Router {
    Router::new().route("/", get(get_summary))
}
