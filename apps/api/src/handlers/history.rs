//! Snapshots históricos de patrimonio (`/v1/history/snapshots`).
//!
//! Los snapshots son fotos manuales, per-user, del ledger (assets/liabilities) en un
//! día civil concreto. Sirven para reconstruir la serie histórica de net worth. Son
//! **CRUD de datos propios** (siempre `owner_user_id = usuario`); no aplican los helpers
//! `LedgerView` household/mine.
//!
//! ## Por qué estas mutaciones NO invalidan la cache de proyección
//! Ningún handler de este módulo llama a `refresh_projection_after_mutation`. Los snapshots
//! son historia congelada: **no son inputs del motor de proyección** (que arranca en el mes 0
//! con el ledger vivo). Invalidar la cache aquí solo tiraría una entrada caliente sin cambiar
//! ni un número de la proyección. Un test de regresión fija esta ausencia.

use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::serialize_decimal_as_f64;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use futurefin_engine::{
    add_months_signed, amortized_segment_value, evaluate_timeline, month_index_of, CashFlowEntry,
    HistoryItem, HistoryItemKind, HistoryObservation, HistoryTimeline, LoanTerms,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotItemResponse {
    /// `source_item_id`: id del asset/liability en captura, o clave del backfill.
    #[schema(value_type = String, format = "uuid")]
    pub item_id: Uuid,
    pub label: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub value: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_frequency: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `asset` | `liability`.
    pub kind: String,
    pub snapshot_date_ymd: String,
    /// `capture` | `backfill`.
    pub source: String,
    /// Σ de los `value` de los items, calculado en Rust (nunca almacenado).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total: Decimal,
    /// Items ordenados por `label ASC`.
    pub items: Vec<SnapshotItemResponse>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureResponse {
    pub snapshots: Vec<SnapshotResponse>,
}

// ---------------------------------------------------------------------------
// Request bodies / queries
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CaptureBody {
    /// Kinds a capturar (`asset` y/o `liability`). Omitido → ambos.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SnapshotItemBody {
    /// Clave de item; ausente → el servidor genera un UUID y lo devuelve.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub item_id: Option<Uuid>,
    pub label: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub value: Decimal,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(default)]
    pub payment_frequency: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSnapshotBody {
    /// `asset` | `liability`.
    pub kind: String,
    #[schema(value_type = String, format = "date")]
    pub snapshot_date: NaiveDate,
    #[serde(default)]
    pub items: Vec<SnapshotItemBody>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSnapshotBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub snapshot_date: Option<NaiveDate>,
    /// Omitido (`None`) → los items existentes se conservan intactos (solo se actualiza la
    /// cabecera/fecha). Presente (incluso `[]`) → reemplazo completo de los items.
    #[serde(default)]
    pub items: Option<Vec<SnapshotItemBody>>,
}

#[derive(Debug, Deserialize)]
pub struct ListSnapshotsQuery {
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub kind: Option<String>,
}

// ---------------------------------------------------------------------------
// Row structs
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct SnapshotHeaderRow {
    id: Uuid,
    kind: String,
    snapshot_date: NaiveDate,
    source: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct SnapshotItemRow {
    snapshot_id: Uuid,
    source_item_id: Uuid,
    label: String,
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
}

/// Item ya validado y listo para insertar.
struct PreparedItem {
    item_id: Uuid,
    label: String,
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
}

// ---------------------------------------------------------------------------
// Validation helpers (bounds copiados de assets.rs / liabilities.rs)
// ---------------------------------------------------------------------------

fn normalize_kind(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "asset" => Ok("asset".into()),
        "liability" => Ok("liability".into()),
        _ => Err(ApiError::BadRequest(
            "invalid_kind: kind must be 'asset' or 'liability'".into(),
        )),
    }
}

fn normalize_label(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest("label must not be empty".into()));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "label must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn normalize_frequency(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "monthly" => Ok("monthly".into()),
        "weekly" => Ok("weekly".into()),
        _ => Err(ApiError::BadRequest(
            "payment_frequency must be monthly or weekly".into(),
        )),
    }
}

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("{field} must be >= 0")));
    }
    Ok(())
}

fn validate_snapshot_date(date: NaiveDate, today: NaiveDate) -> Result<(), ApiError> {
    if date > today {
        return Err(ApiError::BadRequest(
            "snapshot_date_in_future: snapshot_date must not be after today".into(),
        ));
    }
    let min = NaiveDate::from_ymd_opt(1900, 1, 1).expect("1900-01-01 is a valid date");
    if date < min {
        return Err(ApiError::BadRequest(
            "snapshot_date_too_old: snapshot_date must be on or after 1900-01-01".into(),
        ));
    }
    Ok(())
}

/// Valida y normaliza los items de un backfill/PUT. Reglas:
/// - máx 500 (`too_many_items`),
/// - `value` >= 0 (bound de `assets.current_value`),
/// - términos (apr/payment_*) solo en `liability` (`terms_only_for_liabilities`),
/// - apr_percent >= 0, payment_amount > 0, payment_frequency ∈ {monthly, weekly}
///   (bounds de `liabilities.rs`),
/// - `item_id` ausente → UUID nuevo; `item_id` repetido → `duplicate_item_id`.
fn validate_and_prepare_items(
    items: &[SnapshotItemBody],
    is_liability: bool,
) -> Result<Vec<PreparedItem>, ApiError> {
    if items.len() > 500 {
        return Err(ApiError::BadRequest(
            "too_many_items: at most 500 items per snapshot".into(),
        ));
    }
    let mut seen: HashSet<Uuid> = HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let label = normalize_label(&it.label)?;
        assert_non_negative(it.value, "value")?;

        let has_terms = it.apr_percent.is_some()
            || it.payment_amount.is_some()
            || it.payment_frequency.is_some();
        if has_terms && !is_liability {
            return Err(ApiError::BadRequest(
                "terms_only_for_liabilities: apr_percent/payment_amount/payment_frequency are only valid for kind 'liability'".into(),
            ));
        }
        if let Some(apr) = it.apr_percent {
            assert_non_negative(apr, "apr_percent")?;
        }
        if let Some(pa) = it.payment_amount {
            if pa <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "payment_amount must be > 0 when set".into(),
                ));
            }
        }
        let payment_frequency = match &it.payment_frequency {
            None => None,
            Some(f) => Some(normalize_frequency(f)?),
        };

        let item_id = it.item_id.unwrap_or_else(Uuid::new_v4);
        if !seen.insert(item_id) {
            return Err(ApiError::BadRequest(
                "duplicate_item_id: item_id repeated within snapshot".into(),
            ));
        }

        out.push(PreparedItem {
            item_id,
            label,
            value: it.value,
            apr_percent: it.apr_percent,
            payment_amount: it.payment_amount,
            payment_frequency,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Shared DB helpers
// ---------------------------------------------------------------------------

fn build_response(h: SnapshotHeaderRow, items: Vec<SnapshotItemRow>) -> SnapshotResponse {
    let total: Decimal = items.iter().map(|i| i.value).sum();
    SnapshotResponse {
        id: h.id,
        kind: h.kind,
        snapshot_date_ymd: h.snapshot_date.format("%Y-%m-%d").to_string(),
        source: h.source,
        total,
        items: items
            .into_iter()
            .map(|i| SnapshotItemResponse {
                item_id: i.source_item_id,
                label: i.label,
                value: i.value,
                apr_percent: i.apr_percent,
                payment_amount: i.payment_amount,
                payment_frequency: i.payment_frequency,
            })
            .collect(),
        created_at: h.created_at,
        updated_at: h.updated_at,
    }
}

/// Carga cabecera + items (orden `label ASC`) de un snapshot dado. Usable dentro de una
/// transacción (`&mut *tx`) para devolver el estado recién escrito.
async fn load_snapshot_response(
    conn: &mut PgConnection,
    snapshot_id: Uuid,
) -> Result<SnapshotResponse, ApiError> {
    let header: SnapshotHeaderRow = sqlx::query_as(
        r#"SELECT id, kind, snapshot_date, source, created_at, updated_at
           FROM history_snapshots
           WHERE id = $1"#,
    )
    .bind(snapshot_id)
    .fetch_one(&mut *conn)
    .await?;

    let items: Vec<SnapshotItemRow> = sqlx::query_as(
        r#"SELECT snapshot_id, source_item_id, label, value, apr_percent,
                  payment_amount, payment_frequency
           FROM history_snapshot_items
           WHERE snapshot_id = $1
           ORDER BY label ASC"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(build_response(header, items))
}

async fn insert_items(
    conn: &mut PgConnection,
    snapshot_id: Uuid,
    items: &[PreparedItem],
) -> Result<(), ApiError> {
    for it in items {
        sqlx::query(
            r#"INSERT INTO history_snapshot_items
                   (snapshot_id, source_item_id, label, value,
                    apr_percent, payment_amount, payment_frequency)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(snapshot_id)
        .bind(it.item_id)
        .bind(&it.label)
        .bind(it.value)
        .bind(it.apr_percent)
        .bind(it.payment_amount)
        .bind(it.payment_frequency.as_deref())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/history/snapshots/capture",
    tag = "history",
    request_body = CaptureBody,
    responses(
        (status = 200, description = "Snapshots capturados (upsert por día)", body = CaptureResponse),
        (status = 400, description = "kinds vacío o inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn capture_snapshots(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CaptureBody>,
) -> Result<Json<CaptureResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    // Resolver kinds: default ambos; validar; deduplicar preservando orden.
    let requested = match body.kinds {
        None => vec!["asset".to_string(), "liability".to_string()],
        Some(v) => {
            if v.is_empty() {
                return Err(ApiError::BadRequest(
                    "kinds_empty: kinds must not be an empty array".into(),
                ));
            }
            v
        }
    };
    let mut kinds: Vec<String> = Vec::with_capacity(requested.len());
    for k in &requested {
        let norm = normalize_kind(k)?;
        if !kinds.contains(&norm) {
            kinds.push(norm);
        }
    }

    let today = installation_naive_today(&state.pool, iid).await?;

    let mut tx = state.pool.begin().await?;
    let mut out = Vec::with_capacity(kinds.len());
    for kind in kinds {
        // Upsert de cabecera: la captura del mismo día sobrescribe silenciosamente.
        let snapshot_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO history_snapshots
                   (installation_id, owner_user_id, kind, snapshot_date, source)
               VALUES ($1, $2, $3, $4, 'capture')
               ON CONFLICT ON CONSTRAINT history_snapshots_unique_per_day
               DO UPDATE SET source = 'capture', updated_at = now()
               RETURNING id"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .bind(&kind)
        .bind(today)
        .fetch_one(&mut *tx)
        .await?;

        // Reemplazar items: borrar y volver a copiar desde el ledger propio.
        sqlx::query(r#"DELETE FROM history_snapshot_items WHERE snapshot_id = $1"#)
            .bind(snapshot_id)
            .execute(&mut *tx)
            .await?;

        if kind == "asset" {
            // Filas compartidas (owner_user_id IS NULL) excluidas por construcción.
            sqlx::query(
                r#"INSERT INTO history_snapshot_items
                       (snapshot_id, source_item_id, label, value,
                        apr_percent, payment_amount, payment_frequency)
                   SELECT $1, a.id, a.name, a.current_value, NULL, NULL, NULL
                   FROM assets a
                   WHERE a.installation_id = $2 AND a.owner_user_id = $3"#,
            )
            .bind(snapshot_id)
            .bind(iid)
            .bind(user.id.0)
            .execute(&mut *tx)
            .await?;
        } else {
            // Solo pasivos no expirados; se copian los términos del préstamo.
            sqlx::query(
                r#"INSERT INTO history_snapshot_items
                       (snapshot_id, source_item_id, label, value,
                        apr_percent, payment_amount, payment_frequency)
                   SELECT $1, l.id, l.label, l.principal,
                          l.apr_percent, l.payment_amount, l.payment_frequency
                   FROM liabilities l
                   WHERE l.installation_id = $2 AND l.owner_user_id = $3
                     AND (l.payment_end_date IS NULL OR l.payment_end_date >= $4)"#,
            )
            .bind(snapshot_id)
            .bind(iid)
            .bind(user.id.0)
            .bind(today)
            .execute(&mut *tx)
            .await?;
        }

        out.push(load_snapshot_response(&mut *tx, snapshot_id).await?);
    }
    tx.commit().await?;

    // Nota: NO se invalida la cache de proyección (ver doc del módulo).
    Ok(Json(CaptureResponse { snapshots: out }))
}

#[utoipa::path(
    get,
    path = "/v1/history/snapshots",
    tag = "history",
    params(
        ("year" = Option<i32>, Query, description = "Filtro por año civil (1900..=3000), aplicado como rango de fechas."),
        ("kind" = Option<String>, Query, description = "`asset` | `liability`."),
    ),
    responses(
        (status = 200, description = "Snapshots del usuario (orden snapshot_date DESC, kind ASC)", body = [SnapshotResponse]),
        (status = 400, description = "year o kind inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_snapshots(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ListSnapshotsQuery>,
) -> Result<Json<Vec<SnapshotResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    if let Some(y) = q.year {
        if !(1900..=3000).contains(&y) {
            return Err(ApiError::BadRequest(
                "year must be between 1900 and 3000".into(),
            ));
        }
    }
    let kind = match &q.kind {
        None => None,
        Some(k) => Some(normalize_kind(k)?),
    };

    // CRUD own-data only: WHERE fijo installation_id = $1 AND owner_user_id = $2.
    let mut sql = String::from(
        "SELECT id, kind, snapshot_date, source, created_at, updated_at
         FROM history_snapshots
         WHERE installation_id = $1 AND owner_user_id = $2",
    );
    let mut next = 3;
    if q.year.is_some() {
        sql.push_str(&format!(
            " AND snapshot_date >= ${} AND snapshot_date < ${}",
            next,
            next + 1
        ));
        next += 2;
    }
    if kind.is_some() {
        sql.push_str(&format!(" AND kind = ${next}"));
    }
    sql.push_str(" ORDER BY snapshot_date DESC, kind ASC");

    let mut query = sqlx::query_as::<_, SnapshotHeaderRow>(&sql)
        .bind(iid)
        .bind(user.id.0);
    if let Some(y) = q.year {
        let start = NaiveDate::from_ymd_opt(y, 1, 1).expect("valid Jan 1");
        let end = NaiveDate::from_ymd_opt(y + 1, 1, 1).expect("valid next Jan 1");
        query = query.bind(start).bind(end);
    }
    if let Some(k) = kind {
        query = query.bind(k);
    }
    let headers: Vec<SnapshotHeaderRow> = query.fetch_all(&state.pool).await?;
    if headers.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let ids: Vec<Uuid> = headers.iter().map(|h| h.id).collect();
    let item_rows: Vec<SnapshotItemRow> = sqlx::query_as(
        r#"SELECT snapshot_id, source_item_id, label, value, apr_percent,
                  payment_amount, payment_frequency
           FROM history_snapshot_items
           WHERE snapshot_id = ANY($1)
           ORDER BY label ASC"#,
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;

    let mut by_parent: HashMap<Uuid, Vec<SnapshotItemRow>> = HashMap::new();
    for r in item_rows {
        by_parent.entry(r.snapshot_id).or_default().push(r);
    }

    let out: Vec<SnapshotResponse> = headers
        .into_iter()
        .map(|h| {
            let items = by_parent.remove(&h.id).unwrap_or_default();
            build_response(h, items)
        })
        .collect();
    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/v1/history/snapshots",
    tag = "history",
    request_body = CreateSnapshotBody,
    responses(
        (status = 201, description = "Snapshot de backfill creado", body = SnapshotResponse),
        (status = 400, description = "Validación (fecha, items, términos)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
        (status = 409, description = "Ya existe un snapshot para ese (usuario, kind, fecha)"),
    )
)]
pub async fn create_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateSnapshotBody>,
) -> Result<(axum::http::StatusCode, Json<SnapshotResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let kind = normalize_kind(&body.kind)?;
    let is_liability = kind == "liability";
    let today = installation_naive_today(&state.pool, iid).await?;
    validate_snapshot_date(body.snapshot_date, today)?;
    let items = validate_and_prepare_items(&body.items, is_liability)?;

    let mut tx = state.pool.begin().await?;
    // Fecha ocupada → unique-violation sobre history_snapshots_unique_per_day → 409
    // (mapeado en From<sqlx::Error>; el handler no inspecciona el SQLSTATE).
    let snapshot_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO history_snapshots
               (installation_id, owner_user_id, kind, snapshot_date, source)
           VALUES ($1, $2, $3, $4, 'backfill')
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(&kind)
    .bind(body.snapshot_date)
    .fetch_one(&mut *tx)
    .await?;

    insert_items(&mut tx, snapshot_id, &items).await?;
    let resp = load_snapshot_response(&mut tx, snapshot_id).await?;
    tx.commit().await?;

    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

#[utoipa::path(
    put,
    path = "/v1/history/snapshots/{id}",
    tag = "history",
    request_body = UpdateSnapshotBody,
    params(
        ("id" = Uuid, Path, description = "Snapshot id"),
    ),
    responses(
        (status = 200, description = "Snapshot actualizado. `items` omitido → conserva los items; `items` presente (incluso []) → reemplazo completo", body = SnapshotResponse),
        (status = 400, description = "Validación (fecha, items, términos)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Snapshot inexistente o de otro usuario"),
        (status = 409, description = "La fecha destino ya está ocupada"),
    )
)]
pub async fn update_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSnapshotBody>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    // Guardia id + installation + owner: si no es tuyo, 404 (no revelar existencia).
    let existing: Option<SnapshotHeaderRow> = sqlx::query_as(
        r#"SELECT id, kind, snapshot_date, source, created_at, updated_at
           FROM history_snapshots
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .fetch_optional(&state.pool)
    .await?;
    let Some(existing) = existing else {
        return Err(ApiError::NotFound);
    };

    // `kind` es inmutable: los términos permitidos dependen del kind existente.
    let is_liability = existing.kind == "liability";
    let new_date = body.snapshot_date.unwrap_or(existing.snapshot_date);
    let today = installation_naive_today(&state.pool, iid).await?;
    validate_snapshot_date(new_date, today)?;
    // `items` ausente (`None`) → conservar los items existentes; presente (incluso `[]`) →
    // reemplazo completo. Un PUT con solo `snapshot_date` ya no borra los items.
    let items = match &body.items {
        Some(list) => Some(validate_and_prepare_items(list, is_liability)?),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    // Mover a una fecha ocupada → unique-violation → 409. `source` intacto.
    sqlx::query(
        r#"UPDATE history_snapshots
           SET snapshot_date = $1, updated_at = now()
           WHERE id = $2 AND installation_id = $3 AND owner_user_id = $4"#,
    )
    .bind(new_date)
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;

    if let Some(items) = &items {
        sqlx::query(r#"DELETE FROM history_snapshot_items WHERE snapshot_id = $1"#)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        insert_items(&mut tx, id, items).await?;
    }
    let resp = load_snapshot_response(&mut tx, id).await?;
    tx.commit().await?;

    Ok(Json(resp))
}

#[utoipa::path(
    delete,
    path = "/v1/history/snapshots/{id}",
    tag = "history",
    params(
        ("id" = Uuid, Path, description = "Snapshot id"),
    ),
    responses(
        (status = 204, description = "Snapshot borrado (items en cascada)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Snapshot inexistente o de otro usuario"),
    )
)]
pub async fn delete_snapshot(
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
        r#"DELETE FROM history_snapshots
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

// ---------------------------------------------------------------------------
// Serie histórica interpolada (`GET /v1/history/series`)
// ---------------------------------------------------------------------------

/// Punto de la serie histórica agregada. `month_index ≤ 0`, contiguo `k_min..=0`.
/// Los numéricos por punto se serializan como **f64** (misma excepción chart-only
/// que los arrays de `/v1/projection/series` — D4/I3 del architecture contract).
#[derive(Debug, Serialize, ToSchema)]
pub struct HistorySeriesPoint {
    pub month_index: i32,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub assets_total: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub liabilities_total: Decimal,
}

/// Serie histórica por asset (`source_item_id`), agregada entre usuarios.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryAssetSeries {
    /// `source_item_id` del item (id del asset vivo en capturas; clave de backfill si no).
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub asset_name: String,
    /// Valores f64 paralelos a `points`.
    pub values: Vec<f64>,
}

/// Marcador de snapshot: uno por cabecera en scope.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryMarker {
    pub date_ymd: String,
    pub month_index: i32,
    /// Posición x fraccional del marcador: `month_index + (día − 1) / días_del_mes`.
    pub month_fraction: f64,
    /// `asset` | `liability`.
    pub kind: String,
    #[schema(value_type = String, format = "uuid")]
    pub owner_user_id: Uuid,
    /// Σ de los `value` de los items del snapshot.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub total: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HistorySeriesResponse {
    /// Hoy civil de la instalación (`installation.calendar_tz`).
    pub anchor_date_ymd: String,
    /// Primero-de-mes de `anchor_date_ymd` — la fecha del punto `month_index = 0`.
    pub anchor_month_first_ymd: String,
    /// `household` | `mine`.
    pub view: String,
    /// Puntos contiguos ascendentes `k_min..=0` (incluye el mes 0). Vacío sin snapshots.
    pub points: Vec<HistorySeriesPoint>,
    /// Series por asset (solo kind `asset`), orden `asset_name ASC, asset_id ASC`.
    pub asset_series: Vec<HistoryAssetSeries>,
    /// Un marcador por snapshot en scope.
    pub markers: Vec<HistoryMarker>,
}

#[derive(Debug, Clone, FromRow)]
struct SeriesHeaderRow {
    id: Uuid,
    owner_user_id: Uuid,
    kind: String,
    snapshot_date: NaiveDate,
}

#[derive(Debug, Clone, FromRow)]
struct LiveAssetRow {
    id: Uuid,
    name: String,
    current_value: Decimal,
    owner_user_id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
struct LiveLiabilityRow {
    id: Uuid,
    principal: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    owner_user_id: Uuid,
}

/// Términos del préstamo de una observación. Sin apr **y** cuota → `None` (el motor
/// interpola linealmente, mismo resultado que unos términos degenerados). La conversión
/// `weekly → ×52/12` vive en `projection::monthly_payment_from` (fuente única).
fn loan_terms_of(
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    frequency: Option<&str>,
) -> Option<LoanTerms> {
    match (apr_percent, payment_amount) {
        (Some(apr), Some(pay)) => Some(LoanTerms {
            apr_percent: apr,
            monthly_payment: crate::handlers::projection::monthly_payment_from(pay, frequency),
        }),
        _ => None,
    }
}

/// Días del mes civil de `d` (28–31).
fn days_in_month_of(d: NaiveDate) -> i64 {
    (add_months_signed(d, 1) - add_months_signed(d, 0)).num_days()
}

/// Posición x fraccional de una fecha respecto a un ancla primero-de-mes:
/// `month_index + (día − 1) / días_del_mes`. Fuente ÚNICA compartida por los `HistoryMarker`
/// de `/v1/history/series` y por la rejilla fina de `/v1/history/cashflow` — así la escala
/// mes→px del chart no puede divergir entre markers y overlay (disciplina anti off-by-one).
fn month_fraction(date: NaiveDate, anchor_month_first: NaiveDate) -> f64 {
    month_index_of(date, anchor_month_first) as f64
        + (date.day() as f64 - 1.0) / days_in_month_of(date) as f64
}

// ---------------------------------------------------------------------------
// Pipeline compartido de snapshots → serie interpolada
// ---------------------------------------------------------------------------
//
// `fetch_history_scope` + `accumulate_series` extraen el pipeline común a
// `GET /v1/history/series` (sin cash-flow, rejilla mensual) y `GET /v1/history/cashflow`
// (con deltas de cash-flow que moldean la curva, rejilla fina). Refactor **puro**: con un mapa
// de cash-flow vacío y la rejilla mensual, `accumulate_series` reproduce bit a bit la serie de
// snapshots previa (el engine garantiza P3: `cashflow` vacío ⇒ interpolación lineal textual).

/// Filas crudas del scope (mismas 4 queries que la serie histórica). Vacío (`headers` vacío) sin
/// snapshots en scope — en ese caso el resto queda vacío y no se lanzan las 3 queries restantes.
#[derive(Clone)]
struct HistoryScope {
    headers: Vec<SeriesHeaderRow>,
    items_by_snapshot: HashMap<Uuid, Vec<SnapshotItemRow>>,
    live_assets: Vec<LiveAssetRow>,
    live_liabs: Vec<LiveLiabilityRow>,
}

/// Ejecuta las 4 queries del scope (cabeceras, items, assets vivos, pasivos vivos no expirados),
/// todas vía helpers `LedgerView`. Idéntico a lo que hacía `get_history_series` inline; si no hay
/// cabeceras corta antes de lanzar las 3 restantes (como el early-return previo).
async fn fetch_history_scope(
    pool: &sqlx::PgPool,
    view: LedgerView,
    iid: Uuid,
    session_user_id: Uuid,
    today: NaiveDate,
) -> Result<HistoryScope, ApiError> {
    // 1) Cabeceras de snapshot en scope. Household = TODOS los snapshots de la instalación
    //    (owner_user_id es NOT NULL en todos); mine = solo los del usuario.
    let h_scope = view.scope_where("s");
    let headers_sql = format!(
        "SELECT s.id, s.owner_user_id, s.kind, s.snapshot_date
         FROM history_snapshots s
         WHERE {h_scope}
         ORDER BY s.snapshot_date ASC, s.kind ASC, s.owner_user_id ASC"
    );
    let headers: Vec<SeriesHeaderRow> = view
        .bind_scope_as(sqlx::query_as(&headers_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // 0 snapshots en scope → no lanzamos las 3 queries restantes (mismo comportamiento que el
    // early-return previo de get_history_series).
    if headers.is_empty() {
        return Ok(HistoryScope {
            headers,
            items_by_snapshot: HashMap::new(),
            live_assets: Vec::new(),
            live_liabs: Vec::new(),
        });
    }

    // 2) Items de los snapshots en scope (JOIN a la cabecera para reutilizar el scope).
    let i_scope = view.scope_where("s");
    let items_sql = format!(
        "SELECT i.snapshot_id, i.source_item_id, i.label, i.value,
                i.apr_percent, i.payment_amount, i.payment_frequency
         FROM history_snapshot_items i
         JOIN history_snapshots s ON s.id = i.snapshot_id
         WHERE {i_scope}"
    );
    let item_rows: Vec<SnapshotItemRow> = view
        .bind_scope_as(sqlx::query_as(&items_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // 3) Assets vivos del scope. Las filas compartidas (owner_user_id IS NULL) nunca
    //    participan en el histórico → conjunto extra `owner_user_id IS NOT NULL`.
    let a_scope = view.scope_where("a");
    let assets_sql = format!(
        "SELECT a.id, a.name, a.current_value, a.owner_user_id
         FROM assets a
         WHERE {a_scope} AND a.owner_user_id IS NOT NULL"
    );
    let live_assets: Vec<LiveAssetRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // 4) Pasivos vivos no expirados del scope, mismo conjunto extra.
    let l_scope = view.scope_where("l");
    let l_today_arg = view.next_arg_index();
    let liabs_sql = format!(
        "SELECT l.id, l.principal, l.apr_percent, l.payment_amount,
                l.payment_frequency, l.owner_user_id
         FROM liabilities l
         WHERE {l_scope} AND l.owner_user_id IS NOT NULL
           AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${l_today_arg})"
    );
    let live_liabs: Vec<LiveLiabilityRow> = view
        .bind_scope_as(sqlx::query_as(&liabs_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    let mut items_by_snapshot: HashMap<Uuid, Vec<SnapshotItemRow>> = HashMap::new();
    for r in item_rows {
        items_by_snapshot.entry(r.snapshot_id).or_default().push(r);
    }

    Ok(HistoryScope {
        headers,
        items_by_snapshot,
        live_assets,
        live_liabs,
    })
}

/// Resultado de evaluar el scope sobre una rejilla: totales por punto + serie por asset
/// (agrupada por `source_item_id` entre usuarios) + el label más reciente de cada asset.
struct SeriesAccumulation {
    assets_total: Vec<Decimal>,
    liabilities_total: Vec<Decimal>,
    /// `source_item_id` → valores paralelos a la rejilla (suma entre usuarios).
    asset_values: HashMap<Uuid, Vec<Decimal>>,
    /// `source_item_id` → (fecha, label) del snapshot más reciente que lo contiene (fallback name).
    latest_label: HashMap<Uuid, (NaiveDate, String)>,
}

/// Evalúa los timelines por `(owner_user_id, kind)` sobre `grid` y agrega los totales por punto y
/// las series por asset. Función **pura** (sin I/O). `cashflow_by_owner_asset` moldea la curva de
/// los assets observados en ambos extremos de un segmento (tier-2); un mapa vacío ⇒ interpolación
/// lineal / amortización francesa idéntica al histórico previo (P3 del engine).
fn accumulate_series(
    scope: &HistoryScope,
    grid: &[NaiveDate],
    today: NaiveDate,
    cashflow_by_owner_asset: &HashMap<(Uuid, Uuid), Vec<CashFlowEntry>>,
) -> Result<SeriesAccumulation, ApiError> {
    let grid_len = grid.len();

    let mut live_assets_by_owner: HashMap<Uuid, Vec<&LiveAssetRow>> = HashMap::new();
    for a in &scope.live_assets {
        live_assets_by_owner.entry(a.owner_user_id).or_default().push(a);
    }
    let mut live_liabs_by_owner: HashMap<Uuid, Vec<&LiveLiabilityRow>> = HashMap::new();
    for l in &scope.live_liabs {
        live_liabs_by_owner.entry(l.owner_user_id).or_default().push(l);
    }

    // Timelines por (owner_user_id, kind). BTreeMap para iterar en orden determinista.
    let mut groups: BTreeMap<(Uuid, String), Vec<&SeriesHeaderRow>> = BTreeMap::new();
    for h in &scope.headers {
        groups
            .entry((h.owner_user_id, h.kind.clone()))
            .or_default()
            .push(h);
    }

    let mut assets_total = vec![Decimal::ZERO; grid_len];
    let mut liabilities_total = vec![Decimal::ZERO; grid_len];
    let mut asset_values: HashMap<Uuid, Vec<Decimal>> = HashMap::new();
    let mut latest_label: HashMap<Uuid, (NaiveDate, String)> = HashMap::new();

    for ((owner_id, kind_str), group_headers) in &groups {
        let kind = if kind_str == "liability" {
            HistoryItemKind::Liability
        } else {
            HistoryItemKind::Asset
        };

        // Fechas ascendentes y distintas: ORDER BY snapshot_date ASC + unicidad
        // (installation, user, kind, date) lo garantizan dentro del grupo.
        let mut dates: Vec<NaiveDate> = group_headers.iter().map(|h| h.snapshot_date).collect();
        let last_real = *dates.last().expect("group has >= 1 header");
        // Observación virtual «hoy» con las filas vivas del owner, SALVO que el último
        // snapshot real sea de hoy (`<` y no `!=`: nunca romper el orden ascendente si
        // un cambio de calendar_tz dejara un snapshot "en el futuro").
        let append_virtual = last_real < today;
        let total_len = dates.len() + usize::from(append_virtual);

        let mut obs_map: BTreeMap<Uuid, Vec<Option<HistoryObservation>>> = BTreeMap::new();
        for (j, h) in group_headers.iter().enumerate() {
            let Some(items) = scope.items_by_snapshot.get(&h.id) else {
                continue;
            };
            for it in items {
                let obs = obs_map
                    .entry(it.source_item_id)
                    .or_insert_with(|| vec![None; total_len]);
                obs[j] = Some(HistoryObservation {
                    value: it.value,
                    terms: loan_terms_of(
                        it.apr_percent,
                        it.payment_amount,
                        it.payment_frequency.as_deref(),
                    ),
                });
                if kind == HistoryItemKind::Asset {
                    let slot = latest_label
                        .entry(it.source_item_id)
                        .or_insert_with(|| (h.snapshot_date, it.label.clone()));
                    if h.snapshot_date >= slot.0 {
                        *slot = (h.snapshot_date, it.label.clone());
                    }
                }
            }
        }

        if append_virtual {
            dates.push(today);
            let last = total_len - 1;
            match kind {
                HistoryItemKind::Asset => {
                    for a in live_assets_by_owner.get(owner_id).into_iter().flatten() {
                        let obs = obs_map.entry(a.id).or_insert_with(|| vec![None; total_len]);
                        obs[last] = Some(HistoryObservation {
                            value: a.current_value,
                            terms: None,
                        });
                    }
                }
                HistoryItemKind::Liability => {
                    for l in live_liabs_by_owner.get(owner_id).into_iter().flatten() {
                        let obs = obs_map.entry(l.id).or_insert_with(|| vec![None; total_len]);
                        obs[last] = Some(HistoryObservation {
                            value: l.principal,
                            terms: loan_terms_of(
                                l.apr_percent,
                                l.payment_amount,
                                l.payment_frequency.as_deref(),
                            ),
                        });
                    }
                }
            }
        }

        let timeline = HistoryTimeline {
            dates,
            items: obs_map
                .into_iter()
                .map(|(source_item_id, observations)| HistoryItem {
                    source_item_id,
                    kind,
                    observations,
                    // Solo los assets llevan cash-flow (los pasivos ya modelan el principal con
                    // amortización francesa — inyectar la cuota duplicaría). Sin entrada en el
                    // mapa ⇒ vacío ⇒ ruta idéntica al histórico previo (P3).
                    cashflow: if kind == HistoryItemKind::Asset {
                        cashflow_by_owner_asset
                            .get(&(*owner_id, source_item_id))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        vec![]
                    },
                })
                .collect(),
        };

        // Cómputo puro (decenas de snapshots × decenas/centenas de puntos). En la serie mensual y
        // en la rejilla fina weekly se ejecuta inline; solo `resolution=daily` lo envuelve en
        // `spawn_blocking` (ver get_history_cashflow).
        let evaluated = evaluate_timeline(&timeline, grid)
            // Inalcanzable con fechas ordenadas + únicas; señal de bug del servidor.
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;

        for (item, series) in timeline.items.iter().zip(evaluated) {
            match kind {
                HistoryItemKind::Asset => {
                    let acc = asset_values
                        .entry(item.source_item_id)
                        .or_insert_with(|| vec![Decimal::ZERO; grid_len]);
                    for (g, v) in series.iter().enumerate() {
                        assets_total[g] += *v;
                        acc[g] += *v;
                    }
                }
                HistoryItemKind::Liability => {
                    for (g, v) in series.iter().enumerate() {
                        liabilities_total[g] += *v;
                    }
                }
            }
        }
    }

    Ok(SeriesAccumulation {
        assets_total,
        liabilities_total,
        asset_values,
        latest_label,
    })
}

#[utoipa::path(
    get,
    path = "/v1/history/series",
    tag = "history",
    params(
        ("view" = Option<String>, Query, description = "`mine` = solo mis snapshots; omitido u otro valor → `household` (todos los usuarios de la instalación)."),
    ),
    responses(
        (status = 200, description = "Serie histórica interpolada, puntos contiguos `k_min..=0`. Sin snapshots en scope → arrays vacíos.", body = HistorySeriesResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_history_series(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<HistorySeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    // Solo lectura: cualquier miembro (viewer incluido) puede pedir la serie.
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = q.resolve();
    let view_label = if view == LedgerView::Mine { "mine" } else { "household" };

    let today = installation_naive_today(&state.pool, iid).await?;
    // `add_months_signed(d, 0)` devuelve el día 1 del mes de `d` → ancla del mes 0.
    let anchor = add_months_signed(today, 0);

    // ---- Fetch del scope (4 queries LedgerView, pipeline compartido) ------------------------
    let scope = fetch_history_scope(&state.pool, view, iid, user.id.0, today).await?;

    // 0 snapshots en scope → 200 con arrays vacíos.
    if scope.headers.is_empty() {
        return Ok(Json(HistorySeriesResponse {
            anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
            anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
            view: view_label.into(),
            points: Vec::new(),
            asset_series: Vec::new(),
            markers: Vec::new(),
        }));
    }

    // ---- Markers: uno por cabecera; total = Σ items -----------------------------------------
    let markers: Vec<HistoryMarker> = scope
        .headers
        .iter()
        .map(|h| {
            let total: Decimal = scope
                .items_by_snapshot
                .get(&h.id)
                .map(|items| items.iter().map(|i| i.value).sum())
                .unwrap_or(Decimal::ZERO);
            HistoryMarker {
                date_ymd: h.snapshot_date.format("%Y-%m-%d").to_string(),
                month_index: month_index_of(h.snapshot_date, anchor),
                month_fraction: month_fraction(h.snapshot_date, anchor),
                kind: h.kind.clone(),
                owner_user_id: h.owner_user_id,
                total,
            }
        })
        .collect();

    // ---- Rejilla mensual k_min..=0 (primeros-de-mes) ----------------------------------------
    // Las fechas de snapshot están validadas ≤ hoy, así que todo month_index ≤ 0;
    // `.min(0)` es solo un cinturón (p. ej. cambio de calendar_tz a posteriori).
    let k_min = markers.iter().map(|m| m.month_index).min().unwrap_or(0).min(0);
    let grid: Vec<NaiveDate> = (k_min..=0).map(|k| add_months_signed(anchor, k)).collect();
    let grid_len = grid.len();

    // ---- Evaluación (sin cash-flow: serie de snapshots tier-1) ------------------------------
    // Mapa de cash-flow vacío ⇒ interpolación lineal / amortización francesa idéntica bit a bit
    // al histórico previo (P3 del engine). Cómputo puro sub-ms (decenas de snapshots × meses):
    // deliberadamente SIN `spawn_blocking` a esta escala y SIN cache propia.
    let acc = accumulate_series(&scope, &grid, today, &HashMap::new())?;

    // ---- Agregación final --------------------------------------------------------------------
    let live_asset_names: HashMap<Uuid, &str> =
        scope.live_assets.iter().map(|a| (a.id, a.name.as_str())).collect();

    let points: Vec<HistorySeriesPoint> = (0..grid_len)
        .map(|g| HistorySeriesPoint {
            month_index: k_min + g as i32,
            net_worth: acc.assets_total[g] - acc.liabilities_total[g],
            assets_total: acc.assets_total[g],
            liabilities_total: acc.liabilities_total[g],
        })
        .collect();

    let mut asset_series: Vec<HistoryAssetSeries> = acc
        .asset_values
        .iter()
        .map(|(asset_id, values)| {
            // Nombre: el asset vivo gana; si no, el label del snapshot más reciente.
            let asset_name = live_asset_names
                .get(asset_id)
                .map(|n| n.to_string())
                .or_else(|| acc.latest_label.get(asset_id).map(|(_, l)| l.clone()))
                .unwrap_or_default();
            HistoryAssetSeries {
                asset_id: *asset_id,
                asset_name,
                values: values.iter().map(|v| v.to_f64().unwrap_or(0.0)).collect(),
            }
        })
        .collect();
    asset_series.sort_by(|a, b| {
        a.asset_name
            .cmp(&b.asset_name)
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });

    Ok(Json(HistorySeriesResponse {
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
        view: view_label.into(),
        points,
        asset_series,
        markers,
    }))
}

// ---------------------------------------------------------------------------
// Cash-flow histórico (`GET /v1/history/cashflow`)
// ---------------------------------------------------------------------------
//
// Dos capas independientes:
//   1. `months[]` — agregado mensual firmado por kind (KPIs, **Decimal-string**). Independiente de
//      los snapshots: solo un `GROUP BY date(month), kind` sobre la ventana. `expense`/`savings`
//      conservan su signo real (negativos = cargos/aportaciones), `income` positivo, `net` = suma.
//   2. `fine` (opcional) — la curva fina de patrimonio (weekly/daily) donde los deltas de cash-flow
//      **moldean** los assets vinculados sin contradecir los snapshots (tier-2, curva anclada).
//      Presente solo si hay transacciones vinculadas a algún asset Y snapshots que anclar.
//
// **`GET /v1/history/series` queda intacto** (tier-1): este endpoint reutiliza el mismo pipeline
// (`fetch_history_scope` / `accumulate_series`) pero con un mapa de cash-flow no vacío y una
// rejilla fina; jamás toca la serie mensual de snapshots. Sin cache; `spawn_blocking` solo en
// `resolution=daily`. Sus lecturas nunca invalidan la cache de proyección (las transacciones no
// son inputs del engine).

/// Un mes del agregado de cash-flow. Signos reales de la suma (`expense`/`savings` ≤ 0, `income`
/// ≥ 0); `net = expense + income + savings`. **Decimal-string** (son KPIs), redondeado a 2 dp.
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowMonth {
    pub month_index: i32,
    /// Primero-de-mes del mes, `YYYY-MM-01`.
    pub date_ymd: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub savings: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net: Decimal,
}

/// Punto de la rejilla fina: fecha + su posición x fraccional (mismo helper que los markers).
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowFineGridPoint {
    pub date_ymd: String,
    pub month_index: i32,
    pub month_fraction: f64,
}

/// Serie fina por asset (moldeada por cash-flow). Valores **f64** (excepción chart-only, igual que
/// `HistoryAssetSeries`).
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowFineAssetSeries {
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub asset_name: String,
    pub values: Vec<f64>,
}

/// Capa fina del cash-flow: rejilla + series por asset + net worth, todo paralelo a `grid`.
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowFine {
    /// `weekly` | `daily`.
    pub resolution: String,
    pub grid: Vec<CashflowFineGridPoint>,
    pub asset_series: Vec<CashflowFineAssetSeries>,
    /// `Σ assets moldeados − Σ liabilities amortizadas`, evaluado en el MISMO grid fino. f64.
    pub net_worth: Vec<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowResponse {
    /// Hoy civil de la instalación (`installation.calendar_tz`).
    pub anchor_date_ymd: String,
    /// Primero-de-mes de `anchor_date_ymd` — la fecha del punto `month_index = 0`.
    pub anchor_month_first_ymd: String,
    /// `household` | `mine`.
    pub view: String,
    /// Agregado mensual contiguo `-window_months..=0`, ascendente por `month_index`.
    pub months: Vec<CashflowMonth>,
    /// Curva fina moldeada. Ausente si no hay transacciones vinculadas a assets (o sin snapshots).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fine: Option<CashflowFine>,
}

#[derive(Debug, Deserialize)]
pub struct CashflowQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Meses de ventana, default 24, clamp 1..=120.
    #[serde(default)]
    pub window_months: Option<i64>,
    /// `weekly` (default) | `daily`. `daily` solo con `window_months <= 6`.
    #[serde(default)]
    pub resolution: Option<String>,
}

/// Fila cruda del agregado mensual: `(ym, kind)` → Σ amount firmado.
#[derive(Debug, FromRow)]
struct MonthKindRow {
    ym: String,
    kind: Option<String>,
    total: Decimal,
}

/// Fila cruda de una pata de cash-flow: `(owner, asset)` + `(fecha, delta ya normalizado)`.
#[derive(Debug, FromRow)]
struct CashflowLegRow {
    asset_id: Uuid,
    owner_user_id: Uuid,
    op_date: NaiveDate,
    delta: Decimal,
}

const DEFAULT_CASHFLOW_WINDOW_MONTHS: i64 = 24;
const MAX_CASHFLOW_WINDOW_MONTHS: i64 = 120;
/// `resolution=daily` solo se permite con ventanas acotadas (coste del grid diario).
const MAX_DAILY_WINDOW_MONTHS: i32 = 6;

#[utoipa::path(
    get,
    path = "/v1/history/cashflow",
    tag = "history",
    params(
        ("view" = Option<String>, Query, description = "`mine` = solo mis transacciones/snapshots; omitido u otro valor → `household`."),
        ("window_months" = Option<i64>, Query, description = "Meses de ventana (default 24, clamp 1..=120)."),
        ("resolution" = Option<String>, Query, description = "`weekly` (default) | `daily`. `daily` requiere `window_months <= 6`."),
    ),
    responses(
        (status = 200, description = "Cash-flow mensual + curva fina opcional.", body = CashflowResponse),
        (status = 400, description = "`daily_window_too_large` (daily con ventana > 6 meses)."),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_history_cashflow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<CashflowQuery>,
) -> Result<Json<CashflowResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    // Solo lectura: cualquier miembro (viewer incluido).
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery { view: q.view.clone() }.resolve();
    let view_label = if view == LedgerView::Mine { "mine" } else { "household" };

    let window_months: i32 = q
        .window_months
        .unwrap_or(DEFAULT_CASHFLOW_WINDOW_MONTHS)
        .clamp(1, MAX_CASHFLOW_WINDOW_MONTHS) as i32;
    let daily = matches!(q.resolution.as_deref().map(str::trim), Some("daily"));
    if daily && window_months > MAX_DAILY_WINDOW_MONTHS {
        return Err(ApiError::BadRequest(format!(
            "daily_window_too_large: resolution=daily requires window_months <= {MAX_DAILY_WINDOW_MONTHS}"
        )));
    }
    let resolution_label = if daily { "daily" } else { "weekly" };

    let today = installation_naive_today(&state.pool, iid).await?;
    let anchor = add_months_signed(today, 0); // primero-de-mes del mes 0.
    let window_start = add_months_signed(anchor, -window_months);
    let month_end = add_months_signed(anchor, 1); // exclusivo: incluye el mes 0 completo.

    // ---- Agregado mensual firmado por kind --------------------------------------------------
    let m_scope = view.scope_where("t");
    let m_arg = view.next_arg_index();
    let months_sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym, t.kind AS kind, SUM(t.amount) AS total
         FROM transactions t
         WHERE {m_scope} AND t.op_date >= ${m_arg} AND t.op_date < ${end}
         GROUP BY ym, t.kind",
        end = m_arg + 1
    );
    let month_rows: Vec<MonthKindRow> = view
        .bind_scope_as(sqlx::query_as(&months_sql), iid, user.id.0)
        .bind(window_start)
        .bind(month_end)
        .fetch_all(&state.pool)
        .await?;
    let mut by_month_kind: HashMap<(String, String), Decimal> = HashMap::new();
    for r in month_rows {
        if let Some(kind) = r.kind {
            *by_month_kind.entry((r.ym, kind)).or_insert(Decimal::ZERO) += r.total;
        }
    }
    // Cifras KPI a escala fija de 2 decimales (rescale, como `canonical_amount`): así una suma a
    // cero serializa "0.00" y no "0", y todas las cifras del mes comparten formato.
    let money = |d: Decimal| -> Decimal {
        let mut v = d.round_dp(2);
        v.rescale(2);
        v
    };
    let sum_of = |ym: &str, kind: &str| -> Decimal {
        by_month_kind
            .get(&(ym.to_string(), kind.to_string()))
            .copied()
            .unwrap_or(Decimal::ZERO)
    };
    let months: Vec<CashflowMonth> = (-window_months..=0)
        .map(|mi| {
            let m_date = add_months_signed(anchor, mi);
            let ym = m_date.format("%Y-%m").to_string();
            let expense = money(sum_of(&ym, "expense"));
            let income = money(sum_of(&ym, "income"));
            let savings = money(sum_of(&ym, "savings"));
            CashflowMonth {
                month_index: mi,
                date_ymd: m_date.format("%Y-%m-%d").to_string(),
                expense,
                income,
                savings,
                // net = suma exacta de las tres cifras devueltas (el invariante se cumple sobre
                // los strings serializados, no sobre valores pre-redondeo).
                net: money(expense + income + savings),
            }
        })
        .collect();

    // ---- Patas de cash-flow por (owner, asset) ----------------------------------------------
    // Pata cuenta: toda transacción de un batch con `account_asset_id` → delta = amount (+sube la
    // cuenta). Pata destino ahorro: `kind='savings'` con `linked_asset_id` → delta = −amount
    // (una aportación −200 sube el destino en +200). Una savings importada aparece en AMBAS
    // (partida doble correcta: baja la cuenta, sube el destino). Deltas ya normalizados aquí; el
    // engine solo los suma.
    let acc_scope = view.scope_where("t");
    let account_sql = format!(
        "SELECT ti.account_asset_id AS asset_id, t.owner_user_id AS owner_user_id,
                t.op_date AS op_date, t.amount AS delta
         FROM transactions t
         JOIN transaction_imports ti ON ti.id = t.import_id
         WHERE {acc_scope} AND ti.account_asset_id IS NOT NULL"
    );
    let account_legs: Vec<CashflowLegRow> = view
        .bind_scope_as(sqlx::query_as(&account_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;

    let sav_scope = view.scope_where("t");
    let savings_sql = format!(
        "SELECT t.linked_asset_id AS asset_id, t.owner_user_id AS owner_user_id,
                t.op_date AS op_date, (-t.amount) AS delta
         FROM transactions t
         WHERE {sav_scope} AND t.kind = 'savings' AND t.linked_asset_id IS NOT NULL"
    );
    let savings_legs: Vec<CashflowLegRow> = view
        .bind_scope_as(sqlx::query_as(&savings_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;

    let mut cashflow: HashMap<(Uuid, Uuid), Vec<CashFlowEntry>> = HashMap::new();
    for leg in account_legs.into_iter().chain(savings_legs.into_iter()) {
        cashflow
            .entry((leg.owner_user_id, leg.asset_id))
            .or_default()
            .push(CashFlowEntry {
                date: leg.op_date,
                delta: leg.delta,
            });
    }

    // ---- Capa fina (solo si hay vínculos a assets Y snapshots que anclar) --------------------
    let fine = if cashflow.is_empty() {
        None
    } else {
        let scope = fetch_history_scope(&state.pool, view, iid, user.id.0, today).await?;
        if scope.headers.is_empty() {
            None
        } else {
            // Rejilla fina hacia atrás desde HOY (último punto = hoy exacto → empalma con el vivo),
            // paso 7 días (weekly) o 1 (daily), hasta cubrir la ventana.
            let step = if daily { 1 } else { 7 };
            let mut dates_desc = vec![today];
            let mut cursor = today;
            loop {
                let prev = cursor - Duration::days(step);
                if prev < window_start {
                    break;
                }
                dates_desc.push(prev);
                cursor = prev;
            }
            dates_desc.reverse(); // ascendente, termina en hoy.
            let fine_grid = dates_desc;

            // Cómputo puro. `spawn_blocking` SOLO en daily (grid ~180 puntos): mueve datos propios
            // al pool blocking. Weekly (~104 puntos) corre inline como la serie mensual.
            let acc = if daily {
                let scope_c = scope.clone();
                let grid_c = fine_grid.clone();
                let cf_c = cashflow.clone();
                tokio::task::spawn_blocking(move || {
                    accumulate_series(&scope_c, &grid_c, today, &cf_c)
                })
                .await
                .map_err(|_| ApiError::Unavailable)??
            } else {
                accumulate_series(&scope, &fine_grid, today, &cashflow)?
            };

            let live_asset_names: HashMap<Uuid, &str> =
                scope.live_assets.iter().map(|a| (a.id, a.name.as_str())).collect();

            let grid: Vec<CashflowFineGridPoint> = fine_grid
                .iter()
                .map(|d| CashflowFineGridPoint {
                    date_ymd: d.format("%Y-%m-%d").to_string(),
                    month_index: month_index_of(*d, anchor),
                    month_fraction: month_fraction(*d, anchor),
                })
                .collect();

            let net_worth: Vec<f64> = (0..fine_grid.len())
                .map(|g| (acc.assets_total[g] - acc.liabilities_total[g]).to_f64().unwrap_or(0.0))
                .collect();

            let mut asset_series: Vec<CashflowFineAssetSeries> = acc
                .asset_values
                .iter()
                .map(|(asset_id, values)| {
                    let asset_name = live_asset_names
                        .get(asset_id)
                        .map(|n| n.to_string())
                        .or_else(|| acc.latest_label.get(asset_id).map(|(_, l)| l.clone()))
                        .unwrap_or_default();
                    CashflowFineAssetSeries {
                        asset_id: *asset_id,
                        asset_name,
                        values: values.iter().map(|v| v.to_f64().unwrap_or(0.0)).collect(),
                    }
                })
                .collect();
            asset_series.sort_by(|a, b| {
                a.asset_name
                    .cmp(&b.asset_name)
                    .then_with(|| a.asset_id.cmp(&b.asset_id))
            });

            Some(CashflowFine {
                resolution: resolution_label.into(),
                grid,
                asset_series,
                net_worth,
            })
        }
    };

    Ok(Json(CashflowResponse {
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
        view: view_label.into(),
        months,
        fine,
    }))
}

// ---------------------------------------------------------------------------
// Prefill de backfill (`GET /v1/history/snapshots/prefill`)
// ---------------------------------------------------------------------------
//
// Dada una fecha `d` y un `kind`, devuelve, POR ITEM del usuario, el valor que el
// panel de backfill debería pre-rellenar en esa fecha (editable por el usuario). No
// crea ni modifica nada: reconstruye el MISMO timeline own-user que `GET /history/series`
// (snapshots del kind + observación virtual «hoy» con las filas vivas no expiradas, salvo
// que el último snapshot real sea de hoy) y evalúa cada item exactamente en `d`.
//
// Diferencia con la serie: la región **anterior al primer snapshot** no es 0 sino el valor
// del primer snapshot (`basis = "first_snapshot"`), pensado para que el usuario complete el
// pasado hacia atrás. Toda la interpolación intermedia reutiliza el motor
// (`amortized_segment_value`; activos con `terms = None` → lineal en días civiles, idéntico
// a la serie), nunca se reimplementa la amortización.

/// Item pre-rellenado. `basis`: `interpolated` | `first_snapshot` | `live` | `not_owned`.
#[derive(Debug, Serialize, ToSchema)]
pub struct PrefillItemResponse {
    /// `source_item_id` del item (id del asset/liability vivo, o clave del backfill histórico).
    #[schema(value_type = String, format = "uuid")]
    pub item_id: Uuid,
    pub label: String,
    /// Valor sugerido, redondeado a 2 decimales (display-grade; el usuario lo edita).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub value: Decimal,
    /// `true` si el item existía (con valor) en `d`; `false` para `not_owned`.
    pub existed: bool,
    /// Origen del valor: `interpolated` | `first_snapshot` | `live` | `not_owned`.
    pub basis: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_frequency: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefillResponse {
    /// Fecha solicitada, `YYYY-MM-DD`.
    pub date_ymd: String,
    /// `asset` | `liability`.
    pub kind: String,
    /// Items ordenados: `existed=true` primero (`label ASC`), luego `not_owned` (`label ASC`).
    pub items: Vec<PrefillItemResponse>,
}

#[derive(Debug, Deserialize)]
pub struct PrefillQuery {
    /// `asset` | `liability` (requerido).
    #[serde(default)]
    pub kind: Option<String>,
    /// `YYYY-MM-DD` (requerido).
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
struct PrefillHeaderRow {
    id: Uuid,
    snapshot_date: NaiveDate,
}

/// Fila viva unificada (assets y liabilities normalizados a las mismas columnas). Para assets,
/// las columnas de términos llegan como `NULL`.
#[derive(Debug, FromRow)]
struct PrefillLiveRow {
    id: Uuid,
    label: String,
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
}

/// Observación de un item en un punto del timeline: valor + términos crudos (para eco en la
/// respuesta y para construir `LoanTerms` vía [`loan_terms_of`]).
struct PrefillObs {
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
}

fn obs_has_terms(o: &PrefillObs) -> bool {
    o.apr_percent.is_some() || o.payment_amount.is_some() || o.payment_frequency.is_some()
}

/// Evalúa un item en `d`. Devuelve `(valor, existió, basis, índice de observación de inicio
/// de segmento)`. El índice sirve para elegir los términos a devolver (cuota/apr).
fn prefill_eval_item(
    dates: &[NaiveDate],
    obs: &[Option<PrefillObs>],
    d: NaiveDate,
    is_liability: bool,
) -> (Decimal, bool, &'static str, Option<usize>) {
    let m = dates.len();
    // `dates[0]` es el primer snapshot real (la observación virtual «hoy» se añade al final).
    // Antes del primer snapshot: engancha al valor del primer snapshot si el item existía allí.
    if d < dates[0] {
        return match &obs[0] {
            Some(o) => (o.value, true, "first_snapshot", Some(0)),
            None => (Decimal::ZERO, false, "not_owned", None),
        };
    }

    // `d ∈ [dates[0], dates[m-1]]` (validado `d ≤ hoy`; el último punto es hoy o el último
    // snapshot real, ambos ≥ d). `a` = mayor índice con `dates[a] ≤ d`.
    let a = match dates.binary_search(&d) {
        Ok(idx) => idx,
        Err(idx) => idx - 1, // idx ≥ 1 porque dates[0] ≤ d
    };

    if a == m - 1 {
        // d coincide con el último punto: valor observado exacto, o `not_owned`.
        return match &obs[a] {
            Some(o) => (o.value, true, "interpolated", Some(a)),
            None => (Decimal::ZERO, false, "not_owned", None),
        };
    }

    let d_a = dates[a];
    let d_b = dates[a + 1];
    let days_total = (d_b - d_a).num_days();
    let days_from_start = (d - d_a).num_days();

    match (&obs[a], &obs[a + 1]) {
        (Some(lo), Some(ro)) => {
            // Términos de la observación de inicio (fallback a la final), como en la serie.
            // Activos → `None` → el motor interpola linealmente en días civiles.
            let terms = if is_liability {
                loan_terms_of(lo.apr_percent, lo.payment_amount, lo.payment_frequency.as_deref())
                    .or_else(|| {
                        loan_terms_of(ro.apr_percent, ro.payment_amount, ro.payment_frequency.as_deref())
                    })
            } else {
                None
            };
            let value =
                amortized_segment_value(lo.value, ro.value, terms.as_ref(), days_from_start, days_total);
            (value, true, "interpolated", Some(a))
        }
        // Observado solo a la izquierda: su valor exacto en su fecha, `not_owned` en el resto.
        (Some(lo), None) => {
            if d == d_a {
                (lo.value, true, "interpolated", Some(a))
            } else {
                (Decimal::ZERO, false, "not_owned", None)
            }
        }
        // Observado solo a la derecha (`d < d_b` en este segmento) o en ninguno → `not_owned`.
        (None, _) => (Decimal::ZERO, false, "not_owned", None),
    }
}

/// Elige los términos crudos a devolver para un pasivo: los de la observación de inicio de
/// segmento; si no, la observación con términos más cercana a `d`; si no, la fila viva; si no,
/// nada. (Los activos nunca llevan términos.)
fn prefill_pick_terms(
    dates: &[NaiveDate],
    obs: &[Option<PrefillObs>],
    seg_start: Option<usize>,
    d: NaiveDate,
    live: Option<&PrefillLiveRow>,
) -> (Option<Decimal>, Option<Decimal>, Option<String>) {
    if let Some(i) = seg_start {
        if let Some(o) = &obs[i] {
            if obs_has_terms(o) {
                return (o.apr_percent, o.payment_amount, o.payment_frequency.clone());
            }
        }
    }
    let mut best: Option<(i64, &PrefillObs)> = None;
    for (j, slot) in obs.iter().enumerate() {
        if let Some(o) = slot {
            if obs_has_terms(o) {
                let dist = (dates[j] - d).num_days().abs();
                if best.map_or(true, |(bd, _)| dist < bd) {
                    best = Some((dist, o));
                }
            }
        }
    }
    if let Some((_, o)) = best {
        return (o.apr_percent, o.payment_amount, o.payment_frequency.clone());
    }
    if let Some(l) = live {
        if l.apr_percent.is_some() || l.payment_amount.is_some() || l.payment_frequency.is_some() {
            return (l.apr_percent, l.payment_amount, l.payment_frequency.clone());
        }
    }
    (None, None, None)
}

#[utoipa::path(
    get,
    path = "/v1/history/snapshots/prefill",
    tag = "history",
    params(
        ("kind" = String, Query, description = "`asset` | `liability` (requerido)."),
        ("date" = String, Query, description = "`YYYY-MM-DD` (requerido; ≤ hoy civil, ≥ 1900-01-01)."),
    ),
    responses(
        (status = 200, description = "Valores pre-rellenados por item en `date` (own-user). Universo vacío → items []", body = PrefillResponse),
        (status = 400, description = "kind ausente/inválido (`invalid_kind`) o fecha ausente/futura/antigua (`snapshot_date_in_future`/`snapshot_date_too_old`)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn prefill_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<PrefillQuery>,
) -> Result<Json<PrefillResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    // Solo lectura: cualquier miembro (viewer incluido) puede pedir el prefill. Siempre own-user.
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;

    let kind = match &q.kind {
        Some(k) => normalize_kind(k)?,
        None => {
            return Err(ApiError::BadRequest(
                "invalid_kind: kind must be 'asset' or 'liability'".into(),
            ))
        }
    };
    let is_liability = kind == "liability";
    let d = q.date.ok_or_else(|| {
        ApiError::BadRequest("date is required and must be a valid YYYY-MM-DD date".into())
    })?;
    let today = installation_naive_today(&state.pool, iid).await?;
    validate_snapshot_date(d, today)?;

    // ---- Fetch own-user, single-kind (mismas queries que la serie, sin LedgerView) -----------
    let headers: Vec<PrefillHeaderRow> = sqlx::query_as(
        r#"SELECT id, snapshot_date
           FROM history_snapshots
           WHERE installation_id = $1 AND owner_user_id = $2 AND kind = $3
           ORDER BY snapshot_date ASC"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(&kind)
    .fetch_all(&state.pool)
    .await?;

    // Filas vivas propias no expiradas (assets o liabilities), normalizadas a `PrefillLiveRow`.
    let live_rows: Vec<PrefillLiveRow> = if is_liability {
        sqlx::query_as(
            r#"SELECT id, label, principal AS value, apr_percent, payment_amount, payment_frequency
               FROM liabilities
               WHERE installation_id = $1 AND owner_user_id = $2
                 AND (payment_end_date IS NULL OR payment_end_date >= $3)"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .bind(today)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(
            r#"SELECT id, name AS label, current_value AS value,
                      NULL::numeric AS apr_percent, NULL::numeric AS payment_amount,
                      NULL::text AS payment_frequency
               FROM assets
               WHERE installation_id = $1 AND owner_user_id = $2"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .fetch_all(&state.pool)
        .await?
    };

    // ---- Sin timeline (0 snapshots del kind) → universo = filas vivas, basis "live" ----------
    if headers.is_empty() {
        let mut items: Vec<PrefillItemResponse> = live_rows
            .into_iter()
            .map(|r| PrefillItemResponse {
                item_id: r.id,
                label: r.label,
                // Sugerencia display-grade: 2 decimales (la columna es NUMERIC(18,4) igualmente).
                value: r.value.round_dp(2),
                existed: true,
                basis: "live".into(),
                apr_percent: if is_liability { r.apr_percent } else { None },
                payment_amount: if is_liability { r.payment_amount } else { None },
                payment_frequency: if is_liability { r.payment_frequency } else { None },
            })
            .collect();
        sort_prefill_items(&mut items);
        return Ok(Json(PrefillResponse {
            date_ymd: d.format("%Y-%m-%d").to_string(),
            kind,
            items,
        }));
    }

    // ---- Con timeline: mismos items que la serie ---------------------------------------------
    let ids: Vec<Uuid> = headers.iter().map(|h| h.id).collect();
    let item_rows: Vec<SnapshotItemRow> = sqlx::query_as(
        r#"SELECT snapshot_id, source_item_id, label, value, apr_percent,
                  payment_amount, payment_frequency
           FROM history_snapshot_items
           WHERE snapshot_id = ANY($1)"#,
    )
    .bind(&ids)
    .fetch_all(&state.pool)
    .await?;

    let mut items_by_snapshot: HashMap<Uuid, Vec<SnapshotItemRow>> = HashMap::new();
    for r in item_rows {
        items_by_snapshot.entry(r.snapshot_id).or_default().push(r);
    }

    // Fechas ascendentes + observación virtual «hoy» salvo que el último snapshot real sea hoy.
    let mut dates: Vec<NaiveDate> = headers.iter().map(|h| h.snapshot_date).collect();
    let last_real = *dates.last().expect("headers non-empty");
    let append_virtual = last_real < today;
    let total_len = dates.len() + usize::from(append_virtual);

    let mut obs_map: BTreeMap<Uuid, Vec<Option<PrefillObs>>> = BTreeMap::new();
    // Nombre fallback: label del snapshot más reciente que contiene el item.
    let mut latest_label: HashMap<Uuid, (NaiveDate, String)> = HashMap::new();
    for (j, h) in headers.iter().enumerate() {
        let Some(items) = items_by_snapshot.get(&h.id) else {
            continue;
        };
        for it in items {
            let obs = obs_map
                .entry(it.source_item_id)
                .or_insert_with(|| Vec::from_iter(std::iter::repeat_with(|| None).take(total_len)));
            obs[j] = Some(PrefillObs {
                value: it.value,
                apr_percent: it.apr_percent,
                payment_amount: it.payment_amount,
                payment_frequency: it.payment_frequency.clone(),
            });
            let slot = latest_label
                .entry(it.source_item_id)
                .or_insert_with(|| (h.snapshot_date, it.label.clone()));
            if h.snapshot_date >= slot.0 {
                *slot = (h.snapshot_date, it.label.clone());
            }
        }
    }

    if append_virtual {
        dates.push(today);
        let last = total_len - 1;
        for r in &live_rows {
            let obs = obs_map
                .entry(r.id)
                .or_insert_with(|| Vec::from_iter(std::iter::repeat_with(|| None).take(total_len)));
            obs[last] = Some(PrefillObs {
                value: r.value,
                apr_percent: r.apr_percent,
                payment_amount: r.payment_amount,
                payment_frequency: r.payment_frequency.clone(),
            });
        }
    }

    // Unir filas vivas al universo aunque la virtual se saltara (snapshot de hoy): un item
    // vivo nunca-snapshoteado debe aparecer (con observaciones todo-`None` → `not_owned`).
    for r in &live_rows {
        obs_map
            .entry(r.id)
            .or_insert_with(|| Vec::from_iter(std::iter::repeat_with(|| None).take(total_len)));
    }

    let live_by_id: HashMap<Uuid, &PrefillLiveRow> =
        live_rows.iter().map(|r| (r.id, r)).collect();

    let mut items: Vec<PrefillItemResponse> = obs_map
        .iter()
        .map(|(id, obs)| {
            let (value, existed, basis, seg_start) = prefill_eval_item(&dates, obs, d, is_liability);
            let live = live_by_id.get(id).copied();
            let label = live
                .map(|l| l.label.clone())
                .or_else(|| latest_label.get(id).map(|(_, l)| l.clone()))
                .unwrap_or_default();
            let (apr_percent, payment_amount, payment_frequency) = if is_liability {
                prefill_pick_terms(&dates, obs, seg_start, d, live)
            } else {
                (None, None, None)
            };
            PrefillItemResponse {
                item_id: *id,
                label,
                // Sugerencia display-grade: 2 decimales (evita 16+ dígitos de la interpolación
                // Decimal en el input del formulario). Los términos se ecoan SIN redondear
                // (son observaciones copiadas, no valores computados).
                value: value.round_dp(2),
                existed,
                basis: basis.to_string(),
                apr_percent,
                payment_amount,
                payment_frequency,
            }
        })
        .collect();
    sort_prefill_items(&mut items);

    Ok(Json(PrefillResponse {
        date_ymd: d.format("%Y-%m-%d").to_string(),
        kind,
        items,
    }))
}

/// Orden de salida: `existed=true` primero, luego por `label ASC`. `sort_by` es estable, así que
/// los empates de `(existed, label)` conservan el orden por `source_item_id` del `BTreeMap`.
fn sort_prefill_items(items: &mut [PrefillItemResponse]) {
    items.sort_by(|a, b| {
        (!a.existed)
            .cmp(&!b.existed)
            .then_with(|| a.label.cmp(&b.label))
    });
}

pub fn history_router() -> Router {
    Router::new()
        .route("/snapshots/capture", post(capture_snapshots))
        .route("/snapshots/prefill", get(prefill_snapshot))
        .route("/snapshots", get(list_snapshots).post(create_snapshot))
        .route("/snapshots/{id}", put(update_snapshot).delete(delete_snapshot))
        .route("/series", get(get_history_series))
        .route("/cashflow", get(get_history_cashflow))
}
