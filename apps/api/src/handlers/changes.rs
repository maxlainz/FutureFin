//! `GET /v1/changes` — qué se ha tocado desde una fecha.
//!
//! ## Por qué un módulo propio y no una ruta de `installation` ni de `history`
//!
//! No es una propiedad del singleton de instalación: es un **índice transversal** sobre ocho tablas
//! del ledger, y colgarlo de `PATCH/GET /v1/installation` lo dejaría escondido detrás de un recurso
//! de ajustes. Tampoco puede vivir en `history.rs`: ese módulo ya se llama «historia» con un
//! significado muy concreto y muy distinto —la serie de patrimonio interpolada entre snapshots—, y
//! un feed de ediciones bajo `/v1/history/*` se leería como parte de esa curva. Un módulo propio
//! además hace del aviso de abajo un campo de primera clase de su respuesta en vez de una nota al
//! pie de un endpoint que trata de otra cosa.
//!
//! ## Lo que este endpoint NO es
//!
//! **No es una auditoría.** Solo sabe de filas que existen: se apoya en las columnas `updated_at`
//! que las tablas ya mantienen, y **no hay tombstones**, así que un borrado es indistinguible de
//! «nunca existió». Una fila creada y borrada dentro de la ventana no aparece por ningún lado.
//! La respuesta lo declara (`covers_deletions: false` + `deletions_absent_reason`) porque venderlo
//! como auditoría sería mentir, y una lista vacía debe poder distinguirse de «no ha pasado nada».
//!
//! Dos tablas del dominio quedan fuera **porque no tienen `updated_at`**: `categories` y
//! `allocation_rules`. Se publican en `tables_missing_updated_at` en vez de omitirse en silencio:
//! renombrar una categoría o cambiar una regla de reparto NO aparece aquí, y quien lea el feed
//! tiene que poder saberlo sin auditar el esquema.
//!
//! Y una tercera categoría, **excluida por diseño**: `persons` SÍ tiene `updated_at`
//! (migración `20260217120000_persons_installation.sql`) pero no es ledger — hoy ninguna ruta de
//! la aplicación la escribe (solo la lee el fallback de fecha de nacimiento de la proyección),
//! así que un feed de cambios sobre ella siempre estaría vacío. No va en `tables_covered` porque
//! ampliar esa lista cambia el wire (y la paridad MCP de `list_recent_changes`) por una tabla
//! muerta. **Revisar esta exclusión el día que algo escriba en `persons`.**

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Tope duro de filas devueltas. Por encima, la respuesta declara `truncated`.
const MAX_CHANGES_LIMIT: i64 = 500;
const DEFAULT_CHANGES_LIMIT: i64 = 100;

/// Tablas cubiertas, en el orden en que se unen. Es también lo que se publica en `tables_covered`.
const COVERED: &[&str] = &[
    "assets",
    "liabilities",
    "budget_entries",
    "planning_flows",
    "transactions",
    "recurring_transaction_rules",
    "categorization_rules",
    "history_snapshots",
];

/// Tablas del dominio SIN columna `updated_at`, y por tanto invisibles para este feed.
const MISSING_UPDATED_AT: &[&str] = &["categories", "allocation_rules"];

#[derive(Debug, Deserialize)]
pub struct RecentChangesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Obligatorio. Marca de tiempo RFC 3339 (`2026-08-01T00:00:00Z`) o fecha `YYYY-MM-DD`
    /// (que se interpreta como su medianoche UTC).
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentChange {
    /// `asset` | `liability` | `budget_entry` | `planning_flow` | `transaction` |
    /// `recurring_rule` | `categorization_rule` | `snapshot`.
    pub entity: String,
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// Etiqueta legible de la fila (nombre, concepto, patrón…). Una partida de presupuesto no
    /// tiene nombre propio, así que se identifica por el nombre de **su categoría**; puede quedar
    /// vacía si la categoría ya no existe.
    pub label: String,
    /// `created` cuando la fila **nació** dentro de la ventana; `updated` cuando ya existía antes
    /// y se editó dentro. Es la única distinción que los `updated_at` permiten hacer con
    /// honestidad — y ninguna de las dos cubre los borrados.
    pub change: &'static str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Dueño de la fila. Desde 5.0.0 las OCHO tablas del feed lo tienen `NOT NULL` (D14), así
    /// que el campo viaja siempre; el `Option` se conserva por compatibilidad del contrato ya
    /// publicado. Es además quien puede editarla (D21).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentChangesResponse {
    /// Vista efectivamente aplicada: `household` | `mine`.
    pub view: &'static str,
    /// Eco de `since` ya normalizado a RFC 3339 UTC. Sin él, un `since` de fecha suelta
    /// (`2026-08-01`) y uno con hora se leerían igual y no se sabría cuál se aplicó.
    pub since: String,
    /// Instante en que se resolvió la consulta: el `since` del siguiente sondeo.
    pub now: String,
    /// **Siempre `false`.** No hay tombstones en el esquema.
    pub covers_deletions: bool,
    /// Por qué no. Valor único: `no_tombstones`.
    #[schema(value_type = String)]
    pub deletions_absent_reason: &'static str,
    /// Las ocho tablas que se consultan.
    #[schema(value_type = Vec<String>)]
    pub tables_covered: Vec<&'static str>,
    /// Tablas del dominio que **no** se pueden cubrir por no tener `updated_at`.
    #[schema(value_type = Vec<String>)]
    pub tables_missing_updated_at: Vec<&'static str>,
    /// Cambios que cumplen el filtro en total.
    pub item_count: i64,
    /// Cuántos se devuelven de verdad (≤ `limit`).
    pub items_included: i64,
    /// `true` ⟺ `items_included < item_count`. Una lista corta no es «no hay más».
    pub truncated: bool,
    pub changes: Vec<RecentChange>,
}

#[derive(Debug, FromRow)]
struct ChangeRow {
    entity: String,
    id: Uuid,
    label: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    owner_user_id: Option<Uuid>,
}

/// `since` acepta RFC 3339 o `YYYY-MM-DD`. Los códigos son los que ya existen en el catálogo
/// (`date_required`, `date_invalid`): no hace falta uno nuevo para decir lo mismo.
fn parse_since(raw: Option<&str>) -> Result<DateTime<Utc>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(ApiError::BadRequest(
            "date_required: since is required (RFC 3339 timestamp or YYYY-MM-DD)".into(),
        ));
    };
    if let Ok(ts) = DateTime::parse_from_rfc3339(raw) {
        return Ok(ts.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(DateTime::from_naive_utc_and_offset(
            d.and_hms_opt(0, 0, 0).expect("medianoche es válida"),
            Utc,
        ));
    }
    Err(ApiError::BadRequest(
        "date_invalid: since must be an RFC 3339 timestamp or a YYYY-MM-DD date".into(),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/changes",
    tag = "changes",
    params(
        ("view" = Option<String>, Query, description = "`mine` | household."),
        ("since" = String, Query, description = "Obligatorio. RFC 3339 o YYYY-MM-DD."),
        ("limit" = Option<i64>, Query, description = "1..=500; por defecto 100."),
    ),
    responses(
        (status = 200, description = "Filas tocadas desde `since` (sin borrados)", body = RecentChangesResponse),
        (status = 400, description = "since ausente/inválido o limit fuera de rango"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_recent_changes(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<RecentChangesQuery>,
) -> Result<Json<RecentChangesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let out = list_recent_changes_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        q.since.as_deref(),
        q.limit,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: la comparten el handler GET y la tool MCP `list_recent_changes`.
///
/// **Mitad honesta y dicho en la respuesta**: cubre altas y ediciones, nunca borrados. Ver la doc
/// del módulo para el porqué y para las dos tablas que quedan fuera.
///
/// Cache **NONE**: es una lectura pura; no muta nada.
pub(crate) async fn list_recent_changes_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    since_raw: Option<&str>,
    limit: Option<i64>,
) -> Result<RecentChangesResponse, ApiError> {
    let since = parse_since(since_raw)?;
    let limit = limit.unwrap_or(DEFAULT_CHANGES_LIMIT);
    if !(1..=MAX_CHANGES_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(
            "limit_out_of_range: limit must be between 1 and 500".into(),
        ));
    }

    let scope = view.scope_where("");
    let since_arg = view.next_arg_index();
    let limit_arg = since_arg + 1;

    // Una rama por tabla, todas con el MISMO WHERE de scope (`$1` / `$1,$2`) y el mismo `$since`.
    // Las etiquetas se normalizan a `label` para que la unión tenga una sola forma.
    let arms: [(&str, &str, &str); 8] = [
        ("asset", "assets", "name"),
        ("liability", "liabilities", "label"),
        // `budget_entries` no tiene columna de nombre propia (la tuvo y se retiró): una partida
        // se identifica por SU CATEGORÍA, así que la etiqueta sale de ahí en vez de quedarse vacía.
        (
            "budget_entry",
            "budget_entries",
            "COALESCE((SELECT c.name FROM categories c WHERE c.id = budget_entries.category_id), '')",
        ),
        ("planning_flow", "planning_flows", "title"),
        ("transaction", "transactions", "concept"),
        ("recurring_rule", "recurring_transaction_rules", "concept"),
        ("categorization_rule", "categorization_rules", "pattern"),
        (
            "snapshot",
            "history_snapshots",
            "kind || ' @ ' || to_char(snapshot_date, 'YYYY-MM-DD')",
        ),
    ];
    let base = arms
        .iter()
        .map(|(entity, table, label)| {
            format!(
                "SELECT '{entity}'::text AS entity, id, ({label})::text AS label, \
                 created_at, updated_at, owner_user_id \
                 FROM {table} WHERE {scope} AND updated_at >= ${since_arg}"
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ");

    let count_sql = format!("SELECT COUNT(*) FROM ({base}) t");
    let item_count: i64 = view
        .bind_scope_scalar(sqlx::query_scalar(&count_sql), iid, user_id)
        .bind(since)
        .fetch_one(pool)
        .await?;

    // Desempate por `id` para que el orden sea total: dos filas escritas en la misma transacción
    // comparten `now()` al microsegundo y sin desempate el orden sería arbitrario entre llamadas.
    let page_sql = format!(
        "SELECT entity, id, label, created_at, updated_at, owner_user_id \
         FROM ({base}) t ORDER BY updated_at DESC, id ASC LIMIT ${limit_arg}"
    );
    let rows: Vec<ChangeRow> = view
        .bind_scope_as(sqlx::query_as(&page_sql), iid, user_id)
        .bind(since)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    let items_included = rows.len() as i64;
    let changes: Vec<RecentChange> = rows
        .into_iter()
        .map(|r| RecentChange {
            change: if r.created_at >= since {
                "created"
            } else {
                "updated"
            },
            entity: r.entity,
            id: r.id,
            label: r.label,
            created_at: r.created_at,
            updated_at: r.updated_at,
            owner_user_id: r.owner_user_id,
        })
        .collect();

    Ok(RecentChangesResponse {
        view: view.as_str(),
        since: since.to_rfc3339(),
        now: Utc::now().to_rfc3339(),
        covers_deletions: false,
        deletions_absent_reason: "no_tombstones",
        tables_covered: COVERED.to_vec(),
        tables_missing_updated_at: MISSING_UPDATED_AT.to_vec(),
        item_count,
        items_included,
        truncated: items_included < item_count,
        changes,
    })
}

pub fn changes_router() -> Router {
    Router::new().route("/", get(get_recent_changes))
}
