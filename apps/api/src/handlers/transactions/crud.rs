//! Altas manuales, listado mensual, meses con datos, PATCH/DELETE de transacciones y gestión de
//! lotes de import (`/v1/transactions`, `/months`, `/imports`, `/imports/{id}`).
//!
//! Cada mutación invalida la cache de proyección solo en modo `transactions_avg`
//! (`invalidate_projection_if_transactions_avg`); en modo `budget` no hace nada. Ver el contrato en
//! `transactions/mod.rs`.

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::LedgerViewQuery;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    compute_fingerprint, normalize_concept_field, normalize_kind, normalize_notes,
    BatchCreateBody, CreateTransactionBody, ImportBatchResponse, MonthEntry, PatchTransactionBody,
    TransactionResponse, SOURCE_MANUAL,
};
use crate::handlers::transactions::recurring;
use crate::handlers::transactions::{
    assert_asset_in_installation, assert_liability_in_installation, assert_transaction_category,
    invalidate_projection_if_transactions_avg, next_fingerprint_ordinal, row_to_response, TxnRow,
    TXN_SELECT,
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
        return Err(ApiError::BadRequest("amount_zero: amount must not be zero".into()));
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

async fn load_txn(pool: &sqlx::PgPool, id: Uuid) -> Result<TransactionResponse, ApiError> {
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

    let prepared = validate_manual(&state.pool, iid, &body).await?;
    // Recurrencia (opcional): valida el día del mes ANTES de abrir la transacción.
    let recurrence_day = match &body.recurrence {
        Some(rec) => Some(recurring::resolve_rule_day(rec, body.op_date)?),
        None => None,
    };
    // "Hoy" de la instalación para el backfill de meses intermedios (solo si hay recurrencia).
    let today = match recurrence_day {
        Some(_) => Some(installation_naive_today(&state.pool, iid).await?),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    let ordinal = next_fingerprint_ordinal(&mut tx, iid, user.id.0, &prepared.fingerprint).await?;
    // Si hay recurrencia, primero se crea la regla-plantilla; la transacción de origen queda enlazada.
    let cursor = recurring::month_start_of(body.op_date);
    let rule_id = match recurrence_day {
        Some(day) => {
            Some(recurring::insert_rule(&mut tx, iid, user.id.0, &prepared, day, cursor).await?)
        }
        None => None,
    };
    let id = insert_manual(&mut tx, iid, user.id.0, &prepared, ordinal, rule_id).await?;
    // Backfill: un alta con fecha pasada deja creadas TODAS las instancias intermedias hasta hoy en
    // el mismo commit (antes solo aparecían cuando el frontend llamaba a materialize al montar).
    if let (Some(rule_id), Some(day), Some(today)) = (rule_id, recurrence_day, today) {
        recurring::backfill_new_rule(&mut tx, iid, user.id.0, rule_id, &prepared, day, cursor, today)
            .await?;
    }
    tx.commit().await?;

    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
    let resp = load_txn(&state.pool, id).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
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
        let cursor = recurring::month_start_of(b.op_date);
        let rule = match &b.recurrence {
            Some(rec) => {
                let day = recurring::resolve_rule_day(rec, b.op_date)?;
                let rid = recurring::insert_rule(&mut tx, iid, user.id.0, p, day, cursor).await?;
                Some((rid, day))
            }
            None => None,
        };
        let id = insert_manual(&mut tx, iid, user.id.0, p, ord, rule.map(|(rid, _)| rid)).await?;
        // Backfill por ítem: un alta con fecha pasada deja creadas las instancias intermedias.
        if let Some((rid, day)) = rule {
            let today = today.expect("today computed when any recurrence present");
            recurring::backfill_new_rule(&mut tx, iid, user.id.0, rid, p, day, cursor, today).await?;
        }
        next_ord.insert(p.fingerprint.clone(), ord + 1);
        ids.push(id);
    }
    tx.commit().await?;

    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
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
    /// `YYYY-MM`.
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub import_id: Option<Uuid>,
}

/// Parsea `YYYY-MM` → (primer día del mes, primer día del mes siguiente).
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
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
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
    let view = LedgerViewQuery { view: q.view.clone() }.resolve();

    let kind = match &q.kind {
        Some(k) => Some(normalize_kind(k)?),
        None => None,
    };
    let month_range = match &q.month {
        Some(m) => Some(parse_month(m)?),
        None => None,
    };

    let scope = view.scope_where("t");
    let mut arg = view.next_arg_index();
    let mut sql = format!("{TXN_SELECT} WHERE {scope}");
    if month_range.is_some() {
        sql.push_str(&format!(" AND t.op_date >= ${} AND t.op_date < ${}", arg, arg + 1));
        arg += 2;
    }
    if kind.is_some() {
        sql.push_str(&format!(" AND t.kind = ${arg}"));
        arg += 1;
    }
    if q.category_id.is_some() {
        sql.push_str(&format!(" AND t.category_id = ${arg}"));
        arg += 1;
    }
    if q.import_id.is_some() {
        sql.push_str(&format!(" AND t.import_id = ${arg}"));
        // (last placeholder; no further increment needed)
        let _ = arg;
    }
    sql.push_str(" ORDER BY t.op_date DESC, t.created_at DESC, t.id DESC");

    let mut query = view.bind_scope_as(sqlx::query_as::<_, TxnRow>(&sql), iid, user.id.0);
    if let Some((start, end)) = month_range {
        query = query.bind(start).bind(end);
    }
    if let Some(k) = kind {
        query = query.bind(k);
    }
    if let Some(cid) = q.category_id {
        query = query.bind(cid);
    }
    if let Some(imp) = q.import_id {
        query = query.bind(imp);
    }
    let rows: Vec<TxnRow> = query.fetch_all(&state.pool).await?;
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
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
    let view = q.resolve();
    let today = installation_naive_today(&state.pool, iid).await?;
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
        .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;
    let out = rows
        .into_iter()
        .map(|(month, txn_count)| MonthEntry {
            is_complete: month != current_month,
            month,
            txn_count,
        })
        .collect();
    Ok(Json(out))
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

    let current: Option<TxnCore> = sqlx::query_as(
        r#"SELECT import_id, source, op_date, value_date, concept, amount, kind, category_id,
                  linked_asset_id, linked_liability_id, notes, fingerprint, fingerprint_ordinal
           FROM transactions
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
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
        return Err(ApiError::BadRequest("amount_zero: amount must not be zero".into()));
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
                return Err(ApiError::BadRequest(
                    "category requires a kind".into(),
                ));
            }
        }
    }
    assert_asset_in_installation(&state.pool, iid, new_linked_asset).await?;
    assert_liability_in_installation(&state.pool, iid, new_linked_liability).await?;

    // Huella: en manuales se recomputa cuando cambian op_date/amount/concept (y toma un ordinal
    // libre); en importadas NUNCA se recomputa → queda anclada a la del CSV original para que el
    // dedup del re-import siga funcionando pese a la edición.
    let mut tx = state.pool.begin().await?;
    let (new_fp, new_ordinal) = if !is_imported {
        let fp = compute_fingerprint(&current.source, new_op_date, new_amount, &new_concept);
        if fp == current.fingerprint {
            (current.fingerprint.clone(), current.fingerprint_ordinal)
        } else {
            let ord = next_fingerprint_ordinal(&mut tx, iid, user.id.0, &fp).await?;
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
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
    let resp = load_txn(&state.pool, id).await?;
    Ok(Json(resp))
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
    let res = sqlx::query(
        r#"DELETE FROM transactions WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
    Ok(axum::http::StatusCode::NO_CONTENT)
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
    let view = q.resolve();
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
        .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;
    let out = rows
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
        .collect();
    Ok(Json(out))
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
            "confirm_required: pass ?confirm=true to undo this import (deletes its transactions)".into(),
        ));
    }
    let res = sqlx::query(
        r#"DELETE FROM transaction_imports
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    // El borrado del lote cascadea a sus transacciones → cambia el conjunto.
    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
