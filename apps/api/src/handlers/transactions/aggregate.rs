//! Agregación de movimientos (`GET /v1/transactions/aggregate`): los MISMOS filtros del listado,
//! otro `SELECT`.
//!
//! ## Por qué existe
//! «¿Cuánto llevo gastado en Mercadona este año?» es la pregunta más frecuente de las finanzas
//! personales, y hasta 4.4.0 la única forma de responderla por API era bajarse hasta 500 filas y
//! sumarlas fuera. Eso falla de dos maneras, y la segunda es la grave:
//!
//! 1. **Coste**: quinientas filas de ledger para producir un número.
//! 2. **Corrección**: quien suma fuera no aplica `transfer_counterpart_id IS NULL`, así que cuenta
//!    las **transferencias conciliadas** — las dos patas de un traspaso entre cuentas propias, que
//!    ni son gasto ni son ingreso. El resultado es un número plausible y falso, que es el modo de
//!    fallo característico de este repositorio.
//!
//! Por eso el predicado de conciliadas vive DENTRO de la core, no en el llamante, y la respuesta
//! publica `reconciled_excluded_count`: no basta con excluirlas bien, hay que poder demostrar
//! cuántas se excluyeron.
//!
//! ## Paridad con `get_transactions_summary`
//! Los predicados son EXACTAMENTE los de `summary.rs`, que es el otro agregado de flujo del
//! módulo:
//!   * `transfer_counterpart_id IS NULL` (las conciliadas fuera de todo agregado de flujo);
//!   * las **instancias recurrentes SÍ cuentan** (`recurring_rule_id` solo decide qué meses son
//!     «reales» para el promedio, nunca si un movimiento suma);
//!   * signos: magnitud ≥ 0 con `income → +Σ`, `expense`/`savings → −Σ`;
//!   * escala de salida `money_out` (4 decimales, sin cero negativo).
//! Un mismo mes y una misma categoría tienen que dar el mismo número por los dos caminos, y hay un
//! test que lo comprueba contra la respuesta real del summary
//! (`transactions_aggregate.rs::aggregate_matches_get_transactions_summary_month_by_month`). Si
//! alguna vez divergen, uno de los dos miente.
//!
//! Cache: **NONE**. Es lectura; no toca la cache de proyección ni ninguna otra (D5: los GET no
//! mutan).

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::crud::{PreparedFilters, TxnFilters};
use crate::handlers::transactions::{row_to_response, TxnRow, TXN_SELECT};
use crate::money::money_out;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::BTreeSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Movimientos individuales devueltos por defecto en `top`.
const DEFAULT_TOP: i64 = 5;
/// Tope de `top`. Fuera de rango se **rechaza**, no se clampa (criterio del módulo).
const MAX_TOP: i64 = 50;

/// Magnitud ≥ 0 desde la suma firmada, con la convención de `summary.rs`: `income` es positivo tal
/// cual; `expense` y `savings` se guardan negativos y se publican como magnitud.
fn magnitude(kind: &str, signed: Decimal) -> Decimal {
    if kind == "income" {
        signed
    } else {
        -signed
    }
}

// ---------------------------------------------------------------------------
// Respuesta
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct AggregateKindEntry {
    /// `expense` | `income` | `savings`, o `null` para los movimientos sin `kind` en BD.
    pub kind: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_signed: Decimal,
    /// Magnitud ≥ 0. `null` **solo** si `kind` es `null` (sin `kind` no hay convención de signo).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    pub transaction_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AggregateMonthEntry {
    /// `YYYY-MM` de `op_date` (la fecha que manda en todos los cortes del módulo).
    pub month: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_signed: Decimal,
    /// Magnitud ≥ 0. `null` cuando el conjunto entero mezcla `kind` — ver `total_absent_reason`
    /// en la raíz: un mes con gasto e ingreso a la vez no tiene una magnitud única.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    pub transaction_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AggregateCategoryEntry {
    /// `null` = **sin categoría** (no es una categoría llamada «Sin categoría»: es la ausencia).
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    /// `null` cuando `category_id` es `null`.
    pub category_name: Option<String>,
    /// El `kind` de esta fila. Va en la fila y no solo en la raíz para que su `total` tenga
    /// convención de signo propia aunque el conjunto entero mezcle kinds.
    pub kind: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_signed: Decimal,
    /// Magnitud ≥ 0 con la convención del `kind` de ESTA fila. `null` solo si `kind` es `null`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    pub transaction_count: i64,
    /// Porcentaje (0–100, un decimal) que esta fila representa dentro de su propio `kind`. `null`
    /// si el total del kind no es estrictamente positivo (sin base, un porcentaje no significa
    /// nada) o si el `kind` es `null`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub share_pct: Option<Decimal>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AggregateTopEntry {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `YYYY-MM-DD`.
    pub op_date: String,
    pub concept: String,
    /// Importe **con signo**, tal cual está guardado.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub kind: Option<String>,
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AggregateResponse {
    /// Vista aplicada: `household` | `mine`.
    pub view: String,
    /// Movimientos agregados (conciliados ya excluidos).
    pub transaction_count: i64,
    /// Movimientos que cumplían los filtros y **se han excluido por estar conciliados** (patas de
    /// una transferencia interna). Es la cifra que hace auditable la exclusión: sumar el listado a
    /// mano da un número distinto exactamente en esta cantidad de filas.
    pub reconciled_excluded_count: i64,
    /// Σ `amount` **con signo**, tal cual está en BD (los gastos son negativos). Siempre presente.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_signed: Decimal,
    /// Σ como **magnitud ≥ 0**, con la convención de `GET /v1/transactions/summary`
    /// (`income → +Σ`; `expense`/`savings → −Σ`). `null` cuando el conjunto no tiene una
    /// convención única — ver `total_absent_reason`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub total: Option<Decimal>,
    /// Por qué `total` es `null`: `no_transactions` (conjunto vacío), `mixed_kinds` (hay más de un
    /// `kind`: sumar gasto e ingreso como magnitudes no significa nada) o `kind_unset_rows` (hay
    /// movimientos sin `kind` en BD, para los que no existe convención de signo). Filtra por
    /// `kind` para obtener una magnitud.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_absent_reason: Option<String>,
    /// El `kind` único del conjunto, cuando lo hay. Es la base de `total` y de `by_month[].total`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_basis: Option<String>,
    /// `YYYY-MM-DD` del movimiento más antiguo agregado; `null` con el conjunto vacío.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_op_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_op_date: Option<String>,
    /// Desglose por `kind`. Orden fijo: expense, income, savings, sin kind.
    pub by_kind: Vec<AggregateKindEntry>,
    /// Desglose por mes, ascendente. Solo los meses con movimientos.
    pub by_month: Vec<AggregateMonthEntry>,
    /// Desglose por categoría, de mayor a menor magnitud dentro de cada `kind`.
    pub by_category: Vec<AggregateCategoryEntry>,
    /// Los movimientos individuales de mayor importe absoluto. Es el «¿y esto de dónde sale?» sin
    /// tener que paginar el ledger.
    pub top: Vec<AggregateTopEntry>,
    /// Tamaño pedido para `top` (0 lo desactiva).
    pub top_limit: i64,
    /// `true` ⇒ hay más movimientos de los que `top` enseña.
    pub top_truncated: bool,
}

// ---------------------------------------------------------------------------
// Filas crudas
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct AggRow {
    ym: String,
    kind: Option<String>,
    category_id: Option<Uuid>,
    category_name: Option<String>,
    total: Decimal,
    txns: i64,
    first_op_date: NaiveDate,
    last_op_date: NaiveDate,
}

// ---------------------------------------------------------------------------
// Handler HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AggregateQuery {
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub uncategorized: Option<bool>,
    #[serde(default)]
    pub import_id: Option<Uuid>,
    #[serde(default)]
    pub concept_contains: Option<String>,
    #[serde(default)]
    pub min_amount: Option<Decimal>,
    #[serde(default)]
    pub max_amount: Option<Decimal>,
    #[serde(default)]
    pub date_from: Option<NaiveDate>,
    #[serde(default)]
    pub date_to: Option<NaiveDate>,
    /// Cuántos movimientos individuales devolver en `top` (0..50, default 5).
    #[serde(default)]
    pub top: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/v1/transactions/aggregate",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` = solo míos; omitido → household."),
        ("month" = Option<String>, Query, description = "`YYYY-MM`. Excluyente con `date_from`/`date_to`."),
        ("kind" = Option<String>, Query, description = "`expense` | `income` | `savings`. Filtrarlo es lo que hace que `total` (magnitud) exista."),
        ("category_id" = Option<Uuid>, Query, description = "Filtra por categoría."),
        ("uncategorized" = Option<bool>, Query, description = "`true` → solo movimientos SIN categoría. Excluyente con `category_id`."),
        ("import_id" = Option<Uuid>, Query, description = "Filtra por lote de import."),
        ("concept_contains" = Option<String>, Query, description = "Subcadena del concepto (1–200), insensible a mayúsculas y a tildes."),
        ("min_amount" = Option<String>, Query, description = "Cota inferior del importe CON SIGNO."),
        ("max_amount" = Option<String>, Query, description = "Cota superior del importe CON SIGNO."),
        ("date_from" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
        ("date_to" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
        ("top" = Option<i64>, Query, description = "Movimientos individuales de mayor importe absoluto a devolver (0..50, default 5)."),
    ),
    responses(
        (status = 200, description = "Agregado con los mismos filtros del listado. Las transferencias CONCILIADAS quedan excluidas (`reconciled_excluded_count` dice cuántas)", body = AggregateResponse),
        (status = 400, description = "Filtro inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn aggregate_transactions(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<AggregateQuery>,
) -> Result<Json<AggregateResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let out = aggregate_transactions_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        TxnFilters {
            month: q.month.as_deref(),
            kind: q.kind.as_deref(),
            category_id: q.category_id,
            import_id: q.import_id,
            concept_contains: q.concept_contains.as_deref(),
            min_amount: q.min_amount,
            max_amount: q.max_amount,
            date_from: q.date_from,
            date_to: q.date_to,
        },
        q.uncategorized.unwrap_or(false),
        q.top,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `aggregate_transactions`.
///
/// La validación de filtros la hace [`PreparedFilters::prepare`], la MISMA que usa el listado, así
/// que ambos caminos devuelven los mismos 400 y **no puede haber deriva de filtros entre listar y
/// agregar**. La exclusión de las transferencias conciliadas es de esta core, no del llamante.
///
/// Cache: NONE.
pub(crate) async fn aggregate_transactions_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    f: TxnFilters<'_>,
    uncategorized: bool,
    top: Option<i64>,
) -> Result<AggregateResponse, ApiError> {
    let top_limit = top.unwrap_or(DEFAULT_TOP);
    if !(0..=MAX_TOP).contains(&top_limit) {
        return Err(ApiError::BadRequest(format!(
            "limit_out_of_range: top must be between 0 and {MAX_TOP}"
        )));
    }
    let p = PreparedFilters::prepare(view, f, uncategorized)?;
    let scope = view.scope_where("t");
    let filters = p.sql();

    // ---- Agregado por (mes, kind, categoría) ------------------------------------------------
    // Mismo `GROUP BY` que `transactions_summary_core`, con el nombre de la categoría resuelto en
    // el propio JOIN (una query menos que el summary, mismo resultado).
    let sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym,
                t.kind AS kind,
                t.category_id AS category_id,
                c.name AS category_name,
                SUM(t.amount) AS total,
                COUNT(*)::bigint AS txns,
                MIN(t.op_date) AS first_op_date,
                MAX(t.op_date) AS last_op_date
         FROM transactions t
         LEFT JOIN categories c ON c.id = t.category_id
         WHERE {scope}{filters}
           AND t.transfer_counterpart_id IS NULL
         GROUP BY 1, 2, 3, 4"
    );
    let rows: Vec<AggRow> = p
        .bind_as(view.bind_scope_as(sqlx::query_as(&sql), iid, user_id))
        .fetch_all(pool)
        .await?;

    // ---- Conciliadas descartadas -------------------------------------------------------------
    // El complemento exacto del predicado de arriba con los MISMOS filtros: si esta cifra no es 0,
    // sumar el listado a mano da otro número, y esto dice de cuántas filas viene la diferencia.
    let excl_sql = format!(
        "SELECT COUNT(*)::bigint FROM transactions t
         WHERE {scope}{filters} AND t.transfer_counterpart_id IS NOT NULL"
    );
    let reconciled_excluded_count: i64 = p
        .bind_scalar(view.bind_scope_scalar(sqlx::query_scalar(&excl_sql), iid, user_id))
        .fetch_one(pool)
        .await?;

    // ---- Plegado -----------------------------------------------------------------------------
    let mut total_signed = Decimal::ZERO;
    let mut transaction_count: i64 = 0;
    let mut first_op: Option<NaiveDate> = None;
    let mut last_op: Option<NaiveDate> = None;
    let mut kinds_present: BTreeSet<Option<String>> = BTreeSet::new();

    // (kind) → (signed, count)
    let mut by_kind_acc: std::collections::HashMap<Option<String>, (Decimal, i64)> =
        std::collections::HashMap::new();
    // (ym) → (signed, count)
    let mut by_month_acc: std::collections::BTreeMap<String, (Decimal, i64)> =
        std::collections::BTreeMap::new();
    // (kind, category_id) → (name, signed, count)
    let mut by_cat_acc: std::collections::HashMap<
        (Option<String>, Option<Uuid>),
        (Option<String>, Decimal, i64),
    > = std::collections::HashMap::new();

    for r in rows {
        total_signed += r.total;
        transaction_count += r.txns;
        first_op = Some(match first_op {
            Some(d) if d <= r.first_op_date => d,
            _ => r.first_op_date,
        });
        last_op = Some(match last_op {
            Some(d) if d >= r.last_op_date => d,
            _ => r.last_op_date,
        });
        kinds_present.insert(r.kind.clone());

        let k = by_kind_acc
            .entry(r.kind.clone())
            .or_insert((Decimal::ZERO, 0));
        k.0 += r.total;
        k.1 += r.txns;

        let m = by_month_acc.entry(r.ym).or_insert((Decimal::ZERO, 0));
        m.0 += r.total;
        m.1 += r.txns;

        let c = by_cat_acc
            .entry((r.kind, r.category_id))
            .or_insert((r.category_name.clone(), Decimal::ZERO, 0));
        c.1 += r.total;
        c.2 += r.txns;
    }

    // Base de signo del conjunto ENTERO: existe ⟺ hay exactamente un `kind` y no es `null`.
    // Mezclar gasto e ingreso como magnitudes daría un número que no mide nada; devolverlo sería
    // el fallo silencioso de siempre, así que se devuelve `null` con el motivo.
    let (kind_basis, total_absent_reason): (Option<String>, Option<String>) =
        if transaction_count == 0 {
            (None, Some("no_transactions".into()))
        } else if kinds_present.iter().any(|k| k.is_none()) {
            (None, Some("kind_unset_rows".into()))
        } else if kinds_present.len() > 1 {
            (None, Some("mixed_kinds".into()))
        } else {
            let k = kinds_present
                .iter()
                .next()
                .and_then(|k| k.clone())
                .expect("exactamente un kind no nulo");
            (Some(k), None)
        };
    let total = kind_basis
        .as_ref()
        .map(|k| money_out(magnitude(k, total_signed)));

    // ---- by_kind (orden fijo, no por magnitud: es una taxonomía, no un ranking) ---------------
    let kind_order = |k: &Option<String>| match k.as_deref() {
        Some("expense") => 0,
        Some("income") => 1,
        Some("savings") => 2,
        _ => 3,
    };
    let mut by_kind: Vec<AggregateKindEntry> = by_kind_acc
        .iter()
        .map(|(kind, (signed, n))| AggregateKindEntry {
            kind: kind.clone(),
            total_signed: money_out(*signed),
            total: kind.as_deref().map(|k| money_out(magnitude(k, *signed))),
            transaction_count: *n,
        })
        .collect();
    by_kind.sort_by_key(|e| kind_order(&e.kind));

    // ---- by_month ----------------------------------------------------------------------------
    let by_month: Vec<AggregateMonthEntry> = by_month_acc
        .into_iter()
        .map(|(month, (signed, n))| AggregateMonthEntry {
            month,
            total_signed: money_out(signed),
            total: kind_basis.as_ref().map(|k| money_out(magnitude(k, signed))),
            transaction_count: n,
        })
        .collect();

    // ---- by_category (magnitud propia por fila; ranking dentro de su kind) --------------------
    let hundred = Decimal::from(100);
    let mut by_category: Vec<AggregateCategoryEntry> = by_cat_acc
        .into_iter()
        .map(|((kind, category_id), (category_name, signed, n))| {
            let mag = kind.as_deref().map(|k| magnitude(k, signed));
            let share_pct = match (&kind, mag) {
                (Some(k), Some(m)) => {
                    let denom = by_kind_acc
                        .get(&Some(k.clone()))
                        .map(|(s, _)| magnitude(k, *s))
                        .unwrap_or(Decimal::ZERO);
                    if denom > Decimal::ZERO {
                        Some((m * hundred / denom).round_dp(1))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            AggregateCategoryEntry {
                category_id,
                category_name,
                kind,
                total_signed: money_out(signed),
                total: mag.map(money_out),
                transaction_count: n,
                share_pct,
            }
        })
        .collect();
    by_category.sort_by(|a, b| {
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| {
                b.total
                    .unwrap_or(b.total_signed.abs())
                    .cmp(&a.total.unwrap_or(a.total_signed.abs()))
            })
            // Desempate TOTAL y estable: sin él, dos categorías con el mismo gasto salían en el
            // orden que quisiera el `HashMap`, distinto en cada petición.
            .then_with(|| a.category_name.cmp(&b.category_name))
            .then_with(|| a.category_id.cmp(&b.category_id))
    });

    // ---- top-N (movimientos individuales) ----------------------------------------------------
    let mut top_rows: Vec<AggregateTopEntry> = Vec::new();
    if top_limit > 0 {
        let top_sql = format!(
            "{TXN_SELECT} WHERE {scope}{filters}
               AND t.transfer_counterpart_id IS NULL
             ORDER BY abs(t.amount) DESC, t.op_date DESC, t.id DESC
             LIMIT ${}",
            p.next_arg()
        );
        let rows: Vec<TxnRow> = p
            .bind_as(view.bind_scope_as(sqlx::query_as(&top_sql), iid, user_id))
            .bind(top_limit)
            .fetch_all(pool)
            .await?;
        top_rows = rows
            .into_iter()
            .map(row_to_response)
            .map(|t| AggregateTopEntry {
                id: t.id,
                op_date: t.op_date,
                concept: t.concept,
                amount: t.amount,
                kind: t.kind,
                category_id: t.category_id,
                category_name: t.category_name,
            })
            .collect();
    }
    let top_truncated = transaction_count > top_rows.len() as i64;

    Ok(AggregateResponse {
        view: view.as_str().to_string(),
        transaction_count,
        reconciled_excluded_count,
        total_signed: money_out(total_signed),
        total,
        total_absent_reason,
        kind_basis,
        first_op_date: first_op.map(|d| d.format("%Y-%m-%d").to_string()),
        last_op_date: last_op.map(|d| d.format("%Y-%m-%d").to_string()),
        by_kind,
        by_month,
        by_category,
        top: top_rows,
        top_limit,
        top_truncated,
    })
}
