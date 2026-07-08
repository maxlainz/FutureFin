//! Reglas de transacción recurrente (`/v1/transactions/recurring`).
//!
//! Una regla es una **plantilla** per-user (nómina, alquiler, aportación mensual…): concepto,
//! importe firmado, kind, categoría, enlaces y día del mes. `POST /materialize` genera las
//! transacciones reales pendientes en `transactions` (`source='manual'`, `recurring_rule_id` de la
//! regla), una por cada mes civil vencido desde el cursor `last_materialized_month`. Nunca crea
//! movimientos con fecha futura: el mes en curso sólo se materializa cuando su día de recurrencia
//! (ya con clamp de fin de mes) es <= hoy; si aún no ha llegado, ese mes se deja para la primera
//! llamada posterior a esa fecha. El cursor es la única fuente de idempotencia: re-materializar no
//! duplica ni recrea instancias borradas (el cursor ya pasó ese mes).
//!
//! Como el resto del módulo `transactions`, NINGÚN handler de aquí llama a
//! `refresh_projection_after_mutation`: las transacciones (recurrentes o no) no son inputs del
//! motor de proyección (contrato no-cache, regresión `transactions_projection_cache.rs`).

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::crud::PreparedTxn;
use crate::handlers::transactions::next_fingerprint_ordinal;
use crate::handlers::transactions::schema::{
    compute_fingerprint, MaterializeResponse, RecurrenceSpec, RecurringRuleResponse, SOURCE_MANUAL,
};
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgConnection};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers de fecha
// ---------------------------------------------------------------------------

fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid first-of-month")
}

/// Primer día del mes de `date`. Cursor inicial de una regla creada desde una alta manual.
pub(super) fn month_start_of(date: NaiveDate) -> NaiveDate {
    first_of_month(date.year(), date.month())
}

/// `(year, month) + delta` meses (delta con signo), normalizado.
fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    let ny = zero.div_euclid(12) as i32;
    let nm = (zero.rem_euclid(12) + 1) as u32;
    (ny, nm)
}

/// Nº de días del mes civil `(year, month)`.
fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = shift_month(year, month, 1);
    (first_of_month(ny, nm) - first_of_month(year, month)).num_days() as u32
}

/// Resuelve y valida el `day_of_month` de una recurrencia: explícito (1..=31) o, si se omite, el
/// día de `op_date`. Fuera de rango → 400 `recurrence_day_out_of_range`.
pub(super) fn resolve_rule_day(rec: &RecurrenceSpec, op_date: NaiveDate) -> Result<i32, ApiError> {
    let day = match rec.day_of_month {
        Some(d) => {
            if !(1..=31).contains(&d) {
                return Err(ApiError::BadRequest(
                    "recurrence_day_out_of_range: recurrence.day_of_month must be between 1 and 31"
                        .into(),
                ));
            }
            d
        }
        None => op_date.day(),
    };
    Ok(day as i32)
}

// ---------------------------------------------------------------------------
// Inserción de la regla (usado por crud.rs en el alta con recurrencia)
// ---------------------------------------------------------------------------

/// Inserta una regla recurrente derivada de una transacción preparada y devuelve su id. El cursor
/// `last_materialized_month` (primer día de mes) evita que la instancia de origen se re-materialice.
pub(super) async fn insert_rule(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    p: &PreparedTxn,
    day_of_month: i32,
    last_materialized_month: NaiveDate,
) -> Result<Uuid, ApiError> {
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO recurring_transaction_rules
               (installation_id, owner_user_id, concept, amount, kind, category_id,
                day_of_month, linked_asset_id, linked_liability_id, notes, last_materialized_month)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(&p.concept)
    .bind(p.amount)
    .bind(&p.kind)
    .bind(p.category_id)
    .bind(day_of_month)
    .bind(p.linked_asset_id)
    .bind(p.linked_liability_id)
    .bind(p.notes.as_deref())
    .bind(last_materialized_month)
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// GET /v1/transactions/recurring
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct RuleRow {
    id: Uuid,
    concept: String,
    amount: Decimal,
    kind: String,
    category_id: Option<Uuid>,
    category_name: Option<String>,
    day_of_month: i32,
    linked_asset_id: Option<Uuid>,
    linked_liability_id: Option<Uuid>,
    notes: Option<String>,
    last_materialized_month: NaiveDate,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn row_to_response(r: RuleRow) -> RecurringRuleResponse {
    RecurringRuleResponse {
        id: r.id,
        concept: r.concept,
        amount: r.amount,
        kind: r.kind,
        category_id: r.category_id,
        category_name: r.category_name,
        day_of_month: r.day_of_month as u32,
        linked_asset_id: r.linked_asset_id,
        linked_liability_id: r.linked_liability_id,
        notes: r.notes,
        last_materialized_month: r.last_materialized_month.format("%Y-%m-%d").to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const RULE_SELECT: &str = r#"SELECT r.id, r.concept, r.amount, r.kind, r.category_id,
              c.name AS category_name, r.day_of_month, r.linked_asset_id, r.linked_liability_id,
              r.notes, r.last_materialized_month, r.created_at, r.updated_at
       FROM recurring_transaction_rules r
       LEFT JOIN categories c ON c.id = r.category_id"#;

#[utoipa::path(
    get,
    path = "/v1/transactions/recurring",
    tag = "transactions",
    responses(
        (status = 200, description = "Reglas recurrentes del usuario (orden created_at DESC)", body = [RecurringRuleResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_recurring_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<RecurringRuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let sql = format!(
        "{RULE_SELECT} WHERE r.installation_id = $1 AND r.owner_user_id = $2 \
         ORDER BY r.created_at DESC, r.id ASC"
    );
    let rows: Vec<RuleRow> = sqlx::query_as(&sql)
        .bind(iid)
        .bind(user.id.0)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/recurring/materialize
// ---------------------------------------------------------------------------

/// Fila mínima de una regla para el bucle de materialización.
#[derive(Debug, FromRow)]
struct RuleCore {
    id: Uuid,
    concept: String,
    amount: Decimal,
    kind: String,
    category_id: Option<Uuid>,
    day_of_month: i32,
    linked_asset_id: Option<Uuid>,
    linked_liability_id: Option<Uuid>,
    notes: Option<String>,
    last_materialized_month: NaiveDate,
}

#[utoipa::path(
    post,
    path = "/v1/transactions/recurring/materialize",
    tag = "transactions",
    responses(
        (status = 200, description = "Reglas materializadas (idempotente)", body = MaterializeResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn materialize_recurring(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<MaterializeResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let today = installation_naive_today(&state.pool, iid).await?;
    let current_month = first_of_month(today.year(), today.month());

    let mut tx = state.pool.begin().await?;

    // `FOR UPDATE` serializa llamadas concurrentes: dos materializaciones simultáneas no pueden
    // avanzar el mismo cursor a la vez (la segunda espera a que la primera commitee).
    let rules: Vec<RuleCore> = sqlx::query_as(
        r#"SELECT id, concept, amount, kind, category_id, day_of_month,
                  linked_asset_id, linked_liability_id, notes, last_materialized_month
           FROM recurring_transaction_rules
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY created_at ASC, id ASC
           FOR UPDATE"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .fetch_all(&mut *tx)
    .await?;

    let rules_processed = rules.len() as u32;
    let mut materialized = 0u32;

    for rule in &rules {
        // Avanza mes a mes desde el cursor hasta (incl.) el mes civil actual.
        let mut cursor = rule.last_materialized_month;
        loop {
            let (ny, nm) = shift_month(cursor.year(), cursor.month(), 1);
            let next_month = first_of_month(ny, nm);
            if next_month > current_month {
                break;
            }
            // op_date = min(day_of_month, días del mes) → clamp del 31 en meses cortos (feb/abr).
            let dim = days_in_month(ny, nm);
            let day = (rule.day_of_month as u32).min(dim);
            let op_date = NaiveDate::from_ymd_opt(ny, nm, day).expect("valid op date");

            // Nunca materializamos con fecha futura: el ledger es histórico (el alta manual limita
            // `op_date` a hoy). Si el día de la recurrencia del mes en curso aún no ha llegado,
            // paramos ANTES de insertar y SIN avanzar el cursor → ese mes se materializará en la
            // primera llamada cuyo día ya haya vencido (idempotencia intacta: el cursor sólo
            // avanza con inserciones).
            if op_date > today {
                break;
            }

            let fp = compute_fingerprint(SOURCE_MANUAL, op_date, rule.amount, &rule.concept);
            // Ordinal MAX+1 dentro de la tx: una instancia manual idéntica preexistente jamás
            // produce un 409 (la copia toma el siguiente ordinal).
            let ordinal = next_fingerprint_ordinal(&mut tx, iid, user.id.0, &fp).await?;

            sqlx::query(
                r#"INSERT INTO transactions
                       (installation_id, owner_user_id, import_id, source, op_date, value_date,
                        concept, amount, currency, kind, category_id, fingerprint,
                        fingerprint_ordinal, linked_asset_id, linked_liability_id, notes,
                        recurring_rule_id)
                   VALUES ($1, $2, NULL, $3, $4, NULL, $5, $6, 'EUR', $7, $8, $9, $10, $11, $12, $13, $14)"#,
            )
            .bind(iid)
            .bind(user.id.0)
            .bind(SOURCE_MANUAL)
            .bind(op_date)
            .bind(&rule.concept)
            .bind(rule.amount)
            .bind(&rule.kind)
            .bind(rule.category_id)
            .bind(&fp)
            .bind(ordinal)
            .bind(rule.linked_asset_id)
            .bind(rule.linked_liability_id)
            .bind(rule.notes.as_deref())
            .bind(rule.id)
            .execute(&mut *tx)
            .await?;

            cursor = next_month;
            materialized += 1;
        }

        // Avanza el cursor sólo si generó algo (idempotente: 2ª llamada → 0 nuevas).
        if cursor != rule.last_materialized_month {
            sqlx::query(
                r#"UPDATE recurring_transaction_rules
                   SET last_materialized_month = $1, updated_at = now()
                   WHERE id = $2"#,
            )
            .bind(cursor)
            .bind(rule.id)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;

    Ok(Json(MaterializeResponse {
        rules_processed,
        materialized,
    }))
}

// ---------------------------------------------------------------------------
// DELETE /v1/transactions/recurring/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/transactions/recurring/{id}",
    tag = "transactions",
    params(("id" = Uuid, Path, description = "Recurring rule id")),
    responses(
        (status = 204, description = "Regla borrada (las instancias ya materializadas se conservan)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Regla inexistente o de otro usuario"),
    )
)]
pub async fn delete_recurring_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    // La FK `transactions.recurring_rule_id` es ON DELETE SET NULL → las instancias sobreviven.
    let res = sqlx::query(
        r#"DELETE FROM recurring_transaction_rules
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
    Ok(axum::http::StatusCode::NO_CONTENT)
}
