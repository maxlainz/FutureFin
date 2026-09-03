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
pub(crate) const RATIO_DP: u32 = 6;

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
    /// Σ de `expected_amount` de los Próximos **PUNTUALES** (`amount_basis = one_off`) del scope
    /// cuya categoría es de scope `income`. **Sin ventana temporal y sin anualizar**: entra igual
    /// un cobro previsto para el mes que viene que uno con `due_date` a dieciséis años, y entran
    /// también los que **no tienen fecha**. No es un flujo mensual ni comparable con
    /// `income_monthly_equivalent`. Los recurrentes (#148) van aparte en
    /// `upcoming_recurring_monthly_inflow` — son €/MES y sumarlos aquí mezclaría magnitudes.
    /// Para saber hasta dónde llega el horizonte que se está sumando, mira
    /// `upcoming_last_due_date_ymd`; para cuántos conceptos, `upcoming_flows_count`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_inflows_total: Decimal,
    /// Lo mismo para las categorías de scope `expense`. Mismas advertencias: sin ventana, sin
    /// anualizar, con los flujos sin fecha dentro, y solo puntuales.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_outflows_total: Decimal,
    /// **Unidad: €/MES** (#148) — Σ de `expected_amount` de los Próximos recurrentes
    /// (`amount_basis = per_month`) de scope `income`, **sin mirar sus ventanas**: un alquiler
    /// que empieza en 2027 suma igual que uno vigente. Es una intensidad, no un total; jamás
    /// se suma con `upcoming_inflows_total` (€).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_recurring_monthly_inflow: Decimal,
    /// Lo mismo para scope `expense`. €/MES.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub upcoming_recurring_monthly_outflow: Decimal,
    /// Nº de Próximos recurrentes (los que suman en los dos campos €/mes anteriores).
    pub upcoming_recurring_count: i64,
    /// **Unidad: ratio adimensional** (`1.5` = las entradas cubren 1,5 veces las salidas).
    /// `upcoming_inflows_total / upcoming_outflows_total` cuando el denominador es > 0; ausente si
    /// no hay salidas previstas. Es una **fracción** (1.5 = las entradas cubren 1,5 veces las
    /// salidas), no un porcentaje, y hereda la ausencia de ventana de sus dos operandos: puede
    /// dividir un cobro a dieciséis años vista entre un pago del mes que viene. **Base solo
    /// puntuales** (#148): los recurrentes no entran en ninguno de los dos operandos. No lo
    /// compares con `runway_months`, que sí es una medida temporal.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub upcoming_coverage_ratio: Option<Decimal>,
    /// Nº de Próximos (entradas + salidas) del scope, **puntuales Y recurrentes** — cuenta todo
    /// lo que existe, no solo lo que suma en los totales en €. `0` ⟺ no hay ningún Próximo.
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
    /// `true` = el grupo son flujos `per_month` (#148): su `total` está en €/MES, no en €.
    per_month: bool,
    total: Decimal,
    /// Nº de flujos del grupo (fechados y sin fechar).
    flow_count: i64,
    /// `due_date` máxima del grupo; `NULL` si ninguno de sus flujos lleva fecha (los
    /// `per_month` no llevan nunca — CHECK de la tabla).
    last_due_date: Option<NaiveDate>,
}

/// Los agregados de Próximos que publica `financial_health`. Los totales `inflows`/`outflows`
/// son SOLO de puntuales (€); los recurrentes van aparte en €/MES (#148) — sumarlos en el mismo
/// campo sería un error de magnitud (€ + €/mes).
struct UpcomingAgg {
    inflows: Decimal,
    outflows: Decimal,
    recurring_monthly_inflow: Decimal,
    recurring_monthly_outflow: Decimal,
    recurring_count: i64,
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
        r#"SELECT c.scope AS scope, (p.amount_basis = 'per_month') AS per_month,
                  COALESCE(SUM(p.expected_amount), 0::numeric) AS total,
                  COUNT(*)::bigint AS flow_count, MAX(p.due_date) AS last_due_date
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {scope_where}
           GROUP BY c.scope, (p.amount_basis = 'per_month')"#
    );
    let rows: Vec<PlanningScopeAgg> = view
        .bind_scope_as(sqlx::query_as(&sql), installation_id, session_user_id)
        .fetch_all(pool)
        .await?;

    let mut agg = UpcomingAgg {
        inflows: Decimal::ZERO,
        outflows: Decimal::ZERO,
        recurring_monthly_inflow: Decimal::ZERO,
        recurring_monthly_outflow: Decimal::ZERO,
        recurring_count: 0,
        count: 0,
        last_due_date: None,
    };
    for r in rows {
        // Solo `income` y `expense` suman: una categoría de otro scope no es un Próximo, y su
        // recuento tampoco debe describir cifras en las que no entra.
        match (r.scope.as_str(), r.per_month) {
            ("income", false) => agg.inflows += r.total,
            ("expense", false) => agg.outflows += r.total,
            // #148: un `per_month` son €/MES — jamás dentro de un total en €.
            ("income", true) => agg.recurring_monthly_inflow += r.total,
            ("expense", true) => agg.recurring_monthly_outflow += r.total,
            _ => continue,
        }
        if r.per_month {
            agg.recurring_count += r.flow_count;
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
    /// **El PLAN de jubilación de quien pregunta** (5.0.0, D27): estrategia, disparador, mes
    /// efectivo, ahorro necesario, margen y el rojo de D17. Es lo que alimenta la tarjeta «Tu
    /// plan» del Resumen sin obligar a la SPA a pedir además la serie de proyección entera.
    ///
    /// **Todo `null` con `absent_reason: household_aggregate` en `view=household`**: el hogar es
    /// la suma de N planes independientes (uno por miembro, con su estrategia y su edad) y no
    /// tiene uno propio — «el ahorro necesario del hogar» no es una cifra que exista.
    pub plan: SummaryPlan,
}

/// El plan de jubilación resumido. Sale **del mismo objeto que pinta el chart**: se lee de la
/// entrada de cache de proyección del usuario y, si no hay ninguna, se calcula por el camino
/// cacheado (`projection_series_cached`) — que además la deja caliente, así que el GET de la
/// serie que viene detrás es un HIT. Nunca hay una segunda fórmula: si estas cifras y las de
/// `/v1/projection/series` pudieran divergir, no valdrían para nada.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SummaryPlan {
    /// `asap` | `retire_at_age` | `coast` | `partial` | `pension_bridge`.
    pub strategy: Option<String>,
    /// Qué DISPARA la jubilación: `liquid_crossing` (el capital alcanzó el objetivo) o
    /// `target_age` (la edad manda, llegue o no el capital — D17).
    pub retirement_trigger: Option<String>,
    /// Mes EFECTIVO de jubilación, en la rejilla de `points[].month_index` de
    /// `/v1/projection/series` (0 = hoy). `null` con `absent_reason`, y también —con
    /// `absent_reason` nulo— cuando el plan no se jubila dentro del horizonte: eso es un
    /// resultado, no un hueco.
    pub jubilacion_month_index: Option<u32>,
    /// **Ahorro mensual necesario** para llegar al objetivo en la edad elegida, en euros. Es
    /// exactamente `required_contribution_monthly` de `/v1/projection/series` — el mismo número
    /// del mismo solve, con el nombre que se lee en un Resumen. `null` con las estrategias por
    /// cruce: ahí no hay edad contra la que resolver nada.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub required_savings_monthly: Option<Decimal>,
    /// **Margen mensual disponible** (D16/D31), con la base que corresponde a cada estrategia
    /// —declarada en el campo homónimo de `/v1/projection/series`, que es de donde sale—.
    /// `null` cuando la estrategia no publica margen.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub disposable_monthly: Option<Decimal>,
    /// **El rojo de D17**: `true` ⟺ ni invirtiendo cada euro de sobrante se llega al objetivo en
    /// la edad elegida. `null` = la pregunta no aplica a esta estrategia — nunca `false` para
    /// decir «no aplica».
    pub underfunded: Option<bool>,
    /// Por qué el plan viene vacío: `household_aggregate` (la vista suma N planes y no tiene uno)
    /// | `projection_unavailable` (la simulación no se pudo calcular; el Resumen es una lectura y
    /// no se cae por eso). `null` ⟺ el plan de arriba es el del usuario.
    #[schema(value_type = Option<String>)]
    pub absent_reason: Option<&'static str>,

    // ---- 5.0.0 WP6b — el KPI «Éxito del plan» (D25/D28) --------------------------------------
    /// **Probabilidad de éxito de Monte Carlo**: fracción de caminos en los que la cartera no se
    /// agota nunca dentro del horizonte (D22). `0.87` = 87 de cada 100 escenarios.
    ///
    /// **Es EXACTAMENTE el número que dibuja el fan chart** de `GET /v1/projection/bands`: sale
    /// del mismo cache, con los caminos y la semilla por defecto. Si el Resumen lo recalculara
    /// por su cuenta con otra muestra, el KPI y el gráfico enseñarían dos éxitos distintos del
    /// mismo plan en la misma pantalla.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_probability: Option<Decimal>,
    /// Umbral del perfil en PORCENTAJE (`success_threshold_pct`, default 95). Se ecoa aunque el
    /// sorteo no se haya podido hacer: es configuración del usuario, no una salida del modelo.
    pub success_threshold_pct: Option<u32>,
    /// `green` | `amber` | `red` con el semáforo de D28 (verde en el umbral, ámbar hasta 10
    /// puntos porcentuales por debajo). `null` ⟺ no hay probabilidad que colorear.
    #[schema(value_type = Option<String>)]
    pub success_verdict: Option<&'static str>,
    /// Por qué faltan `success_probability`/`success_verdict` cuando el resto del plan SÍ está:
    /// `bands_unavailable` (el sorteo falló; el Resumen es una lectura y no se cae por eso).
    /// `null` ⟺ la probabilidad viaja, o el plan entero está ausente y lo dice `absent_reason`.
    #[schema(value_type = Option<String>)]
    pub success_absent_reason: Option<&'static str>,
}

impl SummaryPlan {
    /// El plan ausente, con su razón. **Todos** los campos van a `null` a la vez —los seis del
    /// plan y los tres del éxito—: publicar uno suelto sería peor que no publicar ninguno.
    fn absent(reason: &'static str) -> Self {
        SummaryPlan {
            strategy: None,
            retirement_trigger: None,
            jubilacion_month_index: None,
            required_savings_monthly: None,
            disposable_monthly: None,
            underfunded: None,
            absent_reason: Some(reason),
            success_probability: None,
            success_threshold_pct: None,
            success_verdict: None,
            // El hueco ya lo explica `absent_reason`; una segunda razón para lo mismo se leería
            // como si hubieran fallado dos cosas distintas.
            success_absent_reason: None,
        }
    }
}

/// El agregado del hogar no tiene plan: es la suma de N simulaciones independientes.
pub(crate) const PLAN_ABSENT_HOUSEHOLD: &str = "household_aggregate";
/// La proyección no se pudo calcular. El Resumen es una LECTURA y no se cae por ello — pero lo
/// dice, en vez de servir seis `null` indistinguibles de «no tienes plan».
pub(crate) const PLAN_ABSENT_PROJECTION_UNAVAILABLE: &str = "projection_unavailable";
/// El sorteo de Monte Carlo falló pero el plan determinista sí está. Se distingue del anterior a
/// propósito: «no sabemos tu probabilidad de éxito» y «no sabemos tu plan» son dos situaciones
/// muy distintas para quien lee el Resumen.
pub(crate) const PLAN_ABSENT_BANDS_UNAVAILABLE: &str = "bands_unavailable";

/// Lee el plan de jubilación del usuario **del objeto que sirve el chart**.
///
/// Orden deliberado: primero las dos densidades de la cache (estas seis cifras no dependen de la
/// densidad — son escalares del plan, no puntos de la serie), y solo si no hay ninguna se calcula
/// por `projection_series_cached` con `hybrid`, que es la densidad que la SPA pide primero.
///
/// **Coste**: un MISS aquí paga una proyección entera más los solves de la estrategia (§B.7:
/// hasta 26 proyecciones). No es coste nuevo del Resumen, es el MISMO que iba a pagar el GET de
/// la serie un instante después — y como se inserta en la cache, ese GET pasa a ser un HIT. Tras
/// un login o una mutación con warm-up, esto es siempre un HIT.
async fn summary_plan(state: &AppState, iid: Uuid, user_id: Uuid) -> SummaryPlan {
    use crate::state::{Density, ProjectionCacheKey};
    for density in [crate::state::Density::Hybrid, Density::Monthly] {
        let key = ProjectionCacheKey {
            installation_id: iid,
            view: LedgerView::Mine,
            owner_user_id: Some(user_id),
            density,
        };
        if let Some(cached) = state.projection_cache_get(&key).await {
            return plan_from_series(&cached);
        }
    }
    match crate::handlers::projection::projection_series_cached(
        state,
        user_id,
        iid,
        LedgerView::Mine,
        None,
        Density::Hybrid,
    )
    .await
    {
        Ok(series) => plan_from_series(&series),
        Err(e) => {
            tracing::warn!(error = %e, "no se pudo resolver el plan de jubilación para /v1/summary");
            SummaryPlan::absent(PLAN_ABSENT_PROJECTION_UNAVAILABLE)
        }
    }
}

/// **El KPI «Éxito del plan» del Resumen** (D28), leído del MISMO sitio que el fan chart.
///
/// Va por `projection_bands_cached` con los caminos y la semilla por defecto, que es exactamente
/// la petición que hace la sección «Riesgo»: en el caso normal esto es un HIT y no cuesta nada, y
/// en un MISS deja la entrada caliente para el GET que viene detrás. **Nunca hay una segunda
/// muestra**: dos ejecuciones de Monte Carlo con semillas distintas darían dos probabilidades
/// distintas del mismo plan, y el usuario vería el KPI del Resumen discrepar del gráfico de
/// Jubilación sin ninguna explicación posible.
///
/// Un fallo aquí **no tumba el Resumen**: se publican los tres campos a `null` con
/// `success_absent_reason`, y el resto del plan sigue viajando.
async fn attach_success(state: &AppState, iid: Uuid, user_id: Uuid, mut plan: SummaryPlan) -> SummaryPlan {
    use crate::handlers::projection_bands::{projection_bands_cached, DEFAULT_BANDS_PATHS};
    if plan.absent_reason.is_some() {
        return plan;
    }
    match projection_bands_cached(
        state,
        user_id,
        iid,
        LedgerView::Mine,
        DEFAULT_BANDS_PATHS,
        None,
    )
    .await
    {
        Ok(bands) => {
            plan.success_probability = bands.success_probability;
            plan.success_threshold_pct = Some(bands.success_threshold_pct);
            plan.success_verdict = Some(bands.success_verdict);
        }
        Err(e) => {
            tracing::warn!(error = %e, "no se pudieron calcular las bandas para el KPI de éxito");
            plan.success_absent_reason = Some(PLAN_ABSENT_BANDS_UNAVAILABLE);
        }
    }
    plan
}

/// Proyección → plan. Copia de campos, sin una sola cuenta: cualquier aritmética aquí sería la
/// segunda implementación de algo que la proyección ya resolvió.
fn plan_from_series(
    s: &crate::handlers::projection::ProjectionSeriesResponse,
) -> SummaryPlan {
    SummaryPlan {
        strategy: s.strategy.clone(),
        retirement_trigger: s.retirement_trigger.map(str::to_string),
        jubilacion_month_index: s.jubilacion_month_index,
        required_savings_monthly: s.required_contribution_monthly,
        disposable_monthly: s.disposable_monthly,
        underfunded: s.underfunded,
        absent_reason: None,
        // Los rellena `attach_success` con la entrada del cache de bandas: aquí no se calcula
        // nada, igual que el resto de esta función.
        success_probability: None,
        success_threshold_pct: None,
        success_verdict: None,
        success_absent_reason: None,
    }
}

#[utoipa::path(
    get,
    path = "/v1/summary",
    tag = "summary",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
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
    let out = summary_core(&state, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// **SWR mínimo del hogar** (5.0.0, §D): el menor `swr_pct` entre los perfiles de los miembros
/// con fila en `installation_memberships`. `None` = el hogar no tiene miembros con perfil legible
/// (inalcanzable con una instalación sana: el solicitante siempre es uno).
///
/// Se resuelve por el MISMO camino que cualquier otro perfil (`resolve_retirement_profile`), así
/// que los defaults y los clamps son idénticos a los que ve el usuario en su formulario: leer el
/// JSONB crudo aquí abriría una segunda interpretación del mismo dato.
async fn household_min_swr_pct(
    pool: &sqlx::PgPool,
    iid: Uuid,
) -> Result<Option<Decimal>, ApiError> {
    let rows: Vec<Option<sqlx::types::Json<crate::handlers::retirement_profile::RetirementProfile>>> =
        sqlx::query_scalar(
            r#"SELECT u.retirement_profile
               FROM installation_memberships m
               JOIN users u ON u.id = m.user_id
               WHERE m.installation_id = $1"#,
        )
        .bind(iid)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|j| {
            crate::handlers::retirement_profile::resolve_retirement_profile(j.map(|x| x.0)).swr_pct
        })
        .min())
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_summary`.
pub(crate) async fn summary_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<SummaryResponse, ApiError> {
    // Un solo alias para no reescribir las ~20 queries de abajo: lo que cambió en 5.0.0 es que
    // esta core necesita además el ESTADO (la cache de proyección), no solo el pool.
    let pool = &state.pool;
    // Una sola query para los escalares de instalación que necesita este handler: fecha civil,
    // inflación (base del runway) y los fire_settings (fuente del ahorro + SWR/tramos del runway).
    let (today, inflation_pct, fire) = installation_calendar_inflation_fire(pool, iid).await?;
    // El SWR salió de `fire_settings` en 5.0.0 (D13): el umbral «runway indefinido» lo fija el
    // perfil del usuario. En `mine` es el del solicitante y ya está.
    //
    // En `household` (WP5, §D) es el **MÍNIMO** de los perfiles de los miembros, y el mínimo no
    // es una preferencia estética: el runway agregado se sirve sobre las filas de TODO el hogar,
    // y «indefinido» significa «esta cartera aguanta para siempre». Basta con que UN miembro
    // considere insostenible esa tasa de retirada para que el hogar no pueda declararse
    // indefinido — usar el máximo (o el del solicitante) permitiría que el más optimista del
    // hogar firmara por todos, que es exactamente el número plausible y falso que aquí no se
    // publica. Con un solo miembro coincide con el suyo, así que nada se mueve.
    let retirement_profile =
        crate::handlers::retirement_profile::load_retirement_profile(pool, user_id).await?;
    let swr_for_runway = match view {
        LedgerView::Mine => retirement_profile.swr_pct,
        LedgerView::Household => {
            household_min_swr_pct(pool, iid).await?.unwrap_or(retirement_profile.swr_pct)
        }
    };
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
        r#"SELECT current_value, expected_annual_return_percent, is_liquid, purchase_price
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

    let asset_rows: Vec<(Decimal, Option<Decimal>, bool, Option<Decimal>)> = view
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

    let total_assets: Decimal = asset_rows.iter().map(|(v, _, _, _)| *v).sum();
    let total_liabilities: Decimal = liab_raw.iter().map(|(p, _, _, _, _)| *p).sum();
    // #178: la base de coste declarada viaja al bucle finito del runway (None = sin coste ⇒ el
    // escalar; el UMBRAL sigue con el escalar — perpetuidad, ver financial-contracts §2.4).
    let liquid_rows: Vec<(Decimal, Option<Decimal>, Option<Decimal>)> = asset_rows
        .iter()
        .filter(|(_, _, is_liquid, _)| *is_liquid)
        .map(|(v, r, _, pp)| (*v, *r, *pp))
        .collect();
    let liquid_assets: Decimal = liquid_rows.iter().map(|(v, _, _)| *v).sum();
    let asset_return_rows: Vec<(Decimal, Option<Decimal>)> =
        asset_rows.iter().map(|(v, r, _, _)| (*v, *r)).collect();

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
        swr_for_runway,
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
        upcoming_recurring_monthly_inflow: upcoming.recurring_monthly_inflow,
        upcoming_recurring_monthly_outflow: upcoming.recurring_monthly_outflow,
        upcoming_recurring_count: upcoming.recurring_count,
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
        plan: match view {
            LedgerView::Mine => {
                attach_success(state, iid, user_id, summary_plan(state, iid, user_id).await).await
            }
            LedgerView::Household => SummaryPlan::absent(PLAN_ABSENT_HOUSEHOLD),
        },
    })
}

pub fn summary_router() -> Router {
    Router::new().route("/", get(get_summary))
}
