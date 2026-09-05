//! Candidatos a movimiento duplicado (`GET /v1/transactions/duplicates`).
//!
//! ## Qué agrupa
//! La **huella canónica** que ya usa el dedup del import (`compute_fingerprint`:
//! `source · op_date · importe a 4 decimales · concepto normalizado`), agrupada por
//! `(owner_user_id, fingerprint)` — el mismo ámbito que la constraint
//! `transactions_unique_fingerprint`. Agrupar solo por huella metería en el mismo saco el mismo
//! recibo de dos personas distintas del hogar, que no es un duplicado sino la vida normal de una
//! instalación compartida.
//!
//! ## Candidato, no veredicto
//! Los duplicados **legítimos existen**: dos cafés de 1,80 € el mismo día en el mismo sitio son
//! dos movimientos reales, y para eso está `fingerprint_ordinal` (la constraint permite N filas
//! con la misma huella si el ordinal difiere). Esta ruta NO afirma que sobren filas: enseña grupos
//! y los datos con los que decidir. El discriminante útil viaja en la respuesta:
//! `spans_multiple_imports` / `distinct_import_count`. Dos filas idénticas **dentro del mismo
//! lote de import** casi siempre son reales (el dedup del import ya impide re-insertar la misma
//! huella de un lote anterior, así que solo llegan ahí si el propio extracto traía dos líneas
//! iguales); repartidas entre lotes distintos —o entre un lote y un alta manual— son el patrón
//! clásico del re-import o del «lo apunté y además lo importé».
//!
//! Cache: **NONE**. Es lectura (D5).

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::crud::{PreparedFilters, TxnFilters};
use crate::money::money_out;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// Criterio de agrupación, publicado como dato (no como prosa) para que el consumidor pueda
/// explicar qué está mirando sin adivinarlo.
const BASIS: &str = "owner + source + op_date + amount(4dp) + normalized_concept";

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateMember {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `YYYY-MM-DD`.
    pub op_date: String,
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub kind: Option<String>,
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    /// `myinvestor` | `n26` | `manual` | …
    pub source: String,
    /// Lote de import del que salió; `null` = alta manual.
    #[schema(value_type = Option<String>, format = "uuid")]
    pub import_id: Option<Uuid>,
    /// Posición dentro de la huella. `0` es el primero; `>0` significa que alguien (import o alta)
    /// aceptó explícitamente convivir con una fila idéntica.
    pub fingerprint_ordinal: i32,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateGroup {
    /// Dueño de las filas del grupo (los grupos nunca cruzan personas).
    #[schema(value_type = String, format = "uuid")]
    pub owner_user_id: Uuid,
    /// `YYYY-MM-DD` compartido por todo el grupo (forma parte de la huella).
    pub op_date: String,
    /// Importe compartido, con signo.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// Concepto del primer miembro. El que comparte el grupo es el concepto NORMALIZADO, así que
    /// dos miembros pueden diferir en mayúsculas, tildes o espacios; míralos en `transactions`.
    pub concept: String,
    pub source: String,
    /// Filas del grupo. Siempre ≥ 2.
    pub transaction_count: i64,
    /// Lotes de import distintos representados en el grupo (las altas manuales cuentan como uno).
    pub distinct_import_count: i64,
    /// `true` ⇒ las filas NO vienen todas del mismo origen: es el patrón del re-import o del
    /// «lo apunté a mano y además lo importé», el candidato fuerte. `false` ⇒ el extracto traía
    /// dos líneas iguales, que casi siempre son dos movimientos reales.
    pub spans_multiple_imports: bool,
    pub transactions: Vec<DuplicateMember>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicatesResponse {
    /// Vista aplicada: `household` | `mine`.
    pub view: String,
    /// Criterio de agrupación, literal y estable.
    pub basis: String,
    /// Grupos devueltos.
    pub group_count: i64,
    /// Grupos que cumplen el criterio en total (puede ser mayor que `group_count`).
    pub group_count_total: i64,
    /// `true` ⇒ `limit` ha recortado la lista.
    pub truncated: bool,
    pub limit: i64,
    /// De más filas a menos; a igualdad, primero los más recientes.
    pub groups: Vec<DuplicateGroup>,
}

#[derive(Debug, FromRow)]
struct GroupRow {
    owner_user_id: Uuid,
    fingerprint: String,
}

#[derive(Debug, FromRow)]
struct MemberRow {
    id: Uuid,
    owner_user_id: Uuid,
    fingerprint: String,
    fingerprint_ordinal: i32,
    op_date: NaiveDate,
    concept: String,
    amount: Decimal,
    kind: Option<String>,
    category_id: Option<Uuid>,
    category_name: Option<String>,
    source: String,
    import_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct DuplicatesQuery {
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub import_id: Option<Uuid>,
    #[serde(default)]
    pub concept_contains: Option<String>,
    #[serde(default)]
    pub date_from: Option<NaiveDate>,
    #[serde(default)]
    pub date_to: Option<NaiveDate>,
    /// Grupos a devolver (1..100, default 20).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/v1/transactions/duplicates",
    tag = "transactions",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
        ("month" = Option<String>, Query, description = "`YYYY-MM`. Excluyente con `date_from`/`date_to`."),
        ("kind" = Option<String>, Query, description = "`expense` | `income` | `savings`."),
        ("import_id" = Option<Uuid>, Query, description = "Limita el barrido a un lote de import."),
        ("concept_contains" = Option<String>, Query, description = "Subcadena del concepto (1–200), insensible a mayúsculas y a tildes."),
        ("date_from" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
        ("date_to" = Option<String>, Query, description = "`YYYY-MM-DD` inclusivo. Excluyente con `month`."),
        ("limit" = Option<i64>, Query, description = "Grupos a devolver (1..100, default 20)."),
    ),
    responses(
        (status = 200, description = "Grupos de movimientos con la MISMA huella canónica. Son CANDIDATOS: los duplicados legítimos existen (`fingerprint_ordinal`)", body = DuplicatesResponse),
        (status = 400, description = "Filtro inválido"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_duplicates(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<DuplicatesQuery>,
) -> Result<Json<DuplicatesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let out = find_duplicate_transactions_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        TxnFilters {
            month: q.month.as_deref(),
            kind: q.kind.as_deref(),
            import_id: q.import_id,
            concept_contains: q.concept_contains.as_deref(),
            date_from: q.date_from,
            date_to: q.date_to,
            ..Default::default()
        },
        q.limit,
    )
    .await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `find_duplicate_transactions`.
/// Comparte el constructor de filtros con el listado y con la agregación, así que «duplicados de
/// este mes» y «movimientos de este mes» hablan del mismo conjunto.
///
/// Cache: NONE.
pub(crate) async fn find_duplicate_transactions_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    f: TxnFilters<'_>,
    limit: Option<i64>,
) -> Result<DuplicatesResponse, ApiError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit_out_of_range: limit must be between 1 and {MAX_LIMIT}"
        )));
    }
    let p = PreparedFilters::prepare(view, f, false)?;
    let scope = view.scope_where("t");
    let filters = p.sql();

    let count_sql = format!(
        "SELECT COUNT(*)::bigint FROM (
             SELECT 1 FROM transactions t
             WHERE {scope}{filters}
             GROUP BY t.owner_user_id, t.fingerprint
             HAVING COUNT(*) > 1
         ) g"
    );
    let group_count_total: i64 = p
        .bind_scalar(view.bind_scope_scalar(sqlx::query_scalar(&count_sql), iid, user_id))
        .fetch_one(pool)
        .await?;

    // Orden TOTAL y determinista: más filas primero, luego el más reciente, y `fingerprint` como
    // desempate final — sin él, dos grupos empatados salían en el orden que quisiera el plan.
    let groups_sql = format!(
        "SELECT t.owner_user_id AS owner_user_id, t.fingerprint AS fingerprint
         FROM transactions t
         WHERE {scope}{filters}
         GROUP BY t.owner_user_id, t.fingerprint
         HAVING COUNT(*) > 1
         ORDER BY COUNT(*) DESC, MAX(t.op_date) DESC, t.fingerprint ASC
         LIMIT ${}",
        p.next_arg()
    );
    let groups: Vec<GroupRow> = p
        .bind_as(view.bind_scope_as(sqlx::query_as(&groups_sql), iid, user_id))
        .bind(limit)
        .fetch_all(pool)
        .await?;

    if groups.is_empty() {
        return Ok(DuplicatesResponse {
            view: view.as_str().to_string(),
            basis: BASIS.to_string(),
            group_count: 0,
            group_count_total,
            truncated: false,
            limit,
            groups: Vec::new(),
        });
    }

    // Miembros de los grupos seleccionados. El par `(owner, fingerprint)` viene de la query de
    // arriba, que ya aplicó el scope, así que aquí basta con anclar la instalación.
    let owners: Vec<Uuid> = groups.iter().map(|g| g.owner_user_id).collect();
    let fps: Vec<String> = groups.iter().map(|g| g.fingerprint.clone()).collect();
    let members: Vec<MemberRow> = sqlx::query_as(
        r#"SELECT t.id, t.owner_user_id, t.fingerprint, t.fingerprint_ordinal, t.op_date,
                  t.concept, t.amount, t.kind, t.category_id, c.name AS category_name,
                  t.source, t.import_id, t.created_at
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
           JOIN (SELECT unnest($2::uuid[]) AS o, unnest($3::text[]) AS fp) g
             ON g.o = t.owner_user_id AND g.fp = t.fingerprint
           WHERE t.installation_id = $1
           ORDER BY t.fingerprint_ordinal ASC, t.created_at ASC, t.id ASC"#,
    )
    .bind(iid)
    .bind(&owners)
    .bind(&fps)
    .fetch_all(pool)
    .await?;

    let mut out: Vec<DuplicateGroup> = Vec::with_capacity(groups.len());
    for g in &groups {
        let mine: Vec<&MemberRow> = members
            .iter()
            .filter(|m| m.owner_user_id == g.owner_user_id && m.fingerprint == g.fingerprint)
            .collect();
        let Some(head) = mine.first() else {
            // Se ha borrado entre las dos queries. No es un grupo: es un grupo que ya no existe.
            continue;
        };
        let imports: HashSet<Option<Uuid>> = mine.iter().map(|m| m.import_id).collect();
        out.push(DuplicateGroup {
            owner_user_id: g.owner_user_id,
            op_date: head.op_date.format("%Y-%m-%d").to_string(),
            amount: money_out(head.amount),
            concept: head.concept.clone(),
            source: head.source.clone(),
            transaction_count: mine.len() as i64,
            distinct_import_count: imports.len() as i64,
            spans_multiple_imports: imports.len() > 1,
            transactions: mine
                .iter()
                .map(|m| DuplicateMember {
                    id: m.id,
                    op_date: m.op_date.format("%Y-%m-%d").to_string(),
                    concept: m.concept.clone(),
                    amount: money_out(m.amount),
                    kind: m.kind.clone(),
                    category_id: m.category_id,
                    category_name: m.category_name.clone(),
                    source: m.source.clone(),
                    import_id: m.import_id,
                    fingerprint_ordinal: m.fingerprint_ordinal,
                    created_at: m.created_at,
                })
                .collect(),
        });
    }

    Ok(DuplicatesResponse {
        view: view.as_str().to_string(),
        basis: BASIS.to_string(),
        group_count: out.len() as i64,
        group_count_total,
        truncated: group_count_total > out.len() as i64,
        limit,
        groups: out,
    })
}
