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
//! elegido con `avg_window` (`3`|`6`|`12`|`ytd`|`all`; alias legado `avg_months` 1..24). El
//! denominador NO es el número de meses del tramo sino `months_with_data` = nº de meses del tramo
//! con ≥1 transacción (promedio ponderado: los meses vacíos no diluyen la media). Si no hay
//! ninguno el denominador es 1 (numerador 0 → avg 0).
//!
//! ## Sin línea derivada de cuotas de pasivo
//! A diferencia de `budget.rs`, la comparativa NO añade una línea derivada de las cuotas de
//! pasivos: `totals.expense_budget` es Σ del presupuesto de las categorías de gasto. Las cuotas
//! reales viven ya en su categoría de gasto ordinaria (los movimientos importados/manuales) → así
//! no se cuentan dos veces.

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    BlockActualAvg, CategoryComparisonLine, SummaryTotals, TransactionsSummaryResponse,
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

/// Promedio ponderado mensual de las transacciones de los 12 meses civiles COMPLETOS anteriores a
/// hoy (ventana medio-abierta `[first_of_month(today) − 12m, first_of_month(today))`). A diferencia
/// del summary de Movimientos, esta ventana **incluye** el último mes completo y excluye solo el mes
/// en curso (parcial). Lo consume la proyección modo B (`savings_source = transactions_avg`).
///
/// Signos (magnitudes ≥ 0, como en el summary): `income` guardado positivo → `income_avg`;
/// `expense` guardado negativo → `expense_avg = −Σ`. `savings` y `kind NULL` NO cuentan para
/// income/expense (pero un mes con solo transacciones de esos tipos SÍ suma a `months_with_data`,
/// mismo criterio que el summary). Denominador = `months_with_data` (meses del tramo con ≥1
/// transacción de cualquier kind); `0` real → todo a cero, el llamante decide el fallback.
pub(crate) struct TransactionsAvg {
    pub income_avg: Decimal,
    pub expense_avg: Decimal,
    /// Promedio mensual (magnitud) de las transacciones `expense` con `linked_liability_id`, por
    /// liability. Mismo denominador `months_with_data`.
    pub per_liability_linked_avg: HashMap<Uuid, Decimal>,
    pub months_with_data: u32,
}

pub(crate) async fn transactions_12m_avg(
    pool: &PgPool,
    installation_id: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<TransactionsAvg, ApiError> {
    let window_end = first_of_month(today.year(), today.month());
    let (sy, sm) = shift_month(today.year(), today.month(), -12);
    let window_start = first_of_month(sy, sm);

    let scope = view.scope_where("t");
    let arg = view.next_arg_index();

    // `months_with_data`: meses distintos del tramo con ≥1 transacción de cualquier kind (incluye
    // `savings` y `kind NULL`), mismo criterio que el summary.
    let months_sql = format!(
        "SELECT COUNT(DISTINCT to_char(t.op_date, 'YYYY-MM'))::int
         FROM transactions t
         WHERE {scope} AND t.op_date >= ${arg} AND t.op_date < ${end}",
        end = arg + 1
    );
    let months_with_data: i32 = view
        .bind_scope_scalar(sqlx::query_scalar(&months_sql), installation_id, session_user_id)
        .bind(window_start)
        .bind(window_end)
        .fetch_one(pool)
        .await?;
    let months_with_data = months_with_data.max(0) as u32;

    if months_with_data == 0 {
        return Ok(TransactionsAvg {
            income_avg: Decimal::ZERO,
            expense_avg: Decimal::ZERO,
            per_liability_linked_avg: HashMap::new(),
            months_with_data: 0,
        });
    }
    let denom = Decimal::from(months_with_data);

    // Suma firmada por kind (solo income/expense).
    let kind_sql = format!(
        "SELECT t.kind AS kind, SUM(t.amount) AS total
         FROM transactions t
         WHERE {scope} AND t.op_date >= ${arg} AND t.op_date < ${end}
           AND t.kind IN ('income', 'expense')
         GROUP BY t.kind",
        end = arg + 1
    );
    let kind_rows: Vec<(Option<String>, Decimal)> = view
        .bind_scope_as(sqlx::query_as(&kind_sql), installation_id, session_user_id)
        .bind(window_start)
        .bind(window_end)
        .fetch_all(pool)
        .await?;
    let mut income_sum = Decimal::ZERO;
    let mut expense_sum = Decimal::ZERO;
    for (kind, total) in kind_rows {
        match kind.as_deref() {
            Some("income") => income_sum += total,
            Some("expense") => expense_sum += total,
            _ => {}
        }
    }

    // Cuotas vinculadas: Σ de expense con `linked_liability_id`, por liability.
    let liab_sql = format!(
        "SELECT t.linked_liability_id AS liability_id, SUM(t.amount) AS total
         FROM transactions t
         WHERE {scope} AND t.op_date >= ${arg} AND t.op_date < ${end}
           AND t.kind = 'expense' AND t.linked_liability_id IS NOT NULL
         GROUP BY t.linked_liability_id",
        end = arg + 1
    );
    let liab_rows: Vec<(Uuid, Decimal)> = view
        .bind_scope_as(sqlx::query_as(&liab_sql), installation_id, session_user_id)
        .bind(window_start)
        .bind(window_end)
        .fetch_all(pool)
        .await?;
    let per_liability_linked_avg: HashMap<Uuid, Decimal> = liab_rows
        .into_iter()
        .map(|(id, total)| (id, (-total) / denom))
        .collect();

    Ok(TransactionsAvg {
        income_avg: income_sum / denom,
        expense_avg: (-expense_sum) / denom,
        per_liability_linked_avg,
        months_with_data,
    })
}

/// Resta híbrida de cuotas de pasivo sobre el promedio real (modo B). Por cada liability **activa**
/// (el llamante ya filtró por `payment_end_date`) se resta de `expense_avg`: su promedio real
/// vinculado si existe, si no su cuota nominal mensual. Devuelve `(income_eff, expense_eff)` con
/// `expense_eff = max(0, expense_avg − Σ resta)`.
///
/// Único punto de verdad de esta fórmula: lo consumen `projection.rs` (input del engine) y
/// `summary.rs` (KPIs de Resumen) para que ambos handlers no diverjan. `active_liabilities` es
/// `(liability_id, cuota_nominal_mensual)`.
pub(crate) fn effective_avg_income_expense(
    avg: &TransactionsAvg,
    active_liabilities: &[(Uuid, Decimal)],
) -> (Decimal, Decimal) {
    let mut liab_payments = Decimal::ZERO;
    for (id, nominal) in active_liabilities {
        liab_payments += avg
            .per_liability_linked_avg
            .get(id)
            .copied()
            .unwrap_or(*nominal);
    }
    let expense_eff = (avg.expense_avg - liab_payments).max(Decimal::ZERO);
    (avg.income_avg, expense_eff)
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
    let view = LedgerViewQuery { view: q.view.clone() }.resolve();
    let today = installation_naive_today(&state.pool, iid).await?;

    // Ventana del promedio: `avg_window` gana; si falta, el alias legado `avg_months` (default 6).
    let window = match &q.avg_window {
        Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "3" => AvgWindow::Months(3),
            "6" => AvgWindow::Months(6),
            "12" => AvgWindow::Months(12),
            "ytd" => AvgWindow::Ytd,
            "all" => AvgWindow::All,
            _ => {
                return Err(ApiError::BadRequest(
                    "avg_window must be one of 3, 6, 12, ytd, all".into(),
                ))
            }
        },
        None => {
            let n = q.avg_months.unwrap_or(DEFAULT_AVG_MONTHS);
            if n == 0 || n > MAX_AVG_MONTHS {
                return Err(ApiError::BadRequest(format!(
                    "avg_months must be between 1 and {MAX_AVG_MONTHS}"
                )));
            }
            AvgWindow::Months(n)
        }
    };

    // Mes seleccionado: (year, month) o el último mes COMPLETO (el anterior al actual).
    let (year, month) = match (q.year, q.month) {
        (Some(y), Some(m)) => {
            if !(1900..=3000).contains(&y) {
                return Err(ApiError::BadRequest("year must be between 1900 and 3000".into()));
            }
            if !(1..=12).contains(&m) {
                return Err(ApiError::BadRequest("month must be between 1 and 12".into()));
            }
            (y, m)
        }
        (None, None) => shift_month(today.year(), today.month(), -1),
        _ => {
            return Err(ApiError::BadRequest(
                "year and month must be provided together".into(),
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
        AvgWindow::All => {
            let scope = view.scope_where("t");
            let sql = format!("SELECT MIN(t.op_date) FROM transactions t WHERE {scope}");
            let min_op: Option<NaiveDate> = view
                .bind_scope_scalar(
                    sqlx::query_scalar::<_, Option<NaiveDate>>(&sql),
                    iid,
                    user.id.0,
                )
                .fetch_one(&state.pool)
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
    let scope = view.scope_where("t");
    let arg = view.next_arg_index();
    let sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym, t.kind AS kind,
                t.category_id AS category_id, SUM(t.amount) AS total
         FROM transactions t
         WHERE {scope} AND t.op_date >= ${arg} AND t.op_date < ${end}
         GROUP BY ym, t.kind, t.category_id",
        end = arg + 1
    );
    let raw: Vec<BucketRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
        .bind(window_start)
        .bind(month_end)
        .fetch_all(&state.pool)
        .await?;
    let mut buckets: HashMap<(String, String, Option<Uuid>), Decimal> = HashMap::new();
    for r in raw {
        let kind = r.kind.unwrap_or_default();
        *buckets.entry((r.ym, kind, r.category_id)).or_insert(Decimal::ZERO) += r.total;
    }

    // `months_with_data` = meses distintos del tramo con ≥1 transacción de cualquier kind/categoría.
    let months_with_data = {
        let mut set: HashSet<&String> = HashSet::new();
        for (ym, _kind, _cat) in buckets.keys() {
            if in_window(ym.as_str()) {
                set.insert(ym);
            }
        }
        set.len() as u32
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
        .bind_scope_as(sqlx::query_as(&bsql), iid, user.id.0)
        .fetch_all(&state.pool)
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

    // ---- Nombres de categoría ----------------------------------------------------------------
    let cat_names: HashMap<Uuid, String> = {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as(r#"SELECT id, name FROM categories WHERE installation_id = $1"#)
                .bind(iid)
                .fetch_all(&state.pool)
                .await?;
        rows.into_iter().collect()
    };

    // ---- Construcción de las líneas por categoría --------------------------------------------
    let avg_denom = Decimal::from(months_with_data.max(1));

    let build_lines = |scope_kind: &str,
                       budget_map: &HashMap<Uuid, Decimal>,
                       income_sign: bool|
     -> Vec<CategoryComparisonLine> {
        // Universo de categorías: presentes en actuals/ventana (buckets del kind) ∪ presupuesto.
        let mut cats: HashSet<Option<Uuid>> = HashSet::new();
        for (y, k, cat) in buckets.keys() {
            if k == scope_kind && (y == &selected_ym || in_window(y.as_str())) {
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
                    .filter(|((y, k, c), _)| k == scope_kind && *c == cat && in_window(y.as_str()))
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
        .filter(|((y, k, _), _)| k == "savings" && in_window(y.as_str()))
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

    Ok(Json(TransactionsSummaryResponse {
        year,
        month,
        is_partial,
        avg_window: window.as_str(),
        window_months,
        months_with_data,
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
    }))
}
