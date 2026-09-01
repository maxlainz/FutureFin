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
//! fallo se loguea y NO convierte la mutación exitosa en 5xx), en el **barrido periódico**
//! (`sweep_all_owners`, ver abajo) y bajo demanda vía `POST /v1/transactions/reconcile`.
//!
//! ## El barrido periódico (3.8.1)
//! Los pases post-mutación son best-effort **por diseño**: si fallan, escriben un `warn` y la
//! mutación sigue siendo un 2xx. El precio es que ese par se queda sin conciliar **para siempre**,
//! porque nada lo reintenta — y el usuario no tiene forma de enterarse, así que tampoco va a pedir
//! el pase manual. `sweep_all_owners` es ese reintento: recorre cada `(installation, owner)` con
//! movimientos sin conciliar y vuelve a pasar el mismo algoritmo. En una instalación sana no
//! encuentra nada (el pase es de punto fijo), y eso es exactamente lo que se espera de él.
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
use std::collections::{HashMap, HashSet};
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

/// EL predicado de candidatura, en un único sitio: mismo owner, misma divisa, importes
/// exactamente opuestos, ambas patas de clase `income`/`expense`, ninguna pata conciliada,
/// par no rechazado y `|Δop_date| ≤ window_days`.
///
/// Lo comparten el pase de escritura (`auto_reconcile_owner`) y la lectura de sugerencias
/// (`suggest_transfer_matches_core`). Es la razón de que exista esta función: dos copias del
/// predicado divergirían, y entonces la lista de sugerencias propondría pares que el pase no
/// haría —o callaría los que sí— sin que nada fallara.
///
/// Las filas `savings` (y las sin clase) NO son candidatas: una pata savings de importe
/// exactamente opuesto a un gasto real dentro de la ventana lo emparejaría y excluiría ese
/// gasto de los agregados — con el patrón «el espacio de ahorro reembolsa una compra
/// concreta» (importes idénticos por construcción) el choque es sistemático, y a diferencia
/// de un par income/expense el neto por bucket NO se conserva. El emparejamiento manual
/// (`reconcile_pair_core`) sigue siendo kind-agnóstico a propósito: ahí decide el usuario.
fn candidates_from_where(window_days: i32) -> String {
    let w = window_days;
    format!(
        r#"
    FROM transactions a
    JOIN transactions b
      ON b.installation_id = a.installation_id
     AND b.owner_user_id  = a.owner_user_id
     AND b.currency = a.currency
     AND b.amount = -a.amount
     AND b.kind IN ('income','expense')
     AND b.transfer_counterpart_id IS NULL
     AND b.op_date >= a.op_date - {w}
     AND b.op_date <= a.op_date + {w}
    WHERE a.installation_id = $1
      AND a.owner_user_id = $2
      AND a.amount < 0
      AND a.kind IN ('income','expense')
      AND a.transfer_counterpart_id IS NULL
      AND NOT EXISTS (
            SELECT 1 FROM transfer_match_rejections r
            WHERE r.transaction_a_id = LEAST(a.id, b.id)
              AND r.transaction_b_id = GREATEST(a.id, b.id)
      )"#
    )
}

/// Orden TOTAL de los candidatos → el greedy es función del contenido de la BD, no del plan de
/// Postgres ni del reloj. También compartido por lectura y escritura: si el orden divergiera, la
/// sugerencia y el pase elegirían pares distintos entre los mismos candidatos.
const CANDIDATES_ORDER: &str =
    " ORDER BY abs(b.op_date - a.op_date) ASC, a.op_date ASC, b.op_date ASC, a.id ASC, b.id ASC";

/// Candidatos a par sobre TODO el dataset del owner, con la pata de salida (`a.amount < 0`)
/// primero — cada par aparece una única vez. `FOR UPDATE OF a, b` bloquea las filas devueltas y
/// re-evalúa el predicado bajo concurrencia (READ COMMITTED): todo lo que llega al greedy sigue
/// sin conciliar y es nuestro hasta el commit.
fn candidates_sql() -> String {
    format!(
        "SELECT a.id AS out_id, b.id AS in_id{}{CANDIDATES_ORDER}\n    FOR UPDATE OF a, b",
        candidates_from_where(AUTO_MATCH_WINDOW_DAYS)
    )
}

#[derive(Debug, FromRow)]
struct CandidateRow {
    out_id: Uuid,
    in_id: Uuid,
}

/// Desambiguación greedy sobre la lista YA ordenada: la primera aparición de cada pata gana y el
/// resto de candidatos que la involucren se descartan en este pase. Función pura y compartida por
/// el pase de escritura y la lectura de sugerencias — lo que se propone es exactamente lo que se
/// haría.
fn greedy_pairs(candidates: &[CandidateRow]) -> Vec<(Uuid, Uuid)> {
    let mut used: HashSet<Uuid> = HashSet::new();
    let mut out = Vec::new();
    for c in candidates {
        if used.contains(&c.out_id) || used.contains(&c.in_id) {
            continue;
        }
        used.insert(c.out_id);
        used.insert(c.in_id);
        out.push((c.out_id, c.in_id));
    }
    out
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

    // Greedy compartido con la lectura de sugerencias (orden total → determinista).
    let mut ids: Vec<Uuid> = Vec::new();
    let mut counterparts: Vec<Uuid> = Vec::new();
    for (out_id, in_id) in greedy_pairs(&candidates) {
        ids.push(out_id);
        counterparts.push(in_id);
        ids.push(in_id);
        counterparts.push(out_id);
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
/// Resultado de un barrido completo.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    /// `(installation, owner)` con movimientos sin conciliar que se han revisado.
    pub owners_scanned: u32,
    pub pairs_created: u32,
    /// Owners cuyo pase falló. No abortan el barrido: se registran y se reintentarán al siguiente.
    pub owners_failed: u32,
}

/// Reintenta la conciliación de **todos** los owners con movimientos sin conciliar.
///
/// Existe porque los pases post-mutación se tragan sus errores (ver el doc del módulo): sin este
/// barrido, un fallo puntual deja un par sin conciliar de forma permanente y silenciosa.
///
/// **Un owner que falla no aborta el barrido.** Cada `(installation, owner)` es independiente, así
/// que un error en uno —un lock, una fila que desaparece a mitad— no debe impedir que los demás se
/// concilien; se cuenta en `owners_failed` y se reintenta en la pasada siguiente.
///
/// Solo mira owners con al menos un movimiento **sin conciliar**: en una instalación al día la
/// consulta no devuelve nada y el barrido no toca la base.
///
/// Toma `AppState` y no un `PgPool` porque conciliar es una mutación de inputs del engine en los
/// modos B/C: cada owner cuyo pase crea pares invalida la cache de proyección (D12a), igual que
/// lo hace el camino HTTP.
pub async fn sweep_all_owners(state: &Arc<AppState>) -> Result<SweepOutcome, ApiError> {
    #[derive(FromRow)]
    struct OwnerRow {
        installation_id: Uuid,
        owner_user_id: Uuid,
    }
    let owners: Vec<OwnerRow> = sqlx::query_as(
        r#"SELECT DISTINCT installation_id, owner_user_id
           FROM transactions
           WHERE transfer_counterpart_id IS NULL"#,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut out = SweepOutcome {
        owners_scanned: owners.len() as u32,
        ..Default::default()
    };
    for o in owners {
        match auto_reconcile_owner(&state.pool, o.installation_id, o.owner_user_id).await {
            Ok(r) => {
                out.pairs_created += r.pairs_created;
                // D12a: conciliar cambia QUÉ cuenta en el promedio 12m (las patas conciliadas
                // salen del numerador y del denominador), así que en los modos que usan
                // transacciones el barrido es una mutación de inputs del engine como cualquier
                // otra y DEBE invalidar. Sin esto, un par recuperado aquí dejaba la proyección
                // cacheada obsoleta: el TTL es deslizante (D7), así que un usuario que la mire
                // una vez por hora la mantiene viva indefinidamente.
                //
                // Solo si el pase creó pares: el caso normal en una instalación sana es 0, y
                // desalojar una cache caliente cada 24 h a cambio de nada sería peor que el bug.
                // El gating por `savings_source` vive dentro del helper — en modo A no invalida.
                if r.pairs_created > 0 {
                    invalidate_projection_if_savings_uses_transactions(
                        state,
                        o.installation_id,
                        o.owner_user_id,
                    )
                    .await;
                }
            }
            Err(e) => {
                out.owners_failed += 1;
                tracing::warn!(
                    installation_id = %o.installation_id,
                    owner_user_id = %o.owner_user_id,
                    error = ?e,
                    "periodic reconcile sweep failed for owner; will retry next run"
                );
            }
        }
    }
    Ok(out)
}

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

    // Conciliar saca un movimiento del conjunto real → puede DESACTIVAR su mes.
    crate::handlers::transactions::recurring::converge_recurring_after_mutation(state, iid).await;
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

    // Desconciliar devuelve un movimiento al conjunto real → puede ACTIVAR su mes.
    crate::handlers::transactions::recurring::converge_recurring_after_mutation(state, iid).await;
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
        crate::handlers::transactions::recurring::converge_recurring_after_mutation(state, iid).await;
        invalidate_projection_if_savings_uses_transactions(state, iid, owner).await;
    }
    Ok(ReconcileRunResponse {
        pairs_created: outcome.pairs_created,
        transactions_reconciled: outcome.transactions_reconciled,
    })
}

// ---------------------------------------------------------------------------
// Sugerencias de par (LECTURA) y confirmación de una sugerencia
// ---------------------------------------------------------------------------

/// Ventana por defecto de la lista de sugerencias, en días.
///
/// Deliberadamente MÁS ANCHA que la del pase automático (5 días). El pase es de punto fijo, así
/// que en una instalación sana **no queda ni un par dentro de sus 5 días**: una lista de
/// sugerencias acotada a esa ventana saldría casi siempre vacía y no serviría para nada. El valor
/// de esta ruta son justamente los pares que el pase NO puede hacer solo —SEPA lento, traspaso a
/// caballo de dos meses—, que son los que necesitan que alguien mire.
pub const DEFAULT_SUGGEST_WINDOW_DAYS: i32 = 30;
/// Tope de la ventana pedible. También es el universo sobre el que se resuelve un `match_id`.
pub const MAX_SUGGEST_WINDOW_DAYS: i32 = 365;
const DEFAULT_SUGGEST_LIMIT: i64 = 20;
const MAX_SUGGEST_LIMIT: i64 = 100;

#[derive(Debug, FromRow)]
struct SuggestRow {
    out_id: Uuid,
    in_id: Uuid,
    out_op_date: chrono::NaiveDate,
    in_op_date: chrono::NaiveDate,
    out_concept: String,
    in_concept: String,
    out_amount: Decimal,
    in_amount: Decimal,
    out_kind: Option<String>,
    in_kind: Option<String>,
    currency: String,
    day_gap: i32,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TransferMatchLeg {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `YYYY-MM-DD`.
    pub op_date: String,
    pub concept: String,
    /// Importe con signo.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub kind: Option<String>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TransferMatchSuggestion {
    /// Identificador **opaco y derivado** del par. Es lo único que hace falta para conciliarlo
    /// (`POST /v1/transactions/transfer-matches/{match_id}`): no se puede fabricar para dos ids
    /// cualesquiera, porque al confirmarlo el servidor solo lo busca entre SUS candidatos.
    pub match_id: String,
    /// Pata de salida (importe negativo).
    pub outgoing: TransferMatchLeg,
    /// Pata de entrada (importe positivo, exactamente opuesto).
    pub incoming: TransferMatchLeg,
    /// Magnitud del traspaso (siempre ≥ 0).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub currency: String,
    /// Días entre las dos fechas de operación.
    pub day_gap: i64,
    /// `true` ⇒ está dentro de la ventana del pase automático. En una instalación al día esto es
    /// raro: el pase ya lo habría conciliado solo.
    pub within_auto_window: bool,
    /// `true` ⇒ alguna de las dos patas casaba también con OTRO movimiento. La propuesta sigue
    /// siendo la que el pase automático elegiría (mismo orden total, mismo greedy), pero conviene
    /// mirarla antes de confirmar.
    pub ambiguous: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TransferMatchSuggestionsResponse {
    /// Ventana aplicada, en días.
    pub window_days: i64,
    /// Ventana del pase automático, para interpretar `within_auto_window`.
    pub auto_window_days: i64,
    pub limit: i64,
    /// Propuestas devueltas (tras el greedy y el `limit`).
    pub suggestion_count: i64,
    /// Pares candidatos ANTES del greedy. Mayor que `suggestion_count` ⇒ había ambigüedad.
    pub candidate_pair_count: i64,
    /// `true` ⇒ `limit` ha recortado la lista de propuestas.
    pub truncated: bool,
    /// Pares que cumplirían el criterio pero están **rechazados** (alguien los desconcilió a
    /// mano). No se proponen; se cuentan para poder explicar por qué falta un par «obvio».
    pub rejected_pairs_excluded: i64,
    pub suggestions: Vec<TransferMatchSuggestion>,
}

/// Identificador derivado y estable de un par. Determinista sobre `(instalación, owner, ids
/// ordenados)`, así que el mismo par produce el mismo `match_id` en dos peticiones seguidas y
/// **nadie puede construir uno para un par que el servidor no considere candidato** (al
/// confirmarlo, la única forma de resolverlo es re-encontrar el par entre los candidatos vivos).
fn match_id_of(iid: Uuid, owner: Uuid, a: Uuid, b: Uuid) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let seed = format!("ffm1|{iid}|{owner}|{lo}|{hi}");
    crate::auth::secret::sha256_hex(seed.as_bytes())[..24].to_string()
}

/// Candidatos crudos (pre-greedy) del owner dentro de `window_days`, con los datos para pintarlos.
/// Usa EL MISMO predicado y EL MISMO orden que el pase de escritura — sin `FOR UPDATE`, porque
/// esto es una lectura y una lectura no bloquea filas (ni las muta: D5).
async fn suggest_candidates(
    pool: &PgPool,
    iid: Uuid,
    owner: Uuid,
    window_days: i32,
) -> Result<Vec<SuggestRow>, ApiError> {
    let sql = format!(
        "SELECT a.id AS out_id, b.id AS in_id,
                a.op_date AS out_op_date, b.op_date AS in_op_date,
                a.concept AS out_concept, b.concept AS in_concept,
                a.amount AS out_amount, b.amount AS in_amount,
                a.kind AS out_kind, b.kind AS in_kind,
                a.currency AS currency,
                abs(b.op_date - a.op_date)::int AS day_gap{}{CANDIDATES_ORDER}",
        candidates_from_where(window_days)
    );
    let rows: Vec<SuggestRow> = sqlx::query_as(&sql)
        .bind(iid)
        .bind(owner)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Core de LECTURA de las sugerencias de conciliación (`GET /v1/transactions/transfer-matches` +
/// tool MCP `suggest_transfer_matches`).
///
/// **Es un GET y no muta nada.** Hasta 4.4.0 la única forma de ver un par candidato era
/// **escribir** (`POST /v1/transactions/reconcile` ejecutaba el pase), y la tentación evidente
/// —un `?dry_run` sobre ese POST— está descartada de antemano: un GET que muta ya costó caro en
/// este repositorio (los GET que borraban pasivos vencidos) y tiene su propia entrada en la
/// arqueología. Rutas distintas, verbos distintos.
///
/// Es **own-user** (como el resto de la conciliación: `reconcile_pair_core` exige que las dos
/// patas sean del usuario), así que no acepta `view` ni publica ninguno.
///
/// Cache: NONE.
pub(crate) async fn suggest_transfer_matches_core(
    pool: &PgPool,
    iid: Uuid,
    owner: Uuid,
    window_days: Option<i32>,
    limit: Option<i64>,
) -> Result<TransferMatchSuggestionsResponse, ApiError> {
    let window_days = window_days.unwrap_or(DEFAULT_SUGGEST_WINDOW_DAYS);
    if !(1..=MAX_SUGGEST_WINDOW_DAYS).contains(&window_days) {
        return Err(ApiError::BadRequest(format!(
            "window_days_out_of_range: window_days must be between 1 and {MAX_SUGGEST_WINDOW_DAYS}"
        )));
    }
    let limit = limit.unwrap_or(DEFAULT_SUGGEST_LIMIT);
    if !(1..=MAX_SUGGEST_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit_out_of_range: limit must be between 1 and {MAX_SUGGEST_LIMIT}"
        )));
    }

    let rows = suggest_candidates(pool, iid, owner, window_days).await?;
    // Ambigüedad: una pata que aparece en más de un candidato. Se calcula ANTES del greedy, que
    // es justo lo que la esconde.
    let mut leg_hits: HashMap<Uuid, u32> = HashMap::new();
    for r in &rows {
        *leg_hits.entry(r.out_id).or_insert(0) += 1;
        *leg_hits.entry(r.in_id).or_insert(0) += 1;
    }
    let candidate_pair_count = rows.len() as i64;

    let flat: Vec<CandidateRow> = rows
        .iter()
        .map(|r| CandidateRow {
            out_id: r.out_id,
            in_id: r.in_id,
        })
        .collect();
    let chosen = greedy_pairs(&flat);
    let suggestion_count_total = chosen.len() as i64;

    let by_pair: HashMap<(Uuid, Uuid), &SuggestRow> =
        rows.iter().map(|r| ((r.out_id, r.in_id), r)).collect();
    let suggestions: Vec<TransferMatchSuggestion> = chosen
        .iter()
        .take(limit as usize)
        .filter_map(|pair| by_pair.get(pair).copied())
        .map(|r| TransferMatchSuggestion {
            match_id: match_id_of(iid, owner, r.out_id, r.in_id),
            outgoing: TransferMatchLeg {
                id: r.out_id,
                op_date: r.out_op_date.format("%Y-%m-%d").to_string(),
                concept: r.out_concept.clone(),
                amount: crate::money::money_out(r.out_amount),
                kind: r.out_kind.clone(),
            },
            incoming: TransferMatchLeg {
                id: r.in_id,
                op_date: r.in_op_date.format("%Y-%m-%d").to_string(),
                concept: r.in_concept.clone(),
                amount: crate::money::money_out(r.in_amount),
                kind: r.in_kind.clone(),
            },
            amount: crate::money::money_out(r.in_amount.abs()),
            currency: r.currency.clone(),
            day_gap: r.day_gap as i64,
            within_auto_window: r.day_gap <= AUTO_MATCH_WINDOW_DAYS,
            ambiguous: leg_hits.get(&r.out_id).copied().unwrap_or(0) > 1
                || leg_hits.get(&r.in_id).copied().unwrap_or(0) > 1,
        })
        .collect();

    // Pares que el criterio encontraría pero que alguien rechazó al desconciliar. Se cuentan para
    // que «¿por qué no me propone este par obvio?» tenga respuesta sin leer la BD a mano.
    let rejected_pairs_excluded: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint
           FROM transfer_match_rejections r
           JOIN transactions a ON a.id = r.transaction_a_id
           JOIN transactions b ON b.id = r.transaction_b_id
           WHERE r.installation_id = $1
             AND r.owner_user_id = $2
             AND a.transfer_counterpart_id IS NULL
             AND b.transfer_counterpart_id IS NULL
             AND a.currency = b.currency
             AND a.amount = -b.amount
             AND abs(b.op_date - a.op_date) <= $3"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(window_days)
    .fetch_one(pool)
    .await?;

    Ok(TransferMatchSuggestionsResponse {
        window_days: window_days as i64,
        auto_window_days: AUTO_MATCH_WINDOW_DAYS as i64,
        limit,
        suggestion_count: suggestions.len() as i64,
        candidate_pair_count,
        truncated: suggestion_count_total > suggestions.len() as i64,
        rejected_pairs_excluded,
        suggestions,
    })
}

/// Confirma una sugerencia por su `match_id` (`POST /v1/transactions/transfer-matches/{match_id}`
/// + tool MCP `confirm_transfer_match`).
///
/// ## Por qué esta forma y no dos UUID
/// El registro de omisiones del MCP dejó `reconcile_pair` fuera del catálogo por un motivo que
/// sigue vigente: emparejar **dos UUID elegidos a dedo** entre cientos es un footgun donde un
/// error saca en silencio los dos movimientos de todos los agregados de flujo (y, en los modos B/C,
/// mueve el ahorro que alimenta la proyección). Un `confirm: true` no arregla eso: el modelo
/// confirmaría con la misma confianza con la que se equivocó.
///
/// Aquí el argumento **no es un par de ids: es el identificador de una propuesta del servidor**.
/// El `match_id` se deriva de `(instalación, owner, ids ordenados)` y solo se resuelve
/// re-buscándolo entre los candidatos vivos, así que el espacio de acciones alcanzables es
/// exactamente el de los pares que el servidor propondría — un par arbitrario **no es
/// expresable**. Y si el par dejó de ser candidato entre la sugerencia y la confirmación (una
/// pata borrada, un importe editado, el pase automático adelantándose), el `match_id` no resuelve
/// y sale un 404 `transfer_match_not_found` en vez de conciliar algo que ya no es lo mirado.
///
/// La validación de fondo (mismo owner, importes opuestos, misma divisa, idempotencia si ya están
/// casados entre sí) y la invalidación COND siguen viviendo en `reconcile_pair_core`: esto no
/// duplica nada, solo cambia CÓMO se nombra el par.
pub(crate) async fn confirm_transfer_match_core(
    state: &Arc<AppState>,
    iid: Uuid,
    owner: Uuid,
    match_id: &str,
) -> Result<ReconcilePairResponse, ApiError> {
    // Universo de resolución: la ventana MÁXIMA, superconjunto de cualquier ventana con la que se
    // hayan pedido las sugerencias. Sigue siendo estrictamente más estrecho que el
    // `reconcile_pair` manual, que no tiene ventana ninguna.
    let rows = suggest_candidates(&state.pool, iid, owner, MAX_SUGGEST_WINDOW_DAYS).await?;
    // Se busca sobre TODOS los candidatos, no solo sobre los ganadores del greedy: si el servidor
    // llegó a considerar el par, confirmarlo es legítimo.
    if let Some(r) = rows
        .iter()
        .find(|r| match_id_of(iid, owner, r.out_id, r.in_id) == match_id)
    {
        return reconcile_pair_core(state, iid, owner, r.out_id, r.in_id).await;
    }

    // Segundo universo: pares YA conciliados **entre sí**. Sin esto, reintentar la confirmación
    // (un timeout de red, un cliente que reenvía) daría 404 sobre un trabajo que sí se hizo, y la
    // tool MCP no podría anunciarse como idempotente. `reconcile_pair_core` ya devuelve el par tal
    // cual cuando las dos patas se apuntan mutuamente, así que aquí basta con resolver el id.
    let done: Vec<CandidateRow> = sqlx::query_as(
        r#"SELECT a.id AS out_id, b.id AS in_id
           FROM transactions a
           JOIN transactions b ON b.id = a.transfer_counterpart_id
           WHERE a.installation_id = $1
             AND a.owner_user_id = $2
             AND a.amount < 0"#,
    )
    .bind(iid)
    .bind(owner)
    .fetch_all(&state.pool)
    .await?;
    if let Some(r) = done
        .iter()
        .find(|r| match_id_of(iid, owner, r.out_id, r.in_id) == match_id)
    {
        return reconcile_pair_core(state, iid, owner, r.out_id, r.in_id).await;
    }

    Err(ApiError::NotFoundWith(
        "transfer_match_not_found: no candidate transfer pair matches this match_id any more; list the suggestions again".into(),
    ))
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

#[derive(Debug, serde::Deserialize)]
pub struct SuggestMatchesQuery {
    /// Ventana máxima entre las dos fechas, en días (1..365, default 30).
    #[serde(default)]
    pub window_days: Option<i32>,
    /// Propuestas a devolver (1..100, default 20).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/v1/transactions/transfer-matches",
    tag = "transactions",
    params(
        ("window_days" = Option<i32>, Query, description = "|Δop_date| máxima entre las dos patas (1..365, default 30). Más ancha que la del pase automático (5) a propósito: los pares de ≤5 días ya los concilia el pase solo."),
        ("limit" = Option<i64>, Query, description = "Propuestas a devolver (1..100, default 20)."),
    ),
    responses(
        (status = 200, description = "Pares candidatos a transferencia interna, propios del usuario. LECTURA pura: no concilia nada", body = TransferMatchSuggestionsResponse),
        (status = 400, description = "`window_days_out_of_range` | `limit_out_of_range`"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn suggest_transfer_matches(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    axum::extract::Query(q): axum::extract::Query<SuggestMatchesQuery>,
) -> Result<Json<TransferMatchSuggestionsResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let out =
        suggest_transfer_matches_core(&state.pool, iid, user.id.0, q.window_days, q.limit).await?;
    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/v1/transactions/transfer-matches/{match_id}",
    tag = "transactions",
    params(("match_id" = String, Path, description = "`match_id` devuelto por `GET /v1/transactions/transfer-matches`")),
    responses(
        (status = 200, description = "Par conciliado (ambas patas)", body = ReconcilePairResponse),
        (status = 400, description = "`already_reconciled` | `transfer_amounts_not_opposite` | `transfer_currency_mismatch`"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "`transfer_match_not_found`: el par ya no es candidato (pata borrada, importe editado o conciliado entre medias)"),
    )
)]
pub async fn confirm_transfer_match(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(match_id): Path<String>,
) -> Result<Json<ReconcilePairResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = confirm_transfer_match_core(&state, iid, user.id.0, &match_id).await?;
    Ok(Json(resp))
}
