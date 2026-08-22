use crate::error::ApiError;
use crate::money::money_out;
use crate::handlers::budget::ledger_budget_totals_for_summary;
use crate::handlers::installation::{
    installation_calendar_inflation_fire, require_installation_member, SavingsSource,
};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::{
    gross_up_net_annual_fire, resolve_effective_savings_inputs, SavingsAvgBasis,
};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::summary::transactions_avg;
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

// ---------------------------------------------------------------------------
// Precisión de salida de los ratios
// ---------------------------------------------------------------------------

/// Decimales de fracción con los que se sirven los ratios (`savings_rate`,
/// `upcoming_coverage_ratio`, `debt_to_assets_ratio`…). 6 decimales de fracción = 4 decimales de
/// porcentaje (`0,0001 %` de resolución), muy por encima del único decimal que pinta la UI.
/// `rust_decimal` produce hasta 28 dígitos por división y `serde::str` los serializaba todos.
const RATIO_DP: u32 = 6;

/// Decimales de `runway_months`. Alineado con `sim_kpis` (`handlers/projection.rs`), que ya
/// redondeaba a 1: el mismo número no puede tener dos precisiones según la superficie.
const RUNWAY_DP: u32 = 1;

/// Redondeo de **presentación** de un ratio. Se aplica SIEMPRE en el último paso (al construir la
/// respuesta) y nunca sobre un valor que alimente otro cálculo: el gross-up, el umbral SWR y el
/// runway se computan con la precisión completa y solo el resultado publicado se recorta.
fn round_ratio(r: Decimal) -> Decimal {
    r.round_dp(RATIO_DP)
}
use utoipa::ToSchema;
use uuid::Uuid;

/// Agregados budget ↔ summary: equivalentes mensuales del presupuesto (cuotas de pasivo ya
/// incluidas dentro desde la 3.7.0), runway sobre los activos líquidos y sumas de Próximos.
#[derive(Debug, Serialize, ToSchema)]
pub struct FinancialHealthMetrics {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_regular_monthly_equivalent: Decimal,
    /// Gasto mensual total: el del presupuesto (cuotas de pasivo incluidas) en modo A; **gasto real
    /// promedio 12m crudo** (cuotas incluidas dentro) en los modos B/C con datos.
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
    /// Procedencia del lado INGRESO del ahorro efectivo: si salió del presupuesto o de un
    /// promedio real, con qué ventana, cuántos meses y qué rango exacto. Sustituye al escalar
    /// `savings_source_months_with_data`, que con dos ventanas ya no podía ser un solo número.
    pub savings_income_basis: SavingsAvgBasis,
    /// Procedencia del lado GASTO.
    pub savings_expense_basis: SavingsAvgBasis,
    /// Ahorro mensual **esperado** (KPI «ahorro real vs esperado»): el neto del presupuesto
    /// (`net_monthly_equivalent` del snapshot de budget, cuotas derivadas incluidas), capturado
    /// ANTES del override B/C — no sigue el modo `savings_source`, así que en B/C puede diferir
    /// del `net_monthly_equivalent` servido arriba.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub savings_expected_monthly_equivalent: Decimal,
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
    let out = summary_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
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

    let budget_totals = ledger_budget_totals_for_summary(pool, iid, user_id, view, today).await?;

    // Base presupuesto (modo A). Los modos B/C con datos sustituyen TODA la base de gasto por el
    // promedio real 12m (y el modo B también el income): `expense_reg` = promedio real crudo (las
    // cuotas de pasivo ya van dentro) y `expense_tot` = el mismo promedio. El runway se calcula
    // sobre `expense_tot`, así que también sigue el modo.
    //
    // Base de presupuesto (modo A). La cuota de pasivo ya vive dentro de
    // `expense_regular_monthly_equivalent` desde la 3.7.0, así que no hay componente derivada que
    // sumar aquí — hacerlo sería el doble conteo que 3.4.0 quitó de los modos reales.
    //
    // `expense_tot` y `net_m` se derivan más abajo de la resolución compartida, así que aquí solo
    // se declaran los dos que ENTRAN en ella.
    let mut income_m = budget_totals.income_monthly_equivalent;
    let mut expense_reg = budget_totals.expense_regular_monthly_equivalent;
    let expense_tot;
    let net_m;

    // Denominador del delta «vs plan» de la tarjeta de ahorro: siempre el neto del presupuesto,
    // capturado ANTES del override B/C — no sigue al modo.
    let savings_expected_monthly_equivalent = budget_totals.net_monthly_equivalent;

    // El promedio real solo se consulta en los modos que lo usan. Antes se calculaba SIEMPRE
    // porque el KPI «ahorro real vs esperado» lo necesitaba en los tres modos; retirado ese KPI,
    // el modo A (default) deja de tocar el ledger en el endpoint más caliente de la app.
    let avg = if source.uses_transactions() {
        Some(
            transactions_avg(
                pool,
                iid,
                user_id,
                view,
                today,
                fire.income_window(),
                fire.expense_window(),
            )
            .await?,
        )
    } else {
        None
    };

    // Resolución compartida con la proyección: mismo override, mismo fallback por lado, misma
    // fuente efectiva. Los escalares de presupuesto se pasan como parámetros porque aquí llevan
    // las cuotas de pasivo dentro y en `projection.rs` no.
    let eff = resolve_effective_savings_inputs(source, income_m, expense_reg, avg.as_ref());
    income_m = eff.income;
    // Contrato de los modos reales (reforma 3.4.0): el promedio de gasto se usa CRUDO — las cuotas
    // de pasivo ya viven dentro de los movimientos. Se mantienen las dos identidades de siempre:
    //   expense_total = expense_regular + expense_derived   (derived = 0 en los tres modos)
    //   net           = income − expense_total
    expense_reg = eff.expense;
    expense_tot = eff.expense;
    net_m = income_m - expense_tot;
    let effective_savings_source = eff.effective_source;
    let savings_income_basis = eff.income_basis;
    let savings_expense_basis = eff.expense_basis;

    // Los ratios se sirven redondeados: son PRESENTACIÓN, no entran en ningún cálculo posterior
    // (ver `round_ratio`). Los dos comparten `dp` a propósito — desde 3.7.0 son idénticos por
    // construcción y el frontend se apoya en `srx !== sr` para decidir si pinta el paréntesis.
    let savings_rate = if income_m > Decimal::ZERO {
        Some(round_ratio(net_m / income_m))
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
        // 1 decimal, alineado con `sim_kpis` (`handlers/projection.rs`): el mismo número no puede
        // publicarse con dos precisiones según por qué puerta entres. El engine sigue exacto.
        RunwayOutcome::Months(m) => (Some(m.round_dp(RUNWAY_DP)), false),
        RunwayOutcome::Indefinite => (None, true),
        RunwayOutcome::NoExpenseBase => (None, false),
    };

    let (upcoming_inflows_total, upcoming_outflows_total) =
        planning_flow_totals_in_out(pool, iid, user_id, view).await?;

    let upcoming_coverage_ratio = if upcoming_outflows_total > Decimal::ZERO {
        Some(round_ratio(upcoming_inflows_total / upcoming_outflows_total))
    } else {
        None
    };

    // `money_out` en la construcción de la respuesta, no antes: estas cifras nacen de
    // divisiones (`cuota semanal × 52 / 12`, `sum / meses reales`) y arrastraban ~25 decimales
    // hasta el JSON. Los valores crudos siguen usándose arriba para ratios y runway.
    let financial_health = FinancialHealthMetrics {
        income_monthly_equivalent: money_out(income_m),
        expense_regular_monthly_equivalent: money_out(expense_reg),
        expense_total_monthly_equivalent: money_out(expense_tot),
        net_monthly_equivalent: money_out(net_m),
        savings_rate,
        liquid_assets_total: liquid_assets,
        runway_months,
        runway_is_indefinite,
        upcoming_inflows_total,
        upcoming_outflows_total,
        upcoming_coverage_ratio,
        savings_source: effective_savings_source,
        savings_income_basis,
        savings_expense_basis,
        savings_expected_monthly_equivalent: money_out(savings_expected_monthly_equivalent),
    };

    let net_worth = total_assets - total_liabilities;

    let debt_to_assets_ratio = if total_assets > Decimal::ZERO {
        Some(round_ratio(total_liabilities / total_assets))
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
