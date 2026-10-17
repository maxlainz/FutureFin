//! Planes y bloques: lectura (con bloques y cambios anidados), alta, edición, borrado y
//! activación. Los cambios viven en [`super::changes`].
//!
//! Contrato de escritura, idéntico en todas las cores de este módulo y de `changes.rs`:
//!
//! 1. el handler comprueba sesión, pertenencia y `role_can_write` (un viewer → 403);
//! 2. la core abre una transacción y pasa la puerta [`lock_plan_for_write`]: el plan existe en la
//!    instalación (404 `plan_not_found`), no es de hogar (400 `household_plans_not_available`,
//!    hasta F3) y es del usuario de la sesión (403 `not_row_owner`, D21 — el rol `owner` tampoco
//!    la salta). La fila queda bloqueada, así que las subidas de `revision` se serializan;
//! 3. escribe, sube `plans.revision` y `plans.updated_at` ([`bump_revision`]) y confirma;
//! 4. tras el commit, `refresh_projection_after_mutation` (invalidación completa por instalación,
//!    sin warm-up). En F2-1 los planes todavía no alimentan la proyección —eso es F2-4—, pero
//!    invalidar ya es barato y deja la regla escrita antes de que haga falta.

use super::changes::{change_from_row, ChangeRow, PlanChangeResponse, CHANGE_COLS};
use super::validate::{
    assumptions_from_stored, assumptions_to_stored, parse_assumptions_write, PlanAssumptions,
};
use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{require_row_owner, LedgerView, LedgerViewQuery};
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgConnection, PgPool};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Nombre del bloque que nace con cada plan.
pub const DEFAULT_BLOCK_NAME: &str = "General";

// ─── Errores ────────────────────────────────────────────────────────────────────────────────

pub(crate) fn plan_not_found() -> ApiError {
    ApiError::NotFoundWith("plan_not_found: no plan with that id in this installation".into())
}

pub(crate) fn plan_block_not_found() -> ApiError {
    ApiError::NotFoundWith("plan_block_not_found: no block with that id in this plan".into())
}

pub(crate) fn household_plans_not_available() -> ApiError {
    ApiError::BadRequest(
        "household_plans_not_available: household plans arrive with shared finances (6.0 F3); until then every plan belongs to one person and is written by that person only".into(),
    )
}

fn plan_not_in_scope() -> ApiError {
    ApiError::NotFoundWith(
        "plan_not_in_scope: that plan belongs to another household member; read it with view=household".into(),
    )
}

/// 23505 sobre `plans_one_active_*`: dos activaciones simultáneas de la misma persona. Con el
/// bloqueo de [`activate_plan_core`] no debería alcanzarse; es la red por si algo escribe
/// `is_active` por otro camino.
fn activation_error(e: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("23505") {
            return ApiError::ConflictWith(
                "plan_activation_conflict: another activation of this person's plans won the race; read the plans again and retry".into(),
            );
        }
    }
    e.into()
}

// ─── Tipos ──────────────────────────────────────────────────────────────────────────────────

/// Un plan con sus bloques y, dentro de cada bloque, sus cambios en orden cronológico.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// Dueño del plan. `null` = plan de hogar (solo esquema hasta F3: la API no los crea).
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
    pub name: String,
    /// Como mucho un plan activo por persona. El activo es el que alimentará la proyección
    /// (F2-4); en F2-1 todavía no la mueve.
    pub is_active: bool,
    pub assumptions: PlanAssumptions,
    /// Sube en toda escritura del plan, de sus bloques o de sus cambios, y al (des)activarlo.
    pub revision: i64,
    pub sort_index: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub blocks: Vec<PlanBlockResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanBlockResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub plan_id: Uuid,
    pub name: String,
    /// `false` = sus cambios siguen guardados pero el plan los ignora.
    pub enabled: bool,
    pub sort_index: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Cronológico: `start_date`, luego alta.
    pub changes: Vec<PlanChangeResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlansListResponse {
    /// Vista aplicada: `mine` | `household`.
    pub view: &'static str,
    /// El activo primero; después `sort_index` y alta.
    pub plans: Vec<PlanResponse>,
}

/// Resultado de activar o desactivar: qué plan queda activo y cuáles dejaron de estarlo.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanActivationResponse {
    /// Plan activo de la persona tras la operación; `null` = solo la realidad.
    #[schema(value_type = Option<String>, format = "uuid")]
    pub active_plan_id: Option<Uuid>,
    #[schema(value_type = Vec<String>)]
    pub deactivated_plan_ids: Vec<Uuid>,
    /// `false` si no había nada que cambiar (activar el que ya estaba activo, o desactivar sin
    /// ninguno activo). Entonces no se sube `revision` ni se invalida nada.
    pub changed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlanBody {
    pub name: String,
    /// Omitido o `null` = todos los supuestos por defecto. Claves desconocidas → 400
    /// `plan_assumptions_invalid`.
    #[serde(default)]
    #[schema(value_type = Option<PlanAssumptions>)]
    pub assumptions: Option<Value>,
    #[serde(default)]
    pub sort_index: Option<i32>,
    /// `true` pediría un plan de hogar: 400 `household_plans_not_available` hasta F3.
    #[serde(default)]
    pub household: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPlanBody {
    #[serde(default)]
    pub name: Option<String>,
    /// REEMPLAZA los supuestos enteros: lo que no se mande vuelve a su default.
    #[serde(default)]
    #[schema(value_type = Option<PlanAssumptions>)]
    pub assumptions: Option<Value>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlanBlockBody {
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPlanBlockBody {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PlanRow {
    id: Uuid,
    owner_user_id: Option<Uuid>,
    name: String,
    is_active: bool,
    assumptions: Value,
    revision: i64,
    sort_index: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const PLAN_COLS: &str = "p.id, p.owner_user_id, p.name, p.is_active, p.assumptions, p.revision, \
                         p.sort_index, p.created_at, p.updated_at";

#[derive(Debug, FromRow)]
struct BlockRow {
    id: Uuid,
    plan_id: Uuid,
    name: String,
    enabled: bool,
    sort_index: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const BLOCK_COLS: &str = "id, plan_id, name, enabled, sort_index, created_at, updated_at";

// ─── Helpers compartidos ────────────────────────────────────────────────────────────────────

fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "name_empty: name must not be empty".into(),
        ));
    }
    if t.chars().count() > 200 {
        return Err(ApiError::BadRequest(
            "name_too_long: name must be at most 200 characters".into(),
        ));
    }
    Ok(t.to_string())
}

/// Puerta de escritura de un plan (ver la doc del módulo). Bloquea la fila hasta el commit.
pub(crate) async fn lock_plan_for_write(
    conn: &mut PgConnection,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
) -> Result<(), ApiError> {
    let owner: Option<Option<Uuid>> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM plans WHERE id = $1 AND installation_id = $2 FOR UPDATE"#,
    )
    .bind(plan_id)
    .bind(iid)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(owner) = owner else {
        return Err(plan_not_found());
    };
    let Some(owner) = owner else {
        return Err(household_plans_not_available());
    };
    require_row_owner(owner, user_id)
}

/// Toda escritura del plan o de algo suyo sube su `revision` (sello de cache de F2-4) y su
/// `updated_at` (lo que ve `GET /v1/changes`).
pub(crate) async fn bump_revision(conn: &mut PgConnection, plan_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(r#"UPDATE plans SET revision = revision + 1, updated_at = now() WHERE id = $1"#)
        .bind(plan_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn block_response(r: BlockRow, changes: Vec<PlanChangeResponse>) -> PlanBlockResponse {
    PlanBlockResponse {
        id: r.id,
        plan_id: r.plan_id,
        name: r.name,
        enabled: r.enabled,
        sort_index: r.sort_index,
        created_at: r.created_at,
        updated_at: r.updated_at,
        changes,
    }
}

/// Anida bloques y cambios en los planes dados, con dos consultas en total.
async fn assemble_plans(pool: &PgPool, rows: Vec<PlanRow>) -> Result<Vec<PlanResponse>, ApiError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let blocks: Vec<BlockRow> = sqlx::query_as(&format!(
        "SELECT {BLOCK_COLS} FROM plan_blocks WHERE plan_id = ANY($1) \
         ORDER BY sort_index ASC, created_at ASC, id ASC"
    ))
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let changes: Vec<ChangeRow> = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLS} FROM plan_changes WHERE plan_id = ANY($1) \
         ORDER BY start_date ASC, created_at ASC, id ASC"
    ))
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let mut changes_by_block: HashMap<Uuid, Vec<PlanChangeResponse>> = HashMap::new();
    for c in changes {
        let block_id = c.block_id;
        changes_by_block
            .entry(block_id)
            .or_default()
            .push(change_from_row(c)?);
    }
    let mut blocks_by_plan: HashMap<Uuid, Vec<PlanBlockResponse>> = HashMap::new();
    for b in blocks {
        let changes = changes_by_block.remove(&b.id).unwrap_or_default();
        blocks_by_plan
            .entry(b.plan_id)
            .or_default()
            .push(block_response(b, changes));
    }

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(PlanResponse {
            id: r.id,
            owner_user_id: r.owner_user_id,
            name: r.name,
            is_active: r.is_active,
            assumptions: assumptions_from_stored(r.assumptions)?,
            revision: r.revision,
            sort_index: r.sort_index,
            created_at: r.created_at,
            updated_at: r.updated_at,
            blocks: blocks_by_plan.remove(&r.id).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Un plan por id, sin filtro de vista (las cores de escritura ya han pasado la puerta).
pub(crate) async fn fetch_plan(
    pool: &PgPool,
    iid: Uuid,
    plan_id: Uuid,
) -> Result<PlanResponse, ApiError> {
    let row: Option<PlanRow> = sqlx::query_as(&format!(
        "SELECT {PLAN_COLS} FROM plans p WHERE p.id = $1 AND p.installation_id = $2"
    ))
    .bind(plan_id)
    .bind(iid)
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(plan_not_found)?;
    Ok(assemble_plans(pool, vec![row]).await?.remove(0))
}

async fn fetch_block(
    pool: &PgPool,
    plan_id: Uuid,
    block_id: Uuid,
) -> Result<PlanBlockResponse, ApiError> {
    let row: Option<BlockRow> = sqlx::query_as(&format!(
        "SELECT {BLOCK_COLS} FROM plan_blocks WHERE id = $1 AND plan_id = $2"
    ))
    .bind(block_id)
    .bind(plan_id)
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(plan_block_not_found)?;
    let changes: Vec<ChangeRow> = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLS} FROM plan_changes WHERE block_id = $1 \
         ORDER BY start_date ASC, created_at ASC, id ASC"
    ))
    .bind(block_id)
    .fetch_all(pool)
    .await?;
    let changes = changes
        .into_iter()
        .map(change_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(block_response(row, changes))
}

// ─── Lectura ────────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/plans",
    tag = "plans",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default) = tus planes; `household` = los de todo el hogar, de solo lectura. Cualquier otro valor → 400 `invalid_view`."),
    ),
    responses(
        (status = 200, description = "Planes con bloques y cambios anidados", body = PlansListResponse),
        (status = 400, description = "invalid_view"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn list_plans(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<PlansListResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_plans_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP (la reutilizará la tool MCP `list_plans`, F2-6).
pub(crate) async fn list_plans_core(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<PlansListResponse, ApiError> {
    let scope = view.scope_where("p");
    let sql = format!(
        "SELECT {PLAN_COLS} FROM plans p WHERE {scope} \
         ORDER BY p.is_active DESC, p.sort_index ASC, p.created_at ASC, p.id ASC"
    );
    let rows: Vec<PlanRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;
    Ok(PlansListResponse {
        view: view.as_str(),
        plans: assemble_plans(pool, rows).await?,
    })
}

#[utoipa::path(
    get,
    path = "/v1/plans/{id}",
    tag = "plans",
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("view" = Option<String>, Query, description = "`mine` (default): el plan de otro miembro → 404 `plan_not_in_scope`; `household`: cualquiera del hogar."),
    ),
    responses(
        (status = 200, description = "Plan con bloques y cambios", body = PlanResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "plan_not_found | plan_not_in_scope"),
    )
)]
pub async fn get_plan(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<PlanResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = get_plan_core(&state.pool, iid, user.id.0, q.resolve()?, id).await?;
    Ok(Json(out))
}

pub(crate) async fn get_plan_core(
    pool: &PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    plan_id: Uuid,
) -> Result<PlanResponse, ApiError> {
    let plan = fetch_plan(pool, iid, plan_id).await?;
    if view == LedgerView::Mine && plan.owner_user_id != Some(user_id) {
        return Err(plan_not_in_scope());
    }
    Ok(plan)
}

// ─── Planes: escritura ──────────────────────────────────────────────────────────────────────

/// Resuelve sesión + pertenencia + rol de escritura. Un viewer → 403 pelado (motivo: el ROL).
pub(crate) async fn require_writer(
    state: &AppState,
    jar: &CookieJar,
) -> Result<(Uuid, Uuid), ApiError> {
    let user = require_session_user(jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    Ok((iid, user.id.0))
}

#[utoipa::path(
    post,
    path = "/v1/plans",
    tag = "plans",
    request_body = CreatePlanBody,
    responses(
        (status = 201, description = "Plan creado (inactivo) con su bloque «General»", body = PlanResponse),
        (status = 400, description = "name_empty | name_too_long | plan_assumptions_invalid | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn create_plan(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreatePlanBody>,
) -> Result<(StatusCode, Json<PlanResponse>), ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    let out = create_plan_core(&state, iid, user_id, body).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Core sin HTTP. El plan nace INACTIVO —activarlo es un acto aparte— y con un bloque «General».
pub(crate) async fn create_plan_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    body: CreatePlanBody,
) -> Result<PlanResponse, ApiError> {
    if body.household == Some(true) {
        return Err(household_plans_not_available());
    }
    let name = normalize_name(&body.name)?;
    let assumptions = match &body.assumptions {
        Some(v) => parse_assumptions_write(v)?,
        None => PlanAssumptions::default(),
    };

    let mut tx = state.pool.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO plans (installation_id, owner_user_id, name, assumptions, sort_index)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(&name)
    .bind(assumptions_to_stored(&assumptions))
    .bind(body.sort_index.unwrap_or(0))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(r#"INSERT INTO plan_blocks (plan_id, name) VALUES ($1, $2)"#)
        .bind(id)
        .bind(DEFAULT_BLOCK_NAME)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_plan(&state.pool, iid, id).await
}

#[utoipa::path(
    patch,
    path = "/v1/plans/{id}",
    tag = "plans",
    request_body = PatchPlanBody,
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 200, description = "Plan actualizado", body = PlanResponse),
        (status = 400, description = "patch_empty | name_empty | name_too_long | plan_assumptions_invalid | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found"),
    )
)]
pub async fn patch_plan(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchPlanBody>,
) -> Result<Json<PlanResponse>, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    Ok(Json(patch_plan_core(&state, iid, user_id, id, body).await?))
}

pub(crate) async fn patch_plan_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    body: PatchPlanBody,
) -> Result<PlanResponse, ApiError> {
    if body.name.is_none() && body.assumptions.is_none() && body.sort_index.is_none() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one of name, assumptions, sort_index".into(),
        ));
    }
    let name = body.name.as_deref().map(normalize_name).transpose()?;
    let assumptions = body
        .assumptions
        .as_ref()
        .map(parse_assumptions_write)
        .transpose()?
        .map(|a| assumptions_to_stored(&a));

    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    sqlx::query(
        r#"UPDATE plans
           SET name = COALESCE($2, name),
               assumptions = COALESCE($3, assumptions),
               sort_index = COALESCE($4, sort_index),
               revision = revision + 1,
               updated_at = now()
           WHERE id = $1"#,
    )
    .bind(plan_id)
    .bind(name)
    .bind(assumptions)
    .bind(body.sort_index)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_plan(&state.pool, iid, plan_id).await
}

#[utoipa::path(
    delete,
    path = "/v1/plans/{id}",
    tag = "plans",
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 204, description = "Borrado, con sus bloques y cambios. Si era el activo, la persona vuelve a «solo la realidad»."),
        (status = 400, description = "household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found"),
    )
)]
pub async fn delete_plan(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    delete_plan_core(&state, iid, user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_plan_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    sqlx::query(r#"DELETE FROM plans WHERE id = $1 AND installation_id = $2"#)
        .bind(plan_id)
        .bind(iid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(())
}

// ─── Activación ─────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/plans/{id}/activate",
    tag = "plans",
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 200, description = "El plan queda activo y cualquier otro de la persona deja de estarlo", body = PlanActivationResponse),
        (status = 400, description = "household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found"),
        (status = 409, description = "plan_activation_conflict"),
    )
)]
pub async fn activate_plan(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<Json<PlanActivationResponse>, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    Ok(Json(
        activate_plan_core(&state, iid, user_id, Some(id)).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/v1/plans/deactivate",
    tag = "plans",
    responses(
        (status = 200, description = "Ningún plan de la persona queda activo: la proyección vuelve a ser solo la realidad", body = PlanActivationResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
    )
)]
pub async fn deactivate_plans(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<PlanActivationResponse>, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    Ok(Json(activate_plan_core(&state, iid, user_id, None).await?))
}

/// Core sin HTTP de activar (`Some(plan)`) y desactivar (`None` = desactivar todos los de la
/// persona: solo la realidad).
///
/// **Exactamente uno activo, también con carreras.** La transacción bloquea TODAS las filas de
/// planes de la persona (`FOR UPDATE`, en orden de id y ANTES de pasar la puerta del plan pedido,
/// para que dos activaciones no se crucen en un interbloqueo), desactiva y después activa. Dos activaciones simultáneas se serializan: la
/// segunda espera, relee y desactiva lo que la primera activó. El 409
/// `plan_activation_conflict` (23505 sobre `plans_one_active_per_owner`) queda como red.
pub(crate) async fn activate_plan_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Option<Uuid>,
) -> Result<PlanActivationResponse, ApiError> {
    let mut tx = state.pool.begin().await?;
    // PRIMERO todas las filas de la persona, en orden de id; DESPUÉS la puerta del plan pedido.
    // Al revés (el pedido primero) dos activaciones de A y de B se cruzan: cada una tiene su
    // plan y espera el de la otra → interbloqueo → 500.
    sqlx::query(
        r#"SELECT id FROM plans
           WHERE installation_id = $1 AND owner_user_id = $2
           ORDER BY id
           FOR UPDATE"#,
    )
    .bind(iid)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if let Some(id) = plan_id {
        lock_plan_for_write(&mut tx, iid, user_id, id).await?;
    }

    let deactivated: Vec<Uuid> = sqlx::query_scalar(
        r#"UPDATE plans
           SET is_active = false, revision = revision + 1, updated_at = now()
           WHERE installation_id = $1 AND owner_user_id = $2 AND is_active
             AND id IS DISTINCT FROM $3
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(plan_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut activated = false;
    if let Some(id) = plan_id {
        let res = sqlx::query(
            r#"UPDATE plans
               SET is_active = true, revision = revision + 1, updated_at = now()
               WHERE id = $1 AND NOT is_active"#,
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(activation_error)?;
        activated = res.rows_affected() == 1;
    }
    tx.commit().await.map_err(activation_error)?;

    let changed = activated || !deactivated.is_empty();
    if changed {
        refresh_projection_after_mutation(state, iid, user_id).await;
    }
    Ok(PlanActivationResponse {
        active_plan_id: plan_id,
        deactivated_plan_ids: deactivated,
        changed,
    })
}

// ─── Bloques ────────────────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/v1/plans/{id}/blocks",
    tag = "plans",
    request_body = CreatePlanBlockBody,
    params(("id" = Uuid, Path, description = "Plan id")),
    responses(
        (status = 201, description = "Bloque creado", body = PlanBlockResponse),
        (status = 400, description = "name_empty | name_too_long | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found"),
    )
)]
pub async fn create_plan_block(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePlanBlockBody>,
) -> Result<(StatusCode, Json<PlanBlockResponse>), ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    let out = create_plan_block_core(&state, iid, user_id, id, body).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

pub(crate) async fn create_plan_block_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    body: CreatePlanBlockBody,
) -> Result<PlanBlockResponse, ApiError> {
    let name = normalize_name(&body.name)?;
    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let block_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO plan_blocks (plan_id, name, enabled, sort_index)
           VALUES ($1, $2, $3, $4)
           RETURNING id"#,
    )
    .bind(plan_id)
    .bind(&name)
    .bind(body.enabled.unwrap_or(true))
    .bind(body.sort_index.unwrap_or(0))
    .fetch_one(&mut *tx)
    .await?;
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;
    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_block(&state.pool, plan_id, block_id).await
}

#[utoipa::path(
    patch,
    path = "/v1/plans/{id}/blocks/{block_id}",
    tag = "plans",
    request_body = PatchPlanBlockBody,
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("block_id" = Uuid, Path, description = "Block id"),
    ),
    responses(
        (status = 200, description = "Bloque actualizado", body = PlanBlockResponse),
        (status = 400, description = "patch_empty | name_empty | name_too_long | household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found | plan_block_not_found"),
    )
)]
pub async fn patch_plan_block(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path((id, block_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PatchPlanBlockBody>,
) -> Result<Json<PlanBlockResponse>, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    Ok(Json(
        patch_plan_block_core(&state, iid, user_id, id, block_id, body).await?,
    ))
}

pub(crate) async fn patch_plan_block_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    block_id: Uuid,
    body: PatchPlanBlockBody,
) -> Result<PlanBlockResponse, ApiError> {
    if body.name.is_none() && body.enabled.is_none() && body.sort_index.is_none() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one of name, enabled, sort_index".into(),
        ));
    }
    let name = body.name.as_deref().map(normalize_name).transpose()?;
    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let res = sqlx::query(
        r#"UPDATE plan_blocks
           SET name = COALESCE($3, name),
               enabled = COALESCE($4, enabled),
               sort_index = COALESCE($5, sort_index),
               updated_at = now()
           WHERE id = $1 AND plan_id = $2"#,
    )
    .bind(block_id)
    .bind(plan_id)
    .bind(name)
    .bind(body.enabled)
    .bind(body.sort_index)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Err(plan_block_not_found());
    }
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;
    refresh_projection_after_mutation(state, iid, user_id).await;
    fetch_block(&state.pool, plan_id, block_id).await
}

#[utoipa::path(
    delete,
    path = "/v1/plans/{id}/blocks/{block_id}",
    tag = "plans",
    params(
        ("id" = Uuid, Path, description = "Plan id"),
        ("block_id" = Uuid, Path, description = "Block id"),
    ),
    responses(
        (status = 204, description = "Borrado con sus cambios. Las ventas de otros bloques que vendían un activo que entraba en este quedan sin sujeto (`target_change_id` null)."),
        (status = 400, description = "household_plans_not_available"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer, not a member, or not_row_owner"),
        (status = 404, description = "plan_not_found | plan_block_not_found"),
    )
)]
pub async fn delete_plan_block(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path((id, block_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let (iid, user_id) = require_writer(&state, &jar).await?;
    delete_plan_block_core(&state, iid, user_id, id, block_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_plan_block_core(
    state: &AppState,
    iid: Uuid,
    user_id: Uuid,
    plan_id: Uuid,
    block_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    lock_plan_for_write(&mut tx, iid, user_id, plan_id).await?;
    let res = sqlx::query(r#"DELETE FROM plan_blocks WHERE id = $1 AND plan_id = $2"#)
        .bind(block_id)
        .bind(plan_id)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(plan_block_not_found());
    }
    bump_revision(&mut tx, plan_id).await?;
    tx.commit().await?;
    refresh_projection_after_mutation(state, iid, user_id).await;
    Ok(())
}
