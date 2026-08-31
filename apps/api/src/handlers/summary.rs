use crate::error::ApiError;
use crate::money::money_out;
use crate::handlers::budget::ledger_budget_totals_for_summary;
use crate::handlers::installation::{
    installation_calendar_inflation_fire, require_installation_member, SavingsSource,
};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::{resolve_effective_savings_inputs, SavingsAvgBasis};
use futurefin_engine::gross_up_net_annual_fire;
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

/// Decimales de fracción con los que se sirven las cifras que ya vienen **en porcentaje**
/// (`net_return_*_annual_pct`). 4 decimales de porcentaje = exactamente la misma resolución que
/// `RATIO_DP` (6 decimales de fracción): la política de precisión es una, aunque la unidad del
/// campo sea otra. Igual que `round_ratio`, es redondeo de PRESENTACIÓN — el engine calcula
/// exacto y solo se recorta lo publicado.
const PCT_DP: u32 = 4;

/// `plan` | `actual` | `mixed` a partir de la procedencia de los dos lados del ahorro. Es una
/// función pura sobre los `SavingsAvgBasis` que ya se publican, así que no puede desincronizarse
/// de ellos: si algún día un lado gana una tercera procedencia, este `match` deja de compilar.
fn financial_health_basis(income: &SavingsAvgBasis, expense: &SavingsAvgBasis) -> &'static str {
    match (income.basis, expense.basis) {
        ("budget", "budget") => "plan",
        ("average", "average") => "actual",
        _ => "mixed",
    }
}

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
/// Agregados budget ↔ summary.
///
/// **UNIDADES.** Este objeto es plano y mezcla cinco escalas: importes (euros/mes), fracciones,
/// porcentajes, un ratio adimensional y meses. Cada campo declara la suya abajo con la marca
/// `**Unidad:**`, que viaja al schema de OpenAPI y a la descripción de la tool MCP. **No están en
/// los nombres a propósito**: renombrar `savings_rate` → `savings_rate_fraction` (y su gemelo
/// `debt_to_assets_ratio`) es un cambio breaking sobre dos KPIs de portada que toca la SPA, las dos
/// superficies MCP y el catálogo de textos de ayuda, y compra menos que declararlo — la unidad es
/// una propiedad del CAMPO, constante en todas las respuestas, así que su sitio es el esquema y no
/// 200 bytes repetidos en el endpoint más caliente de la app. Regla de lectura: `savings_rate`
/// `0.35` es 35 %; `net_return_nominal_annual_pct` `3.5556` es 3,5556 %.
#[derive(Debug, Serialize, ToSchema)]
pub struct FinancialHealthMetrics {
    /// **Unidad: euros/mes.** Ingreso mensual equivalente según el modo (`savings_source`); ver
    /// `basis`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    /// **Unidad: euros/mes.**
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_regular_monthly_equivalent: Decimal,
    /// **Unidad: euros/mes.** Gasto mensual total: el del presupuesto (cuotas de pasivo incluidas)
    /// en modo A; **gasto real promedio 12m crudo** (cuotas incluidas dentro) en los modos B/C con
    /// datos.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_total_monthly_equivalent: Decimal,
    /// **Unidad: euros/mes.** `income_monthly_equivalent − expense_total_monthly_equivalent`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_monthly_equivalent: Decimal,
    /// **Unidad: fracción** (`0.35` = 35 %, NO 0,35 %). `net / income` cuando el ingreso es
    /// positivo; ausente si no lo es.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub savings_rate: Option<Decimal>,
    /// **Unidad: euros** (saldo, no flujo). Σ `current_value` de los activos con `is_liquid`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub liquid_assets_total: Decimal,
    /// **Unidad: meses.** Meses que los activos **líquidos** cubren el gasto total mensual,
    /// drenándolos en el MISMO orden que la simulación real — menor rentabilidad esperada primero,
    /// cada saldo restante componiendo la suya (#128; hasta 4.7.x se usaba una media ponderada por
    /// valor, sistemáticamente más corta en carteras mixtas) — y con el gasto creciendo a la
    /// inflación de la instalación (`futurefin_engine::liquid_runway_months`). Desde 4.10.0
    /// (gemelo de #140) el bucle vende **BRUTO**: cubrir el gasto realiza plusvalía, con la
    /// misma escala y la misma `taxable_gain_ratio` que el objetivo — la identidad
    /// «líquidos / gasto» solo sobrevive con impuestos apagados. `null` cuando no hay
    /// base de gasto (`expense_total == 0`) **o** cuando el runway es indefinido (ver
    /// `runway_is_indefinite`). El valor `1200` es el tope del bucle del servidor y significa
    /// «al menos 100 años» (un **suelo**, no una medida exacta).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub runway_months: Option<Decimal>,
    /// `true` cuando la retirada anual — `12 × expense_total_monthly_equivalent`, grosseada por los
    /// tramos fiscales de `fire_settings` igual que el target FIRE — no supera el SWR de la
    /// instalación aplicado a `liquid_assets_total` **y** la cartera líquida tiene rentabilidad
    /// esperada ponderada > 0 (#128: la regla del SWR se validó para carteras invertidas — el
    /// dinero parado al 0 % nunca es «indefinido», por grande que sea el saldo); en ese caso
    /// `runway_months` es `null`. Con gasto 0 el runway tampoco existe pero este campo es `false`
    /// (no hay base). Con `swr_pct = 0` nunca es `true`.
    pub runway_is_indefinite: bool,
    /// Σ de `expected_amount` de **TODOS** los Próximos (`planning_flows`) del scope cuya
    /// categoría es de scope `income`. **Sin ventana temporal y sin anualizar**: entra igual un
    /// cobro previsto para el mes que viene que uno con `due_date` a dieciséis años, y entran
    /// también los que **no tienen fecha**. No es un flujo mensual ni comparable con
    /// `income_monthly_equivalent`. Para saber hasta dónde llega el horizonte que se está sumando,
    /// mira `upcoming_last_due_date_ymd`; para cuántos conceptos, `upcoming_flows_count`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_inflows_total: Decimal,
    /// Lo mismo para las categorías de scope `expense`. Mismas advertencias: sin ventana, sin
    /// anualizar, con los flujos sin fecha dentro.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_outflows_total: Decimal,
    /// **Unidad: ratio adimensional** (`1.5` = las entradas cubren 1,5 veces las salidas).
    /// `upcoming_inflows_total / upcoming_outflows_total` cuando el denominador es > 0; ausente si
    /// no hay salidas previstas. Es una **fracción** (1.5 = las entradas cubren 1,5 veces las
    /// salidas), no un porcentaje, y hereda la ausencia de ventana de sus dos operandos: puede
    /// dividir un cobro a dieciséis años vista entre un pago del mes que viene. No lo compares con
    /// `runway_months`, que sí es una medida temporal.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub upcoming_coverage_ratio: Option<Decimal>,
    /// Nº de Próximos (entradas + salidas) que suman en los dos totales anteriores. `0` ⟺ ambos
    /// totales son 0 de verdad, y no «hay flujos pero se anulan».
    pub upcoming_flows_count: i64,
    /// `due_date` **más lejana** entre los Próximos contados, `YYYY-MM-DD`. `null` cuando ninguno
    /// tiene fecha (o no hay ninguno). Es la ventana que los totales de arriba NO declaran: sin
    /// esto, `upcoming_outflows_total` mezcla un pago del mes que viene con uno de 2042 y las dos
    /// lecturas son indistinguibles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upcoming_last_due_date_ymd: Option<String>,
    /// Fuente **efectiva** del ahorro que produjo los equivalentes mensuales anteriores (tras el
    /// fallback: en modo `transactions_avg` sin datos cae a `budget`). Contrato con el frontend.
    pub savings_source: SavingsSource,
    /// Procedencia del lado INGRESO del ahorro efectivo: si salió del presupuesto o de un
    /// promedio real, con qué ventana, cuántos meses y qué rango exacto. Sustituye al escalar
    /// `savings_source_months_with_data`, que con dos ventanas ya no podía ser un solo número.
    pub savings_income_basis: SavingsAvgBasis,
    /// Procedencia del lado GASTO.
    pub savings_expense_basis: SavingsAvgBasis,
    /// **Plan o realidad**, en una palabra: `"plan"` | `"actual"` | `"mixed"`.
    ///
    /// Se deriva de los dos `*_basis` de arriba (`plan` ⟺ los dos lados salieron del presupuesto;
    /// `actual` ⟺ los dos promediaron movimientos reales; `mixed` ⟺ uno de cada, que es lo normal
    /// en el modo C y lo que pasa en el B cuando un lado se queda sin meses reales).
    ///
    /// Existe por una colisión de nombres, no por gusto: `income_monthly_equivalent`,
    /// `expense_regular_monthly_equivalent`, `expense_total_monthly_equivalent` y
    /// `net_monthly_equivalent` se llaman **exactamente igual** aquí y en
    /// `GET /v1/budget` → `totals`, y allí son SIEMPRE el plan (`totals.basis == "plan"`), mientras
    /// que aquí siguen a `savings_source`. Con `basis != "plan"` las dos cuartetas describen cosas
    /// distintas y restarlas no significa nada. La información ya estaba repartida entre
    /// `savings_source` y los dos `*_basis`; lo que faltaba era el campo que se lee de un vistazo.
    pub basis: &'static str,
    /// **Unidad: euros/mes.** Ahorro mensual **esperado** (KPI «ahorro real vs esperado»): el neto del presupuesto
    /// (`net_monthly_equivalent` del snapshot de budget, cuotas derivadas incluidas), capturado
    /// ANTES del override B/C — no sigue el modo `savings_source`, así que en B/C puede diferir
    /// del `net_monthly_equivalent` servido arriba.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub savings_expected_monthly_equivalent: Decimal,
    /// **Unidad: porcentaje anual** (`"3.5556"` = 3,5556 %/año), NO fracción — al revés que
    /// `savings_rate`.
    ///
    /// Rendimiento anual **nominal** esperado del patrimonio neto, en porcentaje (`"3.5556"` =
    /// 3,5556 %/año). Numerador: la suma de `current_value × expected_annual_return_percent` de
    /// TODOS los activos del scope menos la de `principal × apr_percent` de los pasivos que
    /// **devengan** (#121: modelo con intereses + TIN > 0 + plan vivo — el mismo predicado del
    /// motor, `liability_interest_accrues`); los visibles que no devengan (p. ej. plan vencido
    /// con saldo, #145) pesan solo en el denominador, a coste 0. Denominador: `net_worth`. Un activo sin
    /// rentabilidad configurada o un pasivo sin TIN cuentan como 0 % pero **siguen pesando** en el
    /// denominador. Se **omite** cuando `net_worth ≤ 0` (el cociente cambiaría de signo o
    /// divergiría). Lo calcula `futurefin_engine::net_return_percentages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub net_return_nominal_annual_pct: Option<Decimal>,
    /// **Unidad: porcentaje anual**, igual que su hermano nominal.
    ///
    /// El mismo rendimiento descontada `installation.annual_inflation_assumption_percent`, por
    /// **división de factores** (`(1+n/100)/(1+i/100) − 1`), no por resta de puntos. Presente y
    /// ausente exactamente a la vez que `net_return_nominal_annual_pct`; con inflación 0 son
    /// idénticos.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub net_return_real_annual_pct: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct PlanningScopeAgg {
    scope: String,
    total: Decimal,
    /// Nº de flujos del scope (fechados y sin fechar).
    flow_count: i64,
    /// `due_date` máxima del scope; `NULL` si ninguno de sus flujos lleva fecha.
    last_due_date: Option<NaiveDate>,
}

/// Los tres agregados de Próximos que publica `financial_health`: totales por scope, cuántos
/// flujos los componen y hasta dónde llegan en el calendario.
struct UpcomingAgg {
    inflows: Decimal,
    outflows: Decimal,
    count: i64,
    last_due_date: Option<NaiveDate>,
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
    /// `liabilities.type_tag` normalizado (trim). **`null` = pasivos sin etiqueta**, agregados en
    /// una sola línea.
    ///
    /// Hasta 4.4.0 ese caso viajaba como la cadena literal `"(sin etiqueta)"`: texto de interfaz,
    /// en español, dentro de un campo de datos — indistinguible de un usuario que hubiera
    /// etiquetado de verdad un pasivo con ese nombre, y una cadena que ningún cliente puede
    /// reenviar como filtro. `null` es el mismo criterio que ya usa `category_id` en
    /// `CategoryMonthlySeriesEntry` para «movimientos sin categoría».
    ///
    /// La dimensión se escribe con `type_tag` en `POST`/`PATCH /v1/liabilities` (y viaja en
    /// `LiabilityResponse.type_tag`): es texto libre del usuario, no un enum del servidor, así que
    /// aquí no hay lista cerrada de valores.
    pub type_tag: Option<String>,
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
             AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${liab_today_ph} OR l.principal > 0)
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
                   WHEN l.type_tag IS NULL OR trim(l.type_tag) = '' THEN NULL
                   ELSE trim(l.type_tag)
               END AS type_tag,
               SUM(l.principal) AS total
           FROM liabilities l
           WHERE {liab_scope}
             AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${liab_today_ph} OR l.principal > 0)
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

/// Agregado de Próximos por scope. **Sin ventana temporal**: suma todos los `planning_flows` del
/// scope, con y sin `due_date`. La misma query devuelve ahora el recuento y la fecha máxima, que
/// son lo que permite leer los totales sin inventarse un horizonte (ver `upcoming_*` en
/// [`FinancialHealthMetrics`]); no añade ninguna consulta, solo dos columnas al `GROUP BY` que ya
/// existía.
async fn planning_flow_totals_in_out(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
) -> Result<UpcomingAgg, ApiError> {
    let scope_where = view.scope_where("p");
    let sql = format!(
        r#"SELECT c.scope AS scope, COALESCE(SUM(p.expected_amount), 0::numeric) AS total,
                  COUNT(*)::bigint AS flow_count, MAX(p.due_date) AS last_due_date
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {scope_where}
           GROUP BY c.scope"#
    );
    let rows: Vec<PlanningScopeAgg> = view
        .bind_scope_as(sqlx::query_as(&sql), installation_id, session_user_id)
        .fetch_all(pool)
        .await?;

    let mut agg = UpcomingAgg {
        inflows: Decimal::ZERO,
        outflows: Decimal::ZERO,
        count: 0,
        last_due_date: None,
    };
    for r in rows {
        // Solo `income` y `expense` suman: una categoría de otro scope no es un Próximo, y su
        // recuento tampoco debe describir cifras en las que no entra.
        match r.scope.as_str() {
            "income" => agg.inflows += r.total,
            "expense" => agg.outflows += r.total,
            _ => continue,
        }
        agg.count += r.flow_count;
        agg.last_due_date = match (agg.last_due_date, r.last_due_date) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    Ok(agg)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryResponse {
    /// Vista efectivamente aplicada: `household` | `mine`. **Eco de `?view`, no un dato nuevo.**
    ///
    /// Existe porque en una instalación de un solo usuario `?view=mine` y `?view` omitido
    /// devolvían payloads byte a byte idénticos: era imposible distinguir «mine coincide con el
    /// hogar» de «el parámetro se ignoró». En un hogar de dos personas ésa es exactamente la
    /// pregunta que decide si la cifra que estás citando es la del hogar o la tuya. Reenviar este
    /// valor como `?view=` reproduce esta misma respuesta (`LedgerView::as_str`).
    pub view: &'static str,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_assets: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_liabilities: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_worth: Decimal,
    /// **Unidad: fracción** (`0.42` = 42 %, NO 0,42 %), misma escala que
    /// `financial_health.savings_rate`. Pasivos ÷ activos cuando hay activos; ausente si no.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub debt_to_assets_ratio: Option<Decimal>,
    pub financial_health: FinancialHealthMetrics,
    /// Activos por categoría (solo filas con total positivo).
    pub assets_by_category: Vec<CategoryBreakdownLine>,
    pub liabilities_by_category: Vec<CategoryBreakdownLine>,
    /// Pasivos visibles agrupados por `type_tag` (el mismo predicado que `total_liabilities`:
    /// plan vivo O saldo vivo — #145; solo el vencido y saldado queda fuera), solo líneas con
    /// `SUM(principal) > 0`, orden `total DESC`. La línea con `type_tag: null` agrupa los pasivos
    /// sin etiquetar. Es un corte por una dimensión que el usuario escribe libremente en
    /// `POST`/`PATCH /v1/liabilities` (`type_tag`), no por categoría: el desglose por categoría es
    /// `liabilities_by_category`, y los dos suman lo mismo.
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
        (status = 200, description = "Installation aggregates + financial_health (monthly equivalents según `fire_settings.savings_source`, runway de líquidos con retorno e inflación, rendimiento neto anual esperado del patrimonio —nominal y real—, sumas de Próximos). Los pasivos con `payment_end_date` pasada se **filtran** de las lecturas; nunca se borran (reads never mutate).", body = SummaryResponse),
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

    // Filas `(valor, rentabilidad anual %, líquido)` de TODOS los activos del scope. Antes se
    // pedían dos cosas por separado —la suma total en SQL y las filas de los líquidos— pero el
    // rendimiento neto necesita la rentabilidad de todos, no solo de los líquidos: una sola query
    // sirve a los tres consumidores (total, runway, rendimiento) y el filtro `is_liquid` se hace
    // en Rust. El runway sigue recibiendo EXACTAMENTE las mismas filas que antes.
    let assets_sql = format!(
        r#"SELECT current_value, expected_annual_return_percent, is_liquid
           FROM assets WHERE {asset_scope}"#
    );
    // Ídem con los pasivos: `(principal, TIN %)` con el predicado de visibilidad de #145 (plan
    // vivo o saldo vivo) — el mismo `WHERE` que sirve la suma escalar. Se suma en Rust.
    let liab_sql = format!(
        r#"SELECT principal, apr_percent, repayment_model, payment_amount, payment_end_date
           FROM liabilities
           WHERE {liab_scope}
             AND (payment_end_date IS NULL OR payment_end_date >= ${liab_today_ph} OR principal > 0)"#
    );

    let asset_rows: Vec<(Decimal, Option<Decimal>, bool)> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, user_id)
        .fetch_all(pool)
        .await?;
    let liab_raw: Vec<(Decimal, Option<Decimal>, String, Option<Decimal>, Option<chrono::NaiveDate>)> = view
        .bind_scope_as(sqlx::query_as(&liab_sql), iid, user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;
    // #121 — una sola base de coste de la deuda: el numerador del net_return solo resta el TIN
    // de los pasivos que el motor DE VERDAD devenga (`liability_interest_accrues`, el mismo
    // predicado de `liability_month`). La fila que no devenga NO se excluye: su principal sigue
    // pesando en el denominador (es deuda de verdad) con coste 0 — exactamente el contrato
    // «None cuenta como 0 %» de `net_return_percentages`. `payment_frequency` no hace falta:
    // el predicado solo pregunta si hay cuota > 0, y los modelos que devengan son mensuales
    // por validación.
    let liab_rows: Vec<(Decimal, Option<Decimal>)> = liab_raw
        .iter()
        .map(|(principal, apr, model, payment, end)| {
            // Degradar en LECTURAS (misma filosofía que projection.rs): un literal corrupto
            // escrito fuera de la API cae al modelo sin intereses (coste 0) en vez de tumbar
            // el Resumen entero. La validación ruidosa vive en la escritura.
            let model = crate::handlers::liabilities::RepaymentModel::parse(model)
                .map(crate::handlers::liabilities::RepaymentModel::to_engine)
                .unwrap_or(futurefin_engine::RepaymentModel::FixedPayments);
            let accrues = futurefin_engine::liability_interest_accrues(
                model,
                *apr,
                payment.unwrap_or(Decimal::ZERO),
                *end,
                today,
            );
            (*principal, if accrues { *apr } else { None })
        })
        .collect();

    let total_assets: Decimal = asset_rows.iter().map(|(v, _, _)| *v).sum();
    let total_liabilities: Decimal = liab_raw.iter().map(|(p, _, _, _, _)| *p).sum();
    let liquid_rows: Vec<(Decimal, Option<Decimal>)> = asset_rows
        .iter()
        .filter(|(_, _, is_liquid)| *is_liquid)
        .map(|(v, r, _)| (*v, *r))
        .collect();
    let liquid_assets: Decimal = liquid_rows.iter().map(|(v, _)| *v).sum();
    let asset_return_rows: Vec<(Decimal, Option<Decimal>)> =
        asset_rows.iter().map(|(v, r, _)| (*v, *r)).collect();

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
    // #140 fase 2: el umbral del runway pasa g — la misma venta y el mismo impuesto que el
    // objetivo; dejarlo a g=1 reabriría la asimetría en otra tarjeta.
    let annual_expense_gross = gross_up_net_annual_fire(
        expense_tot * Decimal::from(12u32),
        &fire.tax_brackets,
        fire.taxes_enabled,
        fire.taxable_gain_ratio,
    );
    let (runway_months, runway_is_indefinite) = match liquid_runway_months(
        &liquid_rows,
        expense_tot,
        inflation_pct,
        fire.swr_pct,
        annual_expense_gross,
        &fire.tax_brackets,
        fire.taxes_enabled,
        fire.taxable_gain_ratio,
    ) {
        // 1 decimal, alineado con `sim_kpis` (`handlers/projection.rs`): el mismo número no puede
        // publicarse con dos precisiones según por qué puerta entres. El engine sigue exacto.
        RunwayOutcome::Months(m) => (Some(m.round_dp(RUNWAY_DP)), false),
        RunwayOutcome::Indefinite => (None, true),
        RunwayOutcome::NoExpenseBase => (None, false),
    };

    // Rendimiento neto anual esperado del patrimonio: lo que rinden los activos según la
    // rentabilidad que el usuario configuró en cada uno, menos el interés de los pasivos que
    // DEVENGAN (#121: mismo predicado que el motor — modelo con intereses, TIN > 0, plan vivo),
    // sobre el patrimonio neto (las mismas filas que producen `total_assets` y
    // `total_liabilities`, así que el denominador ES `net_worth`). `None` ⟺ patrimonio ≤ 0.
    // El redondeo es de publicación (`PCT_DP`): el engine devuelve el valor exacto.
    let net_return = futurefin_engine::net_return_percentages(
        &asset_return_rows,
        &liab_rows,
        inflation_pct,
    );
    let net_return_nominal_annual_pct = net_return.as_ref().map(|r| r.nominal_pct.round_dp(PCT_DP));
    let net_return_real_annual_pct = net_return.as_ref().map(|r| r.real_pct.round_dp(PCT_DP));

    let upcoming = planning_flow_totals_in_out(pool, iid, user_id, view).await?;
    let upcoming_inflows_total = upcoming.inflows;
    let upcoming_outflows_total = upcoming.outflows;

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
        upcoming_flows_count: upcoming.count,
        upcoming_last_due_date_ymd: upcoming
            .last_due_date
            .map(|d| d.format("%Y-%m-%d").to_string()),
        savings_source: effective_savings_source,
        // Se deriva ANTES de mover los dos basis al struct (ambos se consumen por valor).
        basis: financial_health_basis(&savings_income_basis, &savings_expense_basis),
        savings_income_basis,
        savings_expense_basis,
        savings_expected_monthly_equivalent: money_out(savings_expected_monthly_equivalent),
        net_return_nominal_annual_pct,
        net_return_real_annual_pct,
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
        view: view.as_str(),
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
