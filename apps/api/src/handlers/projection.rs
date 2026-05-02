//! Monthly projection via `futurefin-engine`: regular budget, liability payments / principals,
//! planning flows (dated + undated 90-day window), asset contributions / drain / growth.

use crate::error::ApiError;
use crate::handlers::budget::ledger_regular_monthly_income_and_expense;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::purge_expired_liabilities;
use crate::handlers::person_view::LedgerView;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use futurefin_engine::{
    project_net_worth_series, EngineError, ProjectionFlowInput, ProjectionInput,
    ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ProjectionSeriesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Meses a proyectar (12–840, ~70 años).
    #[serde(default = "default_projection_months")]
    pub months: u32,
}

fn default_projection_months() -> u32 {
    120
}

fn resolve_ledger_view(q: &ProjectionSeriesQuery) -> LedgerView {
    match q.view.as_deref().map(str::trim) {
        Some("mine") => LedgerView::Mine,
        _ => LedgerView::Household,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectionPoint {
    pub month_index: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_worth: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub contributed_capital: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectionSeriesResponse {
    pub points: Vec<ProjectionPoint>,
    pub months: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub starting_net_worth: Decimal,
    /// Ingresos regulares − gastos regulares (sin líneas derivadas de deuda).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_delta_assumption: Decimal,
    pub model_note: String,
}

#[derive(Debug, FromRow)]
struct AssetEngineRow {
    id: Uuid,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    expected_annual_return_percent: Option<Decimal>,
    monthly_contribution_fixed: Decimal,
    contribution_remainder_weight: Decimal,
}

#[derive(Debug, FromRow)]
struct LiabEngineRow {
    principal: Decimal,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
struct FlowEngineRow {
    expected_amount: Decimal,
    due_date: Option<NaiveDate>,
    scope: String,
}

fn liability_monthly_payment(row: &LiabEngineRow) -> Decimal {
    let Some(amt) = row.payment_amount else {
        return Decimal::ZERO;
    };
    match row.payment_frequency.as_deref() {
        Some("weekly") => (amt * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amt,
    }
}

fn map_engine_err(e: EngineError) -> ApiError {
    ApiError::BadRequest(e.to_string())
}

/// Runs the dossier-style projection for the installation ledger view.
pub(crate) async fn compute_installation_projection(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    horizon_months: u32,
) -> Result<(ProjectionOutput, Decimal), ApiError> {
    let (income_reg, expense_reg) =
        ledger_regular_monthly_income_and_expense(pool, iid, session_user_id, view, today).await?;
    let monthly_net_regular = income_reg - expense_reg;

    let (projection_includes_inflation, annual_inflation_assumption_percent): (
        bool,
        Option<Decimal>,
    ) = sqlx::query_as(
        r#"SELECT projection_includes_inflation, annual_inflation_assumption_percent
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool)
    .await?;

    let inflation_annual_percent =
        if projection_includes_inflation && annual_inflation_assumption_percent.is_some() {
            annual_inflation_assumption_percent
        } else {
            None
        };

    let assets_rows: Vec<AssetEngineRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT id, current_value, purchase_price, is_liquid,
                          expected_annual_return_percent,
                          monthly_contribution_fixed, contribution_remainder_weight
                   FROM assets
                   WHERE installation_id = $1
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT id, current_value, purchase_price, is_liquid,
                          expected_annual_return_percent,
                          monthly_contribution_fixed, contribution_remainder_weight
                   FROM assets
                   WHERE installation_id = $1 AND owner_user_id = $2
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let liabs: Vec<LiabEngineRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT principal, payment_amount, payment_frequency, payment_end_date
                   FROM liabilities WHERE installation_id = $1"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT principal, payment_amount, payment_frequency, payment_end_date
                   FROM liabilities
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let flows_rows: Vec<FlowEngineRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT pf.expected_amount, pf.due_date, c.scope AS scope
                   FROM planning_flows pf
                   JOIN categories c ON c.id = pf.category_id AND c.installation_id = pf.installation_id
                   WHERE pf.installation_id = $1"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT pf.expected_amount, pf.due_date, c.scope AS scope
                   FROM planning_flows pf
                   JOIN categories c ON c.id = pf.category_id AND c.installation_id = pf.installation_id
                   WHERE pf.installation_id = $1 AND pf.owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let assets: Vec<SimAsset> = assets_rows
        .into_iter()
        .map(|r| SimAsset {
            id: r.id,
            value: r.current_value,
            purchase_price: r.purchase_price,
            is_liquid: r.is_liquid,
            expected_annual_return_percent: r.expected_annual_return_percent,
            monthly_contribution_fixed: r.monthly_contribution_fixed.max(Decimal::ZERO),
            contribution_remainder_weight: r.contribution_remainder_weight.max(Decimal::ZERO),
        })
        .collect();

    let liabilities: Vec<ProjectionLiabilityInput> = liabs
        .into_iter()
        .map(|r| ProjectionLiabilityInput {
            principal: r.principal.max(Decimal::ZERO),
            monthly_payment: liability_monthly_payment(&r),
            payment_end: r.payment_end_date,
        })
        .collect();

    let flows: Vec<ProjectionFlowInput> = flows_rows
        .into_iter()
        .filter_map(|r| {
            let is_inflow = match r.scope.as_str() {
                "income" => true,
                "expense" => false,
                _ => return None,
            };
            Some(ProjectionFlowInput {
                is_inflow,
                amount: r.expected_amount.max(Decimal::ZERO),
                due_date: r.due_date,
            })
        })
        .collect();

    let input = ProjectionInput {
        ref_date: today,
        horizon_months,
        income_regular_monthly: income_reg,
        expense_regular_monthly: expense_reg,
        assets,
        liabilities,
        flows,
        inflation_annual_percent,
    };

    let out = project_net_worth_series(&input).map_err(map_engine_err)?;
    Ok((out, monthly_net_regular))
}

#[utoipa::path(
    get,
    path = "/v1/projection/series",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` = vista titular"),
        ("months" = Option<u32>, Query, description = "Horizonte en meses (12–840), default 120"),
    ),
    responses(
        (status = 200, description = "Serie mensual motor dossier", body = ProjectionSeriesResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_projection_series(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ProjectionSeriesQuery>,
) -> Result<Json<ProjectionSeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    let months = q.months.clamp(12, 840);

    purge_expired_liabilities(&state.pool, iid).await?;
    let today = installation_naive_today(&state.pool, iid).await?;
    let view = resolve_ledger_view(&q);

    let (output, monthly_delta_assumption) = compute_installation_projection(
        &state.pool,
        iid,
        user.id.0,
        view,
        today,
        months,
    )
    .await?;

    let starting_net_worth = output
        .net_worth
        .first()
        .copied()
        .unwrap_or(Decimal::ZERO);

    let points: Vec<ProjectionPoint> = output
        .net_worth
        .iter()
        .zip(output.contributed_capital.iter())
        .enumerate()
        .map(|(i, (nw, cc))| ProjectionPoint {
            month_index: i as u32,
            net_worth: *nw,
            contributed_capital: *cc,
        })
        .collect();

    Ok(Json(ProjectionSeriesResponse {
        points,
        months,
        starting_net_worth,
        monthly_delta_assumption,
        model_note:
            "Motor mensual: presupuesto regular, servicio deuda activa, upcoming fechado / 90 días sin fecha, aportaciones y crecimiento por activo."
                .into(),
    }))
}

pub fn projection_router() -> Router {
    Router::new().route("/series", get(get_projection_series))
}
