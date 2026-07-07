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
//! ## Promedio (`avg`)
//! Media sobre los `avg_months` meses civiles COMPLETOS anteriores al seleccionado; denominador
//! = `avg_months` (incluye meses a cero).
//!
//! ## Sin doble conteo de cuotas de pasivo
//! `derived_debt_line` es SOLO el lado presupuesto (Σ `monthly_equivalent` de pasivos activos,
//! reutilizando la lógica de `budget.rs`). Los actuals de las cuotas viven en su categoría de
//! gasto ordinaria (decisión 8) → no se suman dos veces.

use crate::error::ApiError;
use crate::handlers::budget::ledger_budget_totals_for_summary;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    BlockActualAvg, CategoryComparisonLine, DerivedDebtLine, SummaryTotals,
    TransactionsSummaryResponse,
};
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::FromRow;
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
    #[serde(default)]
    pub avg_months: Option<u32>,
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

#[utoipa::path(
    get,
    path = "/v1/transactions/summary",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` | household."),
        ("year" = Option<i32>, Query, description = "Año del mes seleccionado (default: último mes completo)."),
        ("month" = Option<u32>, Query, description = "Mes 1..12 (default: último mes completo)."),
        ("avg_months" = Option<u32>, Query, description = "Ventana del promedio, 1..24 (default 6)."),
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

    let avg_months = q.avg_months.unwrap_or(DEFAULT_AVG_MONTHS);
    if avg_months == 0 || avg_months > MAX_AVG_MONTHS {
        return Err(ApiError::BadRequest(format!(
            "avg_months must be between 1 and {MAX_AVG_MONTHS}"
        )));
    }

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
    let (wy, wm) = shift_month(year, month, -(avg_months as i32));
    let window_start = first_of_month(wy, wm);
    let window_yms: Vec<String> = (1..=avg_months)
        .map(|k| {
            let (y, m) = shift_month(year, month, -(k as i32));
            ym_string(y, m)
        })
        .collect();
    let _ = month_start; // documenta el inicio del mes seleccionado; el rango de query usa window_start.

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

    // ---- Línea derivada de cuotas (solo lado budget), reutilizando budget.rs -----------------
    let budget_totals =
        ledger_budget_totals_for_summary(&state.pool, iid, user.id.0, view, today).await?;
    let derived_budget = budget_totals.expense_derived_monthly_equivalent;

    // ---- Construcción de las líneas por categoría --------------------------------------------
    let avg_denom = Decimal::from(avg_months);

    let build_lines = |scope_kind: &str,
                       budget_map: &HashMap<Uuid, Decimal>,
                       income_sign: bool|
     -> Vec<CategoryComparisonLine> {
        // Universo de categorías: presentes en actuals/ventana (buckets del kind) ∪ presupuesto.
        let mut cats: HashSet<Option<Uuid>> = HashSet::new();
        for (y, k, cat) in buckets.keys() {
            if k == scope_kind && (y == &selected_ym || window_yms.contains(y)) {
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
                let raw_win: Decimal = window_yms
                    .iter()
                    .map(|ym| bucket(&buckets, ym, scope_kind, cat))
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
    let savings_win: Decimal = window_yms
        .iter()
        .map(|ym| bucket_all(&buckets, ym, "savings"))
        .sum();
    let savings_avg = (-savings_win / avg_denom).round_dp(4);

    let income_actual: Decimal = income_categories.iter().map(|l| l.actual).sum();
    let income_avg: Decimal = income_categories.iter().map(|l| l.avg).sum();

    // ---- Totales -----------------------------------------------------------------------------
    let expense_actual: Decimal = expense_categories.iter().map(|l| l.actual).sum();
    let expense_avg: Decimal = expense_categories.iter().map(|l| l.avg).sum();
    let expense_cat_budget: Decimal = expense_categories.iter().map(|l| l.budget).sum();
    let expense_budget_total = expense_cat_budget + derived_budget;
    let income_budget_total: Decimal = income_categories.iter().map(|l| l.budget).sum();
    let net_actual = income_actual - expense_actual;

    Ok(Json(TransactionsSummaryResponse {
        year,
        month,
        is_partial,
        avg_months,
        view: if view == LedgerView::Mine { "mine".into() } else { "household".into() },
        expense_categories,
        income_categories,
        derived_debt_line: DerivedDebtLine {
            label: "Cuotas de pasivos".into(),
            budget: derived_budget,
        },
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
