//! Allocation rules CRUD + reordering.
//!
//! Each rule consumes part of the **monthly surplus** for one target asset, in priority order.
//! See [`crate::handlers::projection`] for how the engine reads these.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationRuleResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub priority: i32,
    /// `fixed` | `percent` | `remainder`
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    /// `amount` | `months_expense` | `income_multiple` | null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_value: Option<Decimal>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAllocationRuleBody {
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub kind: String,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub cap_kind: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_value: Option<Decimal>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchAllocationRuleBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub target_asset_id: Option<Uuid>,
    #[serde(default)]
    pub kind: Option<String>,
    /// `null` JSON clears the amount (only valid for `remainder`).
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub amount: Option<serde_json::Value>,
    /// `null` JSON clears the cap pair.
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub cap: Option<serde_json::Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderBody {
    #[schema(value_type = Vec<String>)]
    pub ids: Vec<Uuid>,
}

#[derive(Debug, FromRow)]
struct RuleRow {
    id: Uuid,
    owner_user_id: Option<Uuid>,
    target_asset_id: Uuid,
    priority: i32,
    kind: String,
    amount: Option<Decimal>,
    cap_kind: Option<String>,
    cap_value: Option<Decimal>,
    enabled: bool,
    notes: Option<String>,
}

fn row_to_response(r: RuleRow) -> AllocationRuleResponse {
    AllocationRuleResponse {
        id: r.id,
        target_asset_id: r.target_asset_id,
        priority: r.priority,
        kind: r.kind,
        amount: r.amount,
        cap_kind: r.cap_kind,
        cap_value: r.cap_value,
        enabled: r.enabled,
        notes: r.notes,
        owner_user_id: r.owner_user_id,
    }
}

fn normalize_kind(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "fixed" => Ok("fixed".into()),
        "percent" => Ok("percent".into()),
        "remainder" => Ok("remainder".into()),
        other => Err(ApiError::BadRequest(format!(
            "rule_kind_invalid: kind must be 'fixed' | 'percent' | 'remainder', got {other:?}"
        ))),
    }
}

fn validate_kind_amount(kind: &str, amount: Option<Decimal>) -> Result<Option<Decimal>, ApiError> {
    match kind {
        "remainder" => Ok(None),
        "fixed" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount_required_for_kind: amount is required for kind=fixed".into())
            })?;
            if v < Decimal::ZERO {
                return Err(ApiError::BadRequest("amount_negative: amount must be >= 0".into()));
            }
            Ok(Some(v))
        }
        "percent" => {
            let v = amount.ok_or_else(|| {
                ApiError::BadRequest("amount_required_for_kind: amount is required for kind=percent".into())
            })?;
            if v < Decimal::ZERO || v > Decimal::from(100) {
                return Err(ApiError::BadRequest(
                    "percent_out_of_range: amount (percent) must be in [0, 100]".into(),
                ));
            }
            Ok(Some(v))
        }
        _ => unreachable!("normalize_kind already validated"),
    }
}

fn normalize_cap_pair(
    kind: Option<&str>,
    value: Option<Decimal>,
) -> Result<(Option<String>, Option<Decimal>), ApiError> {
    match (kind.map(str::trim).filter(|s| !s.is_empty()), value) {
        (None, None) => Ok((None, None)),
        (Some(k @ ("amount" | "months_expense" | "income_multiple")), Some(v)) => {
            if v < Decimal::ZERO {
                return Err(ApiError::BadRequest("cap_value_negative: cap_value must be >= 0".into()));
            }
            Ok((Some(k.into()), Some(v)))
        }
        (Some(other), Some(_)) => Err(ApiError::BadRequest(format!(
            "cap_kind_invalid: cap_kind must be 'amount' | 'months_expense' | 'income_multiple', got {other:?}"
        ))),
        _ => Err(ApiError::BadRequest(
            "cap_pair_incomplete: cap_kind and cap_value must be provided together".into(),
        )),
    }
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 4000 {
                return Err(ApiError::BadRequest(
                    "notes_too_long: notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

/// Verifies that the target asset exists in the same scope. For `?view=mine`, the asset must
/// belong to the user (or be a household row visible to them in their scope).
async fn assert_asset_in_scope(
    conn: &mut PgConnection,
    iid: Uuid,
    asset_id: Uuid,
    owner_filter: SinkScope,
) -> Result<(), ApiError> {
    let ok: bool = match owner_filter {
        None => {
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM assets
                    WHERE id = $1 AND installation_id = $2
                )"#,
            )
            .bind(asset_id)
            .bind(iid)
            .fetch_one(&mut *conn)
            .await?
        }
        Some(uid) => {
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM assets
                    WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3
                )"#,
            )
            .bind(asset_id)
            .bind(iid)
            .bind(uid)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    if !ok {
        return Err(ApiError::BadRequest(
            "target_asset_not_found: target_asset_id must reference an asset in your scope".into(),
        ));
    }
    Ok(())
}

// ===========================================================================
// I1 — el sumidero de la cascada. ÚNICO sitio donde se decide.
// ===========================================================================
//
// El invariante (architecture-contract I1) es: **como mucho un `remainder` sin tope por scope y,
// si existe, es el último de la cascada**. Hasta 4.4.0 estaba REPARTIDO — un trozo en el `create`
// (rechazar el segundo sumidero + colocar la fila nueva justo antes del sumidero), otro en el
// `patch` (dos comprobaciones de conteo), otro en el `delete` y otro en el `reorder` — y esa
// dispersión tenía dos agujeros reales:
//
//   1. `patch` podía **convertir en sumidero** una regla que no era la última (con `n == 0` la
//      guardia de conteo pasaba y nadie movía la prioridad). La cascada quedaba con su sumidero en
//      medio: todo lo que hubiera por debajo dejaba de recibir, en silencio.
//   2. La guardia `sink_must_be_last` del `reorder` resolvía el scope desde la VISTA, y en
//      `household` eso es `owner_user_id IS NULL` — que no casa ninguna fila creada por la API
//      (el alta siempre escribe un owner). O sea: no llegaba a mirar nada.
//
// La forma de arreglarlo no es añadir una tercera guardia: es dejar de comprobar el cambio y pasar
// a comprobar el **estado resultante**. `assert_sink_invariant` es una POST-condición sobre lo ya
// escrito, y `commit_with_sink_invariant` es **el único `tx.commit()` del módulo**. Un camino de
// escritura nuevo que se olvide de llamarlo no corrompe nada: su transacción se cae al hacer
// `drop` y no escribe. El test `el_modulo_tiene_un_unico_punto_de_commit` fija esa unicidad.

/// La única definición de «sumidero» del repo: `remainder` SIN tope.
///
/// Cualquier sitio que quiera preguntarlo llama aquí en vez de reescribir
/// `kind == "remainder" && cap_kind.is_none()` — la expresión que estaba copiada en el `create`, en
/// las dos guardias del `patch` y en el `delete`. (El `FILTER (WHERE kind = 'remainder' AND
/// cap_kind IS NULL)` de `assets.rs::asset_delete_effects` es la misma condición **en SQL**, dentro
/// de un `COUNT`, y por eso no puede llamar aquí: si algún día cambia la definición del sumidero,
/// esa query es el otro sitio que hay que tocar.)
fn is_sink(kind: &str, cap_kind: Option<&str>) -> bool {
    kind == "remainder" && cap_kind.is_none()
}

/// El scope de I1 es el **owner de la fila**, no la vista del listado.
///
/// No es un detalle: `LedgerView::Household` es `installation_id = $1` — mezcla las filas de todos
/// los miembros en una sola cascada, y en un hogar de dos personas cada una tiene su propio
/// sumidero. Exigir «uno solo, el último» sobre la unión haría imposible que las dos lo tengan.
/// `create`/`patch`/`delete` ya trabajaban por owner; el `reorder` era el único que lo derivaba de
/// la vista, y por eso su guardia estaba muerta.
type SinkScope = Option<Uuid>;

#[derive(Debug, FromRow)]
struct SinkProbeRow {
    id: Uuid,
    priority: i32,
    kind: String,
    cap_kind: Option<String>,
}

/// POST-CONDICIÓN de I1 sobre el estado **ya escrito**, dentro de la transacción.
///
/// Tres desenlaces, con los mismos códigos estables que las guardias que sustituye:
/// - dos o más sumideros → `uncapped_remainder_exists`;
/// - un sumidero que no es el último por `(priority, id)` → `sink_must_be_last`;
/// - **cero sumideros → OK**. No es un descuido: un scope puede no tener sumidero (una instalación
///   recién creada no tiene ninguna regla), y el sobrante se va a `surplus_cash`. Lo que I1 prohíbe
///   es *quedarse sin* el que había, y eso lo comprueban las pre-guardias de `patch`/`delete` con
///   `remainder_required` — una post-condición no puede distinguir «nunca hubo» de «lo borraste».
///
/// `(priority, id)` es el orden EXACTO con el que se lee la cascada (`ORDER BY priority ASC, id
/// ASC` en `list_allocation_rules_core` y en el ensamblado del engine): comparar solo por
/// `priority` dejaría pasar un empate en el que el sumidero pierde el desempate por `id`.
async fn assert_sink_invariant(
    conn: &mut PgConnection,
    iid: Uuid,
    scope: SinkScope,
) -> Result<(), ApiError> {
    let rows: Vec<SinkProbeRow> = match scope {
        None => {
            sqlx::query_as(
                r#"SELECT id, priority, kind, cap_kind FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL"#,
            )
            .bind(iid)
            .fetch_all(&mut *conn)
            .await?
        }
        Some(uid) => {
            sqlx::query_as(
                r#"SELECT id, priority, kind, cap_kind FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?
        }
    };

    let sinks: Vec<&SinkProbeRow> = rows
        .iter()
        .filter(|r| is_sink(&r.kind, r.cap_kind.as_deref()))
        .collect();
    if sinks.len() > 1 {
        return Err(ApiError::BadRequest(
            "uncapped_remainder_exists: only one 'remainder' rule without a cap is allowed per scope".into(),
        ));
    }
    if let Some(sink) = sinks.first() {
        let last = rows
            .iter()
            .max_by_key(|r| (r.priority, r.id))
            .expect("el scope contiene al menos el sumidero que acabamos de encontrar");
        if last.id != sink.id {
            return Err(ApiError::BadRequest(
                "sink_must_be_last: the uncapped 'remainder' rule must remain the last in the cascade".into(),
            ));
        }
    }
    Ok(())
}

/// **EL ÚNICO `tx.commit()` del módulo.** Verifica I1 en cada scope tocado y solo entonces
/// confirma. Un camino de escritura que no pase por aquí no escribe: su transacción se revierte.
async fn commit_with_sink_invariant(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    iid: Uuid,
    scopes: &[SinkScope],
) -> Result<(), ApiError> {
    let mut seen: Vec<SinkScope> = Vec::with_capacity(scopes.len());
    for scope in scopes {
        if seen.contains(scope) {
            continue;
        }
        seen.push(*scope);
        assert_sink_invariant(&mut tx, iid, *scope).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Returns the count of `remainder` rules with NO cap in the given scope. These act as the
/// "catch-all" sink at the end of the cascade.
async fn count_uncapped_remainder_rules(
    conn: &mut PgConnection,
    iid: Uuid,
    owner_filter: SinkScope,
) -> Result<i64, ApiError> {
    let n: i64 = match owner_filter {
        None => {
            sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL
                     AND kind = 'remainder' AND cap_kind IS NULL"#,
            )
            .bind(iid)
            .fetch_one(&mut *conn)
            .await?
        }
        Some(uid) => {
            sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2
                     AND kind = 'remainder' AND cap_kind IS NULL"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(n)
}

/// Returns `(id, priority)` of the uncapped-remainder rule in the scope, if any.
async fn find_uncapped_remainder(
    conn: &mut PgConnection,
    iid: Uuid,
    owner_filter: SinkScope,
) -> Result<Option<(Uuid, i32)>, ApiError> {
    let row: Option<(Uuid, i32)> = match owner_filter {
        None => {
            sqlx::query_as(
                r#"SELECT id, priority FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL
                     AND kind = 'remainder' AND cap_kind IS NULL
                   ORDER BY priority DESC LIMIT 1"#,
            )
            .bind(iid)
            .fetch_optional(&mut *conn)
            .await?
        }
        Some(uid) => {
            sqlx::query_as(
                r#"SELECT id, priority FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2
                     AND kind = 'remainder' AND cap_kind IS NULL
                   ORDER BY priority DESC LIMIT 1"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    Ok(row)
}

/// Prioridad máxima usada en un scope, o 0 si está vacío. `MAX + 1` es «el último de la cascada».
async fn max_priority_in_scope(
    conn: &mut PgConnection,
    iid: Uuid,
    scope: SinkScope,
) -> Result<i32, ApiError> {
    let n: i32 = match scope {
        None => {
            sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(priority), 0) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id IS NULL"#,
            )
            .bind(iid)
            .fetch_one(&mut *conn)
            .await?
        }
        Some(uid) => {
            sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(priority), 0) FROM allocation_rules
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(uid)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(n)
}

/// Qué superficies pueden crear el **sumidero**.
///
/// La asimetría que obliga a distinguirlas: crear un sumidero donde no había redirige TODO el
/// sobrante de golpe, y **no se puede deshacer por el mismo canal** — borrar el único sumidero
/// devuelve `remainder_required`, así que la única salida es un `update` que lo convierta en otra
/// cosa, es decir dos llamadas y saber que hay que darlas. Un formulario que enseña la cascada
/// entera hace evidente ese estado; una conversación, no. Por eso la superficie segura desde el
/// chat es **más estrecha** que «crear cualquier regla»: capadas sí, sumidero no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkPolicy {
    /// La SPA (`POST /v1/allocation-rules`): puede crear el sumidero.
    Allowed,
    /// Superficies conversacionales (la tool MCP `create_allocation_rule`): no puede.
    Forbidden,
}

#[utoipa::path(
    get,
    path = "/v1/allocation-rules",
    tag = "allocation-rules",
    params(
        ("view" = Option<String>, Query, description = "`mine` = rows attributed to the signed-in user; omit = full household."),
    ),
    responses(
        (status = 200, description = "Rules ordered by priority ascending", body = [AllocationRuleResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_allocation_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<AllocationRuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_allocation_rules_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_allocation_rules`.
pub(crate) async fn list_allocation_rules_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<AllocationRuleResponse>, ApiError> {
    let scope = view.scope_where("");
    let sql = format!(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE {scope}
           ORDER BY priority ASC, id ASC"#
    );
    let rows: Vec<RuleRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().map(row_to_response).collect())
}

#[utoipa::path(
    post,
    path = "/v1/allocation-rules",
    tag = "allocation-rules",
    request_body = CreateAllocationRuleBody,
    responses(
        (status = 201, description = "Created", body = AllocationRuleResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn create_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateAllocationRuleBody>,
) -> Result<(StatusCode, Json<AllocationRuleResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp =
        create_allocation_rule_core(&state, iid, user.id.0, body, SinkPolicy::Allowed).await?;
    Ok((StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_allocation_rule`.
///
/// Cierra el flujo que `create_asset` invita a hacer en su propio ejemplo («he abierto un fondo
/// nuevo, mete 200 €/mes») y que hasta ahora se cortaba por la mitad: el activo se creaba y las
/// aportaciones no se podían encaminar. Cierra además una asimetría **destructiva**:
/// `delete_asset` se lleva en cascada las reglas que apuntaban al activo, y ninguna tool sabía
/// recrearlas — una operación reversible en la app era irreversible por MCP.
///
/// `sink_policy` decide si el llamante puede crear el **sumidero** (ver [`SinkPolicy`]).
/// La colocación de la fila nueva es parte de I1 y vive aquí:
/// - sumidero nuevo → `MAX(priority) + 1` del scope (último);
/// - regla normal con sumidero existente → toma la prioridad del sumidero y **empuja al sumidero
///   uno más abajo**, para que siga siendo el último;
/// - sin sumidero → al final.
///
/// **Cache FULL** dentro: las reglas cambian el reparto del ahorro, o sea la proyección.
pub(crate) async fn create_allocation_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateAllocationRuleBody,
    sink_policy: SinkPolicy,
) -> Result<AllocationRuleResponse, ApiError> {
    let kind = normalize_kind(&body.kind)?;
    let amount = validate_kind_amount(&kind, body.amount)?;
    let (cap_kind, cap_value) = normalize_cap_pair(body.cap_kind.as_deref(), body.cap_value)?;
    let notes = normalize_notes(&body.notes)?;
    let enabled = body.enabled.unwrap_or(true);

    let owner: SinkScope = Some(user_id);
    let is_uncapped_remainder = is_sink(&kind, cap_kind.as_deref());

    if is_uncapped_remainder && sink_policy == SinkPolicy::Forbidden {
        return Err(ApiError::BadRequest(
            "sink_creation_not_allowed: creating the uncapped 'remainder' rule (the cascade sink) is not available from this surface; give the rule a cap, or set the sink from the app".into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    assert_asset_in_scope(&mut tx, iid, body.target_asset_id, owner).await?;

    let existing_sink = find_uncapped_remainder(&mut tx, iid, owner).await?;
    if is_uncapped_remainder && existing_sink.is_some() {
        return Err(ApiError::BadRequest(
            "uncapped_remainder_exists: only one 'remainder' rule without a cap is allowed per scope".into(),
        ));
    }

    let new_priority: i32 = match (is_uncapped_remainder, existing_sink) {
        // Regla normal con sumidero: se cuela justo antes y el sumidero baja un puesto, para que
        // siga siendo el último.
        (false, Some((sink_id, sink_priority))) => {
            sqlx::query(
                r#"UPDATE allocation_rules SET priority = priority + 1
                   WHERE id = $1 AND installation_id = $2"#,
            )
            .bind(sink_id)
            .bind(iid)
            .execute(&mut *tx)
            .await?;
            sink_priority
        }
        // Sumidero nuevo (no hay otro: lo acabamos de comprobar) o regla normal sin sumidero: al
        // final de la cascada.
        _ => max_priority_in_scope(&mut tx, iid, owner).await? + 1,
    };

    let row: RuleRow = sqlx::query_as(
        r#"INSERT INTO allocation_rules (
               installation_id, owner_user_id, target_asset_id, priority,
               kind, amount, cap_kind, cap_value, enabled, notes
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, owner_user_id, target_asset_id, priority, kind, amount,
                     cap_kind, cap_value, enabled, notes"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(body.target_asset_id)
    .bind(new_priority)
    .bind(&kind)
    .bind(amount)
    .bind(&cap_kind)
    .bind(cap_value)
    .bind(enabled)
    .bind(&notes)
    .fetch_one(&mut *tx)
    .await?;
    commit_with_sink_invariant(tx, iid, &[owner]).await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(row_to_response(row))
}

#[utoipa::path(
    patch,
    path = "/v1/allocation-rules/{id}",
    tag = "allocation-rules",
    request_body = PatchAllocationRuleBody,
    params(
        ("id" = Uuid, Path, description = "Rule id"),
    ),
    responses(
        (status = 200, description = "Updated", body = AllocationRuleResponse),
        (status = 400, description = "Validation error or would orphan remainder"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Rule missing"),
    )
)]
pub async fn patch_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAllocationRuleBody>,
) -> Result<Json<AllocationRuleResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_allocation_rule_core(&state, iid, user.id.0, id, body, SinkPolicy::Allowed).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_allocation_rule` (subset
/// amount/cap/enabled — sin create/delete/reorder desde chat). La invariante del sink
/// (`remainder_required` / `uncapped_remainder_exists`) vive AQUÍ dentro: reimplementarla en
/// otro camino es la vía rápida a corromper la cascada. Invalidación FULL dentro.
pub(crate) async fn patch_allocation_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchAllocationRuleBody,
    sink_policy: SinkPolicy,
) -> Result<AllocationRuleResponse, ApiError> {
    // Destructuring EXHAUSTIVO y **sin `..`**: añadir un campo al body deja de compilar hasta que
    // alguien decida si cuenta como «algo que actualizar». Ésta era una de las dos únicas cores de
    // PATCH del repo sin `patch_empty` (`assets.rs`, `budget.rs`, `liabilities.rs`, `planning.rs` e
    // `installation.rs` sí lo tienen), y por eso la tool MCP tuvo que escribirse su propia guardia
    // a mano — donde se olvidó `cap_value` y el campo se evaporaba con un 200 (auditoría MCP §5). Una
    // guardia que enumera campos siempre puede olvidarse uno; ésta la verifica el compilador.
    {
        let PatchAllocationRuleBody {
            target_asset_id,
            kind,
            amount,
            cap,
            enabled,
            notes,
        } = &body;
        if target_asset_id.is_none()
            && kind.is_none()
            && amount.is_none()
            && cap.is_none()
            && enabled.is_none()
            && notes.is_none()
        {
            return Err(ApiError::BadRequest(
                "patch_empty: provide at least one field to update".into(),
            ));
        }
    }

    let mut tx = state.pool.begin().await?;
    let current: Option<RuleRow> = sqlx::query_as(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    let new_kind = match &body.kind {
        Some(k) => normalize_kind(k)?,
        None => current.kind.clone(),
    };

    let new_amount = match &body.amount {
        None => {
            // Keep current value, but ensure validity against possibly-changed kind.
            validate_kind_amount(&new_kind, current.amount)?
        }
        Some(v) if v.is_null() => validate_kind_amount(&new_kind, None)?,
        Some(v) => {
            let d: Decimal = if let serde_json::Value::String(s) = v {
                s.trim().parse().map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: amount must be a valid decimal string".into())
                })?
            } else {
                serde_json::from_value(v.clone()).map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: amount must be a valid decimal".into())
                })?
            };
            validate_kind_amount(&new_kind, Some(d))?
        }
    };

    let (new_cap_kind, new_cap_value) = match &body.cap {
        None => (current.cap_kind.clone(), current.cap_value),
        Some(v) if v.is_null() => (None, None),
        Some(v) => {
            let obj = v.as_object().ok_or_else(|| {
                ApiError::BadRequest("cap_type_invalid: cap must be null or an object {kind, value}".into())
            })?;
            let kind = obj.get("kind").and_then(|x| x.as_str());
            let raw_val = obj.get("value");
            let value: Option<Decimal> = match raw_val {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(s)) => Some(s.trim().parse().map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: cap.value must be a valid decimal string".into())
                })?),
                Some(other) => Some(serde_json::from_value(other.clone()).map_err(|_| {
                    ApiError::BadRequest("decimal_invalid: cap.value must be a valid decimal".into())
                })?),
            };
            normalize_cap_pair(kind, value)?
        }
    };

    // La puerta del sumidero es sobre el ESTADO RESULTANTE, no sobre la operación: sin esto,
    // `SinkPolicy::Forbidden` en el create era saltable en dos pasos —crear un `remainder` CON
    // tope (legítimo) y quitárselo aquí con `cap: null`—, y la descripción de la tool prometía lo
    // contrario. La superficie que no puede crear el sumidero tampoco puede fabricarlo editando.
    if sink_policy == SinkPolicy::Forbidden
        && is_sink(&new_kind, new_cap_kind.as_deref())
        && !is_sink(&current.kind, current.cap_kind.as_deref())
    {
        return Err(ApiError::BadRequest(
            "sink_creation_not_allowed: turning a rule into the uncapped 'remainder' (the cascade sink) is not available from this surface; keep a cap, or set the sink from the app".into(),
        ));
    }

    let new_target = body.target_asset_id.unwrap_or(current.target_asset_id);
    if new_target != current.target_asset_id {
        assert_asset_in_scope(&mut tx, iid, new_target, current.owner_user_id).await?;
    }

    let new_enabled = body.enabled.unwrap_or(current.enabled);
    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    // PRE-guardia de I1 que la post-condición NO puede dar: «no te quedes sin el sumidero que
    // tenías». `assert_sink_invariant` acepta cero sumideros (un scope recién creado no tiene
    // ninguno), así que es aquí donde se distingue «nunca hubo» de «acabas de quitarlo».
    let was_sink = is_sink(&current.kind, current.cap_kind.as_deref());
    let becomes_sink = is_sink(&new_kind, new_cap_kind.as_deref());
    if was_sink && !becomes_sink {
        let n = count_uncapped_remainder_rules(&mut tx, iid, current.owner_user_id).await?;
        if n <= 1 {
            return Err(ApiError::BadRequest(
                "remainder_required: scope must keep one uncapped 'remainder' rule (catch-all sink)".into(),
            ));
        }
    }

    // Convertirse en sumidero implica **irse al final**. Hasta 4.4.0 el PATCH no movía la
    // prioridad: con el scope sin sumidero previo, la guardia de conteo pasaba y la cascada se
    // quedaba con su sumidero en medio, comiéndose el sobrante antes de que las reglas de debajo
    // lo vieran. Ahora se recoloca aquí, y si algo se escapara la post-condición lo corta con
    // `sink_must_be_last` en vez de escribirlo.
    let new_priority = if becomes_sink && !was_sink {
        max_priority_in_scope(&mut tx, iid, current.owner_user_id).await? + 1
    } else {
        current.priority
    };

    let updated: RuleRow = sqlx::query_as(
        r#"UPDATE allocation_rules
           SET target_asset_id = $1,
               kind = $2,
               amount = $3,
               cap_kind = $4,
               cap_value = $5,
               enabled = $6,
               notes = $7,
               priority = $8
           WHERE id = $9 AND installation_id = $10
           RETURNING id, owner_user_id, target_asset_id, priority, kind, amount,
                     cap_kind, cap_value, enabled, notes"#,
    )
    .bind(new_target)
    .bind(&new_kind)
    .bind(new_amount)
    .bind(&new_cap_kind)
    .bind(new_cap_value)
    .bind(new_enabled)
    .bind(&new_notes)
    .bind(new_priority)
    .bind(id)
    .bind(iid)
    .fetch_one(&mut *tx)
    .await?;
    commit_with_sink_invariant(tx, iid, &[current.owner_user_id]).await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(row_to_response(updated))
}

#[utoipa::path(
    delete,
    path = "/v1/allocation-rules/{id}",
    tag = "allocation-rules",
    params(
        ("id" = Uuid, Path, description = "Rule id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Cannot delete the last 'remainder' rule"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Rule missing"),
    )
)]
pub async fn delete_allocation_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    delete_allocation_rule_core(&state, iid, user.id.0, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Efectos de borrar una regla, para el preview de la tool MCP `delete_allocation_rule`.
///
/// Lo que importa aquí no es «desaparece una fila» sino **a dónde deja de ir el dinero**: la
/// cifra que la regla se está llevando este mes sale de la MISMA resolución que publica
/// `GET /v1/allocation-rules/resolution`, no de una aproximación propia.
#[derive(Debug, serde::Serialize)]
pub(crate) struct AllocationRuleDeleteEffects {
    pub rule_id: Uuid,
    pub priority: i32,
    /// `fixed` | `percent` | `remainder`.
    pub kind: String,
    pub target_asset_id: Uuid,
    pub target_asset_name: Option<String>,
    /// `true` ⟺ es el sumidero (`remainder` sin tope).
    pub is_sink: bool,
    /// Lo que esta regla encamina **este mes** (`amount_resolved` de la cascada resuelta). `null`
    /// cuando la regla no aparece en la resolución — p.ej. su activo destino quedó fuera del scope.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub amount_resolved_this_month: Option<Decimal>,
    /// `remainder_required` cuando el borrado va a ser rechazado por ser el ÚNICO sumidero del
    /// scope; `null` cuando se puede borrar. Es información, no un error: el preview la enseña para
    /// que el cliente no proponga una confirmación condenada de antemano.
    pub blocked_reason: Option<&'static str>,
}

/// Preview de [`delete_allocation_rule_core`]. Read-only, sin transacción.
#[allow(dead_code)]
pub(crate) async fn allocation_rule_delete_effects(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<AllocationRuleDeleteEffects, ApiError> {
    let current: Option<(String, Option<String>, Option<Uuid>, i32, Uuid)> = sqlx::query_as(
        r#"SELECT kind, cap_kind, owner_user_id, priority, target_asset_id
           FROM allocation_rules
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(pool)
    .await?;
    let Some((kind, cap_kind, owner, priority, target_asset_id)) = current else {
        return Err(ApiError::NotFound);
    };
    let sink = is_sink(&kind, cap_kind.as_deref());

    let blocked_reason = if sink {
        let mut conn = pool.acquire().await?;
        let n = count_uncapped_remainder_rules(&mut conn, iid, owner).await?;
        (n <= 1).then_some("remainder_required")
    } else {
        None
    };

    // La cascada resuelta del mes, en vista household (la que corre el engine por defecto).
    let resolution = allocation_resolution_core(pool, iid, user_id, LedgerView::Household).await?;
    let resolved = resolution.rules.iter().find(|r| r.rule_id == id);

    Ok(AllocationRuleDeleteEffects {
        rule_id: id,
        priority,
        kind,
        target_asset_id,
        target_asset_name: resolved.map(|r| r.target_asset_name.clone()),
        is_sink: sink,
        amount_resolved_this_month: resolved.map(|r| r.amount_resolved),
        blocked_reason,
    })
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_allocation_rule`.
///
/// La pre-guardia `remainder_required` (no te quedes sin sumidero) y la post-condición de
/// [`commit_with_sink_invariant`] se reparten el trabajo: la primera distingue «lo estás quitando»
/// de «nunca hubo», la segunda garantiza que el estado final es legal aunque el scope tuviera datos
/// heredados raros (dos sumideros, sumidero en medio).
///
/// **Cache FULL** dentro.
pub(crate) async fn delete_allocation_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let current: Option<(String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT kind, cap_kind, owner_user_id FROM allocation_rules
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((kind, cap_kind, owner)) = current else {
        return Err(ApiError::NotFound);
    };

    // Borrar el sumidero dejaría el scope huérfano.
    if is_sink(&kind, cap_kind.as_deref()) {
        let n = count_uncapped_remainder_rules(&mut tx, iid, owner).await?;
        if n <= 1 {
            return Err(ApiError::BadRequest(
                "remainder_required: scope must keep one uncapped 'remainder' rule (catch-all sink)".into(),
            ));
        }
    }

    sqlx::query(r#"DELETE FROM allocation_rules WHERE id = $1 AND installation_id = $2"#)
        .bind(id)
        .bind(iid)
        .execute(&mut *tx)
        .await?;
    commit_with_sink_invariant(tx, iid, &[owner]).await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/v1/allocation-rules/reorder",
    tag = "allocation-rules",
    request_body = ReorderBody,
    params(
        ("view" = Option<String>, Query, description = "`mine` = scope by user; omit = household."),
    ),
    responses(
        (status = 200, description = "Reordered", body = [AllocationRuleResponse]),
        (status = 400, description = "ids does not match the scope or has duplicates"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn reorder_allocation_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
    Json(body): Json<ReorderBody>,
) -> Result<Json<Vec<AllocationRuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    // Validate no duplicates.
    let mut seen = std::collections::HashSet::new();
    for id in &body.ids {
        if !seen.insert(*id) {
            return Err(ApiError::BadRequest("ids_not_unique: ids must be unique".into()));
        }
    }

    // Load all current rules in this scope; the request must list exactly the same set.
    let view = q.resolve()?;
    let scope = view.scope_where("");
    let current_sql = format!("SELECT id, owner_user_id FROM allocation_rules WHERE {scope}");
    let current: Vec<(Uuid, Option<Uuid>)> = view
        .bind_scope_as(sqlx::query_as(&current_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;
    let current_set: std::collections::HashSet<Uuid> = current.iter().map(|(id, _)| *id).collect();
    if current_set.len() != body.ids.len() || !body.ids.iter().all(|id| current_set.contains(id)) {
        return Err(ApiError::BadRequest(
            "ids_do_not_match_scope: ids must exactly match the rules in this scope".into(),
        ));
    }

    // Los scopes de I1 tocados por esta reordenación: **todos los owners presentes**, no uno
    // derivado de la vista. En `household` la reordenación renumera filas de varias personas a la
    // vez y cada una tiene su propio sumidero; la guardia anterior resolvía el scope a
    // `owner_user_id IS NULL` y por tanto no comprobaba nada. La comprobación de que cada sumidero
    // sigue siendo el último **de su owner** la hace `commit_with_sink_invariant`, con el mismo
    // código `sink_must_be_last`.
    let touched_scopes: Vec<SinkScope> = current.iter().map(|(_, owner)| *owner).collect();

    let mut tx = state.pool.begin().await?;
    for (idx, id) in body.ids.iter().enumerate() {
        let new_priority = (idx as i32) + 1;
        sqlx::query(
            r#"UPDATE allocation_rules SET priority = $1
               WHERE id = $2 AND installation_id = $3"#,
        )
        .bind(new_priority)
        .bind(id)
        .bind(iid)
        .execute(&mut *tx)
        .await?;
    }
    commit_with_sink_invariant(tx, iid, &touched_scopes).await?;

    let rows_sql = format!(
        r#"SELECT id, owner_user_id, target_asset_id, priority, kind, amount,
                  cap_kind, cap_value, enabled, notes
           FROM allocation_rules
           WHERE {scope}
           ORDER BY priority ASC, id ASC"#
    );
    let rows: Vec<RuleRow> = view
        .bind_scope_as(sqlx::query_as(&rows_sql), iid, user.id.0)
        .fetch_all(&state.pool)
        .await?;

    refresh_projection_after_mutation(&state, iid, user.id.0).await;
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
}

// ---------------------------------------------------------------------------
// GET /v1/allocation-rules/resolution — la cascada resuelta de este mes
// ---------------------------------------------------------------------------

/// Una regla de la cascada, resuelta para el mes en curso.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolvedRule {
    #[schema(value_type = String, format = "uuid")]
    pub rule_id: Uuid,
    pub priority: i32,
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub target_asset_name: String,
    /// `fixed` | `percent` | `remainder`
    pub kind: String,
    /// Lo que la regla PIDIÓ antes de aplicar cap y caja disponible.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_intent: Decimal,
    /// Lo que la regla se llevó de verdad. Si es menor que `amount_intent` sin `skipped_reason`,
    /// la regla fue **recortada** (normalmente por el cap) — no saltada.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_resolved: Decimal,
    /// Techo absoluto del cap ya resuelto en euros (los caps relativos se evalúan con los
    /// escalares efectivos del mes).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_ceiling: Option<Decimal>,
    /// Espacio que quedaba bajo el techo al evaluar la regla.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub cap_room: Option<Decimal>,
    /// `no_cash` | `not_reached` | `cap_full` | `zero_amount` | `invalid_target`, o ausente si la
    /// regla recibió algo. Las razones **no se colapsan** porque tienen remedios distintos:
    /// `no_cash` es «no te sobra dinero» y `not_reached` es «las reglas de arriba se lo comieron».
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// Aporte resuelto por activo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolvedAssetContribution {
    #[schema(value_type = String, format = "uuid")]
    pub asset_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

/// Resolución completa de la cascada del mes en curso.
#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationResolutionResponse {
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view` — ver
    /// `SummaryResponse::view` para el porqué. Aquí decide qué reglas y qué activos entran en la
    /// cascada, así que dos resoluciones distintas pueden diferir solo en este campo.
    pub view: &'static str,
    /// Mes al que corresponde la resolución (`YYYY-MM`).
    pub month: String,
    /// La caja que la cascada reparte de verdad. **Incluye el tramo transitorio de planning**
    /// (desde #126 anclado al mes civil: constante dentro del mes, cambia al cambiar de mes hasta
    /// agotarse la rampa): es la explicación de por qué la aportación del mes 1 no cuadra con el
    /// neto recurrente del summary.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub base_cash: Decimal,
    /// `income − expense − debt_service`: la parte estable. Cuando `debt_service` es `null` el
    /// sustraendo es 0 (la cuota ya está dentro del gasto), así que la identidad sigue cerrando
    /// como `income − expense`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub recurring_net: Decimal,
    /// El tramo de los planning flows del mes en curso (menos la retirada de jubilación si aplica).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub planning_component: Decimal,
    /// Cuota mensual de los pasivos activos que la cascada descuenta. **`null` cuando la cifra no
    /// aplica** — contrato 4.3.1→4.7.x. Desde 4.8.0 (#142, opción 3) la cuota viaja como número
    /// en los TRES modos (en B/C el gasto efectivo ya la restó del promedio: publicarla es
    /// contarla una vez, no dos) y un `0` significa solo «no hay pasivos con cuota activa».
    /// La nullabilidad del campo se conserva por forma.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub debt_service: Option<Decimal>,
    /// Desde 4.8.0 siempre `null` (ver arriba); el literal `included_in_real_expense` se retiró
    /// con el contrato que lo justificaba. Retirar el campo es un breaking §5 aparte.
    #[schema(value_type = Option<String>)]
    pub debt_service_absent_reason: Option<&'static str>,
    /// `true` cuando `planning_component != 0`: avisa de que `base_cash` lleva dentro un término
    /// que se agota en 90 días y que por tanto **no** es un importe mensual estable.
    pub base_includes_transient: bool,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub allocated_total: Decimal,
    /// Lo que ninguna regla absorbió y acaba en `surplus_cash`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub leftover_to_surplus_cash: Decimal,
    pub rules: Vec<ResolvedRule>,
    pub per_asset: Vec<ResolvedAssetContribution>,
}

#[utoipa::path(
    get,
    path = "/v1/allocation-rules/resolution",
    tag = "allocation-rules",
    params(("view" = Option<String>, Query, description = "`mine` | household.")),
    responses(
        (status = 200, description = "Cascada resuelta del mes en curso", body = AllocationResolutionResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_allocation_resolution(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<AllocationResolutionResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out =
        allocation_resolution_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_allocation_resolution`.
///
/// Endpoint **nuevo** en vez de envolver `list_allocation_rules`: convertir aquel array en un
/// objeto habría roto el contrato. Construye su propio `ProjectionInput` con horizonte 1 (mismo
/// coste que `GET /v1/assets`, una tanda de SELECTs) y **no** pasa por la cache de proyección —
/// coherente con `assets_projection_context`.
pub(crate) async fn allocation_resolution_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<AllocationResolutionResponse, ApiError> {
    use crate::handlers::installation::load_fire_settings;
    use crate::handlers::projection::{build_installation_projection_input, map_engine_err};

    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    let fire_settings = load_fire_settings(pool, iid).await?;
    let built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        today,
        1,
        Decimal::ZERO,
        Some(&fire_settings),
        None,
    )
    .await?;
    let alloc =
        futurefin_engine::first_month_allocation(&built.input).map_err(map_engine_err)?;

    // `priority` y `kind` no viven en el engine: se releen de la tabla y se mapean por id.
    let meta = list_allocation_rules_core(pool, iid, user_id, view).await?;

    let rules: Vec<ResolvedRule> = alloc
        .rules
        .iter()
        .filter_map(|r| {
            let rule_id = *built.allocation_rule_ids.get(r.rule_index)?;
            let m = meta.iter().find(|m| m.id == rule_id)?;
            let (target_asset_id, target_asset_name) =
                built.asset_id_name.get(r.target_index).cloned()?;
            Some(ResolvedRule {
                rule_id,
                priority: m.priority,
                target_asset_id,
                target_asset_name,
                kind: m.kind.clone(),
                amount_intent: r.amount_intent.round_dp(4),
                amount_resolved: r.amount_resolved.round_dp(4),
                cap_ceiling: r.cap_ceiling.map(|v: Decimal| v.round_dp(4)),
                cap_room: r.cap_room.map(|v: Decimal| v.round_dp(4)),
                skipped_reason: r.skipped_reason.map(|s| skip_reason_wire(s).to_string()),
            })
        })
        .collect();

    let per_asset: Vec<ResolvedAssetContribution> = built
        .asset_id_name
        .iter()
        .zip(alloc.per_asset.iter())
        .map(|((asset_id, name), amount)| ResolvedAssetContribution {
            asset_id: *asset_id,
            name: name.clone(),
            amount: amount.round_dp(4),
        })
        .collect();

    let allocated_total: Decimal = alloc.per_asset.iter().copied().sum();

    Ok(AllocationResolutionResponse {
        view: view.as_str(),
        month: today.format("%Y-%m").to_string(),
        base_cash: alloc.base_cash.round_dp(4),
        recurring_net: alloc.recurring_net.round_dp(4),
        planning_component: alloc.planning_component.round_dp(4),
        // Misma regla que `simulate_projection`: la razón se decide en el ensamblado
        // (`BuiltProjection::debt_service_absent_reason`, gate `expense_from_avg`) y las dos
        // superficies la consumen — nunca se re-deriva aquí a partir del modo.
        debt_service: built
            .debt_service_absent_reason
            .is_none()
            .then(|| alloc.debt_service.round_dp(4)),
        debt_service_absent_reason: built.debt_service_absent_reason,
        base_includes_transient: !alloc.planning_component.is_zero(),
        allocated_total: allocated_total.round_dp(4),
        leftover_to_surplus_cash: alloc.leftover.round_dp(4),
        rules,
        per_asset,
    })
}

/// Nombre en el wire de cada razón. Se mapea a mano (y no con `Serialize` en el engine) para que
/// el crate del motor no acabe conociendo el formato de la API.
fn skip_reason_wire(r: futurefin_engine::AllocationSkipReason) -> &'static str {
    use futurefin_engine::AllocationSkipReason as R;
    match r {
        R::NoCash => "no_cash",
        R::NotReached => "not_reached",
        R::CapFull => "cap_full",
        R::ZeroAmount => "zero_amount",
        R::InvalidTarget => "invalid_target",
        R::InRetirement => "in_retirement",
    }
}

// ---------------------------------------------------------------------------
// GET /v1/allocation-rules/goals — cuándo se llena cada tope
// ---------------------------------------------------------------------------

/// Resuelve el tope de una regla a **euros absolutos** con los escalares del mes en curso.
///
/// Es la única implementación del lado API (la otra vive dentro del engine, `resolve_cap_ceiling`,
/// y es privada). La consumen `GET /v1/assets` —para publicar `contribution_target_amount`— y el
/// endpoint de objetivos de aquí abajo. Que sea una sola importa: el «objetivo» que enseña la
/// pantalla de Activos y el «techo» contra el que se calcula el ETA tienen que ser el MISMO número,
/// o el usuario ve una fecha para un objetivo que no es el que la app le muestra.
///
/// El acuerdo con el engine está fijado por `allocation_resolution.rs::goal_ceilings_match_the_engine_resolution`.
pub(crate) fn resolve_cap_ceiling_eur(
    cap_kind: &str,
    cap_value: Decimal,
    income_monthly: Decimal,
    expense_with_debt: Decimal,
) -> Option<Decimal> {
    // Adaptador string→enum SIN fórmula propia: desde la Ola 1 (issue #96) la única
    // implementación del techo es `futurefin_engine::resolve_cap_ceiling` — este helper existía
    // porque el motor emitía `cap_ceiling: null` en meses sin sobrante y había que duplicar la
    // resolución; el motor ya lo resuelve siempre, y duplicar la aritmética aquí es exactamente
    // lo que dejó de hacer falta.
    let cap = match cap_kind {
        "amount" => futurefin_engine::AllocationCap::Amount(cap_value),
        "months_expense" => futurefin_engine::AllocationCap::MonthsExpense(cap_value),
        "income_multiple" => futurefin_engine::AllocationCap::IncomeMultiple(cap_value),
        _ => return None,
    };
    futurefin_engine::resolve_cap_ceiling(Some(cap), expense_with_debt, income_monthly)
}

/// Un objetivo de la cascada: una regla **con tope**, su techo en euros y cuándo se alcanza.
#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationGoal {
    #[schema(value_type = String, format = "uuid")]
    pub rule_id: Uuid,
    pub priority: i32,
    #[schema(value_type = String, format = "uuid")]
    pub target_asset_id: Uuid,
    pub target_asset_name: String,
    /// `amount` | `months_expense` | `income_multiple`.
    pub cap_kind: String,
    /// El tope **tal y como está configurado** (euros para `amount`, un número de meses o un
    /// múltiplo para los otros dos). No es el techo en euros: ése es `ceiling`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub cap_value: Decimal,
    /// El techo resuelto a euros con los escalares de HOY.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub ceiling: Decimal,
    /// **De dónde sale `ceiling`, y si se mueve.** Enumeración cerrada:
    ///
    /// - `fixed_amount` — el tope ya venía en euros: el techo es constante y el ETA es exacto.
    /// - `income_multiple_today` — `n × ingreso mensual`. El engine usa el ingreso regular
    ///   **constante** hasta la jubilación, así que el techo tampoco se mueve antes de ella.
    /// - `months_expense_today` — `n × (gasto + cuota de deuda)`. **Éste sí se mueve**: la cuota
    ///   baja según se amortizan los pasivos, así que el techo real DECRECE con el tiempo y el ETA
    ///   calculado con el techo de hoy es **conservador** (la fecha real llega antes o igual).
    pub ceiling_basis: &'static str,
    /// Valor actual del activo destino (mes 0 de la proyección).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub current_value: Decimal,
    /// `current_value / ceiling × 100`. `null` cuando el techo es 0 (no hay porcentaje que dar,
    /// y un `100` ahí significaría «ya está» cuando lo que pasa es que el objetivo es vacío).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub progress_pct: Option<Decimal>,
    /// Primer mes de la proyección en que el activo destino alcanza `ceiling`. **Es un número de
    /// mes** (misma base que `points[].month_index` de la proyección), no una posición de array:
    /// 0 = ya alcanzado hoy. `null` con `eta_absent_reason` puesto.
    pub eta_month_index: Option<u32>,
    /// Primer día del mes de `eta_month_index` (`YYYY-MM-DD`), para no obligar a nadie a rehacer la
    /// aritmética de calendario. `null` exactamente cuando `eta_month_index` lo es.
    pub eta_month_ymd: Option<String>,
    /// `already_reached` | `not_within_horizon` | `zero_ceiling`. `null` ⟺ hay ETA futura.
    /// `not_within_horizon` **no** es «nunca»: es «no dentro de los `horizon_months` simulados».
    #[schema(value_type = Option<String>)]
    pub eta_absent_reason: Option<&'static str>,
}

/// Objetivos derivados de los topes de la cascada.
#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationGoalsResponse {
    /// Vista efectivamente aplicada: `household` | `mine`. Decide qué reglas y qué activos entran.
    pub view: &'static str,
    /// Mes 0 de la proyección con la que se cruzan los techos (`YYYY-MM-DD`).
    pub anchor_date_ymd: String,
    /// Horizonte simulado, y de dónde sale (`lifespan_90` | `fallback_no_demographics`). Un
    /// `not_within_horizon` significa exactamente «más allá de estos meses».
    pub horizon_months: u32,
    pub horizon_basis: String,
    /// Reglas de la cascada **sin tope**, que por tanto no son un objetivo: el sumidero y
    /// cualquier `fixed`/`percent` sin cap. Se publica el número para que una lista corta no se
    /// lea como «no tienes reglas».
    pub rules_without_cap: i64,
    pub goals: Vec<AllocationGoal>,
}

#[utoipa::path(
    get,
    path = "/v1/allocation-rules/goals",
    tag = "allocation-rules",
    params(("view" = Option<String>, Query, description = "`mine` | household.")),
    responses(
        (status = 200, description = "Objetivos (topes) de la cascada con su fecha estimada", body = AllocationGoalsResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_allocation_goals(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<AllocationGoalsResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = allocation_goals_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: la comparten el handler GET y la tool MCP `list_goals`.
///
/// **Por qué no hay tabla `goals`.** El tope YA es el objetivo: `cap_kind='months_expense'` con
/// valor 6 es literalmente «un fondo de emergencia de 6 meses», y `cap_kind='amount'` es un
/// objetivo en euros. Una tabla nueva duplicaría ese número y las dos copias se separarían — la
/// misma lección que dejaron las contribuciones por activo (failure-archaeology). Lo único que
/// faltaba era el **cuándo**, y eso sale de cruzar la serie por activo de la proyección con el
/// techo del tope.
///
/// **Cero fórmulas nuevas**: la serie la produce `project_net_worth_series` (el mismo motor que
/// `/v1/projection/series`) y el techo, `resolve_cap_ceiling_eur` con los escalares del ensamblado.
/// Aquí solo se busca el primer cruce.
///
/// **No pasa por la cache de proyección** — coherente con `allocation_resolution_core` y con
/// `assets_projection_context` — pero sí corre bajo el techo de concurrencia
/// `heavy::run_projection_sim`, porque a diferencia de aquellas dos simula el horizonte COMPLETO.
pub(crate) async fn allocation_goals_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<AllocationGoalsResponse, ApiError> {
    use crate::handlers::projection::{
        build_installation_projection_input, map_engine_err, resolve_projection_context,
    };
    use futurefin_engine::{add_months_signed, project_net_worth_series};

    let ctx = resolve_projection_context(pool, iid, user_id, None).await?;
    let built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        ctx.today,
        ctx.months,
        ctx.inflation_annual_percent,
        Some(&ctx.fire_settings),
        None,
    )
    .await?;

    let income_monthly = built.input.income_regular_monthly;
    let expense_with_debt = built.input.expense_regular_monthly + built.debt_service_monthly;

    let sim_input = built.input.clone();
    let output = crate::heavy::run_projection_sim("allocation goals", move || {
        project_net_worth_series(&sim_input)
    })
    .await?
    .map_err(map_engine_err)?;

    // `priority` / `cap_*` no viven en el engine: se releen de la tabla y se mapean por id.
    let meta = list_allocation_rules_core(pool, iid, user_id, view).await?;

    let mut goals: Vec<AllocationGoal> = Vec::new();
    let mut rules_without_cap: i64 = 0;

    for (rule_index, rule_id) in built.allocation_rule_ids.iter().enumerate() {
        let Some(m) = meta.iter().find(|m| m.id == *rule_id) else {
            continue;
        };
        let (Some(cap_kind), Some(cap_value)) = (m.cap_kind.as_deref(), m.cap_value) else {
            rules_without_cap += 1;
            continue;
        };
        let Some(ceiling) =
            resolve_cap_ceiling_eur(cap_kind, cap_value, income_monthly, expense_with_debt)
        else {
            rules_without_cap += 1;
            continue;
        };
        let target_index = built.input.allocation_rules[rule_index].target_index;
        let Some((target_asset_id, target_asset_name)) =
            built.asset_id_name.get(target_index).cloned()
        else {
            continue;
        };
        let Some(series) = output.per_asset_series.get(target_index) else {
            continue;
        };
        let current_value = series.first().copied().unwrap_or(Decimal::ZERO);

        let progress_pct = (ceiling > Decimal::ZERO)
            .then(|| (current_value / ceiling * Decimal::from(100)).round_dp(1));

        let (eta_month_index, eta_absent_reason) = if ceiling <= Decimal::ZERO {
            (None, Some("zero_ceiling"))
        } else if current_value >= ceiling {
            (None, Some("already_reached"))
        } else {
            match series.iter().position(|v| *v >= ceiling) {
                Some(k) => (Some(k as u32), None),
                None => (None, Some("not_within_horizon")),
            }
        };

        // `add_months_signed` normaliza al día 1 del mes resultante, así que pasarle `today`
        // directamente ya devuelve el primer día del mes del ETA.
        let eta_month_ymd = eta_month_index
            .map(|k| add_months_signed(ctx.today, k as i32).format("%Y-%m-%d").to_string());

        goals.push(AllocationGoal {
            rule_id: *rule_id,
            priority: m.priority,
            target_asset_id,
            target_asset_name,
            cap_kind: cap_kind.to_string(),
            cap_value,
            ceiling: ceiling.round_dp(4),
            ceiling_basis: match cap_kind {
                "amount" => "fixed_amount",
                "income_multiple" => "income_multiple_today",
                _ => "months_expense_today",
            },
            current_value: current_value.round_dp(4),
            progress_pct,
            eta_month_index,
            eta_month_ymd,
            eta_absent_reason,
        });
    }

    goals.sort_by_key(|g| g.priority);

    Ok(AllocationGoalsResponse {
        view: view.as_str(),
        anchor_date_ymd: ctx.today.format("%Y-%m-%d").to_string(),
        horizon_months: ctx.months,
        horizon_basis: ctx.horizon_basis,
        rules_without_cap,
        goals,
    })
}

pub fn allocation_rules_router() -> Router {
    Router::new()
        .route("/", get(list_allocation_rules).post(create_allocation_rule))
        .route("/resolution", get(get_allocation_resolution))
        .route("/goals", get(get_allocation_goals))
        .route("/reorder", post(reorder_allocation_rules))
        .route("/{id}", patch(patch_allocation_rule).delete(delete_allocation_rule))
}

#[cfg(test)]
mod sink_guard_tests {
    /// **El invariante del sumidero se apoya en que este módulo tenga UN solo punto de commit.**
    ///
    /// `commit_with_sink_invariant` verifica I1 sobre el estado ya escrito y solo entonces
    /// confirma; cualquier camino de escritura que abra su propia transacción y la cierre por su
    /// cuenta se saltaría la comprobación. Este test lee el propio fichero y fija que solo hay una
    /// llamada a `tx.commit()` — la de dentro del helper. Si añades un camino nuevo, no cierres su
    /// transacción a mano: pásasela al helper.
    #[test]
    fn el_modulo_tiene_un_unico_punto_de_commit() {
        let src = include_str!("allocation_rules.rs");
        // El patrón se compone en tiempo de ejecución para que ESTA línea no se cuente a sí misma
        // (el test vive en el mismo fichero que audita).
        let needle = format!("tx.{}()", "commit");
        // Se cuentan solo las líneas de CÓDIGO: la prosa de arriba menciona el commit a propósito
        // y no debe contar.
        let commits = src
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("///") && t.contains(&needle)
            })
            .count();
        assert_eq!(
            commits, 1,
            "todo commit de este módulo debe pasar por commit_with_sink_invariant (I1)"
        );
    }
}
