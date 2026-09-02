use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CategoryScope {
    Asset,
    Liability,
    Income,
    Expense,
}

impl CategoryScope {
    fn as_str(self) -> &'static str {
        match self {
            CategoryScope::Asset => "asset",
            CategoryScope::Liability => "liability",
            CategoryScope::Income => "income",
            CategoryScope::Expense => "expense",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s.trim() {
            "asset" => Ok(Self::Asset),
            "liability" => Ok(Self::Liability),
            "income" => Ok(Self::Income),
            "expense" => Ok(Self::Expense),
            _ => Err(ApiError::BadRequest("category_scope_invalid: category scope must be one of asset, liability, income, expense".into())),
        }
    }
}

/// Nombre con el que nace la categoría por defecto de gasto (`seed_default_categories`, y la
/// migración `20260902120000` cuando adopta la sembrada de una instalación anterior). Es solo el
/// nombre de arranque: la designación vive en `categories.is_fallback`, no en el texto, así que
/// renombrarla desde Ajustes no la degrada.
pub(crate) const FALLBACK_EXPENSE_NAME: &str = "Otros gastos";
/// El gemelo de ingresos. Ver [`FALLBACK_EXPENSE_NAME`].
pub(crate) const FALLBACK_INCOME_NAME: &str = "Otros ingresos";

/// `true` si `(scope, name)` es el par con el que nace la categoría por defecto de ese scope.
/// Lo usa el seed; el resto del código pregunta por `is_fallback`, nunca por el nombre.
pub(crate) fn is_seeded_fallback(scope: &str, name: &str) -> bool {
    matches!(
        (scope, name),
        ("expense", FALLBACK_EXPENSE_NAME) | ("income", FALLBACK_INCOME_NAME)
    )
}

/// Id de la categoría POR DEFECTO de `scope` en esta instalación (4.15.0). **Solo lectura**: no
/// crea nada. La existencia la garantizan la migración `20260902120000` (backfill + creación por
/// instalación) y `seed_default_categories`; si aun así falta, es un 400 con nombre propio
/// —`fallback_category_missing`— y no un 500 mudo desde el CHECK de la base.
///
/// Genérica sobre el ejecutor a propósito: la llaman handlers con el pool, el restore de backup
/// con la conexión de SU transacción (la categoría resuelta tiene que ser la que ve esa
/// transacción, no la que vería una conexión distinta) y el import con el pool.
pub(crate) async fn fallback_category_id<'e, E>(
    exec: E,
    installation_id: Uuid,
    scope: &str,
) -> Result<Uuid, ApiError>
where
    E: sqlx::PgExecutor<'e>,
{
    let id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM categories
           WHERE installation_id = $1 AND scope = $2 AND is_fallback"#,
    )
    .bind(installation_id)
    .bind(scope)
    .fetch_optional(exec)
    .await?;
    id.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "fallback_category_missing: this installation has no default category for scope '{scope}'; mark one with PATCH /v1/categories/{{id}} {{\"is_fallback\": true}}"
        ))
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub scope: CategoryScope,
    pub name: String,
    pub sort_index: i32,
    /// `true` ⟺ es la categoría POR DEFECTO de su scope (4.15.0): a ella van los ingresos/gastos
    /// que llegan sin categoría (import sin regla, alta manual, `clear_category`, restore de un
    /// backup antiguo). Hay exactamente una por instalación y scope, y solo en `income`/`expense`
    /// (índice único parcial + CHECK en la base). No se puede borrar (`category_is_fallback`) ni
    /// desmarcar: se cambia designando otra con `PATCH {"is_fallback": true}`.
    pub is_fallback: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCategoryBody {
    pub scope: CategoryScope,
    pub name: String,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchCategoryBody {
    pub name: Option<String>,
    pub sort_index: Option<i32>,
    /// `true` designa esta categoría como destino por defecto de su scope (income/expense): los
    /// movimientos de esa clase sin categoría caen aquí. Desmarca la anterior en la misma
    /// transacción. `false` se rechaza (`fallback_cannot_be_unset`): se cambia designando otra.
    #[serde(default)]
    pub is_fallback: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListCategoriesQuery {
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteCategoryQuery {
    /// Required when the category is still referenced by assets, liabilities, budget or planning flows (same scope as source).
    #[serde(default)]
    pub remap_to: Option<Uuid>,
}

/// Quién apunta a una categoría, **desglosado por tabla**. Es el preview de `delete_category`.
///
/// El total es el que decide si `remap_to` es obligatorio; el desglose es lo que convierte un
/// «hay 214 referencias» en una decisión informada («213 son movimientos y 1 es una partida de
/// presupuesto»). Sin desglose, confirmar un borrado desde el chat es confirmar a ciegas.
#[derive(Debug, serde::Serialize)]
pub(crate) struct CategoryDeleteEffects {
    /// `asset` | `liability` | `income` | `expense`. `remap_to` debe compartirlo.
    pub scope: String,
    pub name: String,
    /// Suma de los seis contadores con FK bloqueante. **`liabilities_expense_attribution` NO entra**
    /// (su FK es `SET NULL`), igual que las `categorization_rules`.
    pub references_total: i64,
    pub assets: i64,
    pub liabilities: i64,
    pub budget_entries: i64,
    pub planning_flows: i64,
    pub transactions: i64,
    pub recurring_rules: i64,
    /// Cuotas de pasivo cuya **atribución de gasto** (`liabilities.expense_category_id`) apunta
    /// aquí. No bloquea el borrado —la FK es `ON DELETE SET NULL`— pero un remap **sí** se la
    /// lleva consigo: si se borra sin remap, la atribución se degrada a `NULL` en silencio.
    pub liabilities_expense_attribution: i64,
    /// Reglas de categorización que asignan esta categoría. Su FK es `ON DELETE SET NULL`, así que
    /// no bloquean ni se remapean: quedan **degradadas** (una regla que ya no asigna nada).
    pub categorization_rules_degraded: i64,
    /// `true` ⟺ `references_total > 0`, es decir: el borrado exige nombrar `remap_to`.
    pub remap_required: bool,
    /// `true` ⟺ la categoría es la POR DEFECTO de su scope (4.15.0) → el borrado es imposible
    /// (`category_is_fallback`), con o sin `remap_to`. Viaja en el preview para que quien lo lee
    /// no proponga un borrado que la confirmación va a rechazar.
    pub target_is_fallback: bool,
}

pub(crate) async fn category_delete_effects(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<CategoryDeleteEffects, ApiError> {
    // Nota: las `categorization_rules` NO cuentan en el total (su `assign_category_id` es ON DELETE
    // SET NULL → una regla degradada nunca bloquea el borrado de una categoría), ni tampoco
    // `liabilities.expense_category_id` (también SET NULL). Las `transactions` y las
    // `recurring_transaction_rules` sí (su `category_id` es ON DELETE RESTRICT → deben remapearse
    // antes de borrar). Los dos contadores no bloqueantes viajan aparte porque el remap SÍ mueve
    // la atribución de las cuotas, y quien confirma un borrado tiene que poder verlo.
    type Counts = (i64, i64, i64, i64, i64, i64, i64, i64);
    let row: Option<(String, String, bool)> = sqlx::query_as(
        r#"SELECT scope, name, is_fallback FROM categories WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    let Some((scope, name, target_is_fallback)) = row else {
        return Err(ApiError::NotFound);
    };

    let c: Counts = sqlx::query_as(
        r#"SELECT
               COALESCE((SELECT COUNT(*)::bigint FROM assets
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM liabilities
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM budget_entries
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM planning_flows
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM transactions
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM recurring_transaction_rules
                         WHERE installation_id = $1 AND category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM liabilities
                         WHERE installation_id = $1 AND expense_category_id = $2), 0),
               COALESCE((SELECT COUNT(*)::bigint FROM categorization_rules
                         WHERE installation_id = $1 AND assign_category_id = $2), 0)"#,
    )
    .bind(installation_id)
    .bind(category_id)
    .fetch_one(pool)
    .await?;

    let references_total = c.0 + c.1 + c.2 + c.3 + c.4 + c.5;
    Ok(CategoryDeleteEffects {
        scope,
        name,
        references_total,
        assets: c.0,
        liabilities: c.1,
        budget_entries: c.2,
        planning_flows: c.3,
        transactions: c.4,
        recurring_rules: c.5,
        liabilities_expense_attribution: c.6,
        categorization_rules_degraded: c.7,
        remap_required: references_total > 0,
        target_is_fallback,
    })
}

/// `(scope, is_fallback)` de una categoría de la instalación, o `None` si no existe.
async fn category_scope_row(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<Option<(String, bool)>, ApiError> {
    let s: Option<(String, bool)> = sqlx::query_as(
        r#"SELECT scope, is_fallback FROM categories WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    Ok(s)
}

#[derive(Debug, FromRow)]
struct CategoryRow {
    id: Uuid,
    scope: String,
    name: String,
    sort_index: i32,
    is_fallback: bool,
}

fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "name_empty: name must not be empty".into(),
        ));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "name_too_long: name must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn row_to_response(r: CategoryRow) -> Result<CategoryResponse, ApiError> {
    Ok(CategoryResponse {
        id: r.id,
        scope: CategoryScope::parse(&r.scope)?,
        name: r.name,
        sort_index: r.sort_index,
        is_fallback: r.is_fallback,
    })
}

#[utoipa::path(
    get,
    path = "/v1/categories",
    tag = "categories",
    params(
        ("scope" = Option<String>, Query, description = "Filter: asset | liability | income | expense"),
    ),
    responses(
        (status = 200, description = "Categories for the installation", body = [CategoryResponse]),
        (status = 400, description = "Invalid scope filter"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation not initialized"),
    )
)]
pub async fn list_categories(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ListCategoriesQuery>,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_categories_core(&state.pool, iid, q.scope.as_deref()).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_categories`. Per-installation
/// (las categorías no tienen owner — no acepta `view`).
pub(crate) async fn list_categories_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    scope: Option<&str>,
) -> Result<Vec<CategoryResponse>, ApiError> {
    let scope_filter: Option<String> = match scope.map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(CategoryScope::parse(s)?.as_str().to_string()),
    };

    let rows: Vec<CategoryRow> = if let Some(ref sc) = scope_filter {
        sqlx::query_as(
            r#"SELECT id, scope, name, sort_index, is_fallback
               FROM categories
               WHERE installation_id = $1 AND scope = $2
               ORDER BY sort_index ASC, name ASC"#,
        )
        .bind(iid)
        .bind(sc)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            r#"SELECT id, scope, name, sort_index, is_fallback
               FROM categories
               WHERE installation_id = $1
               ORDER BY scope ASC, sort_index ASC, name ASC"#,
        )
        .bind(iid)
        .fetch_all(pool)
        .await?
    };

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_response(r)?);
    }
    Ok(out)
}

#[utoipa::path(
    post,
    path = "/v1/categories",
    tag = "categories",
    request_body = CreateCategoryBody,
    responses(
        (status = 201, description = "Created", body = CategoryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation not initialized"),
        (status = 409, description = "Duplicate name in scope"),
    )
)]
pub async fn create_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateCategoryBody>,
) -> Result<(axum::http::StatusCode, Json<CategoryResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_category_core(&state.pool, iid, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_category`. Las categorías
/// no invalidan la cache de proyección (contrato histórico: ningún handler del módulo lo hace).
/// 409 en duplicado (unique instalación+scope+nombre) vía el mapeo global de sqlx.
pub(crate) async fn create_category_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    body: CreateCategoryBody,
) -> Result<CategoryResponse, ApiError> {
    let name = normalize_name(&body.name)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let row: CategoryRow = sqlx::query_as(
        r#"INSERT INTO categories (installation_id, scope, name, sort_index)
           VALUES ($1, $2, $3, $4)
           RETURNING id, scope, name, sort_index, is_fallback"#,
    )
    .bind(iid)
    .bind(body.scope.as_str())
    .bind(&name)
    .bind(sort_index)
    .fetch_one(pool)
    .await?;

    row_to_response(row)
}

#[utoipa::path(
    patch,
    path = "/v1/categories/{id}",
    tag = "categories",
    request_body = PatchCategoryBody,
    params(
        ("id" = Uuid, Path, description = "Category id"),
    ),
    responses(
        (status = 200, description = "Updated", body = CategoryResponse),
        (status = 400, description = "Validation error (`fallback_cannot_be_unset`, `fallback_scope_invalid`)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Category missing"),
        (status = 409, description = "Duplicate name in scope"),
    )
)]
pub async fn patch_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchCategoryBody>,
) -> Result<Json<CategoryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_category_core(&state.pool, iid, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_category`.
///
/// `scope` es **inmutable** — no está en el body y no puede estarlo: mover una categoría de
/// `expense` a `income` dejaría a cada fila que la referencia apuntando a una categoría del scope
/// equivocado, y ninguna FK lo impide (la comprobación de scope vive en los handlers de assets,
/// liabilities, budget y planning, no en la base).
///
/// **Cache NONE** (contrato histórico del módulo: ningún handler de categorías invalida). Renombrar
/// una categoría no mueve ni un número de la proyección; su `category_id` es lo que viaja al engine.
/// 409 en duplicado `(instalación, scope, nombre)` vía el mapeo global de sqlx.
///
/// ## `is_fallback` (4.15.0)
/// - `Some(true)` → **swap atómico**: desmarca la categoría por defecto anterior del MISMO scope y
///   marca ésta, en una sola transacción y en ese orden (índice único parcial).
/// - `Some(false)` → 400 `fallback_cannot_be_unset`. La designación se mueve, no se apaga.
/// - `Some(true)` sobre un scope `asset`/`liability` → 400 `fallback_scope_invalid`.
pub(crate) async fn patch_category_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    id: Uuid,
    body: PatchCategoryBody,
) -> Result<CategoryResponse, ApiError> {
    // Destructuring EXHAUSTIVO y sin `..`: añadir un campo al body deja de compilar hasta que
    // alguien decida si cuenta como «algo que actualizar» (mismo criterio que
    // `patch_allocation_rule_core`).
    {
        let PatchCategoryBody { name, sort_index, is_fallback } = &body;
        if name.is_none() && sort_index.is_none() && is_fallback.is_none() {
            return Err(ApiError::BadRequest(
                "patch_empty: provide name, sort_index and/or is_fallback".into(),
            ));
        }
    }

    // `is_fallback: false` NO existe como operación. Desmarcar dejaría a la instalación sin
    // destino para los ingresos/gastos sin categoría —el estado que 4.15.0 vino a eliminar— y el
    // siguiente import fallaría con `fallback_category_missing` sin que nadie relacionara las dos
    // cosas. La designación se MUEVE marcando otra, nunca se apaga.
    if body.is_fallback == Some(false) {
        return Err(ApiError::BadRequest(
            "fallback_cannot_be_unset: the default category is moved by designating another one with is_fallback true, never by unsetting this one".into(),
        ));
    }
    let designate_fallback = body.is_fallback == Some(true);

    let row: Option<CategoryRow> = sqlx::query_as(
        r#"SELECT id, scope, name, sort_index, is_fallback
           FROM categories
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };

    if designate_fallback && !matches!(current.scope.as_str(), "income" | "expense") {
        return Err(ApiError::BadRequest(
            "fallback_scope_invalid: only income and expense categories can be the default one; assets and liabilities always carry an explicit category".into(),
        ));
    }

    let new_name = match &body.name {
        Some(s) => normalize_name(s)?,
        None => current.name.clone(),
    };
    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    // El swap va en UNA transacción y en ESTE orden —desmarcar la anterior, marcar la nueva—
    // porque el índice único es PARCIAL sobre `(installation_id, scope) WHERE is_fallback`: dos
    // marcadas a la vez violan la unicidad aunque sea por un instante dentro de la transacción.
    // El orden inverso da un 23505 con cara de conflicto de nombre.
    let mut tx = pool.begin().await?;
    if designate_fallback {
        sqlx::query(
            r#"UPDATE categories SET is_fallback = false
               WHERE installation_id = $1 AND scope = $2 AND is_fallback AND id <> $3"#,
        )
        .bind(iid)
        .bind(&current.scope)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    // `is_fallback OR $3`: este UPDATE nunca desmarca (el `false` ya se rechazó arriba), así que
    // un PATCH de nombre no puede degradar la categoría por defecto por omisión.
    let updated: CategoryRow = sqlx::query_as(
        r#"UPDATE categories
           SET name = $1, sort_index = $2, is_fallback = is_fallback OR $3
           WHERE id = $4 AND installation_id = $5
           RETURNING id, scope, name, sort_index, is_fallback"#,
    )
    .bind(&new_name)
    .bind(new_sort)
    .bind(designate_fallback)
    .bind(id)
    .bind(iid)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    row_to_response(updated)
}

#[utoipa::path(
    delete,
    path = "/v1/categories/{id}",
    tag = "categories",
    params(
        ("id" = Uuid, Path, description = "Category id"),
        ("remap_to" = Option<Uuid>, Query, description = "Target category id (same scope) when rows still reference the deleted category"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Invalid remap, category still in use without remap_to, or the default category of its scope (`category_is_fallback`)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Category missing"),
    )
)]
pub async fn delete_category(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Query(q): Query<DeleteCategoryQuery>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    delete_category_core(&state.pool, iid, id, q.remap_to).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_category`.
///
/// Un `create_category` sin contraparte es un pozo sin fondo: el catálogo es **compartido por toda
/// la instalación**, así que cada categoría que un agente cree por error se queda ahí para siempre.
/// Esta core es la contraparte, y su preview natural es [`category_delete_effects`]: enseña quién
/// apunta a la categoría y obliga a **nombrar el destino** del remap antes de confirmar.
///
/// Reglas del remap, todas comprobadas antes de tocar nada:
/// - la categoría POR DEFECTO de su scope (4.15.0) no se borra nunca → 400 `category_is_fallback`,
///   comprobado ANTES de contar referencias (una fallback vacía tampoco se puede borrar);
/// - con referencias bloqueantes y sin `remap_to` → 400 `category_in_use`;
/// - `remap_to` == la propia categoría → 400 `remap_to_same_category`;
/// - `remap_to` inexistente en la instalación → 400 `remap_to_not_found`;
/// - `remap_to` de otro scope → 400 `remap_to_scope_mismatch`.
///
/// **Cache NONE** (contrato histórico del módulo). Ojo: el remap TOCA `assets`, `liabilities`,
/// `budget_entries` y `planning_flows`, que sí son inputs del engine — pero solo cambia su
/// `category_id`, que el engine no lee: agrega por importe, no por categoría.
pub(crate) async fn delete_category_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    id: Uuid,
    remap_to: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some((scope_src, src_is_fallback)) = category_scope_row(pool, iid, id).await? else {
        return Err(ApiError::NotFound);
    };

    // ANTES de contar referencias: da igual que esté vacía. Sin categoría por defecto, el primer
    // ingreso o gasto sin categoría —un import sin regla, un `clear_category`— se queda sin
    // destino y revienta contra el CHECK de la base. Para cambiarla se designa otra
    // (`PATCH {"is_fallback": true}`), y entonces ésta ya se puede borrar.
    if src_is_fallback {
        return Err(ApiError::BadRequest(
            "category_is_fallback: this is the default category of its scope and cannot be deleted; designate another one first with is_fallback true".into(),
        ));
    }

    let refs = category_delete_effects(pool, iid, id).await?.references_total;

    // El remap corre **siempre que se pida**, no solo cuando hay referencias bloqueantes.
    //
    // Hasta 4.4.0 la condición era `refs > 0`, y con `refs == 0` el `remap_to` se ignoraba en
    // silencio: la llamada devolvía 204 y quien la hizo no tenía forma de saber que su destino no
    // se había usado. El caso concreto que lo destapa es `liabilities.expense_category_id`, cuya
    // FK es `ON DELETE SET NULL` y por eso NO cuenta en `references_total`: borrar con `remap_to`
    // una categoría de gasto usada solo como atribución de cuotas degradaba esa atribución a
    // `NULL` — justo lo que el `remap_to` pedía evitar.
    if refs > 0 && remap_to.is_none() {
        return Err(ApiError::BadRequest(
            "category_in_use: category is in use; pass remap_to query parameter with another category id of the same scope"
                .into(),
        ));
    }

    if let Some(target) = remap_to {
        if target == id {
            return Err(ApiError::BadRequest(
                "remap_to_same_category: remap_to must differ from the category being deleted".into(),
            ));
        }
        let Some((scope_tgt, _)) = category_scope_row(pool, iid, target).await? else {
            return Err(ApiError::BadRequest(
                "remap_to_not_found: remap_to category was not found in this installation".into(),
            ));
        };
        if scope_src != scope_tgt {
            return Err(ApiError::BadRequest(
                "remap_to_scope_mismatch: remap_to category must have the same scope as the deleted category".into(),
            ));
        }

        let mut tx = pool.begin().await?;

        sqlx::query(
            r#"UPDATE assets SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE liabilities SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE budget_entries SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE planning_flows SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // Las transacciones y las reglas recurrentes referencian la categoría con ON DELETE
        // RESTRICT → hay que remaparlas (las `categorization_rules` no: su FK es SET NULL y se
        // degradan solas).
        sqlx::query(
            r#"UPDATE transactions SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"UPDATE recurring_transaction_rules SET category_id = $1, updated_at = now()
               WHERE installation_id = $2 AND category_id = $3"#,
        )
        .bind(target)
        .bind(iid)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // `liabilities.expense_category_id` (3.4.0) es SET NULL — no cuenta en
        // `references_total` ni bloquea el borrado — pero cuando el usuario remapea una
        // categoría de gasto, la atribución de las cuotas debe seguirla en vez de degradarse a
        // NULL por el FK al borrar.
        if scope_src == "expense" {
            sqlx::query(
                r#"UPDATE liabilities SET expense_category_id = $1, updated_at = now()
                   WHERE installation_id = $2 AND expense_category_id = $3"#,
            )
            .bind(target)
            .bind(iid)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }

        let del = sqlx::query(r#"DELETE FROM categories WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        if del.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
        return Ok(());
    }

    let res = sqlx::query(r#"DELETE FROM categories WHERE id = $1 AND installation_id = $2"#)
        .bind(id)
        .bind(iid)
        .execute(pool)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(())
}

pub fn categories_router() -> Router {
    Router::new()
        .route("/", get(list_categories).post(create_category))
        .route("/{id}", patch(patch_category).delete(delete_category))
}
