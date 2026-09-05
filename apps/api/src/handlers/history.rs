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
use crate::handlers::validate_window_months;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Modelo de amortización que tenía el pasivo al capturar la foto (#129, 4.7.0). `null` en
    /// items de activo y en snapshots anteriores (⇒ interpolación lineal, el default de su época).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repayment_model: Option<String>,
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
    /// Nº de items del snapshot **según la base de datos**, independiente de que `items` viaje o
    /// no. Con `items_included = false` es la única forma de saber si el snapshot tiene contenido.
    pub item_count: i64,
    /// `true` ⟺ `items` trae el detalle. Con `false`, `items` llega **vacío por supresión**, no
    /// porque el snapshot esté vacío.
    ///
    /// Hasta 4.4.0 la supresión del detalle (la que hace la tool MCP `list_snapshots` sin
    /// `include_items`) dejaba `items: []`, exactamente el mismo JSON que un snapshot sin ningún
    /// ítem. Un consumidor no podía distinguir «no te he mandado el detalle» de «aquí no hay
    /// nada», y `total` seguía siendo correcto, lo que hacía la contradicción aún más confusa
    /// (un total de 12.000 € con cero ítems).
    pub items_included: bool,
    /// Items ordenados por `label ASC`. Vacío si `items_included` es `false` — mira `item_count`.
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
    /// Modelo de amortización del pasivo EN AQUEL MOMENTO (#129). Opcional: ausente = no se
    /// sabe ⇒ interpolación lineal. Solo con kind `liability`.
    #[serde(default)]
    pub repayment_model: Option<String>,
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
    repayment_model: Option<String>,
}

/// Item ya validado y listo para insertar.
struct PreparedItem {
    item_id: Uuid,
    label: String,
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    repayment_model: Option<String>,
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
        return Err(ApiError::BadRequest("label_empty: label must not be empty".into()));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "label_too_long: label must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn normalize_frequency(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "monthly" => Ok("monthly".into()),
        "weekly" => Ok("weekly".into()),
        _ => Err(ApiError::BadRequest(
            "payment_frequency_invalid: payment_frequency must be monthly or weekly".into(),
        )),
    }
}

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("negative_amount: {field} must be >= 0")));
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
                    "payment_amount_not_positive: payment_amount must be > 0 when set".into(),
                ));
            }
        }
        let payment_frequency = match &it.payment_frequency {
            None => None,
            Some(f) => Some(normalize_frequency(f)?),
        };
        // #129: mismo dominio que el CHECK de la columna; «rechazar, no defaultear» (§2.6).
        let repayment_model = match it.repayment_model.as_deref() {
            None => None,
            Some(m @ ("fixed_payments" | "french" | "interest_only" | "revolving")) => {
                Some(m.to_string())
            }
            Some(other) => {
                return Err(ApiError::BadRequest(format!(
                    "snapshot_repayment_model_invalid: repayment_model must be one of fixed_payments, french, interest_only, revolving (got {other})"
                )))
            }
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
            repayment_model,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Shared DB helpers
// ---------------------------------------------------------------------------

fn build_response(h: SnapshotHeaderRow, items: Vec<SnapshotItemRow>) -> SnapshotResponse {
    build_response_with_items(h, items, true)
}

/// Igual que [`build_response`] pero pudiendo **suprimir** el detalle por ítem.
///
/// La supresión vive aquí, en la core, y no en el llamante: cuando la hacía la capa MCP borrando
/// `items` de la respuesta ya construida, `item_count`/`items_included` no existían y el resultado
/// era indistinguible de un snapshot vacío. `total` e `item_count` se calculan SIEMPRE sobre los
/// ítems reales, se manden o no.
fn build_response_with_items(
    h: SnapshotHeaderRow,
    items: Vec<SnapshotItemRow>,
    include_items: bool,
) -> SnapshotResponse {
    let total: Decimal = items.iter().map(|i| i.value).sum();
    let item_count = items.len() as i64;
    SnapshotResponse {
        id: h.id,
        kind: h.kind,
        snapshot_date_ymd: h.snapshot_date.format("%Y-%m-%d").to_string(),
        source: h.source,
        total,
        item_count,
        items_included: include_items,
        items: if include_items {
            items
                .into_iter()
                .map(|i| SnapshotItemResponse {
                    item_id: i.source_item_id,
                    label: i.label,
                    value: i.value,
                    apr_percent: i.apr_percent,
                    payment_amount: i.payment_amount,
                    payment_frequency: i.payment_frequency,
                    repayment_model: i.repayment_model,
                })
                .collect()
        } else {
            Vec::new()
        },
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
                  payment_amount, payment_frequency, repayment_model
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
                    apr_percent, payment_amount, payment_frequency, repayment_model)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(snapshot_id)
        .bind(it.item_id)
        .bind(&it.label)
        .bind(it.value)
        .bind(it.apr_percent)
        .bind(it.payment_amount)
        .bind(it.payment_frequency.as_deref())
        .bind(it.repayment_model.as_deref())
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
    let resp = capture_snapshots_core(&state.pool, iid, user.id.0, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `capture_snapshot`. Upsert por día
/// civil (recapturar el mismo día SOBRESCRIBE la foto con el ledger vivo). Contrato D12: los
/// snapshots no son inputs del engine → nunca invalida la cache de proyección.
pub(crate) async fn capture_snapshots_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    body: CaptureBody,
) -> Result<CaptureResponse, ApiError> {
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

    let today = installation_naive_today(pool, iid).await?;

    let mut tx = pool.begin().await?;
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
        .bind(user_id)
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
            // El snapshot es POR USUARIO: se filtra `owner_user_id = $3`, así que solo entran
            // las filas del capturador. Desde 5.0.0 la columna es NOT NULL (D14) y toda fila
            // tiene dueño, así que ya no hay «compartidas» que excluir.
            sqlx::query(
                r#"INSERT INTO history_snapshot_items
                       (snapshot_id, source_item_id, label, value,
                        apr_percent, payment_amount, payment_frequency, repayment_model)
                   SELECT $1, a.id, a.name, a.current_value, NULL, NULL, NULL, NULL
                   FROM assets a
                   WHERE a.installation_id = $2 AND a.owner_user_id = $3"#,
            )
            .bind(snapshot_id)
            .bind(iid)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        } else {
            // Pasivos con plan vivo o con saldo vivo (#145); se copian los términos.
            sqlx::query(
                r#"INSERT INTO history_snapshot_items
                       (snapshot_id, source_item_id, label, value,
                        apr_percent, payment_amount, payment_frequency, repayment_model)
                   SELECT $1, l.id, l.label, l.principal,
                          l.apr_percent, l.payment_amount, l.payment_frequency, l.repayment_model
                   FROM liabilities l
                   WHERE l.installation_id = $2 AND l.owner_user_id = $3
                     AND (l.payment_end_date IS NULL OR l.payment_end_date >= $4 OR l.principal > 0)"#,
            )
            .bind(snapshot_id)
            .bind(iid)
            .bind(user_id)
            .bind(today)
            .execute(&mut *tx)
            .await?;
        }

        out.push(load_snapshot_response(&mut *tx, snapshot_id).await?);
    }
    tx.commit().await?;

    // Nota: NO se invalida la cache de proyección (ver doc del módulo).
    Ok(CaptureResponse { snapshots: out })
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
    // Camino HTTP: conjunto entero, detalle incluido, sin `COUNT` — contrato REST intacto (mismo
    // patrón que `list_transactions`).
    let (out, _total) =
        list_snapshots_core(&state.pool, iid, user.id.0, q.year, q.kind.as_deref(), true, None, 0)
            .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_snapshots`. Siempre own-user
/// (el CRUD de snapshots no acepta `?view`).
///
/// **Paginación con el mismo contrato que `list_transactions_core`**: con `limit = None` (el
/// handler HTTP) no se emite `LIMIT`/`OFFSET` ni la query de `COUNT`; con `limit = Some(n)` (la
/// tool MCP) la paginación baja a SQL y `total_count` sale de un `COUNT(*)` con los mismos
/// filtros. Hasta 4.4.0 este listado no tenía cota ninguna: cada snapshot son dos fechas, un
/// total y N ítems, y un usuario que fotografía su patrimonio cada mes acumula uno por mes y kind
/// indefinidamente — crecía con el uso normal, igual que las reglas de categorización que ya
/// llevaban paginación desde 3.8.0.
///
/// `include_items = false` suprime el detalle por ítem **dentro de la core**, que es donde se puede
/// declarar la supresión (`items_included` / `item_count`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn list_snapshots_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    year: Option<i32>,
    kind: Option<&str>,
    include_items: bool,
    limit: Option<i64>,
    offset: i64,
) -> Result<(Vec<SnapshotResponse>, i64), ApiError> {
    if let Some(y) = year {
        if !(1900..=3000).contains(&y) {
            return Err(ApiError::BadRequest(
                "year_out_of_range: year must be between 1900 and 3000".into(),
            ));
        }
    }
    let kind = match kind {
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
    if year.is_some() {
        sql.push_str(&format!(
            " AND snapshot_date >= ${} AND snapshot_date < ${}",
            next,
            next + 1
        ));
        next += 2;
    }
    if kind.is_some() {
        sql.push_str(&format!(" AND kind = ${next}"));
        next += 1;
    }
    // Desempate por `id` para que la paginación sea estable: `snapshot_date DESC, kind ASC` deja
    // empatados los snapshots del mismo día y kind, y sin orden total dos páginas consecutivas
    // pueden repetir u omitir filas.
    sql.push_str(" ORDER BY snapshot_date DESC, kind ASC, id ASC");
    if limit.is_some() {
        sql.push_str(&format!(" LIMIT ${next} OFFSET ${}", next + 1));
    }

    let year_range = year.map(|y| {
        (
            NaiveDate::from_ymd_opt(y, 1, 1).expect("valid Jan 1"),
            NaiveDate::from_ymd_opt(y + 1, 1, 1).expect("valid next Jan 1"),
        )
    });

    let mut query = sqlx::query_as::<_, SnapshotHeaderRow>(&sql)
        .bind(iid)
        .bind(user_id);
    if let Some((start, end)) = year_range {
        query = query.bind(start).bind(end);
    }
    if let Some(k) = kind.clone() {
        query = query.bind(k);
    }
    if let Some(n) = limit {
        query = query.bind(n).bind(offset);
    }
    let headers: Vec<SnapshotHeaderRow> = query.fetch_all(pool).await?;

    // Sin `limit` el total ES la página: nos ahorramos el COUNT y el camino HTTP no cambia.
    let total_count: i64 = match limit {
        None => headers.len() as i64,
        Some(_) => {
            let mut count_sql = String::from(
                "SELECT COUNT(*) FROM history_snapshots \
                 WHERE installation_id = $1 AND owner_user_id = $2",
            );
            let mut cnext = 3;
            if year_range.is_some() {
                count_sql.push_str(&format!(
                    " AND snapshot_date >= ${} AND snapshot_date < ${}",
                    cnext,
                    cnext + 1
                ));
                cnext += 2;
            }
            if kind.is_some() {
                count_sql.push_str(&format!(" AND kind = ${cnext}"));
            }
            let mut cq = sqlx::query_scalar::<_, i64>(&count_sql).bind(iid).bind(user_id);
            if let Some((start, end)) = year_range {
                cq = cq.bind(start).bind(end);
            }
            if let Some(k) = kind {
                cq = cq.bind(k);
            }
            cq.fetch_one(pool).await?
        }
    };

    if headers.is_empty() {
        return Ok((Vec::new(), total_count));
    }

    let ids: Vec<Uuid> = headers.iter().map(|h| h.id).collect();
    let item_rows: Vec<SnapshotItemRow> = sqlx::query_as(
        r#"SELECT snapshot_id, source_item_id, label, value, apr_percent,
                  payment_amount, payment_frequency, repayment_model
           FROM history_snapshot_items
           WHERE snapshot_id = ANY($1)
           ORDER BY label ASC"#,
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let mut by_parent: HashMap<Uuid, Vec<SnapshotItemRow>> = HashMap::new();
    for r in item_rows {
        by_parent.entry(r.snapshot_id).or_default().push(r);
    }

    let page: Vec<SnapshotResponse> = headers
        .into_iter()
        .map(|h| {
            let items = by_parent.remove(&h.id).unwrap_or_default();
            build_response_with_items(h, items, include_items)
        })
        .collect();
    Ok((page, total_count))
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

    let resp = create_snapshot_core(&state.pool, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_snapshot` (backfill).
///
/// **Grabar el pasado es lo que una conversación hace mejor que un formulario** («en enero de 2023
/// tenía 40.000 € en el fondo»), y por eso esta core existe: la validación —`normalize_kind`,
/// `validate_snapshot_date` contra el HOY civil de la instalación, los bounds por ítem y el 409 de
/// `(usuario, kind, fecha)`— tiene que ser la misma por los dos caminos. La fecha ocupada llega
/// como unique-violation sobre `history_snapshots_unique_per_day` y la mapea `From<sqlx::Error>`;
/// aquí nadie inspecciona el SQLSTATE.
///
/// **Cache NONE (contrato D12)**: los snapshots no son inputs del engine, así que esta core NO
/// llama a `refresh_projection_after_mutation`. La ausencia está fijada por
/// `snapshot_mutations_do_not_touch_projection_cache`.
pub(crate) async fn create_snapshot_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    body: CreateSnapshotBody,
) -> Result<SnapshotResponse, ApiError> {
    let kind = normalize_kind(&body.kind)?;
    let is_liability = kind == "liability";
    let today = installation_naive_today(pool, iid).await?;
    validate_snapshot_date(body.snapshot_date, today)?;
    let items = validate_and_prepare_items(&body.items, is_liability)?;

    let mut tx = pool.begin().await?;
    // Fecha ocupada → unique-violation sobre history_snapshots_unique_per_day → 409
    // (mapeado en From<sqlx::Error>; el handler no inspecciona el SQLSTATE).
    let snapshot_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO history_snapshots
               (installation_id, owner_user_id, kind, snapshot_date, source)
           VALUES ($1, $2, $3, $4, 'backfill')
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(&kind)
    .bind(body.snapshot_date)
    .fetch_one(&mut *tx)
    .await?;

    insert_items(&mut tx, snapshot_id, &items).await?;
    let resp = load_snapshot_response(&mut tx, snapshot_id).await?;
    tx.commit().await?;

    // Nota: NO se invalida la cache de proyección (ver doc del módulo).
    Ok(resp)
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

    let resp = update_snapshot_core(&state.pool, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PUT y la tool MCP `update_snapshot`.
///
/// Dos reglas que no se pueden reimplementar por fuera sin romper algo:
/// - **`kind` es inmutable**: los términos por ítem (`apr_percent`/`payment_*`) solo son válidos en
///   `liability`, y se validan contra el kind YA guardado, no contra uno que venga en el body.
/// - **`items` ausente conserva; presente (incluso `[]`) reemplaza**. Un PUT que solo mueve la
///   fecha no puede vaciar el snapshot.
///
/// Mover a una fecha ocupada es una unique-violation → 409, igual que en el alta.
///
/// **Cache NONE (contrato D12)**, misma razón que [`create_snapshot_core`].
pub(crate) async fn update_snapshot_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: UpdateSnapshotBody,
) -> Result<SnapshotResponse, ApiError> {
    // Guardia id + installation + owner: si no es tuyo, 404 (no revelar existencia).
    let existing: Option<SnapshotHeaderRow> = sqlx::query_as(
        r#"SELECT id, kind, snapshot_date, source, created_at, updated_at
           FROM history_snapshots
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some(existing) = existing else {
        return Err(ApiError::NotFound);
    };

    // `kind` es inmutable: los términos permitidos dependen del kind existente.
    let is_liability = existing.kind == "liability";
    let new_date = body.snapshot_date.unwrap_or(existing.snapshot_date);
    let today = installation_naive_today(pool, iid).await?;
    validate_snapshot_date(new_date, today)?;
    // `items` ausente (`None`) → conservar los items existentes; presente (incluso `[]`) →
    // reemplazo completo. Un PUT con solo `snapshot_date` ya no borra los items.
    let items = match &body.items {
        Some(list) => Some(validate_and_prepare_items(list, is_liability)?),
        None => None,
    };

    let mut tx = pool.begin().await?;
    // Mover a una fecha ocupada → unique-violation → 409. `source` intacto.
    sqlx::query(
        r#"UPDATE history_snapshots
           SET snapshot_date = $1, updated_at = now()
           WHERE id = $2 AND installation_id = $3 AND owner_user_id = $4"#,
    )
    .bind(new_date)
    .bind(id)
    .bind(iid)
    .bind(user_id)
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

    // Nota: NO se invalida la cache de proyección (ver doc del módulo).
    Ok(resp)
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

    delete_snapshot_core(&state.pool, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_snapshot`. Los items
/// caen en cascada; NUNCA invalida la cache (contrato D12 del módulo).
pub(crate) async fn delete_snapshot_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res = sqlx::query(
        r#"DELETE FROM history_snapshots
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Serie histórica interpolada (`GET /v1/history/series`)
// ---------------------------------------------------------------------------

/// Punto de la serie histórica agregada. `month_index ≤ 0`, contiguo `k_min..=0`.
/// Los numéricos por punto se serializan como **f64 redondeado a 2 decimales** (misma excepción
/// chart-only que los arrays de `/v1/projection/series` — D4/I3 del architecture contract — más
/// el recorte de publicación de [`CHART_DP`]).
#[derive(Debug, Serialize, ToSchema)]
pub struct HistorySeriesPoint {
    pub month_index: i32,
    /// Patrimonio neto histórico (`assets_total − liabilities_total`), **o `null`**.
    ///
    /// Es `null` en TODA la serie cuando `liabilities_snapshotted == false`: sin el pasivo
    /// fotografiado entero, `liabilities_total` vale 0 (o solo parte de la deuda) y la resta no
    /// sería un patrimonio neto, sino el total de activos con nombre de patrimonio neto. Se
    /// publicaba como número y coincidía exactamente con `assets_total`, así que un cliente
    /// obtenía dos patrimonios distintos —éste y el de `GET /v1/summary`— sin nada que le dijera
    /// cuál mirar. Con `null` la cifra equivocada deja de ser dable: quien la quiera tiene que
    /// pasar por el flag. `assets_total` y `liabilities_total` se publican igual que siempre.
    ///
    /// **Nunca se omite**: viaja como `null` explícito para que «no lo sé» no se confunda con
    /// «campo ausente en una versión vieja».
    // `required` explícito: `Option<T>` saldría de `required` por defecto y el contrato diría
    // «puede faltar», que es justo lo que este campo NO hace. Es nullable **y** obligatorio.
    #[serde(serialize_with = "serialize_opt_decimal_as_chart_f64")]
    #[schema(value_type = Option<f64>, required = true)]
    pub net_worth: Option<Decimal>,
    #[serde(serialize_with = "serialize_decimal_as_chart_f64")]
    #[schema(value_type = f64)]
    pub assets_total: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_chart_f64")]
    #[schema(value_type = f64)]
    pub liabilities_total: Decimal,
}

/// Decimales de publicación de las series de chart del histórico: **2**.
///
/// Los valores nacen de una interpolación (`(v1 − v0) · días/días` para los activos, la
/// amortización francesa para los pasivos), así que arrastraban la escala completa de
/// `rust_decimal` hasta el f64 y de ahí al JSON: `78012.333333333333333333333` ocupa 25 caracteres
/// por punto y ~290 puntos × (activos + 3 totales) los multiplica. Ningún consumidor los usa a esa
/// precisión — el chart posiciona píxeles y un agente cita euros —, así que la única función de
/// los otros trece decimales es gastar ventana de contexto y sugerir una exactitud que la
/// interpolación no tiene.
///
/// Es redondeo de **publicación**, igual que `money_out` y `round_ratio`: la interpolación y el
/// anclaje siguen calculándose exactos y solo se recorta la copia que se serializa.
const CHART_DP: u32 = 2;

/// Decimales de `month_fraction`. 4 decimales = 1/10.000 de mes ≈ 4 minutos; la rejilla más fina
/// que existe es diaria (1/31 ≈ 0,032). Lo que sobra es ruido de la división en f64
/// (`0.4838709677419355`).
const MONTH_FRACTION_DP: f64 = 10_000.0;

/// `Decimal` → f64 de chart, ya recortado a [`CHART_DP`].
fn chart_f64(d: Decimal) -> f64 {
    d.round_dp(CHART_DP).to_f64().unwrap_or(0.0)
}

/// Redondeo de publicación de `month_fraction` (ver [`MONTH_FRACTION_DP`]).
fn round_month_fraction(f: f64) -> f64 {
    (f * MONTH_FRACTION_DP).round() / MONTH_FRACTION_DP
}

/// `Decimal` → f64 recortado a [`CHART_DP`]. Excepción chart-only D4/I3: solo para arrays de
/// chart, jamás para un KPI escalar.
fn serialize_decimal_as_chart_f64<S: serde::Serializer>(
    d: &Decimal,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_f64(chart_f64(*d))
}

/// `Option<Decimal>` → f64 recortado a [`CHART_DP`] o `null` explícito. Misma excepción chart-only
/// (D4/I3). `serialize_none` emite `null`, no omite el campo — el punto SIEMPRE lleva `net_worth`.
fn serialize_opt_decimal_as_chart_f64<S: serde::Serializer>(
    d: &Option<Decimal>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match d {
        Some(v) => s.serialize_f64(chart_f64(*v)),
        None => s.serialize_none(),
    }
}

/// Serie histórica por asset, agregada entre usuarios. **Una por activo, no una por item de
/// snapshot**: los items se resuelven antes a una identidad común ([`resolve_item_identity`]).
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryAssetSeries {
    /// Id canónico del item: el del **asset vivo** cuando el nombre lo identifica sin ambigüedad
    /// (es la clave por la que el chart junta pasado y futuro), y si no un `source_item_id` del
    /// grupo — serie solo-histórica, que por diseño no está en `/v1/assets`.
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub asset_name: String,
    /// Valores f64 paralelos a `points`, redondeados a 2 decimales ([`CHART_DP`]).
    pub values: Vec<f64>,
}

/// Marcador de snapshot: uno por cabecera en scope.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryMarker {
    pub date_ymd: String,
    pub month_index: i32,
    /// Posición x fraccional del marcador dentro de la rejilla de `points`:
    /// `month_index + (día − 1) / días_del_mes`, redondeada a 4 decimales
    /// ([`MONTH_FRACTION_DP`]). Sirve para dibujar el marcador **entre** dos puntos mensuales sin
    /// que el consumidor tenga que saber cuántos días tiene ese mes; `month_index` a secas lo
    /// pegaría al día 1.
    pub month_fraction: f64,
    /// `asset` | `liability`.
    pub kind: String,
    /// **De dónde sale este snapshot**: `capture` (foto que la app tomó de los activos/pasivos
    /// vivos en esa fecha) | `backfill` (valores que el usuario tecleó a posteriori para una fecha
    /// pasada).
    ///
    /// No es cosmético. Un `backfill` puede estar en CUALQUIER fecha pasada, y un hogar que ancla
    /// su histórico muy atrás —hasta su propia fecha de nacimiento— genera cientos de puntos de
    /// interpolación entre ese ancla y el primer dato real. Sin este campo, ese ancla se presenta
    /// entre los markers exactamente igual que una foto tomada por la app, y a la pregunta
    /// «¿cuándo empecé a ahorrar?» la serie contesta con la fecha del ancla. Con `source` y
    /// `total` a la vista, un backfill de importe ~0 en una fecha remota se reconoce por lo que es.
    pub source: String,
    #[schema(value_type = String, format = "uuid")]
    pub owner_user_id: Uuid,
    /// Σ de los `value` de los items del snapshot.
    #[serde(serialize_with = "serialize_decimal_as_chart_f64")]
    #[schema(value_type = f64)]
    pub total: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HistorySeriesResponse {
    /// Hoy civil de la instalación (`installation.calendar_tz`).
    pub anchor_date_ymd: String,
    /// Primero-de-mes de `anchor_date_ymd` — la **etiqueta de mes** del punto `month_index = 0`.
    /// Ese punto se **evalúa** en `anchor_date_ymd` (hoy), no aquí: los meses pasados se evalúan en
    /// su día 1, y el mes en curso, que está a medias, en hoy — así el último punto empalma con el
    /// patrimonio vivo y cuadra con `GET /v1/summary`.
    pub anchor_month_first_ymd: String,
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view`.
    pub view: &'static str,
    /// **Ventana efectivamente emitida**, en meses hacia atrás desde el mes 0 (`points.len()` es
    /// como mucho `window_months + 1`). Eco del `window_months` pedido o, si se omitió, del
    /// default [`DEFAULT_HISTORY_WINDOW_MONTHS`].
    ///
    /// Hasta 4.4.0 omitir el parámetro devolvía la serie **desde el snapshot más antiguo del
    /// scope**, y nada en la respuesta lo decía. Un hogar que hubiera anclado su histórico muy
    /// atrás recibía ~290 puntos —los primeros doscientos interpolando entre 0 € y unos cientos—
    /// en el default, es decir en el peor caso posible. Ahora el default está acotado y la
    /// respuesta declara qué ventana usó.
    pub window_months: u32,
    /// `true` ⟺ hay snapshots **anteriores** a la ventana emitida: la serie está recortada y
    /// existe más histórico. Pídelo con `window_months` mayor (máximo 1200, que en la práctica es
    /// «todo»). Con `false`, lo que ves es todo lo que hay.
    pub window_truncated: bool,
    /// Fecha del snapshot **más antiguo del scope**, `YYYY-MM-DD` — esté dentro o fuera de la
    /// ventana. `null` ⟺ no hay ningún snapshot. Junto a `window_truncated` responde «¿desde
    /// cuándo hay datos?» sin obligar a repetir la llamada con la ventana máxima.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_snapshot_date_ymd: Option<String>,
    /// El mismo snapshot expresado en la rejilla de `points` (`≤ 0`; menor que `-window_months`
    /// cuando `window_truncated` es `true`). `null` ⟺ no hay ninguno.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_snapshot_month_index: Option<i32>,
    /// Puntos contiguos ascendentes `k_min..=0` (incluye el mes 0). Vacío sin snapshots.
    pub points: Vec<HistorySeriesPoint>,
    /// Series por asset (solo kind `asset`), orden `asset_name ASC, asset_id ASC`.
    pub asset_series: Vec<HistoryAssetSeries>,
    /// Un marcador por snapshot en scope.
    pub markers: Vec<HistoryMarker>,
    /// `true` ⟺ el pasivo del scope está fotografiado **entero** (ver
    /// `liabilities_fully_snapshotted`): hay al menos un snapshot y **todos** los usuarios que
    /// aportan serie tienen alguna cabecera de kind `liability`.
    ///
    /// Con `false`, `points[].liabilities_total` es 0 (o solo parte de la deuda) **por ausencia de
    /// datos**, no porque no haya deuda: los timelines se agrupan a partir de los snapshots
    /// existentes, y un kind sin cabecera no tiene timeline ni fallback a las filas vivas. Sin este
    /// flag, «no lo he fotografiado» y «no debo nada» son indistinguibles, y el `net_worth`
    /// histórico de alguien con hipoteca se leía como si no la tuviera (auditoría MCP §2).
    ///
    /// **Es el interruptor de `points[].net_worth`**: `net_worth == null` ⟺
    /// `liabilities_snapshotted == false`. Un solo invariante, comprobable de un vistazo. La deuda
    /// viva está en `GET /v1/liabilities` y en `GET /v1/summary`.
    pub liabilities_snapshotted: bool,
}

#[derive(Debug, Clone, FromRow)]
struct SeriesHeaderRow {
    id: Uuid,
    owner_user_id: Uuid,
    kind: String,
    snapshot_date: NaiveDate,
    /// `capture` | `backfill`. Solo lo consume el marker: la interpolación no distingue.
    source: String,
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
    /// Nombre vivo del pasivo. Solo lo consume la resolución de identidad
    /// ([`resolve_item_identity`]); la interpolación nunca mira etiquetas.
    label: String,
    principal: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    repayment_model: String,
    owner_user_id: Uuid,
}

/// Términos del préstamo de una observación. Sin apr **y** cuota → `None` (el motor
/// interpola linealmente, mismo resultado que unos términos degenerados). La conversión
/// `weekly → ×52/12` vive en `projection::monthly_payment_from` (fuente única).
///
/// `repayment_model` (#129): el literal capturado en el snapshot. `None` o un literal corrupto
/// degradan a `None` — el motor lo lee como «no lo sé» ⇒ ley lineal, el default de la época
/// pre-4.7.0 (misma filosofía de degradar-en-lecturas que `projection.rs`).
fn loan_terms_of(
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    frequency: Option<&str>,
    repayment_model: Option<&str>,
) -> Option<LoanTerms> {
    match (apr_percent, payment_amount) {
        (Some(apr), Some(pay)) => Some(LoanTerms {
            apr_percent: apr,
            monthly_payment: crate::handlers::projection::monthly_payment_from(pay, frequency),
            repayment_model: repayment_model
                .and_then(|m| crate::handlers::liabilities::RepaymentModel::parse(m).ok())
                .map(crate::handlers::liabilities::RepaymentModel::to_engine),
        }),
        _ => None,
    }
}

/// ¿Está el pasivo del scope fotografiado **entero**?
///
/// `true` ⟺ hay alguna cabecera en scope **y** todos los usuarios que aportan serie (los que
/// tienen alguna cabecera, de cualquier kind) tienen además alguna de kind `liability`.
///
/// **No es un `any`**, y la diferencia solo se ve en el hogar: con Alice fotografiando su hipoteca
/// y Bob sin fotografiar la suya, `any` daría `true` y el agregado restaría **media deuda** —
/// exactamente el error que este flag existe para impedir, pero más difícil de detectar, porque
/// la cifra ya no coincide con `assets_total` y parece un patrimonio neto de verdad. `all` es el
/// único predicado bajo el que `assets_total − liabilities_total` es realmente el neto del scope.
///
/// Un usuario **sin deuda** no queda condenado a que su hogar no tenga neto histórico: declara la
/// ausencia capturando un snapshot de pasivo, que escribe la cabecera aunque no haya ni una fila
/// viva (`capture_snapshots_core` hace el upsert antes de copiar items). Eso es un hecho afirmado
/// por el usuario, no una ausencia interpretada por el servidor — que es justo la distinción que
/// el flag defiende.
///
/// Scope vacío → `false`: sin snapshots no hay nada fotografiado (y `points` va vacío, así que la
/// nulabilidad de `net_worth` no llega a observarse).
fn liabilities_fully_snapshotted(headers: &[SeriesHeaderRow]) -> bool {
    let mut owners: HashSet<Uuid> = HashSet::new();
    let mut with_liability: HashSet<Uuid> = HashSet::new();
    for h in headers {
        owners.insert(h.owner_user_id);
        if h.kind == "liability" {
            with_liability.insert(h.owner_user_id);
        }
    }
    !owners.is_empty() && owners.iter().all(|o| with_liability.contains(o))
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
        "SELECT s.id, s.owner_user_id, s.kind, s.snapshot_date, s.source
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
                i.apr_percent, i.payment_amount, i.payment_frequency, i.repayment_model
         FROM history_snapshot_items i
         JOIN history_snapshots s ON s.id = i.snapshot_id
         WHERE {i_scope}"
    );
    let item_rows: Vec<SnapshotItemRow> = view
        .bind_scope_as(sqlx::query_as(&items_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // 3) Assets vivos del scope. El filtro extra `owner_user_id IS NOT NULL` que había aquí
    //    —«las filas compartidas nunca participan en el histórico»— se retiró en 5.0.0: la
    //    migración `20260902200100_ledger_owner_not_null.sql` (D14) asignó las filas legadas al
    //    owner más antiguo y dejó la columna `NOT NULL`, así que el predicado era una tautología
    //    que se leía como una regla viva.
    let a_scope = view.scope_where("a");
    let assets_sql = format!(
        "SELECT a.id, a.name, a.current_value, a.owner_user_id
         FROM assets a
         WHERE {a_scope}"
    );
    let live_assets: Vec<LiveAssetRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // 4) Pasivos del scope con plan vivo o saldo vivo (#145). Mismo caso que los activos: el
    //    `owner_user_id IS NOT NULL` murió con la migración de D14.
    let l_scope = view.scope_where("l");
    let l_today_arg = view.next_arg_index();
    let liabs_sql = format!(
        "SELECT l.id, l.label, l.principal, l.apr_percent, l.payment_amount,
                l.payment_frequency, l.repayment_model, l.owner_user_id
         FROM liabilities l
         WHERE {l_scope}
           AND (l.payment_end_date IS NULL OR l.payment_end_date >= ${l_today_arg} OR l.principal > 0)"
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
/// (agrupada por id canónico entre usuarios) + el label más reciente de cada asset.
struct SeriesAccumulation {
    assets_total: Vec<Decimal>,
    liabilities_total: Vec<Decimal>,
    /// Id canónico ([`resolve_item_identity`]) → valores paralelos a la rejilla (suma entre
    /// usuarios). Con el id del activo VIVO cuando el nombre lo identifica sin ambigüedad.
    asset_values: HashMap<Uuid, Vec<Decimal>>,
    /// Id canónico → (fecha, label) del snapshot más reciente que lo contiene (fallback name).
    latest_label: HashMap<Uuid, (NaiveDate, String)>,
}

/// Clave de comparación de una etiqueta de item (NO la de almacenamiento: `normalize_label` sigue
/// mandando en la escritura). Solo la usa [`resolve_item_identity`]: recorta y pliega mayúsculas
/// para que «Cuenta corriente» y «cuenta corriente» sean el mismo item.
fn identity_label_key(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Resuelve la IDENTIDAD de los items de snapshot de un grupo `(owner, kind)`:
/// `source_item_id` → id canónico. Solo devuelve las entradas que MUEVEN algo (un id que se
/// queda como está no aparece), así que un mapa vacío significa «no había nada que resolver».
///
/// **Por qué existe.** `source_item_id` es la clave de identidad ENTRE snapshots, pero el
/// servidor la genera (`Uuid::new_v4`, `validate_and_prepare_items`) cuando el cliente no la
/// manda — y la tool MCP `create_snapshot` ni siquiera la expone, así que N fotos de la misma
/// cuenta llegan como N items distintos. Sin resolver, cada uno es un timeline propio: el LOCF
/// de #130 los APILA (con N snapshots el total se multiplica por N y cae a 0 en el punto vivo,
/// donde todos cuentan como borrados) y el chart pinta N líneas homónimas cuya leyenda no
/// empalma con ningún activo vivo. El síntoma visible eran 25 `asset_series` para 5 activos.
///
/// **Regla**, aplicada siempre DENTRO del grupo (jamás entre usuarios ni entre kinds):
/// 1. Los items se agrupan por su etiqueta más reciente ([`identity_label_key`]).
/// 2. Un grupo cuya etiqueta case con EXACTAMENTE una fila viva del owner se canoniza al id de
///    esa fila — así el histórico empalma con la observación virtual «hoy» y el `asset_id` que
///    publica la serie es el del activo vivo, que es por el que junta el chart. Si no casa
///    ninguna (activo borrado, backfill libre), al MENOR `source_item_id` del grupo:
///    determinista, y sigue siendo una clave que existió en los datos.
/// 3. **Ambigüedad ⇒ no se toca nada.** Si dos items del MISMO snapshot cayeran en la misma
///    identidad, esa identidad se disuelve entera (cada id vuelve a ser el suyo). Dos filas con
///    el mismo nombre en una foto son dos cosas distintas que el usuario llamó igual: fusionarlas
///    perdería una de las dos observaciones, y perder una observación es perder dinero.
///
/// Con datos bien formados —el camino de la SPA, que reenvía el `item_id` del prefill— cada
/// grupo de etiqueta tiene un solo id y esta función devuelve un mapa vacío: **no-op bit a bit**.
fn resolve_item_identity(
    scope: &HistoryScope,
    group_headers: &[&SeriesHeaderRow],
    live_rows: &[(Uuid, &str)],
) -> HashMap<Uuid, Uuid> {
    // 1) Etiqueta más reciente de cada `source_item_id` del grupo (misma regla que el nombre que
    //    publica la serie: gana el snapshot más reciente que contiene el item).
    let mut latest: BTreeMap<Uuid, (NaiveDate, String)> = BTreeMap::new();
    for h in group_headers {
        let Some(items) = scope.items_by_snapshot.get(&h.id) else {
            continue;
        };
        for it in items {
            let slot = latest
                .entry(it.source_item_id)
                .or_insert_with(|| (h.snapshot_date, it.label.clone()));
            if h.snapshot_date >= slot.0 {
                *slot = (h.snapshot_date, it.label.clone());
            }
        }
    }

    // 2) Cubos por etiqueta y filas vivas indexadas por nombre (solo las inequívocas: dos activos
    //    vivos que se llamen igual no permiten decidir a cuál pertenece la foto).
    let mut buckets: BTreeMap<String, BTreeSet<Uuid>> = BTreeMap::new();
    for (id, (_, label)) in &latest {
        buckets
            .entry(identity_label_key(label))
            .or_default()
            .insert(*id);
    }
    let live_ids: HashSet<Uuid> = live_rows.iter().map(|(id, _)| *id).collect();
    let mut live_by_label: HashMap<String, Option<Uuid>> = HashMap::new();
    for (id, label) in live_rows {
        live_by_label
            .entry(identity_label_key(label))
            .and_modify(|slot| *slot = None)
            .or_insert(Some(*id));
    }

    let mut canonical: HashMap<Uuid, Uuid> = HashMap::new();
    for (label, ids) in &buckets {
        let live_in_bucket: Vec<Uuid> =
            ids.iter().copied().filter(|id| live_ids.contains(id)).collect();
        let canon = match live_in_bucket.as_slice() {
            // Una foto ya traía el id vivo: ese manda (es el que el chart junta).
            [only] => *only,
            [] => match live_by_label.get(label) {
                Some(Some(live_id)) => *live_id,
                // Sin fila viva que case: serie solo-histórica, una por etiqueta.
                _ => *ids.iter().next().expect("bucket no vacío"),
            },
            // Dos filas vivas distintas fotografiadas bajo la misma etiqueta: ambiguo, no se toca.
            _ => continue,
        };
        for id in ids {
            if *id != canon {
                canonical.insert(*id, canon);
            }
        }
    }

    // 3) Disolución por colisión. Converge: cada vuelta retira al menos una entrada del mapa (dos
    //    items del mismo snapshot no pueden ser AMBOS identidad — sus `source_item_id` son únicos
    //    por el UNIQUE (snapshot_id, source_item_id)), y el mapa es finito.
    while !canonical.is_empty() {
        let mut clash: HashSet<Uuid> = HashSet::new();
        for h in group_headers {
            let Some(items) = scope.items_by_snapshot.get(&h.id) else {
                continue;
            };
            let mut seen: HashSet<Uuid> = HashSet::with_capacity(items.len());
            for it in items {
                let canon = canonical
                    .get(&it.source_item_id)
                    .copied()
                    .unwrap_or(it.source_item_id);
                if !seen.insert(canon) {
                    clash.insert(canon);
                }
            }
        }
        if clash.is_empty() {
            break;
        }
        canonical.retain(|_, canon| !clash.contains(canon));
    }

    canonical
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
        //
        // Desde 4.0.0 esto pasó de conveniente a IMPRESCINDIBLE. La rejilla de la serie evalúa su
        // último punto en `today` (ver `history_series_core`), así que esta observación es el ancla
        // de ese punto: con ella, `evaluate_item_at` cae en `a == m-1` y devuelve el valor vivo
        // exacto. Sin ella, `e > dates[m-1]` llevaría a la rama «tras el último snapshot: 0» y el
        // punto 0 de la serie valdría CERO. No la quites sin tocar también la rejilla.
        let append_virtual = last_real < today;
        let total_len = dates.len() + usize::from(append_virtual);

        // Identidad de los items ANTES de montar los timelines: N fotos de la misma cuenta
        // llegan con N `source_item_id` distintos cuando el cliente no manda `item_id`, y sin
        // resolverlas cada una sería un timeline propio que se apila sobre las demás. Mapa vacío
        // (datos bien formados) ⇒ ruta idéntica bit a bit a la anterior. Ver
        // [`resolve_item_identity`].
        let live_labeled: Vec<(Uuid, &str)> = match kind {
            HistoryItemKind::Asset => live_assets_by_owner
                .get(owner_id)
                .into_iter()
                .flatten()
                .map(|a| (a.id, a.name.as_str()))
                .collect(),
            HistoryItemKind::Liability => live_liabs_by_owner
                .get(owner_id)
                .into_iter()
                .flatten()
                .map(|l| (l.id, l.label.as_str()))
                .collect(),
        };
        let identity = resolve_item_identity(scope, group_headers, &live_labeled);
        let canonical_id = |src: Uuid| identity.get(&src).copied().unwrap_or(src);

        let mut obs_map: BTreeMap<Uuid, Vec<Option<HistoryObservation>>> = BTreeMap::new();
        for (j, h) in group_headers.iter().enumerate() {
            let Some(items) = scope.items_by_snapshot.get(&h.id) else {
                continue;
            };
            for it in items {
                let item_id = canonical_id(it.source_item_id);
                let obs = obs_map
                    .entry(item_id)
                    .or_insert_with(|| vec![None; total_len]);
                // `resolve_item_identity` disuelve toda identidad que colisionara dentro de un
                // snapshot, así que aquí nunca se pisa una observación (pisarla sería perder
                // dinero en silencio: el total de la foto dejaría de cuadrar).
                debug_assert!(obs[j].is_none(), "dos items del mismo snapshot en una identidad");
                obs[j] = Some(HistoryObservation {
                    value: it.value,
                    terms: loan_terms_of(
                        it.apr_percent,
                        it.payment_amount,
                        it.payment_frequency.as_deref(),
                        it.repayment_model.as_deref(),
                    ),
                });
                if kind == HistoryItemKind::Asset {
                    let slot = latest_label
                        .entry(item_id)
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
                            // Observación virtual «hoy» = ledger vivo ⇒ el modelo es el ACTUAL.
                            terms: loan_terms_of(
                                l.apr_percent,
                                l.payment_amount,
                                l.payment_frequency.as_deref(),
                                Some(l.repayment_model.as_str()),
                            ),
                        });
                    }
                }
            }
        }

        let timeline = HistoryTimeline {
            dates,
            // #130: el último punto es el ledger vivo ⟺ se añadió la observación virtual «hoy».
            // Solo esa ausencia significa borrado/vendido; en una captura intermedia, un item
            // ausente arrastra su último valor (LOCF).
            last_is_live_ledger: append_virtual,
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
            .map_err(|e| ApiError::BadRequest(format!("history_timeline_invalid: {e}")))?;

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

/// Query de `GET /v1/history/series`. `window_months` e `include_asset_series` llegaron con la
/// revisión de verbosidad MCP (issue #2) y son aditivos: omitidos, la respuesta es idéntica a la
/// histórica (toda la serie + series por activo).
#[derive(Debug, Deserialize)]
pub struct HistorySeriesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Limita la rejilla emitida a los últimos N meses (1..=1200; fuera de rango es 400
    /// `window_months_out_of_range`, NO se clampa). La interpolación sigue anclándose en TODOS los
    /// snapshots; solo se recortan los puntos/markers devueltos.
    ///
    /// **Omitido = [`DEFAULT_HISTORY_WINDOW_MONTHS`]**, no «todo». Para pedir todo el histórico,
    /// `window_months=1200` (el máximo; nada puede haber más atrás porque el tope de la ventana ES
    /// el tope del producto). La respuesta declara la ventana usada y si recortó algo
    /// (`window_months`, `window_truncated`, `first_snapshot_date_ymd`).
    #[serde(default)]
    pub window_months: Option<i64>,
    /// `false` omite `asset_series` (payload por activo × puntos). Default true en HTTP.
    #[serde(default)]
    pub include_asset_series: Option<bool>,
}

/// Cota del windowing de la serie histórica (100 años, el mismo techo que el runway). Pedirla
/// explícitamente es la forma de decir «todo el histórico».
///
/// `pub(crate)` **solo** para que `mcp::schema_bounds_parity` pueda compararla con el literal del
/// `#[schemars(range(max = 1200))]` de `HistoryParams`: la macro exige un literal, así que la
/// única red posible es un test que los enfrente.
pub(crate) const MAX_HISTORY_WINDOW_MONTHS: i64 = 1200;

/// Ventana por defecto de `GET /v1/history/series` cuando no se pide `window_months`: **10 años**.
///
/// El default anterior era «desde el snapshot más antiguo», que es literalmente el peor caso: en
/// una instalación con un ancla de backfill remota son ~290 puntos y ~26 KB, con los primeros
/// doscientos interpolando entre 0 € y unos cientos de euros. Ningún consumidor —ni el chart ni un
/// agente— pide «todo» por defecto; lo pide quien de verdad lo quiere, y ahora tiene que decirlo
/// (`window_months=1200`).
///
/// 10 años y no 5: es el tramo en el que un patrimonio real ya tiene forma (una hipoteca, un
/// cambio de trabajo, un mercado bajista) y sigue cabiendo en ~121 puntos.
const DEFAULT_HISTORY_WINDOW_MONTHS: i64 = 120;


#[utoipa::path(
    get,
    path = "/v1/history/series",
    tag = "history",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
        ("window_months" = Option<i64>, Query, description = "Limita la serie a los últimos N meses (1..=1200; fuera de rango → 400 `window_months_out_of_range`). Omitido = 120 (10 años); usa 1200 para todo el histórico. La respuesta ecoa `window_months` y marca `window_truncated`."),
        ("include_asset_series" = Option<bool>, Query, description = "`false` omite `asset_series`. Default `true`."),
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
    Query(q): Query<HistorySeriesQuery>,
) -> Result<Json<HistorySeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    // Solo lectura: cualquier miembro (viewer incluido) puede pedir la serie.
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery { view: q.view.clone() }.resolve()?;
    let out = history_series_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        q.window_months,
        q.include_asset_series.unwrap_or(true),
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_history`.
pub(crate) async fn history_series_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    window_months: Option<i64>,
    include_asset_series: bool,
) -> Result<HistorySeriesResponse, ApiError> {
    validate_window_months(window_months, MAX_HISTORY_WINDOW_MONTHS)?;
    let view_label = view.as_str();

    let today = installation_naive_today(pool, iid).await?;
    // `add_months_signed(d, 0)` devuelve el día 1 del mes de `d` → ancla del mes 0.
    let anchor = add_months_signed(today, 0);

    // ---- Fetch del scope (4 queries LedgerView, pipeline compartido) ------------------------
    let scope = fetch_history_scope(pool, view, iid, user_id, today).await?;

    // Sobre el scope COMPLETO, no sobre los markers recortados por `window_months`: un snapshot de
    // pasivo anterior a la ventana sigue anclando la interpolación dentro de ella, así que
    // `liabilities_total` sí es significativo. Con `headers` vacío da `false` sin caso especial.
    let liabilities_snapshotted = liabilities_fully_snapshotted(&scope.headers);

    // Ventana efectiva. Se resuelve ANTES del early-return para que una respuesta vacía declare
    // igualmente qué ventana se aplicó (si no, «no hay datos» y «no hay datos EN ESTA VENTANA»
    // volverían a ser indistinguibles, que es la clase de hueco que esta fase cierra).
    let window = window_months.unwrap_or(DEFAULT_HISTORY_WINDOW_MONTHS);

    // 0 snapshots en scope → 200 con arrays vacíos.
    if scope.headers.is_empty() {
        return Ok(HistorySeriesResponse {
            anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
            anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
            view: view_label,
            window_months: window as u32,
            window_truncated: false,
            first_snapshot_date_ymd: None,
            first_snapshot_month_index: None,
            points: Vec::new(),
            asset_series: Vec::new(),
            markers: Vec::new(),
            liabilities_snapshotted,
        });
    }

    // Snapshot más antiguo del SCOPE (antes de recortar): es lo que permite decir «hay más
    // histórico del que estás viendo» sin repetir la llamada con la ventana máxima. `headers` ya
    // viene `ORDER BY snapshot_date ASC` de `fetch_history_scope`.
    let first_snapshot_date = scope.headers.first().map(|h| h.snapshot_date);

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
                month_fraction: round_month_fraction(month_fraction(h.snapshot_date, anchor)),
                kind: h.kind.clone(),
                source: h.source.clone(),
                owner_user_id: h.owner_user_id,
                total,
            }
        })
        .collect();

    // ---- Rejilla mensual k_min..=0 (primeros-de-mes) ----------------------------------------
    // Las fechas de snapshot están validadas ≤ hoy, así que todo month_index ≤ 0;
    // `.min(0)` es solo un cinturón (p. ej. cambio de calendar_tz a posteriori).
    // `window_months` recorta la rejilla emitida (y los markers devueltos); la interpolación
    // sigue anclándose en TODOS los snapshots del scope.
    let k_min_full = markers.iter().map(|m| m.month_index).min().unwrap_or(0).min(0);
    // Ya validado en la entrada de la core (`validate_window_months`): `window` es un valor de
    // rango, no clampado. Un clamp aquí volvería a inventar una ventana distinta en silencio.
    let k_min = k_min_full.max(-(window as i32));
    // ¿Se ha quedado histórico fuera? Es la mitad honesta del default acotado: recortar sin
    // decirlo sería exactamente el fallo que 4.3.1 arregló en `window_months` fuera de rango.
    let window_truncated = k_min_full < k_min;
    let markers: Vec<HistoryMarker> = markers
        .into_iter()
        .filter(|m| m.month_index >= k_min)
        .collect();
    // La rejilla ETIQUETA meses (primeros-de-mes) pero el punto `month_index = 0` se EVALÚA en
    // `today`. Es el único punto cuyo mes está a medias, y evaluarlo el día 1 dejaba la serie hasta
    // 30 días por detrás del patrimonio vivo: con dos snapshots del mes en curso, la curva
    // terminaba 1.640 € por debajo de dos fotos reales del propio usuario, una tomada hoy, y los
    // activos que solo aparecían en la foto más reciente valían 0 en TODA la ventana (auditoría MCP §2).
    // Un solo hecho explicaba los tres síntomas.
    //
    // `today >= anchor` por construcción (`anchor` es su primero-de-mes), así que la rejilla sigue
    // estrictamente ascendente. `anchor` NO se toca: es la etiqueta de mes y la clave de alineación
    // con la rejilla de la proyección — moverla rompería el empalme del chart.
    //
    // La otra mitad de esto es la observación virtual de más arriba: con `g = today` y
    // `dates.last() == today`, `evaluate_item_at` cae en `a == m-1` y devuelve el valor vivo
    // EXACTO. Sin ella, `e > dates[m-1]` llevaría a la rama «tras el último snapshot: 0» y el punto
    // 0 valdría cero. Las dos piezas van juntas.
    let mut grid: Vec<NaiveDate> = (k_min..=0).map(|k| add_months_signed(anchor, k)).collect();
    if let Some(last) = grid.last_mut() {
        *last = today;
    }
    let grid_len = grid.len();

    // ---- Evaluación (sin cash-flow: serie de snapshots tier-1) ------------------------------
    // Mapa de cash-flow vacío ⇒ interpolación lineal / amortización francesa idéntica bit a bit
    // al histórico previo (P3 del engine). Cómputo puro sub-ms (decenas de snapshots × meses):
    // deliberadamente SIN `spawn_blocking` a esta escala y SIN cache propia.
    let acc = accumulate_series(&scope, &grid, today, &HashMap::new())?;

    // ---- Agregación final --------------------------------------------------------------------
    let live_asset_names: HashMap<Uuid, &str> =
        scope.live_assets.iter().map(|a| (a.id, a.name.as_str())).collect();

    // `net_worth` existe ⟺ el pasivo está fotografiado entero. Sin eso la resta no es un neto y no
    // se publica: `assets_total` sigue ahí, y el cliente tiene que decidir a la vista del flag en
    // vez de leer un número que se parece al patrimonio y no lo es.
    let points: Vec<HistorySeriesPoint> = (0..grid_len)
        .map(|g| HistorySeriesPoint {
            month_index: k_min + g as i32,
            net_worth: liabilities_snapshotted
                .then(|| acc.assets_total[g] - acc.liabilities_total[g]),
            assets_total: acc.assets_total[g],
            liabilities_total: acc.liabilities_total[g],
        })
        .collect();

    let mut asset_series: Vec<HistoryAssetSeries> = if include_asset_series {
        acc.asset_values
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
                    values: values.iter().map(|v| chart_f64(*v)).collect(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    asset_series.sort_by(|a, b| {
        a.asset_name
            .cmp(&b.asset_name)
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });

    Ok(HistorySeriesResponse {
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
        view: view_label,
        window_months: window as u32,
        window_truncated,
        first_snapshot_date_ymd: first_snapshot_date.map(|d| d.format("%Y-%m-%d").to_string()),
        // Mismo `month_index_of` que los markers: la referencia es la rejilla, no el calendario.
        first_snapshot_month_index: first_snapshot_date.map(|d| month_index_of(d, anchor)),
        points,
        asset_series,
        markers,
        liabilities_snapshotted,
    })
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
/// ≥ 0). **Decimal-string** (son KPIs), redondeado a 2 dp.
///
/// Publica **dos** netos, deliberadamente distintos y con la fórmula dentro del nombre. El campo se
/// llamaba `net` y era `expense + income + savings`, mientras `get_transactions_summary.net_actual`
/// se llama igual y **no** incluye el ahorro: dos cosas distintas con el mismo nombre en el mismo
/// catálogo. Un abril con 3.710,97 € movidos a inversión salía `net: -3075.26` y se leía como «perdí
/// 3.075 €» cuando había sido un mes excelente (auditoría MCP §6). La asimetría de signos entre las dos
/// tools no obliga a elegir convención aquí: `income + expense` con `expense ≤ 0` es literalmente el
/// mismo número que `income_mag − expense_mag`. Lo que faltaba era que el nombre lo dijera.
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
    /// `expense + income + savings`: variación de caja del mes. **INCLUYE los traspasos a ahorro**,
    /// así que un mes excelente con una aportación grande sale negativo — no es una pérdida.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub cash_delta: Decimal,
    /// `income + expense` (con `expense ≤ 0`) = ingresos menos gastos. **NO incluye el ahorro.**
    /// Es el mismo número que `totals.net_actual` de `GET /v1/transactions/summary` para ese mes,
    /// allí expresado con magnitudes ≥ 0. Es la cifra que responde a «¿fue buen mes?».
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_minus_expense: Decimal,
}

/// Punto de la rejilla fina: fecha + su posición x fraccional (mismo helper que los markers).
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowFineGridPoint {
    pub date_ymd: String,
    /// Mes de la rejilla mensual en que cae este punto (`≤ 0`). Varios puntos finos comparten
    /// `month_index`: es la etiqueta del mes, no la posición del punto.
    pub month_index: i32,
    /// Posición x **fraccional** dentro de la rejilla mensual:
    /// `month_index + (día − 1) / días_del_mes`, redondeada a 4 decimales
    /// ([`MONTH_FRACTION_DP`]). Es lo que separa dos puntos del mismo mes; sin ella la curva fina
    /// se apilaría toda sobre el día 1.
    pub month_fraction: f64,
}

/// Serie fina por asset (moldeada por cash-flow). Valores **f64** (excepción chart-only, igual que
/// `HistoryAssetSeries`).
#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowFineAssetSeries {
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub asset_name: String,
    /// Valores paralelos a `grid`, redondeados a 2 decimales ([`CHART_DP`]).
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
    ///
    /// **`null` cuando el pasivo del scope no está fotografiado entero** — mismo invariante que
    /// `HistorySeriesPoint.net_worth` (4.4.0, issue #82). Sin snapshots de pasivo esto valía
    /// exactamente `Σ assets` y se seguía llamando `net_worth`: el mismo campo mal nombrado que
    /// la serie mensual, y aquí PEOR, porque esta respuesta ni siquiera publicaba el flag con el
    /// que sospechar. Ahora lo publica (`CashflowResponse.liabilities_snapshotted`).
    /// Viaja como `null` EXPLÍCITO, nunca omitido: para un cliente LLM «ausente» es ambiguo
    /// (¿no aplica? ¿versión vieja? ¿bug?) y `null` no. Mismo criterio que la serie mensual.
    pub net_worth: Option<Vec<f64>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CashflowResponse {
    /// Hoy civil de la instalación (`installation.calendar_tz`).
    pub anchor_date_ymd: String,
    /// Primero-de-mes de `anchor_date_ymd` — la fecha del punto `month_index = 0`.
    pub anchor_month_first_ymd: String,
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view`.
    pub view: &'static str,
    /// Agregado mensual contiguo `-window_months..=0`, ascendente por `month_index`.
    pub months: Vec<CashflowMonth>,
    /// `true` sólo si TODOS los usuarios del scope tienen algún snapshot de pasivo. Cuando es
    /// false no existe patrimonio neto histórico y `fine.net_worth` no viaja: lo que hay son
    /// activos (`fine.asset_series`). Mismo predicado `all`-por-usuario que `/v1/history/series`.
    pub liabilities_snapshotted: bool,
    /// Curva fina moldeada. `null` cuando no se ha podido (o no se ha pedido) construir; el porqué
    /// está en `fine_absent_reason`, nunca hay que adivinarlo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fine: Option<CashflowFine>,
    /// **Por qué falta `fine`.** `null` ⟺ `fine` viaja. Valores:
    ///
    /// - `not_requested` — el llamante no la pidió (`include_curve` de la tool MCP; el endpoint
    ///   HTTP la pide siempre).
    /// - `window_too_large_for_curve` — la ventana supera
    ///   [`MAX_FINE_CURVE_WINDOW_MONTHS`]; el agregado mensual `months[]` sí cubre la ventana
    ///   entera. Pide la curva con una ventana menor.
    /// - `no_asset_linked_transactions` — no hay ninguna transacción ligada a un activo (ni por
    ///   cuenta de import ni por destino de ahorro), así que no hay nada que moldee la curva.
    /// - `no_snapshots_to_anchor` — hay movimientos pero ningún snapshot al que anclar la curva;
    ///   sin ancla sería una curva de deltas flotando en el vacío.
    ///
    /// Hasta 4.4.0 las tres últimas causas producían exactamente la misma respuesta —el campo
    /// simplemente no estaba— y «no tengo datos», «no me lo has pedido» y «te lo he recortado por
    /// tamaño» eran indistinguibles.
    pub fine_absent_reason: Option<&'static str>,
}

#[derive(Debug, Deserialize)]
pub struct CashflowQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Meses de ventana, default 24, rango 1..=120 (fuera de rango → 400
    /// `window_months_out_of_range`).
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
/// Cota de la ventana del cash-flow. Fuera de rango se rechaza: ver `validate_window_months`.
const MAX_CASHFLOW_WINDOW_MONTHS: i64 = 120;
/// `resolution=daily` solo se permite con ventanas acotadas (coste del grid diario).
const MAX_DAILY_WINDOW_MONTHS: i32 = 6;

/// Ventana máxima con **curva fina**. El agregado mensual sigue llegando hasta
/// `MAX_CASHFLOW_WINDOW_MONTHS` (120); lo que se acota es la capa que crece por ACTIVO.
///
/// Es el peor caso del catálogo: la rejilla weekly avanza de 7 en 7 días, así que 120 meses son
/// ~522 puntos **por activo** — un hogar con cinco activos vinculados se lleva ~2.600 números en
/// una sola respuesta. Con 36 meses son ~157 puntos por activo, y la curva sigue contando la
/// historia que un overlay de patrimonio necesita (el propio chart de la app pide 24 semanales y 6
/// diarios).
///
/// **No es un 400.** Pasarse no rompe la llamada: se devuelve el agregado mensual completo, `fine`
/// llega `null` y `fine_absent_reason` dice `window_too_large_for_curve`. Un error habría obligado
/// a reintentar para conseguir unos `months[]` que sí eran servibles.
const MAX_FINE_CURVE_WINDOW_MONTHS: i32 = 36;

#[utoipa::path(
    get,
    path = "/v1/history/cashflow",
    tag = "history",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
        ("window_months" = Option<i64>, Query, description = "Meses de ventana (default 24, rango 1..=120; fuera de rango → 400 `window_months_out_of_range`). Por encima de 36 el agregado mensual llega igual, pero la curva fina se omite con `fine_absent_reason = window_too_large_for_curve`."),
        ("resolution" = Option<String>, Query, description = "`weekly` (default) | `daily`. `daily` requiere `window_months <= 6`."),
    ),
    responses(
        (status = 200, description = "Cash-flow mensual + curva fina opcional (`fine`; cuando falta, `fine_absent_reason` dice por qué).", body = CashflowResponse),
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
    let view = LedgerViewQuery { view: q.view.clone() }.resolve()?;
    let out = history_cashflow_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        q.window_months,
        q.resolution.as_deref(),
        true,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_history_cashflow`. Con
/// `include_fine = false` omite la curva fina (y sus queries de patas): el agregado mensual es
/// lo útil para un LLM, la curva es payload de chart (opt-in `include_curve` en la tool; el
/// endpoint HTTP siempre pasa `true`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn history_cashflow_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    window_months: Option<i64>,
    resolution: Option<&str>,
    include_fine: bool,
) -> Result<CashflowResponse, ApiError> {
    let view_label = view.as_str();

    validate_window_months(window_months, MAX_CASHFLOW_WINDOW_MONTHS)?;
    let window_months: i32 = window_months.unwrap_or(DEFAULT_CASHFLOW_WINDOW_MONTHS) as i32;
    // `resolution` desconocido es un error, no un weekly silencioso: la respuesta ecoa
    // `resolution` y `resolution:"hourly"` devolvía 200 diciendo "weekly" (auditoría MCP §4, misma
    // clase que `view`).
    let daily = match resolution.map(str::trim) {
        None | Some("") | Some("weekly") => false,
        Some("daily") => true,
        Some(_) => {
            return Err(ApiError::BadRequest(
                "invalid_resolution: resolution must be 'weekly' or 'daily'".into(),
            ))
        }
    };
    if daily && window_months > MAX_DAILY_WINDOW_MONTHS {
        return Err(ApiError::BadRequest(format!(
            "daily_window_too_large: resolution=daily requires window_months <= {MAX_DAILY_WINDOW_MONTHS}"
        )));
    }
    let resolution_label = if daily { "daily" } else { "weekly" };

    let today = installation_naive_today(pool, iid).await?;
    let anchor = add_months_signed(today, 0); // primero-de-mes del mes 0.
    let window_start = add_months_signed(anchor, -window_months);
    let month_end = add_months_signed(anchor, 1); // exclusivo: incluye el mes 0 completo.

    // ---- Agregado mensual firmado por kind --------------------------------------------------
    // Transferencias conciliadas FUERA (es el mismo KPI de flujo que la comparativa: un traspaso
    // interno no es gasto ni ingreso). ASIMETRÍA DELIBERADA con la curva fina de abajo, que SÍ
    // las incluye — NO «arreglar» esto igualando ambas: ver el comentario de las patas.
    let m_scope = view.scope_where("t");
    let m_arg = view.next_arg_index();
    let months_sql = format!(
        "SELECT to_char(t.op_date, 'YYYY-MM') AS ym, t.kind AS kind, SUM(t.amount) AS total
         FROM transactions t
         WHERE {m_scope} AND t.op_date >= ${m_arg} AND t.op_date < ${end}
           AND t.transfer_counterpart_id IS NULL
         GROUP BY ym, t.kind",
        end = m_arg + 1
    );
    let month_rows: Vec<MonthKindRow> = view
        .bind_scope_as(sqlx::query_as(&months_sql), iid, user_id)
        .bind(window_start)
        .bind(month_end)
        .fetch_all(pool)
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
                // Ambos = suma exacta de las cifras YA redondeadas que se devuelven (el invariante
                // se cumple sobre los strings serializados, no sobre valores pre-redondeo).
                cash_delta: money(expense + income + savings),
                income_minus_expense: money(expense + income),
            }
        })
        .collect();

    // ---- Patas de cash-flow por (owner, asset) ----------------------------------------------
    // Pata cuenta: toda transacción de un batch con `account_asset_id` → delta = amount (+sube la
    // cuenta). Pata destino ahorro: `kind='savings'` con `linked_asset_id` → delta = −amount
    // (una aportación −200 sube el destino en +200). Una savings importada aparece en AMBAS
    // (partida doble correcta: baja la cuenta, sube el destino). Deltas ya normalizados aquí; el
    // engine solo los suma.
    //
    // Las transferencias CONCILIADAS **SÍ cuentan aquí**, a propósito (asimetría con `months[]`):
    // la curva fina modela el SALDO de cada cuenta, y un traspaso interno mueve saldo real (sale
    // de X, entra en Y) aunque no sea gasto ni ingreso. Excluirlas haría divergir la curva de los
    // snapshots a los que está anclada y el anclaje absorbería la diferencia como un salto falso.
    // Test que fija la asimetría: `history_cashflow.rs::reconciled_excluded_from_months_but_not_from_fine_curve`.
    // ¿Se intenta siquiera la curva? Dos puertas antes de tocar la BD: que la pidan y que la
    // ventana quepa. Las dos producen una razón publicada, no un silencio.
    let curve_requested = include_fine;
    let curve_window_ok = window_months <= MAX_FINE_CURVE_WINDOW_MONTHS;
    let try_fine = curve_requested && curve_window_ok;

    let mut cashflow: HashMap<(Uuid, Uuid), Vec<CashFlowEntry>> = HashMap::new();
    if try_fine {
        let acc_scope = view.scope_where("t");
        let account_sql = format!(
            "SELECT ti.account_asset_id AS asset_id, t.owner_user_id AS owner_user_id,
                    t.op_date AS op_date, t.amount AS delta
             FROM transactions t
             JOIN transaction_imports ti ON ti.id = t.import_id
             WHERE {acc_scope} AND ti.account_asset_id IS NOT NULL"
        );
        let account_legs: Vec<CashflowLegRow> = view
            .bind_scope_as(sqlx::query_as(&account_sql), iid, user_id)
            .fetch_all(pool)
            .await?;

        let sav_scope = view.scope_where("t");
        let savings_sql = format!(
            "SELECT t.linked_asset_id AS asset_id, t.owner_user_id AS owner_user_id,
                    t.op_date AS op_date, (-t.amount) AS delta
             FROM transactions t
             WHERE {sav_scope} AND t.kind = 'savings' AND t.linked_asset_id IS NOT NULL"
        );
        let savings_legs: Vec<CashflowLegRow> = view
            .bind_scope_as(sqlx::query_as(&savings_sql), iid, user_id)
            .fetch_all(pool)
            .await?;

        for leg in account_legs.into_iter().chain(savings_legs.into_iter()) {
            cashflow
                .entry((leg.owner_user_id, leg.asset_id))
                .or_default()
                .push(CashFlowEntry {
                    date: leg.op_date,
                    delta: leg.delta,
                });
        }
    }

    // ---- Capa fina (solo si hay vínculos a assets Y snapshots que anclar) --------------------
    // El flag se resuelve dentro de la rama fina (es donde se carga el scope) y sale por aquí:
    // sin capa fina no hay serie de neto que cualificar, y `false` es la lectura honesta.
    let mut liabilities_snapshotted = false;
    // `fine_absent_reason` se decide en el MISMO sitio que `fine`: una sola expresión produce el
    // par, así que no pueden desincronizarse (un `Some(fine)` con razón, o un `None` sin ella).
    let mut fine_absent_reason: Option<&'static str> = None;
    let fine = if !curve_requested {
        fine_absent_reason = Some("not_requested");
        None
    } else if !curve_window_ok {
        fine_absent_reason = Some("window_too_large_for_curve");
        None
    } else if cashflow.is_empty() {
        fine_absent_reason = Some("no_asset_linked_transactions");
        None
    } else {
        let scope = fetch_history_scope(pool, view, iid, user_id, today).await?;
        liabilities_snapshotted = liabilities_fully_snapshotted(&scope.headers);
        if scope.headers.is_empty() {
            fine_absent_reason = Some("no_snapshots_to_anchor");
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
                    month_fraction: round_month_fraction(month_fraction(*d, anchor)),
                })
                .collect();

            let net_worth: Vec<f64> = (0..fine_grid.len())
                .map(|g| chart_f64(acc.assets_total[g] - acc.liabilities_total[g]))
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
                        values: values.iter().map(|v| chart_f64(*v)).collect(),
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
                // Mismo invariante que la serie mensual: sin el pasivo entero fotografiado esto
                // NO es patrimonio neto, así que no se publica disfrazado de tal.
                net_worth: liabilities_snapshotted.then_some(net_worth),
            })
        }
    };

    Ok(CashflowResponse {
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        anchor_month_first_ymd: anchor.format("%Y-%m-%d").to_string(),
        view: view_label,
        months,
        liabilities_snapshotted,
        fine,
        fine_absent_reason,
    })
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
    repayment_model: Option<String>,
}

/// Observación de un item en un punto del timeline: valor + términos crudos (para eco en la
/// respuesta y para construir `LoanTerms` vía [`loan_terms_of`]).
struct PrefillObs {
    value: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    repayment_model: Option<String>,
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
                loan_terms_of(
                    lo.apr_percent,
                    lo.payment_amount,
                    lo.payment_frequency.as_deref(),
                    lo.repayment_model.as_deref(),
                )
                .or_else(|| {
                    loan_terms_of(
                        ro.apr_percent,
                        ro.payment_amount,
                        ro.payment_frequency.as_deref(),
                        ro.repayment_model.as_deref(),
                    )
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
        ApiError::BadRequest("date_required: date is required and must be a valid YYYY-MM-DD date".into())
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

    // Filas vivas propias (con plan vivo o saldo vivo, #145), normalizadas a `PrefillLiveRow`.
    let live_rows: Vec<PrefillLiveRow> = if is_liability {
        sqlx::query_as(
            r#"SELECT id, label, principal AS value, apr_percent, payment_amount,
                      payment_frequency, repayment_model
               FROM liabilities
               WHERE installation_id = $1 AND owner_user_id = $2
                 AND (payment_end_date IS NULL OR payment_end_date >= $3 OR principal > 0)"#,
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
                      NULL::text AS payment_frequency, NULL::text AS repayment_model
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
                  payment_amount, payment_frequency, repayment_model
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
                repayment_model: it.repayment_model.clone(),
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
                repayment_model: r.repayment_model.clone(),
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
