//! Conciliación de transferencias internas (3.5.0): el pase automático de emparejado, la
//! conciliación/desconciliación manual de un par y sus rutas (`/v1/transactions/reconcile`,
//! `/v1/transactions/{id}/reconcile`).
//!
//! ## Modelo
//! Un movimiento CONCILIADO (`transfer_counterpart_id IS NOT NULL`) es una pata de una
//! transferencia interna enlazada a su contrapartida (la otra pata, normalmente importada de otro
//! extracto). Sigue visible en los listados, pero `summary.rs` y el `months[]` del cashflow lo
//! excluyen de todos los agregados de flujo. La relación es SIMÉTRICA (ambas patas se apuntan) e
//! INYECTIVA (índice UNIQUE parcial); este módulo es el ÚNICO que escribe los enlaces, siempre
//! ambas patas en la misma transacción.
//!
//! ## El pase automático (`auto_reconcile_owner`)
//! Determinista y de **punto fijo**: tras cualquier mutación del conjunto, correrlo otra vez
//! devuelve `pairs_created = 0`. Candidatos: mismo owner + misma divisa + importes exactamente
//! opuestos + `|Δop_date| ≤ 5 días` + ninguna pata ya conciliada + par no rechazado
//! (`transfer_match_rejections`). Desambiguación greedy con orden TOTAL (Δfecha, fechas, ids) →
//! el resultado es función del contenido de la BD, no del plan de Postgres ni del reloj.
//! Corre post-commit tras toda mutación (best-effort vía `auto_reconcile_after_mutation`: un
//! fallo se loguea y NO convierte la mutación exitosa en 5xx) y bajo demanda vía
//! `POST /v1/transactions/reconcile`.
//!
//! ## Manual
//! Conciliar un par a mano exige importes exactamente opuestos y misma divisa (el par debe seguir
//! sumando cero: conciliar nunca altera el neto del hogar) pero NO la ventana de 5 días — esa es
//! la relajación útil (SEPA lento, traspaso a caballo de dos meses). Desconciliar rompe el par y
//! PERSISTE un rechazo para que el siguiente pase no lo resucite; un PATCH que cambia
//! `amount`/`op_date` rompe el par vía `unlink_pair_no_rejection` (no es un rechazo del usuario).

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    ReconcilePairBody, ReconcilePairResponse, ReconcileRunResponse,
};
use crate::handlers::transactions::{crud, invalidate_projection_if_savings_uses_transactions};
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use sqlx::{FromRow, PgConnection, PgPool};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

/// Ventana del pase automático: |Δop_date| máxima entre las dos patas, en días.
pub const AUTO_MATCH_WINDOW_DAYS: i32 = 5;

// ---------------------------------------------------------------------------
// Pase automático
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ReconcileOutcome {
    pub pairs_created: u32,
    pub transactions_reconciled: u32,
}

/// Candidatos a par sobre TODO el dataset del owner, con la pata de salida (`a.amount < 0`)
/// primero — cada par aparece una única vez. `FOR UPDATE OF a, b` bloquea las filas devueltas y
/// re-evalúa el predicado bajo concurrencia (READ COMMITTED): todo lo que llega al greedy sigue
/// sin conciliar y es nuestro hasta el commit.
fn candidates_sql() -> String {
    let w = AUTO_MATCH_WINDOW_DAYS;
    format!(
        r#"SELECT a.id AS out_id, b.id AS in_id
    FROM transactions a
    JOIN transactions b
      ON b.installation_id = a.installation_id
     AND b.owner_user_id  = a.owner_user_id
     AND b.currency = a.currency
     AND b.amount = -a.amount
     AND b.transfer_counterpart_id IS NULL
     AND b.op_date >= a.op_date - {w}
     AND b.op_date <= a.op_date + {w}
    WHERE a.installation_id = $1
      AND a.owner_user_id = $2
      AND a.amount < 0
      AND a.transfer_counterpart_id IS NULL
      AND NOT EXISTS (
            SELECT 1 FROM transfer_match_rejections r
            WHERE r.transaction_a_id = LEAST(a.id, b.id)
              AND r.transaction_b_id = GREATEST(a.id, b.id)
      )
    ORDER BY abs(b.op_date - a.op_date) ASC, a.op_date ASC, b.op_date ASC, a.id ASC, b.id ASC
    FOR UPDATE OF a, b"#
    )
}

#[derive(Debug, FromRow)]
struct CandidateRow {
    out_id: Uuid,
    in_id: Uuid,
}

/// Pase de auto-conciliación sobre todo el dataset del owner. Idempotente por construcción
/// (punto fijo): las patas ya conciliadas no son candidatas, así que un segundo pase inmediato
/// devuelve `pairs_created = 0`. NO invalida la cache de proyección — eso es del caller (las
/// mutaciones invalidan una sola vez, después del pase).
pub(crate) async fn auto_reconcile_owner(
    pool: &PgPool,
    iid: Uuid,
    owner: Uuid,
) -> Result<ReconcileOutcome, ApiError> {
    let mut tx = pool.begin().await?;
    let candidates: Vec<CandidateRow> = sqlx::query_as(&candidates_sql())
        .bind(iid)
        .bind(owner)
        .fetch_all(&mut *tx)
        .await?;

    // Greedy sobre la lista ya ordenada (orden total → determinista): la primera aparición de
    // cada pata gana; el resto de candidatos que la involucren se descartan en este pase.
    let mut used: HashSet<Uuid> = HashSet::new();
    let mut ids: Vec<Uuid> = Vec::new();
    let mut counterparts: Vec<Uuid> = Vec::new();
    for c in candidates {
        if used.contains(&c.out_id) || used.contains(&c.in_id) {
            continue;
        }
        used.insert(c.out_id);
        used.insert(c.in_id);
        ids.push(c.out_id);
        counterparts.push(c.in_id);
        ids.push(c.in_id);
        counterparts.push(c.out_id);
    }
    if ids.is_empty() {
        tx.rollback().await?;
        return Ok(ReconcileOutcome {
            pairs_created: 0,
            transactions_reconciled: 0,
        });
    }

    // Un solo UPDATE escribe las dos patas de todos los pares → la simetría no puede quedar a
    // medias. El guard `transfer_counterpart_id IS NULL` es defensivo: con los locks del SELECT
    // no puede fallar, y si aun así faltara una fila, se aborta entero antes que dejar un enlace
    // asimétrico.
    let res = sqlx::query(
        r#"UPDATE transactions t
           SET transfer_counterpart_id = v.counterpart_id,
               transfer_reconciled_at = now(),
               transfer_reconciled_source = 'auto',
               updated_at = now()
           FROM (SELECT unnest($3::uuid[]) AS id, unnest($4::uuid[]) AS counterpart_id) v
           WHERE t.id = v.id
             AND t.installation_id = $1
             AND t.owner_user_id = $2
             AND t.transfer_counterpart_id IS NULL"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(&ids)
    .bind(&counterparts)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() != ids.len() as u64 {
        tx.rollback().await?;
        return Err(ApiError::Db(sqlx::Error::Protocol(
            "transfer reconcile pass lost a locked row mid-transaction".into(),
        )));
    }
    tx.commit().await?;

    let pairs = (ids.len() / 2) as u32;
    Ok(ReconcileOutcome {
        pairs_created: pairs,
        transactions_reconciled: pairs * 2,
    })
}

/// Helper post-commit para las mutaciones del conjunto (create/batch/patch/delete/import/
/// materialize): corre el pase y TRAGA errores — la mutación ya está persistida y un reintento
/// del cliente la duplicaría (mismo contrato best-effort que la invalidación de cache). Devuelve
/// `pairs_created` (0 si el pase falló). Llamar SIEMPRE antes de
/// `invalidate_projection_if_savings_uses_transactions`, para que una sola invalidación cubra
/// mutación + pase.
pub(crate) async fn auto_reconcile_after_mutation(state: &Arc<AppState>, iid: Uuid, owner: Uuid) -> u32 {
    match auto_reconcile_owner(&state.pool, iid, owner).await {
        Ok(o) => {
            if o.pairs_created > 0 {
                tracing::info!(pairs = o.pairs_created, "transfer auto-reconcile pass linked pairs");
            }
            o.pairs_created
        }
        Err(e) => {
            tracing::warn!(error = ?e, "post-commit transfer auto-reconcile pass skipped");
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Rotura de par sin rechazo (para PATCH de amount/op_date)
// ---------------------------------------------------------------------------

/// Rompe el par del movimiento `id` (ambas patas) SIN registrar rechazo: lo usa el PATCH cuando
/// cambian `amount`/`op_date` — el par dejó de sumar cero, pero el usuario no lo ha rechazado
/// (volver al importe original re-empareja en el siguiente pase). No-op si no está conciliado.
pub(crate) async fn unlink_pair_no_rejection(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"UPDATE transactions
           SET transfer_counterpart_id = NULL,
               transfer_reconciled_at = NULL,
               transfer_reconciled_source = NULL,
               updated_at = now()
           WHERE installation_id = $1 AND owner_user_id = $2
             AND (id = $3 OR transfer_counterpart_id = $3)"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cores manuales (compartidos por HTTP y MCP)
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct PairLeg {
    id: Uuid,
    amount: Decimal,
    currency: String,
    transfer_counterpart_id: Option<Uuid>,
}

async fn load_leg(
    pool: &PgPool,
    iid: Uuid,
    owner: Uuid,
    id: Uuid,
) -> Result<PairLeg, ApiError> {
    let leg: Option<PairLeg> = sqlx::query_as(
        r#"SELECT id, amount, currency, transfer_counterpart_id
           FROM transactions
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(owner)
    .fetch_optional(pool)
    .await?;
    leg.ok_or(ApiError::NotFound)
}

async fn load_pair_response(
    pool: &PgPool,
    a: Uuid,
    b: Uuid,
) -> Result<ReconcilePairResponse, ApiError> {
    Ok(ReconcilePairResponse {
        transaction: crud::load_txn(pool, a).await?,
        counterpart: crud::load_txn(pool, b).await?,
    })
}

/// Conciliación MANUAL de un par concreto: mismo owner (guard → 404), importes exactamente
/// opuestos, misma divisa; SIN ventana de fecha. Borra un rechazo previo del mismo par (conciliar
/// a mano revierte un «no» anterior). Idempotente si el par ya está conciliado entre sí.
/// Invalidación COND de la cache post-commit dentro.
pub(crate) async fn reconcile_pair_core(
    state: &Arc<AppState>,
    iid: Uuid,
    owner: Uuid,
    id: Uuid,
    counterpart_id: Uuid,
) -> Result<ReconcilePairResponse, ApiError> {
    if id == counterpart_id {
        return Err(ApiError::BadRequest(
            "transfer_same_transaction: a transaction cannot be its own counterpart".into(),
        ));
    }
    let a = load_leg(&state.pool, iid, owner, id).await?;
    let b = load_leg(&state.pool, iid, owner, counterpart_id).await?;

    // Idempotencia: el par ya está conciliado entre sí → devolverlo tal cual.
    if a.transfer_counterpart_id == Some(b.id) {
        return load_pair_response(&state.pool, a.id, b.id).await;
    }
    if a.transfer_counterpart_id.is_some() || b.transfer_counterpart_id.is_some() {
        return Err(ApiError::BadRequest(
            "already_reconciled: one of the legs already has a counterpart (unreconcile it first)"
                .into(),
        ));
    }
    if a.currency != b.currency {
        return Err(ApiError::BadRequest(
            "transfer_currency_mismatch: both legs must share the same currency".into(),
        ));
    }
    if a.amount != -b.amount {
        return Err(ApiError::BadRequest(
            "transfer_amounts_not_opposite: the pair must net to exactly zero".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    // Conciliar a mano revierte un rechazo previo del mismo par.
    sqlx::query(
        r#"DELETE FROM transfer_match_rejections
           WHERE transaction_a_id = LEAST($1::uuid, $2::uuid)
             AND transaction_b_id = GREATEST($1::uuid, $2::uuid)"#,
    )
    .bind(a.id)
    .bind(b.id)
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query(
        r#"UPDATE transactions t
           SET transfer_counterpart_id = v.counterpart_id,
               transfer_reconciled_at = now(),
               transfer_reconciled_source = 'manual',
               updated_at = now()
           FROM (VALUES ($3::uuid, $4::uuid), ($4::uuid, $3::uuid)) AS v (id, counterpart_id)
           WHERE t.id = v.id
             AND t.installation_id = $1
             AND t.owner_user_id = $2
             AND t.transfer_counterpart_id IS NULL"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(a.id)
    .bind(b.id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() != 2 {
        // Carrera: alguna pata se concilió entre el SELECT y el UPDATE.
        tx.rollback().await?;
        return Err(ApiError::BadRequest(
            "already_reconciled: one of the legs already has a counterpart (unreconcile it first)"
                .into(),
        ));
    }
    tx.commit().await?;

    invalidate_projection_if_savings_uses_transactions(state, iid, owner).await;
    load_pair_response(&state.pool, a.id, b.id).await
}

/// Desconciliación MANUAL: rompe el par del movimiento `id` y PERSISTE el rechazo para que el
/// siguiente pase automático no lo resucite. Owner-guard → 404; sin contrapartida → 400
/// `not_reconciled`. Invalidación COND post-commit dentro. Devuelve ambas patas ya sueltas.
pub(crate) async fn unreconcile_core(
    state: &Arc<AppState>,
    iid: Uuid,
    owner: Uuid,
    id: Uuid,
) -> Result<ReconcilePairResponse, ApiError> {
    let leg = load_leg(&state.pool, iid, owner, id).await?;
    let Some(counterpart_id) = leg.transfer_counterpart_id else {
        return Err(ApiError::BadRequest(
            "not_reconciled: this transaction has no counterpart".into(),
        ));
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO transfer_match_rejections
               (installation_id, owner_user_id, transaction_a_id, transaction_b_id)
           VALUES ($1, $2, LEAST($3::uuid, $4::uuid), GREATEST($3::uuid, $4::uuid))
           ON CONFLICT (transaction_a_id, transaction_b_id) DO NOTHING"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(id)
    .bind(counterpart_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE transactions
           SET transfer_counterpart_id = NULL,
               transfer_reconciled_at = NULL,
               transfer_reconciled_source = NULL,
               updated_at = now()
           WHERE installation_id = $1 AND owner_user_id = $2 AND id IN ($3, $4)"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(id)
    .bind(counterpart_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    invalidate_projection_if_savings_uses_transactions(state, iid, owner).await;
    load_pair_response(&state.pool, id, counterpart_id).await
}

/// Core del pase explícito (`POST /v1/transactions/reconcile` + tool MCP `reconcile_transfers`):
/// a diferencia del helper post-mutación, aquí el pase ES la acción → los errores se propagan.
/// Invalida la cache solo si enlazó algo (COND, modos B/C).
pub(crate) async fn reconcile_now_core(
    state: &Arc<AppState>,
    iid: Uuid,
    owner: Uuid,
) -> Result<ReconcileRunResponse, ApiError> {
    let outcome = auto_reconcile_owner(&state.pool, iid, owner).await?;
    if outcome.pairs_created > 0 {
        invalidate_projection_if_savings_uses_transactions(state, iid, owner).await;
    }
    Ok(ReconcileRunResponse {
        pairs_created: outcome.pairs_created,
        transactions_reconciled: outcome.transactions_reconciled,
    })
}

// ---------------------------------------------------------------------------
// Handlers HTTP
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions/reconcile",
    tag = "transactions",
    responses(
        (status = 200, description = "Pase de auto-conciliación ejecutado (idempotente: repetirlo devuelve 0)", body = ReconcileRunResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn reconcile_now(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<ReconcileRunResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = reconcile_now_core(&state, iid, user.id.0).await?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/transactions/{id}/reconcile",
    tag = "transactions",
    request_body = ReconcilePairBody,
    params(("id" = Uuid, Path, description = "Una pata del par")),
    responses(
        (status = 200, description = "Par conciliado (ambas patas)", body = ReconcilePairResponse),
        (status = 400, description = "`already_reconciled` | `transfer_amounts_not_opposite` | `transfer_currency_mismatch` | `transfer_same_transaction`"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Movimiento inexistente o de otro usuario"),
    )
)]
pub async fn reconcile_pair(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<ReconcilePairBody>,
) -> Result<Json<ReconcilePairResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = reconcile_pair_core(&state, iid, user.id.0, id, body.counterpart_id).await?;
    Ok(Json(resp))
}

#[utoipa::path(
    delete,
    path = "/v1/transactions/{id}/reconcile",
    tag = "transactions",
    params(("id" = Uuid, Path, description = "Una pata del par")),
    responses(
        (status = 200, description = "Par desconciliado (rechazo persistido: el pase automático no lo re-empareja)", body = ReconcilePairResponse),
        (status = 400, description = "`not_reconciled`"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Movimiento inexistente o de otro usuario"),
    )
)]
pub async fn unreconcile_transaction(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<Json<ReconcilePairResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = unreconcile_core(&state, iid, user.id.0, id).await?;
    Ok(Json(resp))
}
