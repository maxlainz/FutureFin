//! Reglas de transacción recurrente (`/v1/transactions/recurring`).
//!
//! Una regla es una **plantilla** per-user (nómina, alquiler, aportación mensual…): concepto,
//! importe firmado, kind, categoría y enlaces. Las reglas tienen resolución **MENSUAL** (sin día
//! configurable desde 3.2.0).
//!
//! # Convergencia (3.9.0) — las instancias siguen a los datos reales
//!
//! Desde 3.9.0 no hay cursor. El estado objetivo es una **invariante declarativa**:
//!
//! > una instancia de la regla R existe en el mes M ⟺ M es un mes **activo** de la instalación y
//! > `M >= R.origin_month`.
//!
//! **Mes activo** = mes civil **cerrado** con ≥1 movimiento *real* — `recurring_rule_id IS NULL` y
//! no conciliado (una pata de transferencia conciliada es dinero que nunca salió del hogar: no
//! activa nada). El mes en curso nunca se materializa, así que el mes abierto no muestra
//! movimientos sintéticos que distorsionen sus estadísticas.
//!
//! `converge_recurring_for_installation` lleva el ledger a ese estado — **crea** lo que falta y
//! **poda** lo que sobra — y corre post-commit tras cada mutación de transacciones. Consecuencias:
//!
//! * Un mes sin CSV ni altas manuales queda **genuinamente vacío**: desaparece la categoría de
//!   meses «pseudovacíos» (solo recurrentes) que el promedio del engine tenía que excluir aparte.
//! * **Idempotencia por existencia**, no por cursor: el cursor era monotónico, justo lo contrario
//!   de lo que hace falta (un CSV de marzo-2025 importado en abril-2026 dispara un mes que el
//!   cursor ya había pasado). La garantía dura la da el índice único parcial
//!   `transactions_recurring_rule_month_uniq` — el `FOR UPDATE` sobre las reglas no basta, porque
//!   bajo READ COMMITTED no re-evalúa un `NOT EXISTS` que mira `transactions`.
//! * **Borrar una instancia a mano ya no la borra para siempre**: se recrea mientras su mes siga
//!   activo. Para quitarla de verdad hay que borrar la **plantilla**. Decisión firmada del owner.
//! * La activación es de **ámbito instalación** (no por owner): en un hogar donde A importa CSV y
//!   B solo tiene una nómina recurrente, con ámbito por owner B perdería toda su nómina.
//!
//! El mes de **origen** está exento de la poda: su instancia es el movimiento con el que se dio de
//! alta la recurrencia y lleva `recurring_rule_id`, así que no cuenta como movimiento real — sin la
//! exención la convergencia borraría lo que el usuario acaba de teclear. La inserción, en cambio,
//! sí considera el mes de origen (`>= origin_month`) y se apoya en la comprobación de existencia
//! para no duplicarlo: así un mes de origen que quedó sin instancia (la limpieza histórica de la
//! migración 3.9.0 no eximía a nadie) vuelve a materializar si algún día recibe datos.
//!
//! Cache de proyección (contrato en `transactions/mod.rs`): la convergencia crea/borra instancias
//! reales → siempre corre **antes** de la invalidación, para que una sola invalidación cubra la
//! mutación y su convergencia. **Borrar una regla también invalida (COND)**, aunque no borre
//! ninguna fila: el conjunto de transacciones no cambia, pero su CLASIFICACIÓN sí. `real_txns`
//! cuenta `recurring_rule_id IS NULL`, y el mes de origen está exento de la poda, así que puede
//! existir una instancia en un mes sin ningún movimiento real — un mes que el promedio ignora.
//! Al borrar la plantilla, la FK `ON DELETE SET NULL` convierte esa instancia en movimiento
//! real, el mes entra en numerador y denominador, y el ahorro del engine cambia. Hasta 4.0.0 el
//! core recibía un `&PgPool` y era **estructuralmente incapaz** de invalidar: servía proyección
//! vieja indefinidamente mientras alguien la mirara una vez por hora (TTL deslizante).

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::crud::PreparedTxn;
use crate::handlers::transactions::schema::{
    assert_amount_sign_matches_kind, compute_fingerprint, normalize_concept_field, normalize_kind,
    normalize_notes, MaterializeResponse, PatchRecurringRuleBody, RecurringRuleResponse,
    UpdateRecurringRuleResponse, SOURCE_MANUAL,
};
use crate::handlers::transactions::reconcile::{
    auto_reconcile_after_mutation, unlink_pair_no_rejection,
};
use crate::handlers::transactions::{
    assert_asset_in_installation, assert_liability_in_installation,
    invalidate_projection_if_savings_uses_transactions, next_fingerprint_ordinal,
    resolve_category_for_kind,
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

/// Nº máximo de años que el backfill de un alta recurrente con fecha pasada puede reconstruir.
const MAX_RECURRENCE_BACKFILL_YEARS: i32 = 10;

/// Cota inferior del backfill de recurrentes: una `op_date` a más de 10 años en el pasado genera
/// ~cientos de instancias (≈2 queries/mes) en la MISMA transacción del alta. 10 años es una cota
/// generosa que no molesta a ningún uso legítimo. Fuera de cota → 422 `recurrence_too_old`.
pub(super) fn assert_recurrence_not_too_old(
    op_date: NaiveDate,
    today: NaiveDate,
) -> Result<(), ApiError> {
    let floor_year = today.year() - MAX_RECURRENCE_BACKFILL_YEARS;
    // Reconstruye el mismo día/mes 10 años atrás; si ese día no existe (29-feb en año no bisiesto),
    // cae al día 1 de ese mes (la cota es aproximada y generosa, no importa el día exacto).
    let floor = NaiveDate::from_ymd_opt(floor_year, today.month(), today.day())
        .or_else(|| NaiveDate::from_ymd_opt(floor_year, today.month(), 1))
        .expect("valid floor date");
    if op_date < floor {
        return Err(ApiError::Unprocessable(format!(
            "recurrence_too_old: recurrence op_date must be within the last {MAX_RECURRENCE_BACKFILL_YEARS} years"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inserción de la regla (usado por crud.rs en el alta con recurrencia)
// ---------------------------------------------------------------------------

/// Inserta una regla recurrente derivada de una transacción preparada y devuelve su id.
/// `origin_month` (primer día del mes de la transacción de origen) es el **ancla**: la
/// convergencia materializa de ese mes en adelante y nunca poda la instancia que vive en él.
pub(super) async fn insert_rule(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    p: &PreparedTxn,
    origin_month: NaiveDate,
) -> Result<Uuid, ApiError> {
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO recurring_transaction_rules
               (installation_id, owner_user_id, concept, amount, kind, category_id,
                linked_asset_id, linked_liability_id, notes, origin_month)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(&p.concept)
    .bind(p.amount)
    .bind(&p.kind)
    .bind(p.category_id)
    .bind(p.linked_asset_id)
    .bind(p.linked_liability_id)
    .bind(p.notes.as_deref())
    .bind(origin_month)
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
    linked_asset_id: Option<Uuid>,
    linked_liability_id: Option<Uuid>,
    notes: Option<String>,
    origin_month: NaiveDate,
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
        linked_asset_id: r.linked_asset_id,
        linked_liability_id: r.linked_liability_id,
        notes: r.notes,
        origin_month: r.origin_month.format("%Y-%m-%d").to_string(),
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const RULE_SELECT: &str = r#"SELECT r.id, r.concept, r.amount, r.kind, r.category_id,
              c.name AS category_name, r.linked_asset_id, r.linked_liability_id,
              r.notes, r.origin_month, r.created_at, r.updated_at
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
    let out = list_recurring_rules_core(&state.pool, iid, user.id.0).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_recurring_rules`. Siempre
/// own-user (el endpoint no acepta `?view` — no inventarlo en la tool).
pub(crate) async fn list_recurring_rules_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<RecurringRuleResponse>, ApiError> {
    let sql = format!(
        "{RULE_SELECT} WHERE r.installation_id = $1 AND r.owner_user_id = $2 \
         ORDER BY r.created_at DESC, r.id ASC"
    );
    let rows: Vec<RuleRow> = sqlx::query_as(&sql)
        .bind(iid)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(row_to_response).collect())
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/recurring/materialize
// ---------------------------------------------------------------------------

/// Fila mínima de una regla para el bucle de convergencia. Lleva `owner_user_id` porque la
/// convergencia es de **instalación** (la activación de un mes lo es) mientras que las reglas y
/// sus instancias son per-owner.
#[derive(Debug, FromRow)]
struct RuleCore {
    id: Uuid,
    owner_user_id: Uuid,
    concept: String,
    amount: Decimal,
    kind: String,
    category_id: Option<Uuid>,
    linked_asset_id: Option<Uuid>,
    linked_liability_id: Option<Uuid>,
    notes: Option<String>,
    origin_month: NaiveDate,
}

/// Meses **activos** de la instalación: meses civiles **cerrados** (el mes en curso queda fuera)
/// con ≥1 movimiento *real* — `recurring_rule_id IS NULL` y no conciliado. Definición ÚNICA de
/// «mes con datos» del modelo de convergencia. Orden ascendente.
async fn active_months(
    conn: &mut PgConnection,
    iid: Uuid,
    today: NaiveDate,
) -> Result<Vec<NaiveDate>, ApiError> {
    let current_month = first_of_month(today.year(), today.month());
    let months: Vec<NaiveDate> = sqlx::query_scalar(
        r#"SELECT DISTINCT date_trunc('month', op_date::timestamp)::date AS m
           FROM transactions
           WHERE installation_id = $1
             AND op_date < $2
             AND recurring_rule_id IS NULL
             AND transfer_counterpart_id IS NULL
           ORDER BY m"#,
    )
    .bind(iid)
    .bind(current_month)
    .fetch_all(&mut *conn)
    .await?;
    Ok(months)
}

/// Último día del mes al que pertenece `month_start`.
fn last_day_of_month(month_start: NaiveDate) -> NaiveDate {
    let (y, m) = (month_start.year(), month_start.month());
    NaiveDate::from_ymd_opt(y, m, days_in_month(y, m)).expect("valid last day of month")
}

/// Lleva las instancias recurrentes de la instalación al estado que definen las reglas:
/// una instancia de R existe en el mes M ⟺ M es **activo** y `M >= R.origin_month`.
/// Crea lo que falta, poda lo que sobra. Idempotente. Devuelve `(creadas, podadas)`.
///
/// Ámbito **instalación**: la activación de un mes lo es (ver el doc del módulo), así que
/// converger solo al owner que acaba de mutar dejaría sin materializar las reglas de sus
/// convivientes en un mes que ya está activo.
pub(crate) async fn converge_recurring_for_installation(
    conn: &mut PgConnection,
    iid: Uuid,
    today: NaiveDate,
) -> Result<(u32, u32), ApiError> {
    // `FOR UPDATE` serializa convergencias concurrentes de la misma instalación. NO basta por sí
    // solo: bajo READ COMMITTED no re-evalúa la comprobación de existencia de abajo (que mira
    // `transactions`, no esta tabla), así que la garantía dura la da el índice único parcial
    // `transactions_recurring_rule_month_uniq` + el `ON CONFLICT` de la inserción.
    let rules: Vec<RuleCore> = sqlx::query_as(
        r#"SELECT id, owner_user_id, concept, amount, kind, category_id,
                  linked_asset_id, linked_liability_id, notes, origin_month
           FROM recurring_transaction_rules
           WHERE installation_id = $1
           ORDER BY created_at ASC, id ASC
           FOR UPDATE"#,
    )
    .bind(iid)
    .fetch_all(&mut *conn)
    .await?;
    if rules.is_empty() {
        return Ok((0, 0));
    }

    let months = active_months(&mut *conn, iid, today).await?;

    // (1) PODA: instancias fuera de los meses activos. El mes de ORIGEN queda exento — su
    // instancia es el movimiento con el que se dio de alta la recurrencia y lleva
    // `recurring_rule_id`, así que no cuenta como movimiento real: sin la exención la
    // convergencia borraría lo que el usuario acaba de teclear.
    let pruned = sqlx::query(
        r#"DELETE FROM transactions t
           USING recurring_transaction_rules r
           WHERE t.recurring_rule_id = r.id
             AND r.installation_id = $1
             AND date_trunc('month', t.op_date::timestamp)::date <> r.origin_month
             AND NOT (date_trunc('month', t.op_date::timestamp)::date = ANY($2))"#,
    )
    .bind(iid)
    .bind(&months)
    .execute(&mut *conn)
    .await?
    .rows_affected() as u32;

    // (2) ALTA: cada mes activo desde el origen de la regla que aún no tenga instancia suya.
    // `op_date` = último día del mes (contrato 3.2.0: la instancia de M cuenta en las
    // estadísticas de M). El filtro es `>=`, no `>`: la comprobación de existencia ya evita
    // duplicar el origen, y así un mes de origen que quedó sin instancia (la limpieza histórica
    // de 3.9.0 no eximía a nadie) vuelve a materializar si algún día recibe datos.
    let mut created = 0u32;
    for rule in &rules {
        for m in months.iter().filter(|m| **m >= rule.origin_month) {
            let exists: Option<Uuid> = sqlx::query_scalar(
                r#"SELECT id FROM transactions
                   WHERE recurring_rule_id = $1
                     AND op_date >= $2
                     AND op_date < ($2::date + INTERVAL '1 month')::date"#,
            )
            .bind(rule.id)
            .bind(m)
            .fetch_optional(&mut *conn)
            .await?;
            if exists.is_some() {
                continue;
            }

            let op_date = last_day_of_month(*m);
            let fp = compute_fingerprint(SOURCE_MANUAL, op_date, rule.amount, &rule.concept);
            // Ordinal MAX+1: una instancia manual idéntica preexistente jamás produce un 409.
            let ordinal =
                next_fingerprint_ordinal(&mut *conn, iid, rule.owner_user_id, &fp).await?;

            // `ON CONFLICT` con el índice INFERIDO POR TARGET (incluida su cláusula `WHERE`),
            // nunca un `DO NOTHING` pelado: sin el target se tragaría también los conflictos de
            // `transactions_unique_fingerprint` y perdería instancias legítimas en silencio.
            let n = sqlx::query(
                r#"INSERT INTO transactions
                       (installation_id, owner_user_id, import_id, source, op_date, value_date,
                        concept, amount, currency, kind, category_id, fingerprint,
                        fingerprint_ordinal, linked_asset_id, linked_liability_id, notes,
                        recurring_rule_id)
                   VALUES ($1, $2, NULL, $3, $4, NULL, $5, $6, 'EUR', $7, $8, $9, $10, $11, $12, $13, $14)
                   ON CONFLICT (recurring_rule_id, date_trunc('month', op_date::timestamp))
                       WHERE recurring_rule_id IS NOT NULL
                   DO NOTHING"#,
            )
            .bind(iid)
            .bind(rule.owner_user_id)
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
            .execute(&mut *conn)
            .await?
            .rows_affected();
            created += n as u32;
        }
    }

    Ok((created, pruned))
}

/// Pasada de convergencia **best-effort post-commit**, calcada de
/// `auto_reconcile_after_mutation`: un fallo se loguea y NUNCA convierte una escritura ya
/// persistida en un 5xx (el cliente reintentaría y duplicaría el movimiento).
///
/// ORDEN CONTRACTUAL en todos los puntos de enganche:
/// `commit → auto_reconcile (cambia QUÉ es real) → converge (cambia el CONJUNTO) → invalidar`.
/// Converge **antes** de invalidar, o la cache se repuebla desde un conjunto pre-convergencia.
pub(crate) async fn converge_recurring_after_mutation(state: &Arc<AppState>, iid: Uuid) {
    let today = match installation_naive_today(&state.pool, iid).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = ?e, "post-commit recurring converge skipped (no calendar)");
            return;
        }
    };
    let mut conn = match state.pool.acquire().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = ?e, "post-commit recurring converge skipped (no connection)");
            return;
        }
    };
    match converge_recurring_for_installation(&mut conn, iid, today).await {
        Ok((created, pruned)) => {
            if created > 0 || pruned > 0 {
                tracing::info!(created, pruned, "recurring converge pass changed instances");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "post-commit recurring converge skipped"),
    }
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
    let resp = materialize_recurring_core(&state, iid, user.id.0).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `materialize_recurring`.
///
/// Desde 3.9.0 es una **pasada de convergencia bajo demanda**: el endpoint sobrevive como red de
/// auto-reparación (una convergencia post-commit best-effort que falló, un `.ffbackup` restaurado
/// de una versión anterior, una regla creada después de los datos), y en régimen estacionario es
/// un no-op. Idempotente **por existencia**, no por cursor.
///
/// La convergencia es de ámbito instalación, así que `rules_processed` cuenta las reglas de la
/// instalación entera, no solo las del usuario que llama.
pub(crate) async fn materialize_recurring_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
) -> Result<MaterializeResponse, ApiError> {
    let today = installation_naive_today(&state.pool, iid).await?;

    let mut tx = state.pool.begin().await?;

    let rules_processed: i64 =
        sqlx::query_scalar(r#"SELECT count(*) FROM recurring_transaction_rules WHERE installation_id = $1"#)
            .bind(iid)
            .fetch_one(&mut *tx)
            .await?;
    let (materialized, pruned) = converge_recurring_for_installation(&mut tx, iid, today).await?;

    tx.commit().await?;

    if materialized > 0 || pruned > 0 {
        auto_reconcile_after_mutation(state, iid, user_id).await;
    }
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    Ok(MaterializeResponse {
        rules_processed: rules_processed.max(0) as u32,
        materialized,
        pruned,
    })
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
    delete_recurring_rule_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_recurring_rule`. Borra
/// solo la plantilla — la FK `transactions.recurring_rule_id` es ON DELETE SET NULL, así que
/// las instancias ya materializadas sobreviven. Pero al perder el enlace pasan a contar como
/// movimientos REALES, y eso puede activar un mes que el promedio ignoraba: converge y
/// **invalida (COND)**. Ver el porqué en la cabecera del módulo.
pub(crate) async fn delete_recurring_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res = sqlx::query(
        r#"DELETE FROM recurring_transaction_rules
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
    converge_recurring_after_mutation(state, iid).await;
    invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// PATCH /v1/transactions/recurring/{id} (5.0.0)
// ---------------------------------------------------------------------------

/// Alcance de una edición de plantilla, con la semántica de una app de calendario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecurringApplyTo {
    /// Plantilla + TODAS las instancias ya materializadas (incluida la de origen).
    All,
    /// Solo la plantilla: lo ya materializado no cambia; las materializaciones futuras usan los
    /// valores nuevos.
    Future,
}

impl RecurringApplyTo {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("all") => Ok(Self::All),
            Some("future") => Ok(Self::Future),
            _ => Err(ApiError::BadRequest(
                "apply_to_invalid: apply_to is required and must be 'all' or 'future'".into(),
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Future => "future",
        }
    }
}

/// Fila de una instancia enlazada, con todo lo que el UPDATE por fila necesita recomponer.
#[derive(Debug, FromRow)]
struct InstanceRow {
    id: Uuid,
    import_id: Option<Uuid>,
    source: String,
    op_date: NaiveDate,
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
    path = "/v1/transactions/recurring/{id}",
    tag = "transactions",
    request_body = PatchRecurringRuleBody,
    params(("id" = Uuid, Path, description = "Recurring rule id")),
    responses(
        (status = 200, description = "Plantilla editada (y, con apply_to=all, sus instancias)", body = UpdateRecurringRuleResponse),
        (status = 400, description = "Validación (apply_to_invalid, patch_empty, *_set_and_clear, amount_sign_mismatch, savings_no_category, category_scope_mismatch, linked_*_not_found…)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Regla inexistente o de otro usuario"),
    )
)]
pub async fn patch_recurring_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchRecurringRuleBody>,
) -> Result<Json<UpdateRecurringRuleResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = update_recurring_rule_core(&state, iid, user.id.0, id, body, false).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: la comparten el handler PATCH y la tool MCP `update_recurring_rule`.
///
/// Edita la plantilla `id` (guardia id + instalación + owner → 404) y, con `apply_to = all`,
/// aplica **los mismos campos del patch** —solo esos— a todas sus instancias materializadas en la
/// MISMA transacción SQL. Validación idéntica a la del PATCH de transacciones (signo↔kind cuando
/// viene `amount`, `resolve_category_for_kind`, links de la instalación, `*_set_and_clear`).
///
/// **Huella de dedup de las instancias (`all`)**: se RECOMPUTA cuando cambian `amount` o
/// `concept`, con ordinal libre — la política de las filas manuales del PATCH individual
/// (`patch_transaction_core`), que es lo que son: la convergencia las inserta con
/// `compute_fingerprint(manual, op_date, rule.amount, rule.concept)`, y la huella de una fila
/// manual es una función de esos campos (el restore de `.ffbackup` también la recomputa así).
/// Dejar la vieja haría que una instancia de 900 € llevara la huella de 800 €. Una instancia
/// importada (`import_id` no nulo, solo alcanzable vía restore) conserva la suya, igual que en el
/// PATCH individual. `op_date` nunca se toca.
///
/// **Pares conciliados**: si cambia `amount`, el par de cada instancia se rompe SIN rechazo
/// (`unlink_pair_no_rejection`), como en el PATCH individual, y post-commit corren el pase de
/// conciliación y la convergencia antes de invalidar (orden contractual del módulo).
///
/// **Cache (COND)**: `all` reescribe importes/clasificación de movimientos reales → invalida en
/// los modos B/C. `future` **no invalida**: no cambia ninguna fila de `transactions` (ni su
/// conjunto, ni sus importes, ni su `recurring_rule_id`, que es lo que cambia la clasificación
/// en el borrado), y ninguna lectura del motor mira `recurring_transaction_rules`. Las
/// materializaciones futuras que usen los valores nuevos llegan con la mutación que activa su
/// mes, que invalida por su cuenta. Regresión: `transactions_projection_cache.rs`.
///
/// `dry_run = true` ejecuta TODO (validación, UPDATEs, recuento) dentro de la transacción y la
/// deshace: es lo que permite al preview MCP enseñar el nº exacto de instancias sin SQL propio.
/// Sin efectos post-commit.
pub(crate) async fn update_recurring_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchRecurringRuleBody,
    dry_run: bool,
) -> Result<UpdateRecurringRuleResponse, ApiError> {
    let apply_to = RecurringApplyTo::parse(body.apply_to.as_deref())?;

    // Mensajes COMPLETOS y literales (ver `patch_transaction_core`): `error_codes_parity.rs`
    // extrae los códigos de los literales del fuente.
    for (msg, puesto, borrado) in [
        (
            "category_set_and_clear: category_id and clear_category are mutually exclusive",
            body.category_id.is_some(),
            body.clear_category == Some(true),
        ),
        (
            "linked_asset_set_and_clear: linked_asset_id and clear_linked_asset are mutually exclusive",
            body.linked_asset_id.is_some(),
            body.clear_linked_asset == Some(true),
        ),
        (
            "linked_liability_set_and_clear: linked_liability_id and clear_linked_liability are mutually exclusive",
            body.linked_liability_id.is_some(),
            body.clear_linked_liability == Some(true),
        ),
        (
            "notes_set_and_clear: notes and clear_notes are mutually exclusive",
            body.notes.is_some(),
            body.clear_notes == Some(true),
        ),
    ] {
        if puesto && borrado {
            return Err(ApiError::BadRequest(msg.into()));
        }
    }

    let touches_concept = body.concept.is_some();
    let touches_amount = body.amount.is_some();
    let touches_classification =
        body.kind.is_some() || body.category_id.is_some() || body.clear_category == Some(true);
    let touches_asset = body.linked_asset_id.is_some() || body.clear_linked_asset == Some(true);
    let touches_liability =
        body.linked_liability_id.is_some() || body.clear_linked_liability == Some(true);
    let touches_notes = body.notes.is_some() || body.clear_notes == Some(true);
    if !(touches_concept
        || touches_amount
        || touches_classification
        || touches_asset
        || touches_liability
        || touches_notes)
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one of concept, amount, kind, category_id, clear_category, linked_asset_id, clear_linked_asset, linked_liability_id, clear_linked_liability, notes, clear_notes".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;

    // Bloquea la plantilla: una convergencia concurrente (que la lee `FOR UPDATE`) espera a que
    // esta edición termine, así que nunca materializa con valores a medio cambiar.
    let current: Option<RuleCore> = sqlx::query_as(
        r#"SELECT id, owner_user_id, concept, amount, kind, category_id,
                  linked_asset_id, linked_liability_id, notes, origin_month
           FROM recurring_transaction_rules
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3
           FOR UPDATE"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    let new_concept = match &body.concept {
        Some(c) => normalize_concept_field(c)?,
        None => current.concept.clone(),
    };
    let new_amount = body.amount.map(|a| a.round_dp(4)).unwrap_or(current.amount);
    if new_amount.is_zero() {
        return Err(ApiError::BadRequest(
            "amount_zero: amount must not be zero".into(),
        ));
    }
    let new_kind = match &body.kind {
        Some(k) => normalize_kind(k)?,
        None => current.kind.clone(),
    };
    // Signo↔kind solo cuando el patch fija el importe: misma regla que el PATCH de transacciones.
    if touches_amount {
        assert_amount_sign_matches_kind(new_amount, &new_kind)?;
    }
    let requested_category = if body.clear_category == Some(true) {
        None
    } else {
        body.category_id.or(current.category_id)
    };
    let new_category =
        resolve_category_for_kind(&state.pool, iid, &new_kind, requested_category).await?;
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
    assert_asset_in_installation(&state.pool, iid, new_linked_asset).await?;
    assert_liability_in_installation(&state.pool, iid, new_linked_liability).await?;

    sqlx::query(
        r#"UPDATE recurring_transaction_rules
           SET concept = $1, amount = $2, kind = $3, category_id = $4, linked_asset_id = $5,
               linked_liability_id = $6, notes = $7, updated_at = now()
           WHERE id = $8 AND installation_id = $9 AND owner_user_id = $10"#,
    )
    .bind(&new_concept)
    .bind(new_amount)
    .bind(&new_kind)
    .bind(new_category)
    .bind(new_linked_asset)
    .bind(new_linked_liability)
    .bind(new_notes.as_deref())
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    let mut instances_updated: i64 = 0;
    let mut pairing_changed = false;
    if apply_to == RecurringApplyTo::All {
        let instances: Vec<InstanceRow> = sqlx::query_as(
            r#"SELECT id, import_id, source, op_date, concept, amount, kind, category_id,
                      linked_asset_id, linked_liability_id, notes, fingerprint, fingerprint_ordinal
               FROM transactions
               WHERE recurring_rule_id = $1 AND installation_id = $2 AND owner_user_id = $3
               ORDER BY op_date ASC, id ASC
               FOR UPDATE"#,
        )
        .bind(id)
        .bind(iid)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;

        for inst in &instances {
            // Solo los campos que vienen en el patch; el resto conserva lo que la fila tenga
            // (una instancia retocada a mano no pierde lo que el patch no nombra).
            let concept = if touches_concept { new_concept.clone() } else { inst.concept.clone() };
            let amount = if touches_amount { new_amount } else { inst.amount };
            let (kind, category) = if touches_classification {
                (Some(new_kind.clone()), new_category)
            } else {
                (inst.kind.clone(), inst.category_id)
            };
            let linked_asset = if touches_asset { new_linked_asset } else { inst.linked_asset_id };
            let linked_liability =
                if touches_liability { new_linked_liability } else { inst.linked_liability_id };
            let notes = if touches_notes { new_notes.clone() } else { inst.notes.clone() };

            if amount != inst.amount {
                unlink_pair_no_rejection(&mut tx, iid, user_id, inst.id).await?;
                pairing_changed = true;
            }
            let (fp, ordinal) = if inst.import_id.is_none() {
                let fp = compute_fingerprint(&inst.source, inst.op_date, amount, &concept);
                if fp == inst.fingerprint {
                    (inst.fingerprint.clone(), inst.fingerprint_ordinal)
                } else {
                    let ord = next_fingerprint_ordinal(&mut tx, iid, user_id, &fp).await?;
                    (fp, ord)
                }
            } else {
                (inst.fingerprint.clone(), inst.fingerprint_ordinal)
            };

            sqlx::query(
                r#"UPDATE transactions
                   SET concept = $1, amount = $2, kind = $3, category_id = $4,
                       linked_asset_id = $5, linked_liability_id = $6, notes = $7,
                       fingerprint = $8, fingerprint_ordinal = $9, updated_at = now()
                   WHERE id = $10 AND installation_id = $11 AND owner_user_id = $12"#,
            )
            .bind(&concept)
            .bind(amount)
            .bind(kind.as_deref())
            .bind(category)
            .bind(linked_asset)
            .bind(linked_liability)
            .bind(notes.as_deref())
            .bind(&fp)
            .bind(ordinal)
            .bind(inst.id)
            .bind(iid)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            instances_updated += 1;
        }
    }

    let sql = format!("{RULE_SELECT} WHERE r.id = $1");
    let row: RuleRow = sqlx::query_as(&sql).bind(id).fetch_one(&mut *tx).await?;
    let resp = UpdateRecurringRuleResponse {
        rule: row_to_response(row),
        apply_to: apply_to.as_str().to_string(),
        instances_updated,
    };

    if dry_run {
        tx.rollback().await?;
        return Ok(resp);
    }
    tx.commit().await?;

    if instances_updated > 0 {
        if pairing_changed {
            auto_reconcile_after_mutation(state, iid, user_id).await;
            converge_recurring_after_mutation(state, iid).await;
        }
        invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    }
    Ok(resp)
}
