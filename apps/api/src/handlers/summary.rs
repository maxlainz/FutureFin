use crate::error::ApiError;
use crate::handlers::budget::ledger_budget_totals_for_summary;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::purge_expired_liabilities;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// MVP aggregates (budget ↔ summary): monthly equivalents from budget entries + liability-derived lines,
/// runway on liquid assets, raw sums for upcoming flows.
#[derive(Debug, Serialize, ToSchema)]
pub struct FinancialHealthMetrics {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_regular_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_derived_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_total_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_monthly_equivalent: Decimal,
    /// `net / income` when income is positive.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub savings_rate: Option<Decimal>,
    /// Income − recurring expenses (excludes liability-derived payment rows).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_net_excluding_derived_debt: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub savings_rate_excluding_derived_debt: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub liquid_assets_total: Decimal,
    /// Liquid assets ÷ total monthly expenses (including derived debt payments) when expenses are positive.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub runway_months: Option<Decimal>,
    /// Sum of **expected_amount** for income-category flows (not annualized).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_inflows_total: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_outflows_total: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub upcoming_coverage_ratio: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct PlanningScopeAgg {
    scope: String,
    total: Decimal,
}

#[derive(Debug, Serialize, ToSchema, FromRow)]
pub struct CategoryBreakdownLine {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub category_name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total: Decimal,
}

#[derive(Debug, Serialize, ToSchema, FromRow)]
pub struct TypeTagBreakdownLine {
    /// Normalized `liabilities.type_tag`; empty/null aggregated as «(sin etiqueta)».
    pub type_tag: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total: Decimal,
}

async fn load_breakdown_lines(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
) -> Result<
    (
        Vec<CategoryBreakdownLine>,
        Vec<CategoryBreakdownLine>,
        Vec<TypeTagBreakdownLine>,
    ),
    ApiError,
> {
    let assets: Vec<CategoryBreakdownLine> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT c.id AS category_id, c.name AS category_name,
                          COALESCE(SUM(a.current_value), 0::numeric) AS total
                   FROM assets a
                   INNER JOIN categories c ON c.id = a.category_id AND c.installation_id = a.installation_id
                   WHERE a.installation_id = $1 AND c.scope = 'asset'
                   GROUP BY c.id, c.name
                   HAVING COALESCE(SUM(a.current_value), 0) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT c.id AS category_id, c.name AS category_name,
                          COALESCE(SUM(a.current_value), 0::numeric) AS total
                   FROM assets a
                   INNER JOIN categories c ON c.id = a.category_id AND c.installation_id = a.installation_id
                   WHERE a.installation_id = $1 AND c.scope = 'asset' AND a.owner_user_id = $2
                   GROUP BY c.id, c.name
                   HAVING COALESCE(SUM(a.current_value), 0) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let liabilities_cat: Vec<CategoryBreakdownLine> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT c.id AS category_id, c.name AS category_name,
                          COALESCE(SUM(l.principal), 0::numeric) AS total
                   FROM liabilities l
                   INNER JOIN categories c ON c.id = l.category_id AND c.installation_id = l.installation_id
                   WHERE l.installation_id = $1 AND c.scope = 'liability'
                   GROUP BY c.id, c.name
                   HAVING COALESCE(SUM(l.principal), 0) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT c.id AS category_id, c.name AS category_name,
                          COALESCE(SUM(l.principal), 0::numeric) AS total
                   FROM liabilities l
                   INNER JOIN categories c ON c.id = l.category_id AND c.installation_id = l.installation_id
                   WHERE l.installation_id = $1 AND c.scope = 'liability' AND l.owner_user_id = $2
                   GROUP BY c.id, c.name
                   HAVING COALESCE(SUM(l.principal), 0) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let liabilities_tag: Vec<TypeTagBreakdownLine> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT
                       CASE
                           WHEN l.type_tag IS NULL OR trim(l.type_tag) = '' THEN '(sin etiqueta)'
                           ELSE trim(l.type_tag)
                       END AS type_tag,
                       SUM(l.principal) AS total
                   FROM liabilities l
                   WHERE l.installation_id = $1
                   GROUP BY 1
                   HAVING SUM(l.principal) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT
                       CASE
                           WHEN l.type_tag IS NULL OR trim(l.type_tag) = '' THEN '(sin etiqueta)'
                           ELSE trim(l.type_tag)
                       END AS type_tag,
                       SUM(l.principal) AS total
                   FROM liabilities l
                   WHERE l.installation_id = $1 AND l.owner_user_id = $2
                   GROUP BY 1
                   HAVING SUM(l.principal) > 0
                   ORDER BY total DESC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    Ok((assets, liabilities_cat, liabilities_tag))
}

async fn planning_flow_totals_in_out(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
) -> Result<(Decimal, Decimal), ApiError> {
    let rows: Vec<PlanningScopeAgg> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT c.scope AS scope, COALESCE(SUM(p.expected_amount), 0::numeric) AS total
                   FROM planning_flows p
                   JOIN categories c ON c.id = p.category_id
                   WHERE p.installation_id = $1
                   GROUP BY c.scope"#,
            )
            .bind(installation_id)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT c.scope AS scope, COALESCE(SUM(p.expected_amount), 0::numeric) AS total
                   FROM planning_flows p
                   JOIN categories c ON c.id = p.category_id
                   WHERE p.installation_id = $1 AND p.owner_user_id = $2
                   GROUP BY c.scope"#,
            )
            .bind(installation_id)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let mut inflows = Decimal::ZERO;
    let mut outflows = Decimal::ZERO;
    for r in rows {
        match r.scope.as_str() {
            "income" => inflows += r.total,
            "expense" => outflows += r.total,
            _ => {}
        }
    }
    Ok((inflows, outflows))
}

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
    /// Liabilities ÷ assets when assets > 0; omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub debt_to_assets_ratio: Option<Decimal>,
    pub financial_health: FinancialHealthMetrics,
    /// Activos por categoría (solo filas con total positivo).
    pub assets_by_category: Vec<CategoryBreakdownLine>,
    pub liabilities_by_category: Vec<CategoryBreakdownLine>,
    /// Pasivos agrupados por `type_tag`.
    pub liabilities_by_type_tag: Vec<TypeTagBreakdownLine>,
}

#[utoipa::path(
    get,
    path = "/v1/summary",
    tag = "summary",
    params(
        ("view" = Option<String>, Query, description = "`mine` = sums for rows attributed to the signed-in user; omit = household."),
    ),
    responses(
        (status = 200, description = "Installation aggregates + MVP financial_health (budget-aligned monthly equivalents, runway, upcoming sums); purges expired liability payment plans first", body = SummaryResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_summary(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<SummaryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    purge_expired_liabilities(&state.pool, iid).await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    let view = q.resolve();

    let (total_assets, total_liabilities, liquid_assets): (Decimal, Decimal, Decimal) =
        match view {
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
                let liq: Decimal = sqlx::query_scalar(
                    r#"SELECT COALESCE(SUM(current_value), 0) FROM assets
                       WHERE installation_id = $1 AND is_liquid = true"#,
                )
                .bind(iid)
                .fetch_one(&state.pool)
                .await?;
                (ta, tl, liq)
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
                let liq: Decimal = sqlx::query_scalar(
                    r#"SELECT COALESCE(SUM(current_value), 0) FROM assets
                       WHERE installation_id = $1 AND owner_user_id = $2 AND is_liquid = true"#,
                )
                .bind(iid)
                .bind(user.id.0)
                .fetch_one(&state.pool)
                .await?;
                (ta, tl, liq)
            }
        };

    let budget_totals =
        ledger_budget_totals_for_summary(&state.pool, iid, user.id.0, view, today).await?;

    let income_m = budget_totals.income_monthly_equivalent;
    let expense_reg = budget_totals.expense_regular_monthly_equivalent;
    let expense_der = budget_totals.expense_derived_monthly_equivalent;
    let expense_tot = budget_totals.expense_total_monthly_equivalent;
    let net_m = budget_totals.net_monthly_equivalent;

    let monthly_net_excluding_derived_debt = income_m - expense_reg;

    let savings_rate = if income_m > Decimal::ZERO {
        Some(net_m / income_m)
    } else {
        None
    };

    let savings_rate_excluding_derived_debt = if income_m > Decimal::ZERO {
        Some(monthly_net_excluding_derived_debt / income_m)
    } else {
        None
    };

    let runway_months = if expense_tot > Decimal::ZERO {
        Some(liquid_assets / expense_tot)
    } else {
        None
    };

    let (upcoming_inflows_total, upcoming_outflows_total) =
        planning_flow_totals_in_out(&state.pool, iid, user.id.0, view).await?;

    let upcoming_coverage_ratio = if upcoming_outflows_total > Decimal::ZERO {
        Some(upcoming_inflows_total / upcoming_outflows_total)
    } else {
        None
    };

    let financial_health = FinancialHealthMetrics {
        income_monthly_equivalent: income_m,
        expense_regular_monthly_equivalent: expense_reg,
        expense_derived_monthly_equivalent: expense_der,
        expense_total_monthly_equivalent: expense_tot,
        net_monthly_equivalent: net_m,
        savings_rate,
        monthly_net_excluding_derived_debt,
        savings_rate_excluding_derived_debt,
        liquid_assets_total: liquid_assets,
        runway_months,
        upcoming_inflows_total,
        upcoming_outflows_total,
        upcoming_coverage_ratio,
    };

    let net_worth = total_assets - total_liabilities;

    let debt_to_assets_ratio = if total_assets > Decimal::ZERO {
        Some(total_liabilities / total_assets)
    } else {
        None
    };

    let (assets_by_category, liabilities_by_category, liabilities_by_type_tag) =
        load_breakdown_lines(&state.pool, iid, user.id.0, view).await?;

    Ok(Json(SummaryResponse {
        total_assets,
        total_liabilities,
        net_worth,
        debt_to_assets_ratio,
        financial_health,
        assets_by_category,
        liabilities_by_category,
        liabilities_by_type_tag,
    }))
}

pub fn summary_router() -> Router {
    Router::new().route("/", get(get_summary))
}
