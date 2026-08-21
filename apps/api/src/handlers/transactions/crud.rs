//! Altas manuales, listado mensual, meses con datos, PATCH/DELETE de transacciones y gestión de
//! lotes de import (`/v1/transactions`, `/months`, `/imports`, `/imports/{id}`).
//!
//! Cada mutación invalida la cache de proyección solo en los modos que usan transacciones
//! (`transactions_avg` y `budget_income_real_expense`, vía
//! `invalidate_projection_if_savings_uses_transactions`); en modo `budget` no hace nada. Ver el
//! contrato en `transactions/mod.rs`.

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::reconcile::{auto_reconcile_after_mutation, unlink_pair_no_rejection};
use crate::handlers::transactions::recurring;
use crate::handlers::transactions::schema::{
    compute_fingerprint, like_needle, normalize_concept_field, normalize_kind, normalize_notes,
    sql_fold_concept_expr, BatchCreateBody, BatchPatchBody, CreateTransactionBody,
    ImportBatchResponse, MonthEntry, PatchTransactionBody, TransactionResponse, SOURCE_MANUAL,
};
use crate::handlers::transactions::{
    assert_asset_in_installation, assert_liability_in_installation, assert_transaction_category,
    invalidate_projection_if_savings_uses_transactions, next_fingerprint_ordinal, row_to_response,
    TxnRow, TXN_SELECT,
};
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::{FromRow, PgConnection};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

const MAX_BATCH: usize = 1000;

/// Tope del PATCH en lote. Mucho más bajo que `MAX_BATCH` porque aquí el llamante enumera los ids
/// uno a uno (los acaba de listar), y 200 ya cubre de sobra el caso «desglosar una categoría cajón»
/// sin convertir un error de cliente en una reescritura masiva.
const MAX_PATCH_BATCH: usize = 200;

// ---------------------------------------------------------------------------
// Prepared (validated) transaction ready to insert
// ---------------------------------------------------------------------------

pub(super) struct PreparedTxn {
    pub(super) op_date: NaiveDate,
    pub(super) value_date: Option<NaiveDate>,
    pub(super) concept: String,
    pub(super) amount: Decimal,
    pub(super) kind: String,
    pub(super) category_id: Option<Uuid>,
    pub(super) linked_asset_id: Option<Uuid>,
    pub(super) linked_liability_id: Option<Uuid>,
    pub(super) notes: Option<String>,
    pub(super) fingerprint: String,
}

async fn validate_manual(
    pool: &sqlx::PgPool,
    iid: Uuid,
    body: &CreateTransactionBody,
) -> Result<PreparedTxn, ApiError> {
    let kind = normalize_kind(&body.kind)?;
    let concept = normalize_concept_field(&body.concept)?;
    let amount = body.amount.round_dp(4);
    if amount.is_zero() {
        return Err(ApiError::BadRequest(
            "amount_zero: amount must not be zero".into(),
        ));
    }
    assert_transaction_category(pool, iid, &kind, body.category_id).await?;
    assert_asset_in_installation(pool, iid, body.linked_asset_id).await?;
    assert_liability_in_installation(pool, iid, body.linked_liability_id).await?;
    let notes = normalize_notes(&body.notes)?;
    let fingerprint = compute_fingerprint(SOURCE_MANUAL, body.op_date, amount, &concept);
    Ok(PreparedTxn {
        op_date: body.op_date,
        value_date: body.value_date,
        concept,
        amount,
        kind,
        category_id: body.category_id,
        linked_asset_id: body.linked_asset_id,
        linked_liability_id: body.linked_liability_id,
        notes,
        fingerprint,
    })
}

/// Inserta una transacción manual con `import_id NULL` en `ordinal` y devuelve su id.
/// `recurring_rule_id` enlaza la instancia a su regla recurrente (o `None` para movimientos sueltos).
pub(super) async fn insert_manual(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    p: &PreparedTxn,
    ordinal: i32,
    recurring_rule_id: Option<Uuid>,
) -> Result<Uuid, ApiError> {
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO transactions
               (installation_id, owner_user_id, import_id, source, op_date, value_date,
                concept, amount, currency, kind, category_id, fingerprint, fingerprint_ordinal,
                linked_asset_id, linked_liability_id, notes, recurring_rule_id)
           VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, 'EUR', $8, $9, $10, $11, $12, $13, $14, $15)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(SOURCE_MANUAL)
    .bind(p.op_date)
    .bind(p.value_date)
    .bind(&p.concept)
    .bind(p.amount)
    .bind(&p.kind)
    .bind(p.category_id)
    .bind(&p.fingerprint)
    .bind(ordinal)
    .bind(p.linked_asset_id)
    .bind(p.linked_liability_id)
    .bind(p.notes.as_deref())
    .bind(recurring_rule_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

/// Inserta un movimiento manual y, si `is_recurring`, crea su regla recurrente (resolución
/// mensual) y enlaza el movimiento a ella, en el MISMO commit `tx`. Devuelve el id del movimiento
/// de origen. Secuencia compartida por `create_transaction` y el bucle de `create_batch` (el
/// manejo del `ordinal` queda fuera: se pasa ya resuelto).
///
/// **Ya no backfillea aquí** (3.9.0): las instancias de los meses intermedios las crea la
/// convergencia post-commit, y solo en los meses **activos**. Un alta con fecha pasada en meses
/// sin movimientos reales ya no genera relleno sintético — que es justo el objetivo del cambio.
async fn insert_manual_with_recurrence(
    tx: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    p: &PreparedTxn,
    ordinal: i32,
    is_recurring: bool,
) -> Result<Uuid, ApiError> {
    let rule_id = if is_recurring {
        let origin_month = recurring::month_start_of(p.op_date);
        Some(recurring::insert_rule(&mut *tx, iid, owner, p, origin_month).await?)
    } else {
        None
    };
    insert_manual(&mut *tx, iid, owner, p, ordinal, rule_id).await
}

pub(super) async fn load_txn(pool: &sqlx::PgPool, id: Uuid) -> Result<TransactionResponse, ApiError> {
    let sql = format!("{TXN_SELECT} WHERE t.id = $1");
    let row: TxnRow = sqlx::query_as(&sql).bind(id).fetch_one(pool).await?;
    Ok(row_to_response(row))
}

// ---------------------------------------------------------------------------
// POST /v1/transactions  (alta manual individual)
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions",
    tag = "transactions",
    request_body = CreateTransactionBody,
    responses(
        (status = 201, description = "Transacción manual creada", body = TransactionResponse),
        (status = 400, description = "Validación"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 409, description = "Huella duplicada (misma fecha/importe/concepto/ordinal)"),
    )
)]
pub async fn create_transaction(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateTransactionBody>,
) -> Result<(axum::http::StatusCode, Json<TransactionResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_transaction_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_transaction`. La
/// invalidación condicionada de la cache (COND: solo modos B/C) vive DENTRO, post-commit —
/// así el contrato es idéntico por ambos caminos. El caller ya validó sesión + rol.
pub(crate) async fn create_transaction_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateTransactionBody,
) -> Result<TransactionResponse, ApiError> {
    let prepared = validate_manual(&state.pool, iid, &body).await?;
    // Recurrencia (opcional): marcador sin campos — las reglas tienen resolución mensual.
    let is_recurring = body.recurrence.is_some();
    // "Hoy" de la instalación para el backfill de meses intermedios (solo si hay recurrencia).
    let today = if is_recurring {
        Some(installation_naive_today(&state.pool, iid).await?)
    } else {
        None
    };
    // Cota al backfill: una recurrencia con fecha demasiado antigua generaría cientos de instancias.
    if let Some(today) = today {
        recurring::assert_recurrence_not_too_old(body.op_date, today)?;
    }

    let mut tx = state.pool.begin().await?;
    let ordinal = next_fingerprint_ordinal(&mut tx, iid, user_id, &prepared.fingerprint).await?;
    let id =
        insert_manual_with_recurrence(&mut tx, iid, user_id, &prepared, ordinal, is_recurring)
            .await?;
    tx.commit().await?;

    // Orden contractual: conciliar (cambia QUÉ cuenta como real) → converger (cambia el CONJUNTO)
    // → invalidar. Una sola invalidación cubre las tres.
    auto_reconcile_after_mutation(state, iid, user_id).await;
    recurring::converge_recurring_after_mutation(state, iid).await;
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    load_txn(&state.pool, id).await
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/batch  (alta manual multilínea)
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions/batch",
    tag = "transactions",
    request_body = BatchCreateBody,
    responses(
        (status = 201, description = "Transacciones manuales creadas", body = [TransactionResponse]),
        (status = 400, description = "Validación"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 409, description = "Huella duplicada"),
    )
)]
pub async fn create_batch(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<BatchCreateBody>,
) -> Result<(axum::http::StatusCode, Json<Vec<TransactionResponse>>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    if body.transactions.is_empty() {
        return Err(ApiError::BadRequest(
            "batch must contain at least one transaction".into(),
        ));
    }
    if body.transactions.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!(
            "batch must contain at most {MAX_BATCH} transactions"
        )));
    }

    let mut prepared = Vec::with_capacity(body.transactions.len());
    for b in &body.transactions {
        prepared.push(validate_manual(&state.pool, iid, b).await?);
    }
    // "Hoy" de la instalación para el backfill (solo si algún ítem trae recurrencia).
    let today = if body.transactions.iter().any(|b| b.recurrence.is_some()) {
        Some(installation_naive_today(&state.pool, iid).await?)
    } else {
        None
    };
    // Cota al backfill por ítem recurrente (pre-tx, reutilizando el `today` ya calculado).
    if let Some(today) = today {
        for b in &body.transactions {
            if b.recurrence.is_some() {
                recurring::assert_recurrence_not_too_old(b.op_date, today)?;
            }
        }
    }

    let mut tx = state.pool.begin().await?;
    // Contador de ordinal por huella dentro del batch (arranca en el MAX+1 de la BD).
    let mut next_ord: HashMap<String, i32> = HashMap::new();
    let mut ids = Vec::with_capacity(prepared.len());
    // La recurrencia se acepta por ítem del batch (el modal de efectivo del frontend usa /batch).
    for (b, p) in body.transactions.iter().zip(prepared.iter()) {
        let ord = match next_ord.get(&p.fingerprint) {
            Some(&o) => o,
            None => next_fingerprint_ordinal(&mut tx, iid, user.id.0, &p.fingerprint).await?,
        };
        let id =
            insert_manual_with_recurrence(&mut tx, iid, user.id.0, p, ord, b.recurrence.is_some())
                .await?;
        next_ord.insert(p.fingerprint.clone(), ord + 1);
        ids.push(id);
    }
    tx.commit().await?;

    auto_reconcile_after_mutation(&state, iid, user.id.0).await;
        recurring::converge_recurring_after_mutation(&state, iid).await;
    invalidate_projection_if_savings_uses_transactions(&state, iid, user.id.0).await;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(load_txn(&state.pool, id).await?);
    }
    Ok((axum::http::StatusCode::CREATED, Json(out)))
}

// ---------------------------------------------------------------------------
// GET /v1/transactions  (listado mensual con filtros)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListTxnQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// `YYYY-MM`. Excluyente con `date_from`/`date_to`.
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub import_id: Option<Uuid>,
    /// Subcadena del concepto, insensible a mayúsculas y a tildes.
    #[serde(default)]
    pub concept_contains: Option<String>,
    /// Cota inferior del importe **con signo** (los gastos son negativos).
    #[serde(default)]
    pub min_amount: Option<Decimal>,
    /// Cota superior del importe **con signo**.
    #[serde(default)]
    pub max_amount: Option<Decimal>,
    /// `YYYY-MM-DD` inclusivo. Excluyente con `month`.
    #[serde(default)]
    pub date_from: Option<NaiveDate>,
    /// `YYYY-MM-DD` inclusivo. Excluyente con `month`.
    #[serde(default)]
    pub date_to: Option<NaiveDate>,
}

/// Filtros de `list_transactions_core`, agrupados a propósito: el core ya tomaba diez parámetros
/// posicionales y los ejes de búsqueda lo habrían llevado a quince — precisamente el terreno donde
/// un argumento cruzado no lo detecta el compilador.
#[derive(Debug, Default, Clone)]
pub(crate) struct TxnFilters<'a> {
    pub month: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub category_id: Option<Uuid>,
    pub import_id: Option<Uuid>,
    pub concept_contains: Option<&'a str>,
    pub min_amount: Option<Decimal>,
    pub max_amount: Option<Decimal>,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
}

/// Parsea `YYYY-MM` → (primer día del mes, primer día del mes siguiente).
/// Primer día del mes `YYYY-MM`. Lo comparte el backfill de reglas (`from_month`).
pub(crate) fn parse_month_start(raw: &str) -> Result<NaiveDate, ApiError> {
    parse_month(raw).map(|(start, _)| start)
}

fn parse_month(raw: &str) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let mut parts = raw.trim().splitn(2, '-');
    let year: i32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::BadRequest("month must be YYYY-MM".into()))?;
    let month: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::BadRequest("month must be YYYY-MM".into()))?;
    let start = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| ApiError::BadRequest("month must be a valid YYYY-MM".into()))?;
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = NaiveDate::from_ymd_opt(ny, nm, 1).expect("valid next month");
    Ok((start, end))
}

#[utoipa::path(
    get,
    path = "/v1/transactions",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` = solo mías; omitido → household."),
        ("month" = Option<String>, Query, description = "`YYYY-MM`; filtra por `op_date` en ese mes."),
        ("kind" = Option<String>, Query, description = "`expense` | `income` | `savings`."),
        ("category_id" = Option<Uuid>, Query, description = "Filtra por categoría."),
        ("import_id" = Option<Uuid>, Query, description = "Filtra por lote de import."),
        ("concept_contains" = Option<String>, Query, description = "Subcadena del concepto (1–200), insensible a mayúsculas y a tildes: `cafe` encuentra `CAFÉ`. Los comodines `%` y `_` se tratan como texto literal."),
        ("min_amount" = Option<String>, Query, description = "Cota inferior del importe CON SIGNO (los gastos son negativos)."),
        ("max_amount" = Option<String>, Query, description = "Cota superior del importe CON SIGNO: `max_amount=-50` son los gastos de 50 € o más."),
        ("date_from" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
        ("date_to" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
    ),
    responses(
        (status = 200, description = "Transacciones (orden op_date DESC)", body = [TransactionResponse]),
        (status = 400, description = "Filtro inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_transactions(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ListTxnQuery>,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve();
    let (out, _total) = list_transactions_core(
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
        None,
        0,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_transactions`.
/// La validación de filtros vive aquí para que ambos caminos devuelvan los mismos 400.
///
/// Paginación: con `limit = None` (el handler HTTP) no se emite `LIMIT`/`OFFSET` ni la query de
/// `COUNT` — el conjunto entero, contrato REST intacto. Con `limit = Some(n)` (la tool MCP) la
/// paginación baja a SQL y `total_count` sale de un `COUNT(*)` con los mismos filtros: la DB ya
/// no materializa el conjunto entero para servir una página.
pub(crate) async fn list_transactions_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    f: TxnFilters<'_>,
    limit: Option<i64>,
    offset: i64,
) -> Result<(Vec<TransactionResponse>, i64), ApiError> {
    let TxnFilters {
        month,
        kind,
        category_id,
        import_id,
        concept_contains,
        min_amount,
        max_amount,
        date_from,
        date_to,
    } = f;

    let kind = match kind {
        Some(k) => Some(normalize_kind(k)?),
        None => None,
    };
    // `month` y el rango libre son dos formas de decir lo mismo: aceptar ambas obligaría a definir
    // qué gana, y cualquier respuesta sería una trampa silenciosa para el llamante.
    if month.is_some() && (date_from.is_some() || date_to.is_some()) {
        return Err(ApiError::BadRequest(
            "month and date_from/date_to are mutually exclusive: use one or the other".into(),
        ));
    }
    if let (Some(from), Some(to)) = (date_from, date_to) {
        if from > to {
            return Err(ApiError::BadRequest(
                "date_from must not be after date_to".into(),
            ));
        }
    }
    if let (Some(lo), Some(hi)) = (min_amount, max_amount) {
        if lo > hi {
            return Err(ApiError::BadRequest(
                "min_amount must not be greater than max_amount (both are signed: expenses are negative)".into(),
            ));
        }
    }
    let concept_needle = match concept_contains {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.chars().count() > 200 {
                return Err(ApiError::BadRequest(
                    "concept_contains must be between 1 and 200 characters".into(),
                ));
            }
            Some(like_needle(trimmed))
        }
        None => None,
    };
    let month_range = match month {
        Some(m) => Some(parse_month(m)?),
        None => None,
    };

    let scope = view.scope_where("t");
    let mut arg = view.next_arg_index();
    let mut filters = String::new();
    if month_range.is_some() {
        filters.push_str(&format!(
            " AND t.op_date >= ${} AND t.op_date < ${}",
            arg,
            arg + 1
        ));
        arg += 2;
    }
    if kind.is_some() {
        filters.push_str(&format!(" AND t.kind = ${arg}"));
        arg += 1;
    }
    if category_id.is_some() {
        filters.push_str(&format!(" AND t.category_id = ${arg}"));
        arg += 1;
    }
    if import_id.is_some() {
        filters.push_str(&format!(" AND t.import_id = ${arg}"));
        arg += 1;
    }
    if concept_needle.is_some() {
        // El patrón llega ya plegado y escapado desde `like_needle`; la columna se pliega con la
        // misma tabla vía `translate` (colación-independiente, ver `schema.rs`).
        filters.push_str(&format!(
            " AND {} LIKE ${arg} ESCAPE '\\'",
            sql_fold_concept_expr("t.concept")
        ));
        arg += 1;
    }
    if min_amount.is_some() {
        filters.push_str(&format!(" AND t.amount >= ${arg}"));
        arg += 1;
    }
    if max_amount.is_some() {
        filters.push_str(&format!(" AND t.amount <= ${arg}"));
        arg += 1;
    }
    if date_from.is_some() {
        filters.push_str(&format!(" AND t.op_date >= ${arg}"));
        arg += 1;
    }
    if date_to.is_some() {
        // Inclusivo: «hasta el 31» incluye el 31. Un `<` exclusivo es el off-by-one-day clásico.
        filters.push_str(&format!(" AND t.op_date <= ${arg}"));
        arg += 1;
    }

    let mut sql = format!("{TXN_SELECT} WHERE {scope}{filters}");
    sql.push_str(" ORDER BY t.op_date DESC, t.created_at DESC, t.id DESC");
    if limit.is_some() {
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", arg, arg + 1));
    }

    // Cierre que aplica los binds de filtro en el MISMO orden en que se emitieron los
    // placeholders — compartido por la query principal y el COUNT.
    macro_rules! bind_filters {
        ($q:expr) => {{
            let mut query = $q;
            if let Some((start, end)) = month_range {
                query = query.bind(start).bind(end);
            }
            if let Some(ref k) = kind {
                query = query.bind(k.clone());
            }
            if let Some(cid) = category_id {
                query = query.bind(cid);
            }
            if let Some(imp) = import_id {
                query = query.bind(imp);
            }
            if let Some(ref n) = concept_needle {
                query = query.bind(n.clone());
            }
            if let Some(lo) = min_amount {
                query = query.bind(lo);
            }
            if let Some(hi) = max_amount {
                query = query.bind(hi);
            }
            if let Some(from) = date_from {
                query = query.bind(from);
            }
            if let Some(to) = date_to {
                query = query.bind(to);
            }
            query
        }};
    }

    let mut query =
        bind_filters!(view.bind_scope_as(sqlx::query_as::<_, TxnRow>(&sql), iid, user_id));
    if let Some(l) = limit {
        query = query.bind(l).bind(offset);
    }
    let rows: Vec<TxnRow> = query.fetch_all(pool).await?;

    let total_count: i64 = match limit {
        None => rows.len() as i64,
        Some(_) => {
            let count_sql =
                format!("SELECT COUNT(*)::bigint FROM transactions t WHERE {scope}{filters}");
            bind_filters!(view.bind_scope_scalar(sqlx::query_scalar(&count_sql), iid, user_id))
                .fetch_one(pool)
                .await?
        }
    };

    Ok((rows.into_iter().map(row_to_response).collect(), total_count))
}

// ---------------------------------------------------------------------------
// PATCH /v1/transactions/batch — reclasificación en lote
// ---------------------------------------------------------------------------

/// Resultado del PATCH en lote.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct BatchPatchResponse {
    pub updated: i64,
    /// Hasta 20 `resumen` («fecha · concepto · importe (kind)»), para verificar que se tocó lo
    /// correcto sin releer nada. Con más ítems se trunca y se marca `resumen_truncated`.
    pub resumen: Vec<String>,
    pub resumen_truncated: bool,
}

const BATCH_RESUMEN_MAX: usize = 20;

#[utoipa::path(
    patch,
    path = "/v1/transactions/batch",
    tag = "transactions",
    request_body = BatchPatchBody,
    responses(
        (status = 200, description = "Lote actualizado", body = BatchPatchResponse),
        (status = 400, description = "Validación (lote vacío, tope, sin campos)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Algún id no existe o no es del usuario (cero filas tocadas)"),
    )
)]
pub async fn patch_batch(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<BatchPatchBody>,
) -> Result<Json<BatchPatchResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let out = patch_transactions_batch_core(&state, iid, user.id.0, body).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_transactions`.
///
/// **Todo o nada** en una única transacción (mismo criterio que `create_batch`): un id ajeno o
/// inexistente ⇒ 404 nombrándolo y cero filas tocadas. Un resultado parcial obligaría al llamante a
/// reconciliar estado, que es justo lo que un lote viene a evitar.
///
/// **Una sola invalidación COND** al final, fuera del bucle: el caso real —16 recategorizaciones
/// seguidas en modo C— tiraba la cache de proyección 16 veces.
///
/// El conjunto de campos es cerrado (`kind`, `category_id`, `notes`): ninguno entra en la huella de
/// dedup (`source · op_date · amount · concept`) ni en el emparejado de transferencias
/// (`op_date`, `amount`), así que el lote no recomputa huellas, no rompe pares y no dispara el pase
/// de auto-conciliación. Eso es lo que lo hace seguro, y por eso no admite `amount`/`op_date`.
pub(crate) async fn patch_transactions_batch_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: BatchPatchBody,
) -> Result<BatchPatchResponse, ApiError> {
    if body.ids.is_empty() {
        return Err(ApiError::BadRequest("ids must not be empty".into()));
    }
    if body.ids.len() > MAX_PATCH_BATCH {
        return Err(ApiError::BadRequest(format!(
            "batch must contain at most {MAX_PATCH_BATCH} ids"
        )));
    }
    let clear_category = body.clear_category.unwrap_or(false);
    let clear_notes = body.clear_notes.unwrap_or(false);
    if body.kind.is_none()
        && body.category_id.is_none()
        && !clear_category
        && body.notes.is_none()
        && !clear_notes
    {
        return Err(ApiError::BadRequest(
            "nothing to update: provide kind, category_id/clear_category or notes/clear_notes".into(),
        ));
    }
    if body.category_id.is_some() && clear_category {
        return Err(ApiError::BadRequest(
            "category_id and clear_category are mutually exclusive".into(),
        ));
    }
    if body.notes.is_some() && clear_notes {
        return Err(ApiError::BadRequest(
            "notes and clear_notes are mutually exclusive".into(),
        ));
    }
    let kind = match &body.kind {
        Some(k) => Some(normalize_kind(k)?),
        None => None,
    };
    let notes = match &body.notes {
        Some(n) => Some(normalize_notes(&Some(n.clone()))?.unwrap_or_default()),
        None => None,
    };

    // Deduplicar preservando el orden: repetir un id no debe contarlo dos veces en `updated`.
    let mut ids: Vec<Uuid> = Vec::with_capacity(body.ids.len());
    for id in &body.ids {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }

    let mut tx = state.pool.begin().await?;

    // Carga + owner-guard ANTES de escribir: si falta uno, no se ha tocado nada todavía.
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        op_date: NaiveDate,
        concept: String,
        amount: Decimal,
        kind: Option<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        r#"SELECT id, op_date, concept, amount, kind FROM transactions
           WHERE id = ANY($1) AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(&ids)
    .bind(iid)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    if rows.len() != ids.len() {
        let found: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !found.contains(id))
            .take(5)
            .map(|id| id.to_string())
            .collect();
        // 404 y no 403: un movimiento de otro usuario no revela su existencia, igual que el PATCH
        // individual. Se nombran hasta 5 ids para que el llamante no tenga que buscar a ciegas.
        return Err(ApiError::NotFoundWith(format!(
            "{} of {} ids are unknown or not yours (e.g. {}); nothing was updated",
            ids.len() - rows.len(),
            ids.len(),
            missing.join(", ")
        )));
    }

    // El par (kind, categoría) resultante se valida UNA vez por fila afectada: la categoría podría
    // no encajar con el kind que quede tras el merge.
    let effective_category = if clear_category {
        None
    } else {
        body.category_id
    };
    if kind.is_some() || body.category_id.is_some() || clear_category {
        for r in &rows {
            let k = kind.clone().or_else(|| r.kind.clone());
            match k {
                Some(k) => {
                    assert_transaction_category(&state.pool, iid, &k, effective_category).await?
                }
                None if effective_category.is_some() => {
                    return Err(ApiError::BadRequest(
                        "category requires a kind: set kind in the same batch".into(),
                    ))
                }
                None => {}
            }
        }
    }

    let mut sets: Vec<String> = Vec::new();
    let mut arg = 1;
    if kind.is_some() {
        sets.push(format!("kind = ${arg}"));
        arg += 1;
    }
    if body.category_id.is_some() || clear_category {
        sets.push(format!("category_id = ${arg}"));
        arg += 1;
    }
    if notes.is_some() || clear_notes {
        sets.push(format!("notes = ${arg}"));
        arg += 1;
    }
    let sql = format!(
        "UPDATE transactions SET {}, updated_at = now() WHERE id = ANY(${arg})",
        sets.join(", ")
    );
    let mut q = sqlx::query(&sql);
    if let Some(k) = &kind {
        q = q.bind(k);
    }
    if body.category_id.is_some() || clear_category {
        q = q.bind(effective_category);
    }
    if notes.is_some() || clear_notes {
        q = q.bind(if clear_notes { None } else { notes.clone() });
    }
    let done = q.bind(&ids).execute(&mut *tx).await?;
    tx.commit().await?;

    // UNA sola invalidación para todo el lote (post-commit), no una por ítem.
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;

    let mut resumen: Vec<String> = rows
        .iter()
        .take(BATCH_RESUMEN_MAX)
        .map(|r| {
            format!(
                "{} · {} · {} ({})",
                r.op_date,
                r.concept,
                r.amount,
                kind.as_deref().or(r.kind.as_deref()).unwrap_or("-")
            )
        })
        .collect();
    let resumen_truncated = rows.len() > BATCH_RESUMEN_MAX;
    resumen.shrink_to_fit();
    Ok(BatchPatchResponse {
        updated: done.rows_affected() as i64,
        resumen,
        resumen_truncated,
    })
}

// ---------------------------------------------------------------------------
// GET /v1/transactions/months
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/transactions/months",
    tag = "transactions",
    params(("view" = Option<String>, Query, description = "`mine` | household.")),
    responses(
        (status = 200, description = "Meses con datos (orden DESC)", body = [MonthEntry]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_months(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<MonthEntry>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_months_core(&state.pool, iid, user.id.0, q.resolve()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_transaction_months`.
pub(crate) async fn list_months_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<MonthEntry>, ApiError> {
    let today = installation_naive_today(pool, iid).await?;
    let current_month = today.format("%Y-%m").to_string();

    let scope = view.scope_where("t");
    let sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS month, COUNT(*)::bigint AS txn_count
         FROM transactions t
         WHERE {scope}
         GROUP BY to_char(t.op_date, 'YYYY-MM')
         ORDER BY month DESC"
    );
    let rows: Vec<(String, i64)> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(month, txn_count)| MonthEntry {
            is_complete: month != current_month,
            month,
            txn_count,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// PATCH /v1/transactions/{id}
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct TxnCore {
    import_id: Option<Uuid>,
    source: String,
    op_date: NaiveDate,
    #[allow(dead_code)]
    value_date: Option<NaiveDate>,
    concept: String,
    amount: Decimal,
    kind: Option<String>,
    category_id: Option<Uuid>,
    linked_asset_id: Option<Uuid>,
    linked_liability_id: Option<Uuid>,
    notes: Option<String>,
    fingerprint: String,
    fingerprint_ordinal: i32,
}

#[utoipa::path(
    patch,
    path = "/v1/transactions/{id}",
    tag = "transactions",
    request_body = PatchTransactionBody,
    params(("id" = Uuid, Path, description = "Transaction id")),
    responses(
        (status = 200, description = "Transacción actualizada", body = TransactionResponse),
        (status = 400, description = "Validación"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Transacción inexistente o de otro usuario"),
        (status = 409, description = "Huella duplicada tras recomputar"),
    )
)]
pub async fn patch_transaction(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchTransactionBody>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_transaction_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_transaction`. Merge campo a
/// campo con flags `clear_*`, política de huella (manual recomputa / importada anclada) e
/// invalidación COND post-commit dentro. Owner-guard → 404 (solo movimientos propios).
pub(crate) async fn patch_transaction_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchTransactionBody,
) -> Result<TransactionResponse, ApiError> {
    let current: Option<TxnCore> = sqlx::query_as(
        r#"SELECT import_id, source, op_date, value_date, concept, amount, kind, category_id,
                  linked_asset_id, linked_liability_id, notes, fingerprint, fingerprint_ordinal
           FROM transactions
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    // op_date/amount/concept son editables tanto en manuales como en importadas. La diferencia está
    // en la huella de dedup (ver más abajo): en manuales se recomputa; en importadas queda anclada a
    // la del CSV original, para que un re-import del mismo archivo siga detectando el duplicado
    // aunque el usuario haya reubicado la fecha o corregido el importe/concepto.
    let is_imported = current.import_id.is_some();

    let new_op_date = body.op_date.unwrap_or(current.op_date);
    let new_amount = body.amount.map(|a| a.round_dp(4)).unwrap_or(current.amount);
    if new_amount.is_zero() {
        return Err(ApiError::BadRequest(
            "amount_zero: amount must not be zero".into(),
        ));
    }
    let new_concept = match &body.concept {
        Some(c) => normalize_concept_field(c)?,
        None => current.concept.clone(),
    };
    let new_value_date = if body.clear_value_date == Some(true) {
        None
    } else {
        body.value_date.or(current.value_date)
    };
    let new_kind = match &body.kind {
        Some(k) => Some(normalize_kind(k)?),
        None => current.kind.clone(),
    };
    let new_category = if body.clear_category == Some(true) {
        None
    } else {
        body.category_id.or(current.category_id)
    };
    let new_linked_asset = if body.clear_linked_asset == Some(true) {
        None
    } else {
        body.linked_asset_id.or(current.linked_asset_id)
    };
    let new_linked_liability = if body.clear_linked_liability == Some(true) {
        None
    } else {
        body.linked_liability_id.or(current.linked_liability_id)
    };
    let new_notes = if body.clear_notes == Some(true) {
        None
    } else {
        match &body.notes {
            Some(_) => normalize_notes(&body.notes)?,
            None => current.notes.clone(),
        }
    };

    // Validaciones kind↔categoría y links.
    match &new_kind {
        Some(k) => assert_transaction_category(&state.pool, iid, k, new_category).await?,
        None => {
            if new_category.is_some() {
                return Err(ApiError::BadRequest("category requires a kind".into()));
            }
        }
    }
    assert_asset_in_installation(&state.pool, iid, new_linked_asset).await?;
    assert_liability_in_installation(&state.pool, iid, new_linked_liability).await?;

    // ¿El PATCH toca los campos que definen el emparejado de transferencia? Un par cuyos importes
    // ya no se cancelan (o cuyas fechas se separan) dejaría dinero real oculto → se rompe DENTRO
    // de la misma transacción, SIN registrar rechazo (no es una decisión del usuario sobre el par:
    // volver al valor original re-empareja en el siguiente pase).
    let pairing_changed = new_op_date != current.op_date || new_amount != current.amount;

    // Huella: en manuales se recomputa cuando cambian op_date/amount/concept (y toma un ordinal
    // libre); en importadas NUNCA se recomputa → queda anclada a la del CSV original para que el
    // dedup del re-import siga funcionando pese a la edición.
    let mut tx = state.pool.begin().await?;
    if pairing_changed {
        unlink_pair_no_rejection(&mut tx, iid, user_id, id).await?;
    }
    let (new_fp, new_ordinal) = if !is_imported {
        let fp = compute_fingerprint(&current.source, new_op_date, new_amount, &new_concept);
        if fp == current.fingerprint {
            (current.fingerprint.clone(), current.fingerprint_ordinal)
        } else {
            let ord = next_fingerprint_ordinal(&mut tx, iid, user_id, &fp).await?;
            (fp, ord)
        }
    } else {
        (current.fingerprint.clone(), current.fingerprint_ordinal)
    };

    sqlx::query(
        r#"UPDATE transactions
           SET op_date = $1, value_date = $2, concept = $3, amount = $4, kind = $5,
               category_id = $6, linked_asset_id = $7, linked_liability_id = $8, notes = $9,
               fingerprint = $10, fingerprint_ordinal = $11, updated_at = now()
           WHERE id = $12 AND installation_id = $13 AND owner_user_id = $14"#,
    )
    .bind(new_op_date)
    .bind(new_value_date)
    .bind(&new_concept)
    .bind(new_amount)
    .bind(new_kind.as_deref())
    .bind(new_category)
    .bind(new_linked_asset)
    .bind(new_linked_liability)
    .bind(new_notes.as_deref())
    .bind(&new_fp)
    .bind(new_ordinal)
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Los nuevos amount/op_date pueden abrir (o reabrir) un emparejado → pase antes de invalidar.
    if pairing_changed {
        auto_reconcile_after_mutation(state, iid, user_id).await;
        recurring::converge_recurring_after_mutation(state, iid).await;
    }
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    load_txn(&state.pool, id).await
}

// ---------------------------------------------------------------------------
// DELETE /v1/transactions/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/transactions/{id}",
    tag = "transactions",
    params(("id" = Uuid, Path, description = "Transaction id")),
    responses(
        (status = 204, description = "Transacción borrada"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Transacción inexistente o de otro usuario"),
    )
)]
pub async fn delete_transaction(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    delete_transaction_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Lectura de un movimiento PROPIO por id (owner-guard → 404). La usa el preview de la tool MCP
/// `delete_transaction` — cero SQL en el módulo mcp.
pub(crate) async fn get_transaction_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<TransactionResponse, ApiError> {
    let owned: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM transactions
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    if owned.is_none() {
        return Err(ApiError::NotFound);
    }
    load_txn(pool, id).await
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_transaction`.
/// Hard delete con owner-guard → 404; invalidación COND post-delete dentro.
pub(crate) async fn delete_transaction_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res = sqlx::query(
        r#"DELETE FROM transactions WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    // La FK ON DELETE SET NULL ya desconcilió a la posible superviviente; el pase le busca otra
    // contrapartida (punto fijo) antes de la única invalidación.
    auto_reconcile_after_mutation(state, iid, user_id).await;
    recurring::converge_recurring_after_mutation(state, iid).await;
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// GET /v1/transactions/imports  · DELETE /v1/transactions/imports/{id}
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct ImportRow {
    id: Uuid,
    source: String,
    account_asset_id: Option<Uuid>,
    account_asset_name: Option<String>,
    original_filename: Option<String>,
    created_at: chrono::DateTime<Utc>,
    txn_count: i64,
}

#[utoipa::path(
    get,
    path = "/v1/transactions/imports",
    tag = "transactions",
    params(("view" = Option<String>, Query, description = "`mine` | household.")),
    responses(
        (status = 200, description = "Lotes de import (orden created_at DESC)", body = [ImportBatchResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_imports(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<ImportBatchResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_imports_core(&state.pool, iid, user.id.0, q.resolve()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_transaction_imports`.
pub(crate) async fn list_imports_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<ImportBatchResponse>, ApiError> {
    let scope = view.scope_where("ti");
    let sql = format!(
        "SELECT ti.id, ti.source, ti.account_asset_id, a.name AS account_asset_name,
                ti.original_filename, ti.created_at,
                (SELECT COUNT(*)::bigint FROM transactions t WHERE t.import_id = ti.id) AS txn_count
         FROM transaction_imports ti
         LEFT JOIN assets a ON a.id = ti.account_asset_id
         WHERE {scope}
         ORDER BY ti.created_at DESC, ti.id DESC"
    );
    let rows: Vec<ImportRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| ImportBatchResponse {
            id: r.id,
            source: r.source,
            account_asset_id: r.account_asset_id,
            account_asset_name: r.account_asset_name,
            original_filename: r.original_filename,
            created_at: r.created_at,
            txn_count: r.txn_count,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct DeleteImportQuery {
    #[serde(default)]
    pub confirm: bool,
}

#[utoipa::path(
    delete,
    path = "/v1/transactions/imports/{id}",
    tag = "transactions",
    params(
        ("id" = Uuid, Path, description = "Import batch id"),
        ("confirm" = bool, Query, description = "Debe ser `true` (borra el lote y sus transacciones en cascada)."),
    ),
    responses(
        (status = 204, description = "Lote borrado (transacciones en cascada)"),
        (status = 400, description = "confirm != true"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Lote inexistente o de otro usuario"),
    )
)]
pub async fn delete_import(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Query(q): Query<DeleteImportQuery>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    if !q.confirm {
        return Err(ApiError::BadRequest(
            "confirm_required: pass ?confirm=true to undo this import (deletes its transactions)"
                .into(),
        ));
    }
    delete_import_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE (que exige `?confirm=true`) y la tool MCP
/// `delete_import` (patrón preview/confirm). Borra el lote Y sus transacciones en cascada →
/// invalidación COND dentro.
pub(crate) async fn delete_import_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res = sqlx::query(
        r#"DELETE FROM transaction_imports
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    // El borrado del lote cascadea a sus transacciones → cambia el conjunto. Las contrapartidas
    // supervivientes de otros lotes quedaron sueltas (FK SET NULL) → pase antes de invalidar.
    auto_reconcile_after_mutation(state, iid, user_id).await;
    recurring::converge_recurring_after_mutation(state, iid).await;
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    Ok(())
}
