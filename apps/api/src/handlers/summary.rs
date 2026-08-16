use crate::error::ApiError;
use crate::handlers::budget::ledger_budget_totals_for_summary;
use crate::handlers::installation::{
    installation_calendar_inflation_fire, require_installation_member, SavingsSource,
};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::{gross_up_net_annual_fire, liability_monthly_payment};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::summary::{effective_avg_income_expense, transactions_12m_avg};
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use futurefin_engine::{liquid_runway_months, RunwayOutcome};
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
    /// Cuotas mensuales derivadas de los pasivos **activos** (`payment_end_date` nula o futura).
    /// En modo A son la línea derivada del presupuesto; en los modos B/C (con datos) la base pasa a
    /// ser el gasto real promedio y este campo es exactamente el servicio de deuda nominal, de modo
    /// que `expense_total = expense_regular + expense_derived` sigue valiendo en los tres modos.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_derived_monthly_equivalent: Decimal,
    /// Gasto mensual total: presupuesto regular + cuotas derivadas en modo A; **gasto real promedio
    /// 12m (con resta híbrida de cuotas) + servicio de deuda** en los modos B/C con datos.
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
    /// Meses que los activos **líquidos** cubren el gasto total mensual, componiendo la rentabilidad
    /// esperada de esos activos (media ponderada por valor) y con el gasto creciendo a la inflación
    /// de la instalación (`futurefin_engine::liquid_runway_months`). `null` cuando no hay base de
    /// gasto (`expense_total == 0`) **o** cuando el runway es indefinido (ver `runway_is_indefinite`).
    /// El valor `1200` es el tope del bucle del servidor y significa «al menos 100 años» (un
    /// **suelo**, no una medida exacta).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub runway_months: Option<Decimal>,
    /// `true` cuando la retirada anual — `12 × expense_total_monthly_equivalent`, grosseada por los
    /// tramos fiscales de `fire_settings` igual que el target FIRE — no supera el SWR de la
    /// instalación aplicado a `liquid_assets_total`; en ese caso `runway_months` es `null`. Con
    /// gasto 0 el runway tampoco existe pero este campo es `false` (no hay base). Con
    /// `swr_pct = 0` nunca es `true`.
    pub runway_is_indefinite: bool,
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
    /// Fuente **efectiva** del ahorro que produjo los equivalentes mensuales anteriores (tras el
    /// fallback: en modo `transactions_avg` sin datos cae a `budget`). Contrato con el frontend.
    pub savings_source: SavingsSource,
    /// Meses con datos usados por el promedio real cuando `savings_source == transactions_avg`; `0`
    /// en modo `budget` (configurado o por fallback).
    pub savings_source_months_with_data: u32,
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
    today: NaiveDate,
) -> Result<
    (
        Vec<CategoryBreakdownLine>,
        Vec<CategoryBreakdownLine>,
        Vec<TypeTagBreakdownLine>,
    ),
    ApiError,
> {
    let assets_scope = view.scope_where("a");
    let assets_sql = format!(
        r#"SELECT c.id AS category_id, c.name AS category_name,
                  COALESCE(SUM(a.current_value), 0::numeric) AS total
           FROM assets a
           INNER JOIN categories c ON c.id = a.category_id AND c.installation_id = a.installation_id
           WHERE {assets_scope} AND c.scope = 'asset'
           GROUP BY c.id, c.name
           HAVING COALESCE(SUM(a.current_value), 0) > 0
           ORDER BY total DESC"#
    );
    let assets: Vec<CategoryBreakdownLine> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let liab_scope = view.scope_where("l");
    let liab_today_ph = view.next_arg_index();
    let liab_cat_sql = format!(
        r#"SELECT c.id AS category_id, c.name AS category_name,
                  COALESCE(SUM(l.principal), 0::numeric) AS total
           FROM liabilities l
           INNER JOIN categories c ON c.id = l.category_id AND c.installation_id = l.installation_id
           WHERE {liab_scope} AND c.scope = 'liability'
             AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${liab_today_ph})
           GROUP BY c.id, c.name
           HAVING COALESCE(SUM(l.principal), 0) > 0
           ORDER BY total DESC"#
    );
    let liabilities_cat: Vec<CategoryBreakdownLine> = view
        .bind_scope_as(sqlx::query_as(&liab_cat_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    let liab_tag_sql = format!(
        r#"SELECT
               CASE
                   WHEN l.type_tag IS NULL OR trim(l.type_tag) = '' THEN '(sin etiqueta)'
                   ELSE trim(l.type_tag)
               END AS type_tag,
               SUM(l.principal) AS total
           FROM liabilities l
           WHERE {liab_scope}
             AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${liab_today_ph})
           GROUP BY 1
           HAVING SUM(l.principal) > 0
           ORDER BY total DESC"#
    );
    let liabilities_tag: Vec<TypeTagBreakdownLine> = view
        .bind_scope_as(sqlx::query_as(&liab_tag_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    Ok((assets, liabilities_cat, liabilities_tag))
}

async fn planning_flow_totals_in_out(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
) -> Result<(Decimal, Decimal), ApiError> {
    let scope_where = view.scope_where("p");
    let sql = format!(
        r#"SELECT c.scope AS scope, COALESCE(SUM(p.expected_amount), 0::numeric) AS total
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {scope_where}
           GROUP BY c.scope"#
    );
    let rows: Vec<PlanningScopeAgg> = view
        .bind_scope_as(sqlx::query_as(&sql), installation_id, session_user_id)
        .fetch_all(pool)
        .await?;

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
        (status = 200, description = "Installation aggregates + financial_health (monthly equivalents según `fire_settings.savings_source`, runway de líquidos con retorno e inflación, sumas de Próximos). Los pasivos con `payment_end_date` pasada se **filtran** de las lecturas; nunca se borran (reads never mutate).", body = SummaryResponse),
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
    let out = summary_core(&state.pool, iid, user.id.0, q.resolve()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_summary`.
pub(crate) async fn summary_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<SummaryResponse, ApiError> {
    // Una sola query para los escalares de instalación que necesita este handler: fecha civil,
    // inflación (base del runway) y los fire_settings (fuente del ahorro + SWR/tramos del runway).
    let (today, inflation_pct, fire) = installation_calendar_inflation_fire(pool, iid).await?;
    let source = fire.savings_source;

    let asset_scope = view.scope_where("");
    let liab_scope = view.scope_where("");
    let liab_today_ph = view.next_arg_index();

    let total_assets_sql =
        format!("SELECT COALESCE(SUM(current_value), 0) FROM assets WHERE {asset_scope}");
    let total_liab_sql = format!(
        r#"SELECT COALESCE(SUM(principal), 0) FROM liabilities
           WHERE {liab_scope}
             AND (payment_end_date IS NULL OR payment_end_date >= ${liab_today_ph})"#
    );
    // Filas `(valor, rentabilidad anual %)` de los líquidos — mismo scope que antes de la suma. El
    // runway necesita la rentabilidad por activo (media ponderada por valor), así que la suma
    // (`liquid_assets_total`) se hace en Rust en vez de en SQL.
    let liquid_sql = format!(
        r#"SELECT current_value, expected_annual_return_percent
           FROM assets WHERE {asset_scope} AND is_liquid = true"#
    );

    let total_assets: Decimal = view
        .bind_scope_scalar(sqlx::query_scalar(&total_assets_sql), iid, user_id)
        .fetch_one(pool)
        .await?;
    let total_liabilities: Decimal = view
        .bind_scope_scalar(sqlx::query_scalar(&total_liab_sql), iid, user_id)
        .bind(today)
        .fetch_one(pool)
        .await?;
    let liquid_rows: Vec<(Decimal, Option<Decimal>)> = view
        .bind_scope_as(sqlx::query_as(&liquid_sql), iid, user_id)
        .fetch_all(pool)
        .await?;
    let liquid_assets: Decimal = liquid_rows.iter().map(|(v, _)| *v).sum();

    let budget_totals =
        ledger_budget_totals_for_summary(pool, iid, user_id, view, today).await?;

    // Base presupuesto (modo A). Los modos B/C con datos sustituyen TODA la base de gasto por el
    // promedio real 12m (y el modo B también el income): `expense_reg` = gasto real efectivo,
    // `expense_der` = servicio de deuda y `expense_tot` = suma de ambos. El runway se calcula sobre
    // `expense_tot`, así que también sigue el modo.
    let mut income_m = budget_totals.income_monthly_equivalent;
    let mut expense_reg = budget_totals.expense_regular_monthly_equivalent;
    let mut expense_der = budget_totals.expense_derived_monthly_equivalent;
    let mut expense_tot = budget_totals.expense_total_monthly_equivalent;
    let mut net_m = budget_totals.net_monthly_equivalent;

    // Fuente del ahorro efectiva (tras fallback) + meses con datos, para el response.
    let mut effective_savings_source = SavingsSource::Budget;
    let mut savings_source_months_with_data: u32 = 0;

    // `source` viene de `installation_calendar_inflation_savings` (mismo parser del JSONB
    // `fire_settings` que el engine y las mutaciones de transacciones).
    if source.uses_transactions() {
        let avg = transactions_12m_avg(pool, iid, user_id, view, today).await?;
        if avg.months_with_data > 0 {
            // Liabilities activas (por payment_end_date) con su cuota nominal mensual, con la MISMA
            // vista/scope que el resto del summary. La resta híbrida la aplica el helper compartido.
            let liab_scope2 = view.scope_where("");
            let liab_today_ph2 = view.next_arg_index();
            let liab_sql = format!(
                r#"SELECT id, payment_amount, payment_frequency
                   FROM liabilities
                   WHERE {liab_scope2}
                     AND (payment_end_date IS NULL OR payment_end_date >= ${liab_today_ph2})"#
            );
            let active_liabs: Vec<(Uuid, Option<Decimal>, Option<String>)> = view
                .bind_scope_as(sqlx::query_as(&liab_sql), iid, user_id)
                .bind(today)
                .fetch_all(pool)
                .await?;
            let active: Vec<(Uuid, Decimal)> = active_liabs
                .into_iter()
                .map(|(id, amount, freq)| {
                    (id, liability_monthly_payment(amount, freq.as_deref()))
                })
                .collect();
            let debt_service: Decimal = active.iter().map(|(_, q)| *q).sum();
            let (income_eff, expense_eff) = effective_avg_income_expense(&avg, &active);
            // Modo B: income del promedio real. Modo C: income del presupuesto (NO se sobreescribe).
            // `match` exhaustivo (como projection.rs): una variante futura fuerza decisión del
            // compilador en vez de heredar silenciosamente el `else` del modo C.
            match source {
                SavingsSource::TransactionsAvg => income_m = income_eff,
                // Modo C: income del presupuesto; `income_m` conserva su valor previo.
                SavingsSource::BudgetIncomeRealExpense => {}
                // Inalcanzable: la rama está guardada por `source.uses_transactions()`, que es
                // false para `Budget`. No-op explícito para mantener el `match` exhaustivo.
                SavingsSource::Budget => {}
            }
            expense_reg = expense_eff;
            // El `net` debe casar con modo A (`net_monthly_equivalent` de budget.rs incluye las
            // cuotas derivadas) y con la pendiente del chart (que resta el debt service). El KPI
            // `monthly_net_excluding_derived_debt` sigue siendo income − expense_reg (sin cuotas).
            // Con `income_m` (income_eff en B, presupuesto en C) una sola línea sirve a ambos modos.
            net_m = income_m - expense_eff - debt_service;
            // Base de gasto derivada/total también en modo real: la línea «derivada» pasa a ser el
            // servicio de deuda nominal de los pasivos activos y el total la suma con el gasto
            // efectivo. Así se restauran en B/C las dos identidades que en modo A siempre valen:
            //   expense_total = expense_regular + expense_derived
            //   net           = income − expense_total
            // (antes se dejaban los valores de presupuesto, que no casaban con `expense_reg`/`net`).
            expense_der = debt_service;
            expense_tot = expense_eff + debt_service;
            effective_savings_source = source;
            savings_source_months_with_data = avg.months_with_data;
        }
    }

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

    // Runway compuesto: los líquidos rinden su rentabilidad esperada mientras se drenan y el gasto
    // se infla con la inflación de la instalación. El caso «infinito» NO lo decide el tope del
    // bucle sino el SWR de la instalación sobre el gasto anual grosseado con los mismos tramos
    // fiscales que el target FIRE: infinito ⟺ gross_up(12·expense_tot) ≤ liquid·(swr/100) (ver
    // `runway.rs`). Por debajo del umbral y sin rentabilidad ni inflación se reduce EXACTO a
    // `liquid_assets / expense_tot`, que es el contrato histórico.
    let annual_expense_gross = gross_up_net_annual_fire(
        expense_tot * Decimal::from(12u32),
        &fire.tax_brackets,
        fire.taxes_enabled,
    );
    let (runway_months, runway_is_indefinite) = match liquid_runway_months(
        &liquid_rows,
        expense_tot,
        inflation_pct,
        fire.swr_pct,
        annual_expense_gross,
    ) {
        RunwayOutcome::Months(m) => (Some(m), false),
        RunwayOutcome::Indefinite => (None, true),
        RunwayOutcome::NoExpenseBase => (None, false),
    };

    let (upcoming_inflows_total, upcoming_outflows_total) =
        planning_flow_totals_in_out(pool, iid, user_id, view).await?;

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
        runway_is_indefinite,
        upcoming_inflows_total,
        upcoming_outflows_total,
        upcoming_coverage_ratio,
        savings_source: effective_savings_source,
        savings_source_months_with_data,
    };

    let net_worth = total_assets - total_liabilities;

    let debt_to_assets_ratio = if total_assets > Decimal::ZERO {
        Some(total_liabilities / total_assets)
    } else {
        None
    };

    let (assets_by_category, liabilities_by_category, liabilities_by_type_tag) =
        load_breakdown_lines(pool, iid, user_id, view, today).await?;

    Ok(SummaryResponse {
        total_assets,
        total_liabilities,
        net_worth,
        debt_to_assets_ratio,
        financial_health,
        assets_by_category,
        liabilities_by_category,
        liabilities_by_type_tag,
    })
}

pub fn summary_router() -> Router {
    Router::new().route("/", get(get_summary))
}
