//! Comparativa mensual (§A5): mes real vs presupuesto vs promedio histórico.
//!
//! ## Convención de signos (magnitudes ≥ 0 para comparar con el presupuesto)
//! Las transacciones se guardan firmadas (negativo = cargo). Aquí presentamos **magnitudes**:
//! - Gasto de una categoría = `−Σ(amount)` de sus transacciones `expense` (un reembolso positivo
//!   reduce el neto). El presupuesto es un importe mensual positivo → `delta = actual − budget`.
//! - Ingreso = `+Σ(amount)` (`income`, positivos).
//! - Ahorro/Inversión = `−Σ(amount)` (`savings`; una aportación −200 cuenta como +200 ahorrado);
//!   excluido del consumo → tiene su propio bloque, nunca entra en `expense`.
//!
//! ## Promedio ponderado (`avg`)
//! El tramo del promedio es el rango medio-abierto `[window_start, selected)` de meses civiles,
//! elegido con `avg_window` (`3`|`6`|`12`|`ytd`|`all`; alias legado `avg_months` 1..24). De ese
//! tramo solo promedian los meses REALES (≥1 movimiento con `recurring_rule_id IS NULL`), y lo
//! hacen enteros: un mes no real queda fuera del numerador Y del denominador. El denominador es
//! `avg_months`; `months_with_data` (meses con ≥1 movimiento de cualquier tipo) se sigue
//! publicando aparte porque describe lo que hay, no lo que promedia. Sin meses reales no hay
//! promedio: `avg_months = 0`, todas las medias 0, `avg_basis = null` y `avg_unavailable_reason`
//! dice por qué.
//!
//! El denominador es único para todas las líneas, no por categoría: así `Σ avg de categorías`
//! sigue siendo `totals.expense_avg` y la tasa de ahorro promedio no se infla. La contrapartida
//! aceptada es que un mes real sin movimientos de una categoría concreta SÍ cuenta como cero
//! para ella — es la media del hogar, no «cuánto gasto cuando gasto».
//!
//! ## Cuotas de pasivo: atribuidas a su categoría de gasto (3.4.0)
//! Desde 3.4.0 cada pasivo puede declarar `expense_category_id` (obligatorio al crear) y su
//! equivalente mensual entra en el lado Budget de ESA categoría — emparejado con los recibos
//! reales, que ya viven categorizados → `Hipoteca · Real 512 · Budget 500 · Δ +12`. Historia:
//! la v1.8.0 retiró la antigua `derived_debt_line` porque era una fila sintética SIN pareja
//! (solo lado budget, con la cuota real dentro de otra categoría) y la comparativa la contaba
//! dos veces; el emparejamiento por categoría elimina esa asimetría. Pasivos sin categoría
//! asignada (NULL, anteriores a 3.4.0) no aportan nada — comportamiento previo intacto. Nota
//! documentada: presupuestar a mano la cuota en la misma categoría infla su Budget (visible y
//! autocorregible, no silencioso).

use crate::error::ApiError;
use crate::handlers::installation::{
    installation_naive_today, require_installation_member, AvgWindowMode, AvgWindowSpec,
};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    AvgBasis, BlockActualAvg, CategoryComparisonLine, CategoryMonthPoint,
    CategoryMonthlySeriesEntry, CategoryMonthlySeriesResponse, SummaryTotals,
    TransactionsSummaryResponse,
};
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::{FromRow, PgPool};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

const DEFAULT_AVG_MONTHS: u32 = 6;
const MAX_AVG_MONTHS: u32 = 24;

#[derive(Debug, Deserialize)]
pub struct SummaryQuery {
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub month: Option<u32>,
    /// Ventana del promedio: `3`|`6`|`12`|`ytd`|`all`. Gana sobre `avg_months` si vienen ambos.
    #[serde(default)]
    pub avg_window: Option<String>,
    /// Alias legado (1..24 meses). Sólo se usa si `avg_window` está ausente.
    #[serde(default)]
    pub avg_months: Option<u32>,
}

/// Tramo del promedio, resuelto desde `avg_window`/`avg_months`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AvgWindow {
    Months(u32),
    Ytd,
    All,
}

impl AvgWindow {
    /// Valor efectivo para el response: `"3"`/`"6"`/`"12"`/… | `"ytd"` | `"all"`.
    fn as_str(&self) -> String {
        match self {
            AvgWindow::Months(n) => n.to_string(),
            AvgWindow::Ytd => "ytd".into(),
            AvgWindow::All => "all".into(),
        }
    }
}

/// `(year, month) + delta` meses (delta con signo), normalizado.
fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    let ny = zero.div_euclid(12) as i32;
    let nm = (zero.rem_euclid(12) + 1) as u32;
    (ny, nm)
}

fn ym_string(year: i32, month: u32) -> String {
    format!("{year:04}-{month:02}")
}

fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid first-of-month")
}

/// Nº de meses civiles entre dos primeros-de-mes (`b − a`); 0 si `b <= a`.
fn months_between(a: NaiveDate, b: NaiveDate) -> u32 {
    let za = a.year() * 12 + a.month() as i32 - 1;
    let zb = b.year() * 12 + b.month() as i32 - 1;
    (zb - za).max(0) as u32
}

#[derive(Debug, FromRow)]
struct BucketRow {
    ym: String,
    kind: Option<String>,
    category_id: Option<Uuid>,
    total: Decimal,
    /// Movimientos NO recurrentes del bucket. Σ por mes `> 0` ⟺ el mes es real (ver
    /// `in_avg_window`). Mismo predicado que `MonthAgg::real_txns` de `transactions_avg`.
    real_txns: i32,
}

/// Suma raw firmada de `(ym, kind, category)`.
fn bucket(
    m: &HashMap<(String, String, Option<Uuid>), Decimal>,
    ym: &str,
    kind: &str,
    cat: Option<Uuid>,
) -> Decimal {
    m.get(&(ym.to_string(), kind.to_string(), cat))
        .copied()
        .unwrap_or(Decimal::ZERO)
}

/// Suma raw firmada de todas las categorías de `(ym, kind)`.
fn bucket_all(
    m: &HashMap<(String, String, Option<Uuid>), Decimal>,
    ym: &str,
    kind: &str,
) -> Decimal {
    m.iter()
        .filter(|((y, k, _), _)| y == ym && k == kind)
        .map(|(_, v)| *v)
        .sum()
}

/// Un lado (ingreso o gasto) del promedio real: la magnitud, SU denominador y la procedencia
/// exacta de los meses que la produjeron.
///
/// Media y denominador viajan JUNTOS a propósito. Con una sola ventana daba igual exponer el
/// denominador suelto; con dos ventanas, cruzar «numerador de ingreso ÷ denominador de gasto» pasa
/// a ser un error posible — y silencioso, porque el resultado sigue siendo un número plausible.
#[derive(Debug, Clone)]
pub(crate) struct AvgSide {
    /// Magnitud ≥ 0 (los signos se normalizan como en el summary).
    pub avg: Decimal,
    /// Meses REALES que entraron en la media. `0` ⟺ no hay base (el caller cae al presupuesto).
    pub months_with_data: u32,
    /// Ventana configurada tras el clamp, para poder decir «pediste 12, hay 7».
    pub window: AvgWindowSpec,
    /// Mes más antiguo INCLUIDO (`YYYY-MM`). `None` ⟺ `months_with_data == 0`.
    pub first_month: Option<String>,
    /// Mes más reciente INCLUIDO (`YYYY-MM`).
    pub last_month: Option<String>,
    /// `true` ⟺ los meses incluidos NO son consecutivos. Es lo que impide que la UI etiquete
    /// «media de ene–dic 2025» una ventana de 12 meses dispersos en tres años.
    pub has_gaps: bool,
}

impl AvgSide {
    fn empty(window: AvgWindowSpec) -> Self {
        Self {
            avg: Decimal::ZERO,
            months_with_data: 0,
            window,
            first_month: None,
            last_month: None,
            has_gaps: false,
        }
    }
}

/// Promedio real por lado, cada uno con SU ventana.
pub(crate) struct TransactionsAvg {
    pub income: AvgSide,
    pub expense: AvgSide,
}

/// Agregado mensual crudo que devuelve la query única.
#[derive(Debug, FromRow)]
struct MonthAgg {
    ym: String,
    /// Movimientos NO recurrentes del mes. `> 0` ⟺ el mes es real.
    real_txns: i32,
    income_sum: Decimal,
    expense_sum: Decimal,
}

/// Tope del barrido físico en modo `data`: acota lo que se escanea y mantiene sana la etiqueta de
/// rango. Mismo espíritu que `MAX_RECURRENCE_BACKFILL_YEARS`.
const MAX_AVG_LOOKBACK_MONTHS: i32 = 120;

/// Ordinal de mes de un `YYYY-MM`, para detectar huecos.
fn ym_ordinal(ym: &str) -> i32 {
    let y: i32 = ym[..4].parse().unwrap_or(0);
    let m: i32 = ym[5..7].parse().unwrap_or(1);
    y * 12 + (m - 1)
}

/// Elige los meses de un lado a partir de los agregados (que vienen en orden DESC).
///
/// `filter` y no `take_while`: si alguien cambiara el `ORDER BY` de la query, `take_while`
/// truncaría en silencio y el promedio saldría de menos meses de los debidos.
fn select_side<'a>(rows: &'a [MonthAgg], spec: AvgWindowSpec, today: NaiveDate) -> Vec<&'a MonthAgg> {
    let real = rows.iter().filter(|r| r.real_txns > 0);
    match spec.mode {
        AvgWindowMode::Data => real.take(spec.months as usize).collect(),
        AvgWindowMode::Calendar => {
            let (sy, sm) = shift_month(today.year(), today.month(), -(spec.months as i32));
            let floor = ym_string(sy, sm); // comparación lexicográfica ≡ cronológica en YYYY-MM
            real.filter(|r| r.ym >= floor).collect()
        }
    }
}

/// Pliega los meses elegidos en un `AvgSide`.
///
/// EXACTITUD: suma exacta primero, UNA sola división al final. Promediar cocientes por mes
/// introduciría redondeo y rompería la identidad con el comportamiento anterior.
fn fold_side(
    sel: &[&MonthAgg],
    spec: AvgWindowSpec,
    pick: impl Fn(&MonthAgg) -> Decimal,
    negate: bool,
) -> AvgSide {
    if sel.is_empty() {
        return AvgSide::empty(spec);
    }
    let n = sel.len() as u32;
    let sum: Decimal = sel.iter().map(|r| pick(r)).sum();
    let sum = if negate { -sum } else { sum };
    let mut yms: Vec<&str> = sel.iter().map(|r| r.ym.as_str()).collect();
    yms.sort_unstable();
    let span = ym_ordinal(yms[yms.len() - 1]) - ym_ordinal(yms[0]) + 1;
    AvgSide {
        avg: sum / Decimal::from(n),
        months_with_data: n,
        window: spec,
        first_month: Some(yms[0].to_string()),
        last_month: Some(yms[yms.len() - 1].to_string()),
        has_gaps: span != n as i32,
    }
}

/// Promedio real de ingreso y gasto, cada lado con su propia ventana.
///
/// Ventana física medio-abierta `[floor, first_of_month(today))`: el mes EN CURSO queda siempre
/// fuera (es parcial y hundiría la media). Dentro de ese tramo, cada lado elige sus meses según
/// `AvgWindowSpec`.
///
/// **Mes real** = mes con ≥1 transacción `recurring_rule_id IS NULL`. Desde 3.9.0 los recurrentes
/// solo existen en meses activos, así que en régimen normal «mes real» ≡ «mes con datos»; el
/// predicado se conserva porque sigue siendo correcto en los casos residuales (p. ej. el mes de
/// origen de una regla, exento de la poda). Las patas de transferencia CONCILIADAS quedan fuera
/// del numerador Y del denominador: un mes cuyo único contenido son transferencias internas no es
/// un mes con flujo.
///
/// Signos (magnitudes ≥ 0, como en el summary): `income` positivo → `income_avg = Σ`;
/// `expense` negativo → `expense_avg = −Σ`. `savings` y `kind NULL` NO entran.
pub(crate) async fn transactions_avg(
    pool: &PgPool,
    installation_id: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    income: AvgWindowSpec,
    expense: AvgWindowSpec,
) -> Result<TransactionsAvg, ApiError> {
    // En modo `data` hay que barrer hondo (los meses con datos pueden estar dispersos); en
    // `calendar` basta con la mayor de las dos ventanas.
    let lookback = if income.mode == AvgWindowMode::Data || expense.mode == AvgWindowMode::Data {
        MAX_AVG_LOOKBACK_MONTHS
    } else {
        income.months.max(expense.months) as i32
    };
    let window_end = first_of_month(today.year(), today.month());
    let (fy, fm) = shift_month(today.year(), today.month(), -lookback);
    let window_floor = first_of_month(fy, fm);

    let scope = view.scope_where("t");
    let arg = view.next_arg_index();
    let end = arg + 1;

    // UNA sola query (antes eran dos): el `COUNT(*) FILTER` hace que el bit «este mes es real»
    // viaje con la fila, sustituyendo a la vez al conteo de meses y al CTE `real_months` que
    // restringía el numerador. No hace falta `DISTINCT`: el `GROUP BY ym` ya colapsa por mes.
    let sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym,
                COUNT(*) FILTER (WHERE t.recurring_rule_id IS NULL)::int AS real_txns,
                COALESCE(SUM(t.amount) FILTER (WHERE t.kind = 'income'), 0)::numeric AS income_sum,
                COALESCE(SUM(t.amount) FILTER (WHERE t.kind = 'expense'), 0)::numeric AS expense_sum
         FROM transactions t
         WHERE {scope}
           AND t.op_date >= ${arg} AND t.op_date < ${end}
           AND t.transfer_counterpart_id IS NULL
         GROUP BY 1
         ORDER BY 1 DESC"
    );
    let rows: Vec<MonthAgg> = view
        .bind_scope_as(sqlx::query_as(&sql), installation_id, session_user_id)
        .bind(window_floor)
        .bind(window_end)
        .fetch_all(pool)
        .await?;

    let inc_sel = select_side(&rows, income, today);
    let exp_sel = select_side(&rows, expense, today);
    Ok(TransactionsAvg {
        income: fold_side(&inc_sel, income, |r| r.income_sum, false),
        expense: fold_side(&exp_sel, expense, |r| r.expense_sum, true),
    })
}

#[utoipa::path(
    get,
    path = "/v1/transactions/summary",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` | household."),
        ("year" = Option<i32>, Query, description = "Año del mes seleccionado (default: último mes completo)."),
        ("month" = Option<u32>, Query, description = "Mes 1..12 (default: último mes completo)."),
        ("avg_window" = Option<String>, Query, description = "Ventana del promedio: `3`|`6`|`12`|`ytd`|`all`."),
        ("avg_months" = Option<u32>, Query, description = "Alias legado (1..24 meses); `avg_window` gana."),
    ),
    responses(
        (status = 200, description = "Comparativa mensual", body = TransactionsSummaryResponse),
        (status = 400, description = "Parámetros inválidos"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_transactions_summary(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<TransactionsSummaryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery { view: q.view.clone() }.resolve()?;
    let out = transactions_summary_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        q.year,
        q.month,
        q.avg_window.clone(),
        q.avg_months,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_transactions_summary`.
/// La validación de parámetros vive AQUÍ para que ambos caminos devuelvan los mismos
/// códigos 400 estables.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn transactions_summary_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    q_year: Option<i32>,
    q_month: Option<u32>,
    q_avg_window: Option<String>,
    q_avg_months: Option<u32>,
) -> Result<TransactionsSummaryResponse, ApiError> {
    let today = installation_naive_today(pool, iid).await?;

    // Ventana del promedio: `avg_window` gana; si falta, el alias legado `avg_months` (default 6).
    let window = match &q_avg_window {
        Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "3" => AvgWindow::Months(3),
            "6" => AvgWindow::Months(6),
            "12" => AvgWindow::Months(12),
            "ytd" => AvgWindow::Ytd,
            "all" => AvgWindow::All,
            _ => {
                return Err(ApiError::BadRequest(
                    "avg_window_invalid: avg_window must be one of 3, 6, 12, ytd, all".into(),
                ))
            }
        },
        None => {
            let n = q_avg_months.unwrap_or(DEFAULT_AVG_MONTHS);
            if n == 0 || n > MAX_AVG_MONTHS {
                return Err(ApiError::BadRequest(format!(
                    "avg_months_out_of_range: avg_months must be between 1 and {MAX_AVG_MONTHS}"
                )));
            }
            AvgWindow::Months(n)
        }
    };

    // Mes seleccionado: (year, month) o el último mes COMPLETO (el anterior al actual).
    let (year, month) = match (q_year, q_month) {
        (Some(y), Some(m)) => {
            if !(1900..=3000).contains(&y) {
                return Err(ApiError::BadRequest("year_out_of_range: year must be between 1900 and 3000".into()));
            }
            if !(1..=12).contains(&m) {
                return Err(ApiError::BadRequest("month_out_of_range: month must be between 1 and 12".into()));
            }
            (y, m)
        }
        (None, None) => shift_month(today.year(), today.month(), -1),
        _ => {
            return Err(ApiError::BadRequest(
                "year_month_incomplete: year and month must be provided together".into(),
            ))
        }
    };

    let selected_ym = ym_string(year, month);
    let is_partial = year == today.year() && month == today.month();

    let month_start = first_of_month(year, month);
    let (ny, nm) = shift_month(year, month, 1);
    let month_end = first_of_month(ny, nm);

    // `window_start`: primer día del primer mes del tramo del promedio `[window_start, selected)`.
    let window_start = match window {
        AvgWindow::Months(n) => {
            let (wy, wm) = shift_month(year, month, -(n as i32));
            first_of_month(wy, wm)
        }
        // Enero del año del mes seleccionado; con month == 1 coincide con el mes seleccionado → tramo vacío.
        AvgWindow::Ytd => first_of_month(year, 1),
        // Primer día del mes de MIN(op_date) del scope; si NULL o ≥ mes seleccionado → tramo vacío.
        // Las conciliadas no anclan la ventana: «Todo» no debe arrancar en un mes sin datos visibles.
        AvgWindow::All => {
            let scope = view.scope_where("t");
            let sql = format!(
                "SELECT MIN(t.op_date) FROM transactions t WHERE {scope} \
                 AND t.transfer_counterpart_id IS NULL"
            );
            let min_op: Option<NaiveDate> = view
                .bind_scope_scalar(
                    sqlx::query_scalar::<_, Option<NaiveDate>>(&sql),
                    iid,
                    user_id,
                )
                .fetch_one(pool)
                .await?;
            match min_op {
                Some(d) => {
                    let candidate = first_of_month(d.year(), d.month());
                    if candidate < month_start {
                        candidate
                    } else {
                        month_start
                    }
                }
                None => month_start,
            }
        }
    };
    let window_start_ym = ym_string(window_start.year(), window_start.month());
    let window_months = months_between(window_start, month_start);

    // `ym` pertenece al tramo del promedio (medio-abierto): `window_start_ym <= ym < selected_ym`.
    // Comparación lexicográfica de "YYYY-MM" = cronológica (padding `{:04}` de `ym_string`).
    let in_window = |ym: &str| ym >= window_start_ym.as_str() && ym < selected_ym.as_str();

    // ---- Actuals + ventana: transacciones agregadas por (ym, kind, category) ----------------
    // Las transferencias conciliadas siguen VISIBLES en el listado de Movimientos, pero no son
    // gasto ni ingreso → fuera de todos los buckets (actuals, promedio y months_with_data).
    let scope = view.scope_where("t");
    let arg = view.next_arg_index();
    let sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym, t.kind AS kind,
                t.category_id AS category_id, SUM(t.amount) AS total,
                COUNT(*) FILTER (WHERE t.recurring_rule_id IS NULL)::int AS real_txns
         FROM transactions t
         WHERE {scope} AND t.op_date >= ${arg} AND t.op_date < ${end}
           AND t.transfer_counterpart_id IS NULL
         GROUP BY ym, t.kind, t.category_id",
        end = arg + 1
    );
    let raw: Vec<BucketRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .bind(window_start)
        .bind(month_end)
        .fetch_all(pool)
        .await?;
    let mut buckets: HashMap<(String, String, Option<Uuid>), Decimal> = HashMap::new();
    // Movimientos NO recurrentes por mes, para decidir qué meses son reales.
    let mut real_txns_by_ym: HashMap<String, i32> = HashMap::new();
    for r in raw {
        let kind = r.kind.unwrap_or_default();
        *real_txns_by_ym.entry(r.ym.clone()).or_insert(0) += r.real_txns;
        *buckets.entry((r.ym, kind, r.category_id)).or_insert(Decimal::ZERO) += r.total;
    }

    // `months_with_data` = meses distintos del tramo con ≥1 transacción de cualquier kind/categoría,
    // recurrentes incluidos. Es lo que HAY, y por eso sigue siendo la cifra de la pestaña Movimientos.
    let months_with_data = {
        let mut set: HashSet<&String> = HashSet::new();
        for (ym, _kind, _cat) in buckets.keys() {
            if in_window(ym.as_str()) {
                set.insert(ym);
            }
        }
        set.len() as u32
    };

    // `real_months` = meses del tramo con ≥1 movimiento REAL (`recurring_rule_id IS NULL`). Mismo
    // predicado que `transactions_avg`: hasta 3.9.0 esta comparativa dividía entre `months_with_data`
    // y un mes cuyo único contenido eran instancias recurrentes hundía la media de TODAS las
    // categorías (una categoría ausente ese mes contaba como cero). Alinear ambas definiciones era
    // una decisión de producto pendiente; se tomó. Siguen sin ser idénticas: `transactions_avg` tiene
    // además ventanas por lado configurables, mientras que aquí la ventana es siempre de calendario.
    let real_months: HashSet<&str> = real_txns_by_ym
        .iter()
        .filter(|(ym, n)| **n > 0 && in_window(ym.as_str()))
        .map(|(ym, _)| ym.as_str())
        .collect();

    // El tramo que realmente promedia: calendario ∩ meses reales. Se aplica al NUMERADOR y al
    // DENOMINADOR a la vez — excluir un mes solo del denominador dejaría su gasto en el numerador y
    // dispararía las categorías presentes en él (un alquiler de 700 €/mes saldría a 1.720 €).
    let in_avg_window = |ym: &str| in_window(ym) && real_months.contains(ym);

    let avg_months = real_months.len() as u32;
    // Base del promedio: qué meses lo produjeron. `has_gaps` impide una etiqueta mentirosa —
    // con ventana de calendario los meses reales pueden no ser consecutivos (abr y jun reales,
    // may solo-recurrente) y «abr–jun» fingiría un rango contiguo. Misma forma que `AvgSide`.
    let avg_basis = if avg_months == 0 {
        None
    } else {
        let mut yms: Vec<&str> = real_months.iter().copied().collect();
        yms.sort_unstable();
        let span = ym_ordinal(yms[yms.len() - 1]) - ym_ordinal(yms[0]) + 1;
        Some(AvgBasis {
            months: avg_months,
            first_month: yms[0].to_string(),
            last_month: yms[yms.len() - 1].to_string(),
            has_gaps: span != avg_months as i32,
        })
    };
    // Por qué no hay promedio, cuando no lo hay: distingue «no has importado nada» de «solo tienes
    // recurrentes», que piden acciones distintas (importar vs bajar la ventana).
    let avg_unavailable_reason: Option<String> = if avg_months > 0 {
        None
    } else if months_with_data == 0 {
        Some("empty_window".into())
    } else {
        Some("only_recurring_months".into())
    };

    // ---- Presupuesto por categoría (scope income/expense) ------------------------------------
    let bscope = view.scope_where("b");
    let bsql = format!(
        "SELECT b.category_id AS category_id, c.scope AS scope, SUM(b.amount) AS total
         FROM budget_entries b
         JOIN categories c ON c.id = b.category_id
         WHERE {bscope}
         GROUP BY b.category_id, c.scope"
    );
    let brows: Vec<(Uuid, String, Decimal)> = view
        .bind_scope_as(sqlx::query_as(&bsql), iid, user_id)
        .fetch_all(pool)
        .await?;
    let mut expense_budget: HashMap<Uuid, Decimal> = HashMap::new();
    let mut income_budget: HashMap<Uuid, Decimal> = HashMap::new();
    for (cat, scope_s, total) in brows {
        match scope_s.as_str() {
            "expense" => *expense_budget.entry(cat).or_insert(Decimal::ZERO) += total,
            "income" => *income_budget.entry(cat).or_insert(Decimal::ZERO) += total,
            _ => {}
        }
    }

    // ---- Cuotas de pasivo atribuidas a su categoría de gasto (3.4.0) -------------------------
    // El pasivo «publica» su cuota en el presupuesto: el equivalente mensual del plan entra en
    // el lado Budget de SU categoría de gasto, donde se empareja con los recibos reales (que ya
    // viven categorizados) → la fila y el total se igualan. Solo pasivos con categoría asignada
    // (los NULL previos a 3.4.0 se comportan como siempre) y ACTIVOS en el mes seleccionado
    // (fin de plan NULL = indefinido; el mes en que termina aún cuenta). El universo de
    // `build_lines` une las claves del budget_map, así que una categoría solo-cuota materializa
    // su fila aunque no tenga movimientos ni partidas.
    let lscope = view.scope_where("");
    let l_month_ph = view.next_arg_index();
    let lsql = format!(
        "SELECT expense_category_id, payment_amount, payment_frequency
         FROM liabilities
         WHERE {lscope}
           AND expense_category_id IS NOT NULL
           AND payment_amount IS NOT NULL
           AND payment_frequency IS NOT NULL
           AND (payment_end_date IS NULL OR payment_end_date >= ${l_month_ph})"
    );
    let lrows: Vec<(Option<Uuid>, Decimal, String)> = view
        .bind_scope_as(sqlx::query_as(&lsql), iid, user_id)
        .bind(month_start)
        .fetch_all(pool)
        .await?;
    for (cat, amount, freq) in lrows {
        let Some(cat) = cat else { continue };
        *expense_budget.entry(cat).or_insert(Decimal::ZERO) +=
            crate::handlers::budget::monthly_equivalent(amount, &freq);
    }

    // ---- Nombres de categoría ----------------------------------------------------------------
    let cat_names: HashMap<Uuid, String> = {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as(r#"SELECT id, name FROM categories WHERE installation_id = $1"#)
                .bind(iid)
                .fetch_all(pool)
                .await?;
        rows.into_iter().collect()
    };

    // ---- Construcción de las líneas por categoría --------------------------------------------
    // `.max(1)` solo evita la división por cero: con `avg_months == 0` ningún mes pasa
    // `in_avg_window`, así que todos los numeradores son 0 y las medias salen 0 igualmente.
    let avg_denom = Decimal::from(avg_months.max(1));

    let build_lines = |scope_kind: &str,
                       budget_map: &HashMap<Uuid, Decimal>,
                       income_sign: bool|
     -> Vec<CategoryComparisonLine> {
        // Universo de categorías: presentes en actuals/ventana (buckets del kind) ∪ presupuesto.
        // La ventana es la del promedio: una categoría que solo aparece en meses NO reales no
        // materializa una fila fantasma con actual 0, budget 0 y avg 0.
        let mut cats: HashSet<Option<Uuid>> = HashSet::new();
        for (y, k, cat) in buckets.keys() {
            if k == scope_kind && (y == &selected_ym || in_avg_window(y.as_str())) {
                cats.insert(*cat);
            }
        }
        for cat in budget_map.keys() {
            cats.insert(Some(*cat));
        }

        let mut lines: Vec<CategoryComparisonLine> = cats
            .into_iter()
            .map(|cat| {
                let raw_sel = bucket(&buckets, &selected_ym, scope_kind, cat);
                let raw_win: Decimal = buckets
                    .iter()
                    .filter(|((y, k, c), _)| {
                        k == scope_kind && *c == cat && in_avg_window(y.as_str())
                    })
                    .map(|(_, v)| *v)
                    .sum();
                // income → magnitud = +suma; expense → magnitud = −suma.
                let (actual, avg_raw) = if income_sign {
                    (raw_sel, raw_win / avg_denom)
                } else {
                    (-raw_sel, -raw_win / avg_denom)
                };
                let avg = avg_raw.round_dp(4);
                let budget = cat
                    .and_then(|c| budget_map.get(&c).copied())
                    .unwrap_or(Decimal::ZERO);
                let name = cat
                    .and_then(|c| cat_names.get(&c).cloned())
                    .unwrap_or_else(|| "Sin categoría".to_string());
                CategoryComparisonLine {
                    category_id: cat,
                    category_name: name,
                    actual,
                    budget,
                    avg,
                    delta_vs_budget: actual - budget,
                    delta_vs_avg: actual - avg,
                }
            })
            .collect();
        // Orden: categorías con nombre por nombre ASC; "Sin categoría" (null) al final.
        lines.sort_by(|a, b| {
            a.category_id
                .is_none()
                .cmp(&b.category_id.is_none())
                .then_with(|| a.category_name.cmp(&b.category_name))
        });
        lines
    };

    let expense_categories = build_lines("expense", &expense_budget, false);
    let income_categories = build_lines("income", &income_budget, true);

    // ---- Bloques savings / income (agregados) ------------------------------------------------
    let savings_actual = -bucket_all(&buckets, &selected_ym, "savings");
    let savings_win: Decimal = buckets
        .iter()
        .filter(|((y, k, _), _)| k == "savings" && in_avg_window(y.as_str()))
        .map(|(_, v)| *v)
        .sum();
    let savings_avg = (-savings_win / avg_denom).round_dp(4);

    let income_actual: Decimal = income_categories.iter().map(|l| l.actual).sum();
    let income_avg: Decimal = income_categories.iter().map(|l| l.avg).sum();

    // ---- Totales -----------------------------------------------------------------------------
    let expense_actual: Decimal = expense_categories.iter().map(|l| l.actual).sum();
    let expense_avg: Decimal = expense_categories.iter().map(|l| l.avg).sum();
    // Σ presupuesto de categorías de gasto — sin línea derivada de cuotas (sin doble conteo).
    let expense_budget_total: Decimal = expense_categories.iter().map(|l| l.budget).sum();
    let income_budget_total: Decimal = income_categories.iter().map(|l| l.budget).sum();
    let net_actual = income_actual - expense_actual;

    Ok(TransactionsSummaryResponse {
        year,
        month,
        is_partial,
        avg_window: window.as_str(),
        window_months,
        months_with_data,
        avg_months,
        avg_basis,
        avg_unavailable_reason,
        view: if view == LedgerView::Mine { "mine".into() } else { "household".into() },
        expense_categories,
        income_categories,
        savings: BlockActualAvg {
            actual: savings_actual,
            avg: savings_avg,
        },
        income: BlockActualAvg {
            actual: income_actual,
            avg: income_avg,
        },
        totals: SummaryTotals {
            expense_actual,
            expense_budget: expense_budget_total,
            expense_avg,
            income_actual,
            income_budget: income_budget_total,
            income_avg,
            savings_actual,
            savings_avg,
            net_actual,
        },
    })
}

// ---------------------------------------------------------------------------
// Serie mensual por categoría (`GET /v1/transactions/category-series`)
// ---------------------------------------------------------------------------

const DEFAULT_CATEGORY_SERIES_WINDOW: u32 = 12;
const MAX_CATEGORY_SERIES_WINDOW: u32 = 60;

#[derive(Debug, Deserialize)]
pub struct CategorySeriesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// `expense` | `income` (obligatorio).
    pub kind: String,
    /// Limita la respuesta a una categoría (omitido = todas las del `kind` con datos).
    #[serde(default)]
    pub category_id: Option<Uuid>,
    /// Amplitud de la ventana en meses (default 12, clamp 1..=60). El último mes es el actual.
    #[serde(default)]
    pub window_months: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/v1/transactions/category-series",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` | household."),
        ("kind" = String, Query, description = "`expense` | `income`."),
        ("category_id" = Option<Uuid>, Query, description = "Limita a una categoría."),
        ("window_months" = Option<u32>, Query, description = "Ventana en meses (default 12, clamp 1..=60)."),
    ),
    responses(
        (status = 200, description = "Serie mensual por categoría (cero-rellena, magnitudes ≥ 0)", body = CategoryMonthlySeriesResponse),
        (status = 400, description = "kind inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_category_series(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<CategorySeriesQuery>,
) -> Result<Json<CategoryMonthlySeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery { view: q.view.clone() }.resolve()?;
    let out = category_monthly_series_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        &q.kind,
        q.category_id,
        q.window_months,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_category_monthly_series`.
///
/// Emite, por categoría del `kind` con ≥1 movimiento en la ventana, un punto por CADA mes
/// (cero-relleno) con la magnitud ≥ 0 de la convención de la comparativa (gasto = `−Σ`,
/// ingreso = `+Σ`). El dato ya se materializaba en memoria para la comparativa; ningún endpoint
/// emitía la serie mes a mes.
pub(crate) async fn category_monthly_series_core(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    kind: &str,
    category_id: Option<Uuid>,
    window_months: Option<u32>,
) -> Result<CategoryMonthlySeriesResponse, ApiError> {
    let kind = match kind.trim() {
        "expense" => "expense",
        "income" => "income",
        _ => {
            return Err(ApiError::BadRequest(
                "category_series_kind_invalid: kind must be `expense` or `income`".into(),
            ))
        }
    };
    let window = window_months
        .unwrap_or(DEFAULT_CATEGORY_SERIES_WINDOW)
        .clamp(1, MAX_CATEGORY_SERIES_WINDOW);

    let today = installation_naive_today(pool, iid).await?;
    // Ventana: `window` meses civiles ascendentes que TERMINAN en el mes en curso (parcial).
    let (sy, sm) = shift_month(today.year(), today.month(), -(window as i32 - 1));
    let window_start = first_of_month(sy, sm);
    let (ny, nm) = shift_month(today.year(), today.month(), 1);
    let month_end = first_of_month(ny, nm);
    let month_labels: Vec<String> = (0..window)
        .map(|i| {
            let (y, m) = shift_month(sy, sm, i as i32);
            ym_string(y, m)
        })
        .collect();

    let scope = view.scope_where("t");
    let mut arg = view.next_arg_index();
    // Conciliadas fuera: la serie por categoría es un agregado de flujo (misma regla que summary).
    let mut sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym, t.category_id AS category_id,
                SUM(t.amount) AS total
         FROM transactions t
         WHERE {scope} AND t.kind = ${arg} AND t.op_date >= ${d1} AND t.op_date < ${d2}
           AND t.transfer_counterpart_id IS NULL",
        d1 = arg + 1,
        d2 = arg + 2
    );
    arg += 3;
    if category_id.is_some() {
        sql.push_str(&format!(" AND t.category_id = ${arg}"));
    }
    sql.push_str(" GROUP BY ym, t.category_id");

    let mut query = view
        .bind_scope_as(
            sqlx::query_as::<_, (String, Option<Uuid>, Decimal)>(&sql),
            iid,
            user_id,
        )
        .bind(kind)
        .bind(window_start)
        .bind(month_end);
    if let Some(cid) = category_id {
        query = query.bind(cid);
    }
    let rows: Vec<(String, Option<Uuid>, Decimal)> = query.fetch_all(pool).await?;

    // Magnitud ≥ 0: gasto guarda cargos negativos → se niega; ingreso queda tal cual.
    let sign = if kind == "expense" {
        -Decimal::ONE
    } else {
        Decimal::ONE
    };
    let mut by_cat: HashMap<Option<Uuid>, HashMap<String, Decimal>> = HashMap::new();
    for (ym, cat, total) in rows {
        *by_cat
            .entry(cat)
            .or_default()
            .entry(ym)
            .or_insert(Decimal::ZERO) += sign * total;
    }
    // Escala fija de 2 decimales (mismo criterio que los KPIs del cashflow): "25.00", no "25.0000".
    let money = |d: Decimal| -> Decimal {
        let mut v = d.round_dp(2);
        v.rescale(2);
        v
    };

    let cat_names: HashMap<Uuid, String> = {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as(r#"SELECT id, name FROM categories WHERE installation_id = $1"#)
                .bind(iid)
                .fetch_all(pool)
                .await?;
        rows.into_iter().collect()
    };

    let mut series: Vec<CategoryMonthlySeriesEntry> = by_cat
        .into_iter()
        .map(|(cat, by_month)| {
            let months = month_labels
                .iter()
                .map(|ym| CategoryMonthPoint {
                    month: ym.clone(),
                    total: money(by_month.get(ym).copied().unwrap_or(Decimal::ZERO)),
                })
                .collect();
            CategoryMonthlySeriesEntry {
                category_id: cat,
                category_name: cat.and_then(|c| cat_names.get(&c).cloned()),
                months,
            }
        })
        .collect();
    // Nombre ASC; la pseudo-categoría sin nombre (movimientos sin categorizar) al final.
    series.sort_by(|a, b| match (&a.category_name, &b.category_name) {
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.category_id.cmp(&b.category_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.category_id.cmp(&b.category_id),
    });

    Ok(CategoryMonthlySeriesResponse {
        view: if view == LedgerView::Mine {
            "mine".into()
        } else {
            "household".into()
        },
        kind: kind.into(),
        window_months: window,
        series,
    })
}
