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
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use futurefin_engine::{
    add_months_signed, evaluate_timeline, month_index_of, HistoryItem, HistoryItemKind,
    HistoryObservation, HistoryTimeline, LoanTerms,
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

#[derive(Debug, FromRow)]
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

#[derive(Debug, FromRow)]
struct SeriesHeaderRow {
    id: Uuid,
    owner_user_id: Uuid,
    kind: String,
    snapshot_date: NaiveDate,
}

#[derive(Debug, FromRow)]
struct LiveAssetRow {
    id: Uuid,
    name: String,
    current_value: Decimal,
    owner_user_id: Uuid,
}

#[derive(Debug, FromRow)]
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

    // ---- Fetch (4 queries, todas vía helpers LedgerView) ----------------------------------
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
        .bind_scope_as(sqlx::query_as(&headers_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;

    // 0 snapshots en scope → 200 con arrays vacíos.
    if headers.is_empty() {
        return Ok(Json(HistorySeriesResponse {
            anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
            anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
            view: view_label.into(),
            points: Vec::new(),
            asset_series: Vec::new(),
            markers: Vec::new(),
        }));
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
        .bind_scope_as(sqlx::query_as(&items_sql), iid, user.id.0)
        .fetch_all(&state.pool)
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
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, user.id.0)
        .fetch_all(&state.pool)
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
        .bind_scope_as(sqlx::query_as(&liabs_sql), iid, user.id.0)
        .bind(today)
        .fetch_all(&state.pool)
        .await?;

    // ---- Estructuras auxiliares ------------------------------------------------------------
    let mut items_by_snapshot: HashMap<Uuid, Vec<SnapshotItemRow>> = HashMap::new();
    for r in item_rows {
        items_by_snapshot.entry(r.snapshot_id).or_default().push(r);
    }
    let mut live_assets_by_owner: HashMap<Uuid, Vec<&LiveAssetRow>> = HashMap::new();
    for a in &live_assets {
        live_assets_by_owner.entry(a.owner_user_id).or_default().push(a);
    }
    let mut live_liabs_by_owner: HashMap<Uuid, Vec<&LiveLiabilityRow>> = HashMap::new();
    for l in &live_liabs {
        live_liabs_by_owner.entry(l.owner_user_id).or_default().push(l);
    }
    let live_asset_names: HashMap<Uuid, &str> =
        live_assets.iter().map(|a| (a.id, a.name.as_str())).collect();

    // ---- Markers: uno por cabecera; total = Σ items -----------------------------------------
    let markers: Vec<HistoryMarker> = headers
        .iter()
        .map(|h| {
            let total: Decimal = items_by_snapshot
                .get(&h.id)
                .map(|items| items.iter().map(|i| i.value).sum())
                .unwrap_or(Decimal::ZERO);
            let mi = month_index_of(h.snapshot_date, anchor);
            let month_fraction = mi as f64
                + (h.snapshot_date.day() as f64 - 1.0) / days_in_month_of(h.snapshot_date) as f64;
            HistoryMarker {
                date_ymd: h.snapshot_date.format("%Y-%m-%d").to_string(),
                month_index: mi,
                month_fraction,
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

    // ---- Timelines por (owner_user_id, kind) ------------------------------------------------
    // BTreeMap para iterar en orden determinista.
    let mut groups: BTreeMap<(Uuid, String), Vec<&SeriesHeaderRow>> = BTreeMap::new();
    for h in &headers {
        groups
            .entry((h.owner_user_id, h.kind.clone()))
            .or_default()
            .push(h);
    }

    let mut assets_total = vec![Decimal::ZERO; grid_len];
    let mut liabilities_total = vec![Decimal::ZERO; grid_len];
    // Serie por asset agrupada por source_item_id ENTRE usuarios (se suman los valores).
    let mut asset_values: HashMap<Uuid, Vec<Decimal>> = HashMap::new();
    // Nombre fallback: label del snapshot MÁS RECIENTE que contiene el item.
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
            let Some(items) = items_by_snapshot.get(&h.id) else {
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
                })
                .collect(),
        };

        // Cómputo puro sub-ms (decenas de snapshots × decenas de meses): deliberadamente
        // SIN `spawn_blocking` (no bloquea el runtime a esta escala) y SIN cache propia
        // (recalcular es más barato que invalidar bien).
        let evaluated = evaluate_timeline(&timeline, &grid)
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

    // ---- Agregación final --------------------------------------------------------------------
    let points: Vec<HistorySeriesPoint> = (0..grid_len)
        .map(|g| HistorySeriesPoint {
            month_index: k_min + g as i32,
            net_worth: assets_total[g] - liabilities_total[g],
            assets_total: assets_total[g],
            liabilities_total: liabilities_total[g],
        })
        .collect();

    let mut asset_series: Vec<HistoryAssetSeries> = asset_values
        .into_iter()
        .map(|(asset_id, values)| {
            // Nombre: el asset vivo gana; si no, el label del snapshot más reciente.
            let asset_name = live_asset_names
                .get(&asset_id)
                .map(|n| n.to_string())
                .or_else(|| latest_label.get(&asset_id).map(|(_, l)| l.clone()))
                .unwrap_or_default();
            HistoryAssetSeries {
                asset_id,
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

pub fn history_router() -> Router {
    Router::new()
        .route("/snapshots/capture", post(capture_snapshots))
        .route("/snapshots", get(list_snapshots).post(create_snapshot))
        .route("/snapshots/{id}", put(update_snapshot).delete(delete_snapshot))
        .route("/series", get(get_history_series))
}
