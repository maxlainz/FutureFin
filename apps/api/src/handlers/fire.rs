//! FIRE snapshot: MVP heuristics plus engine-backed milestone vs portfolio target (withdrawal rate,
//! CGT on gain slice) using `futurefin-engine` and the same projection series as `/v1/projection/series`.

use crate::error::ApiError;
use crate::handlers::budget::{
    ledger_budget_totals_for_summary, ledger_regular_monthly_income_and_expense,
};
use crate::handlers::projection::{compute_installation_projection, mac_projection_horizon_months};
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::purge_expired_liabilities;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use futurefin_engine::{
    fire_milestone_month, portfolio_target_for_net_annual_need, FireMilestoneInput,
    FireMilestoneOutput, TaxBand,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::Serialize;
use sqlx::FromRow;
use std::str::FromStr;
use std::sync::Arc;
use utoipa::ToSchema;

const MVP_NOTE: &str = "MVP heurístico (25× gasto anual total incl. deuda derivada, meses lineales). Paralelo: hito motor vs objetivo cartera (gasto regular anual, fire_settings JSON).";

#[derive(Debug, FromRow)]
struct FireInstallationRow {
    projection_includes_inflation: bool,
    annual_inflation_assumption_percent: Option<Decimal>,
    projection_target_age: Option<i16>,
    show_age_mode: String,
    fire_settings: sqlx::types::Json<serde_json::Value>,
}

fn decimal_from_settings(settings: &serde_json::Value, key: &str, default: Decimal) -> Decimal {
    settings
        .get(key)
        .and_then(|v| match v {
            serde_json::Value::String(s) => Decimal::from_str(s.trim()).ok(),
            serde_json::Value::Number(n) => n.as_f64().and_then(Decimal::from_f64),
            _ => None,
        })
        .unwrap_or(default)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FireSnapshotResponse {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_worth: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_assets: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_liabilities: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_total_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub annual_expenses_estimate: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub portfolio_target_25x: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub gap_to_target_25x: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub implied_withdrawal_rate_vs_networth: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub estimated_months_to_25x_linear: Option<Decimal>,
    pub projection_includes_inflation: bool,
    pub projection_target_age: Option<i16>,
    pub show_age_mode: String,
    pub mvp_model_note: String,
    /// Target portfolio from engine (regular annual expenses, withdrawal rate, CGT on gains).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub engine_portfolio_target: Option<Decimal>,
    pub engine_fire_reached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_fire_reach_month_index: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/v1/fire/snapshot",
    tag = "fire",
    params(
        ("view" = Option<String>, Query, description = "`mine` = mismos agregados que resumen/presupuesto filtrados por titular."),
    ),
    responses(
        (status = 200, description = "FIRE KPIs + motor", body = FireSnapshotResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_fire_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<FireSnapshotResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    purge_expired_liabilities(&state.pool, iid).await?;
    let today = installation_naive_today(&state.pool, iid).await?;
    let view = q.resolve();

    let inst_row: Option<FireInstallationRow> = sqlx::query_as(
        r#"SELECT projection_includes_inflation, annual_inflation_assumption_percent,
                  projection_target_age, show_age_mode, fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(inst) = inst_row else {
        return Err(ApiError::NotFound);
    };

    let (total_assets, total_liabilities): (Decimal, Decimal) = match view {
        LedgerView::Household => {
            let ta: Decimal = sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(current_value), 0) FROM assets WHERE installation_id = $1"#,
            )
            .bind(iid)
            .fetch_one(&state.pool)
            .await?;
            let tl: Decimal = sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(principal), 0) FROM liabilities WHERE installation_id = $1"#,
            )
            .bind(iid)
            .fetch_one(&state.pool)
            .await?;
            (ta, tl)
        }
        LedgerView::Mine => {
            let ta: Decimal = sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(current_value), 0) FROM assets
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(user.id.0)
            .fetch_one(&state.pool)
            .await?;
            let tl: Decimal = sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(principal), 0) FROM liabilities
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(user.id.0)
            .fetch_one(&state.pool)
            .await?;
            (ta, tl)
        }
    };

    let net_worth = total_assets - total_liabilities;

    let bt = ledger_budget_totals_for_summary(&state.pool, iid, user.id.0, view, today).await?;

    let annual_expenses_estimate = bt.expense_total_monthly_equivalent * Decimal::from(12u32);

    let portfolio_target_25x = if annual_expenses_estimate > Decimal::ZERO {
        Some(annual_expenses_estimate * Decimal::from(25u32))
    } else {
        None
    };

    let gap_to_target_25x = portfolio_target_25x.map(|t| t - net_worth);

    let implied_withdrawal_rate_vs_networth =
        if net_worth > Decimal::ZERO && annual_expenses_estimate > Decimal::ZERO {
            Some(annual_expenses_estimate / net_worth)
        } else {
            None
        };

    let estimated_months_to_25x_linear =
        if let Some(gap) = gap_to_target_25x {
            if gap > Decimal::ZERO && bt.net_monthly_equivalent > Decimal::ZERO {
                Some(gap / bt.net_monthly_equivalent)
            } else {
                None
            }
        } else {
            None
        };

    let (_, expense_reg) =
        ledger_regular_monthly_income_and_expense(&state.pool, iid, user.id.0, view, today).await?;

    let settings = inst.fire_settings.0;
    let wr = decimal_from_settings(&settings, "withdrawal_rate_percent", Decimal::from(4));
    let safety =
        decimal_from_settings(&settings, "safety_multiplier", Decimal::ONE);
    let taxable_ratio =
        decimal_from_settings(&settings, "taxable_gain_ratio", Decimal::new(5, 1));
    let cgt_flat = decimal_from_settings(&settings, "cgt_flat_percent", Decimal::from(19));

    let annual_regular_need = expense_reg * Decimal::from(12u32);

    let bands = vec![TaxBand {
        band_start: Decimal::ZERO,
        rate_percent: cgt_flat,
    }];

    let engine_portfolio_target = if annual_regular_need > Decimal::ZERO && wr > Decimal::ZERO {
        Some(portfolio_target_for_net_annual_need(
            annual_regular_need,
            wr,
            taxable_ratio,
            &bands,
            safety,
        ))
    } else {
        None
    };

    let inflation_annual_percent =
        if inst.projection_includes_inflation && inst.annual_inflation_assumption_percent.is_some() {
            inst.annual_inflation_assumption_percent
        } else {
            None
        };

    let session_birth: Option<NaiveDate> = sqlx::query_scalar(
        r#"SELECT birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    let birth_dates: Vec<Option<NaiveDate>> = vec![session_birth];

    let (fire_horizon_months, _) =
        mac_projection_horizon_months(today, inst.projection_target_age, &birth_dates);

    let (proj_out, _) = compute_installation_projection(
        &state.pool,
        iid,
        user.id.0,
        view,
        today,
        fire_horizon_months,
        inflation_annual_percent,
    )
    .await?;

    let milestone = if let Some(target) = engine_portfolio_target {
        if target > Decimal::ZERO {
            let req: Vec<Decimal> = vec![target; proj_out.net_worth.len()];
            fire_milestone_month(&FireMilestoneInput {
                net_worth_series: proj_out.net_worth.clone(),
                required_portfolio_series: req,
            })
        } else {
            FireMilestoneOutput {
                reached: false,
                reach_month_index: None,
            }
        }
    } else {
        FireMilestoneOutput {
            reached: false,
            reach_month_index: None,
        }
    };

    Ok(Json(FireSnapshotResponse {
        net_worth,
        total_assets,
        total_liabilities,
        income_monthly_equivalent: bt.income_monthly_equivalent,
        expense_total_monthly_equivalent: bt.expense_total_monthly_equivalent,
        net_monthly_equivalent: bt.net_monthly_equivalent,
        annual_expenses_estimate,
        portfolio_target_25x,
        gap_to_target_25x,
        implied_withdrawal_rate_vs_networth,
        estimated_months_to_25x_linear,
        projection_includes_inflation: inst.projection_includes_inflation,
        projection_target_age: inst.projection_target_age,
        show_age_mode: inst.show_age_mode,
        mvp_model_note: MVP_NOTE.into(),
        engine_portfolio_target,
        engine_fire_reached: milestone.reached,
        engine_fire_reach_month_index: milestone.reach_month_index,
    }))
}

pub fn fire_router() -> Router {
    Router::new().route("/snapshot", get(get_fire_snapshot))
}
