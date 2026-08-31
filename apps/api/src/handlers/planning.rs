use crate::error::ApiError;
use crate::handlers::budget::assert_budget_category;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::LedgerViewQuery;
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlanningFlowDirection {
    Inflow,
    Outflow,
}

impl PlanningFlowDirection {
    /// La forma que viaja por el wire, la misma que produce `#[serde(rename_all = "lowercase")]`.
    ///
    /// Existe porque el enum solo tenía `Debug`, así que un `{:?}` en un `format!` publicaba el
    /// identificador de Rust: las tools MCP de escritura devolvían `"… (Outflow)"` —inglés y
    /// capitalizado— mientras las de lectura devolvían `"direction":"outflow"` (auditoría MCP §11b).
    /// Con esto, el único camino de un enum al wire vuelve a ser uno.
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanningFlowDirection::Inflow => "inflow",
            PlanningFlowDirection::Outflow => "outflow",
        }
    }
}

impl std::fmt::Display for PlanningFlowDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Base del importe de un Próximo (#148). `one_off` = `expected_amount` es un TOTAL en euros que
/// impacta una vez (en `due_date`, o repartido en 90 días si no la hay); `per_month` = son
/// **€/MES** durante la ventana `[window_start_date, window_end_date]`. Precedente del patrón
/// (declarar la base en vez de duplicar el campo): `BudgetTotalsResponse::basis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAmountBasis {
    OneOff,
    PerMonth,
}

impl PlanningAmountBasis {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanningAmountBasis::OneOff => "one_off",
            PlanningAmountBasis::PerMonth => "per_month",
        }
    }

    /// Parseo estricto (norma §2.6: rechazar, no defaultear), con el MISMO código de wire por
    /// HTTP y por MCP — la validación vive en la core compartida.
    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw {
            "one_off" => Ok(PlanningAmountBasis::OneOff),
            "per_month" => Ok(PlanningAmountBasis::PerMonth),
            _ => Err(ApiError::BadRequest(
                "amount_basis_invalid: amount_basis must be one_off or per_month".into(),
            )),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanningFlowResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub direction: PlanningFlowDirection,
    pub title: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expected_amount: Decimal,
    /// SIEMPRE presente, también en los puntuales: ningún consumidor debe inferir la unidad del
    /// importe de la presencia de otro campo.
    pub amount_basis: PlanningAmountBasis,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub due_date: Option<NaiveDate>,
    /// Inicio de la ventana recurrente (solo `per_month`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub window_start_date: Option<NaiveDate>,
    /// Fin inclusive de la ventana. Ausente con `per_month` = **abierta, sin fin** (misma
    /// convención declarada que `liabilities.payment_end_date` NULL).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub window_end_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
    pub show_in_chart: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlanningFlowBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub title: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expected_amount: Decimal,
    /// `one_off` (default) | `per_month`. String validada en la core (`amount_basis_invalid`)
    /// para que HTTP y MCP compartan el código de wire.
    #[serde(default)]
    pub amount_basis: Option<String>,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    /// Requerida con `per_month`; prohibida con `one_off`.
    #[serde(default)]
    pub window_start_date: Option<NaiveDate>,
    /// Opcional con `per_month` (ausente = sin fin); prohibida con `one_off`.
    #[serde(default)]
    pub window_end_date: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
    #[serde(default)]
    pub show_in_chart: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchPlanningFlowBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub title: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_amount: Option<Decimal>,
    /// Omit = leave unchanged; `null` = clear; `"YYYY-MM-DD"` = set. (La cadena vacía `""`
    /// también borra — escape hatch heredado, conservado y ahora documentado.)
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<String>, format = "date")]
    pub due_date: Option<Value>,
    /// `one_off` | `per_month`. El PATCH valida el estado RESULTANTE completo: para cambiar de
    /// base hay que dejar coherentes fecha y ventana en el mismo request (p. ej. a `per_month`:
    /// `{"amount_basis":"per_month","due_date":null,"window_start_date":"…"}`). Nada se
    /// auto-borra en silencio.
    #[serde(default)]
    pub amount_basis: Option<String>,
    /// Tri-estado como `due_date`: omit = igual; `null`/`""` = borrar; fecha = fijar.
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<String>, format = "date")]
    pub window_start_date: Option<Value>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<String>, format = "date")]
    pub window_end_date: Option<Value>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
    #[serde(default)]
    pub show_in_chart: Option<bool>,
}

#[derive(Debug, FromRow)]
struct PlanningFlowJoinRow {
    id: Uuid,
    category_id: Uuid,
    scope: String,
    title: String,
    expected_amount: Decimal,
    amount_basis: String,
    due_date: Option<NaiveDate>,
    window_start_date: Option<NaiveDate>,
    window_end_date: Option<NaiveDate>,
    notes: Option<String>,
    sort_index: i32,
    show_in_chart: bool,
}

fn scope_to_direction(scope: &str) -> Result<PlanningFlowDirection, ApiError> {
    match scope {
        "income" => Ok(PlanningFlowDirection::Inflow),
        "expense" => Ok(PlanningFlowDirection::Outflow),
        _ => Err(ApiError::BadRequest(
            "planning_flow_category_scope_unsupported: planning flow category must be income or expense scope".into(),
        )),
    }
}

fn normalize_title(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest("title_empty: title must not be empty".into()));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "title_too_long: title must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn patch_due_date_from_json(
    v: &Value,
) -> Result<Option<NaiveDate>, ApiError> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v
        .as_str()
        .ok_or_else(|| ApiError::BadRequest("due_date_type: due_date must be a string or null".into()))?;
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d").map(Some).map_err(|_| {
        ApiError::BadRequest("due_date_format: due_date must be YYYY-MM-DD".into())
    })
}

/// Tri-estado hermano de `patch_due_date_from_json` para las DOS fechas de ventana (#148). Un
/// solo par de códigos para ambas: `error_codes_parity` extrae literales del fuente, así que no
/// se componen con `format!`.
fn patch_window_date_from_json(v: &Value) -> Result<Option<NaiveDate>, ApiError> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v.as_str().ok_or_else(|| {
        ApiError::BadRequest("window_date_type: window dates must be a string or null".into())
    })?;
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d").map(Some).map_err(|_| {
        ApiError::BadRequest("window_date_format: window dates must be YYYY-MM-DD".into())
    })
}

/// La forma que cada base exige (#148), validada sobre el estado RESULTANTE — la usa el create y
/// el patch, así que un PATCH no puede dejar una fila que el CHECK de la tabla rechazaría con un
/// 500 opaco.
fn validate_flow_shape(
    basis: PlanningAmountBasis,
    due_date: Option<NaiveDate>,
    window_start: Option<NaiveDate>,
    window_end: Option<NaiveDate>,
) -> Result<(), ApiError> {
    match basis {
        PlanningAmountBasis::OneOff => {
            if window_start.is_some() || window_end.is_some() {
                return Err(ApiError::BadRequest(
                    "window_requires_per_month: window_start_date/window_end_date only apply to amount_basis per_month".into(),
                ));
            }
        }
        PlanningAmountBasis::PerMonth => {
            if due_date.is_some() {
                return Err(ApiError::BadRequest(
                    "per_month_excludes_due_date: a per_month flow uses a window, not due_date".into(),
                ));
            }
            let Some(start) = window_start else {
                return Err(ApiError::BadRequest(
                    "window_start_required: amount_basis per_month requires window_start_date".into(),
                ));
            };
            if let Some(end) = window_end {
                if end < start {
                    return Err(ApiError::BadRequest(
                        "window_end_before_start: window_end_date must not be before window_start_date".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Cota de 100 años sobre las fechas de ventana — código hermano de `due_date_out_of_range`,
/// mismo bound compartido (`max_user_settable_future_date`), literal completo a propósito.
async fn validate_window_date_range(
    pool: &sqlx::PgPool,
    iid: Uuid,
    dates: [Option<NaiveDate>; 2],
) -> Result<(), ApiError> {
    if dates.iter().all(|d| d.is_none()) {
        return Ok(());
    }
    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    let max = crate::handlers::max_user_settable_future_date(today);
    for d in dates.into_iter().flatten() {
        if d > max {
            return Err(ApiError::BadRequest(
                "window_date_out_of_range: window dates must not be more than 100 years in the future".into(),
            ));
        }
    }
    Ok(())
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                Ok(None)
            } else if t.len() > 4000 {
                Err(ApiError::BadRequest(
                    "notes_too_long: notes must be at most 4000 characters".into(),
                ))
            } else {
                Ok(Some(t.into()))
            }
        }
    }
}

fn row_to_response(r: PlanningFlowJoinRow) -> Result<PlanningFlowResponse, ApiError> {
    Ok(PlanningFlowResponse {
        id: r.id,
        category_id: r.category_id,
        direction: scope_to_direction(r.scope.as_str())?,
        title: r.title,
        expected_amount: r.expected_amount,
        // El CHECK de la tabla garantiza que esto no falla; el parse mantiene un único camino
        // TEXT → enum.
        amount_basis: PlanningAmountBasis::parse(r.amount_basis.as_str())?,
        due_date: r.due_date,
        window_start_date: r.window_start_date,
        window_end_date: r.window_end_date,
        notes: r.notes,
        sort_index: r.sort_index,
        show_in_chart: r.show_in_chart,
    })
}

#[utoipa::path(
    get,
    path = "/v1/planning/flows",
    tag = "planning",
    params(
        ("view" = Option<String>, Query, description = "`mine` = flows attributed to the signed-in user; omit = household."),
    ),
    responses(
        (status = 200, description = "Planning flows", body = Vec<PlanningFlowResponse>),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_planning_flows(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<PlanningFlowResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_planning_flows_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_planning_flows`.
pub(crate) async fn list_planning_flows_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: crate::handlers::person_view::LedgerView,
) -> Result<Vec<PlanningFlowResponse>, ApiError> {
    let scope = view.scope_where("p");
    let sql = format!(
        r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                  p.expected_amount, p.amount_basis, p.due_date,
                  p.window_start_date, p.window_end_date, p.notes, p.sort_index,
                  p.show_in_chart
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {scope}
           ORDER BY p.sort_index ASC, p.due_date ASC NULLS LAST, p.title ASC"#
    );
    let rows: Vec<PlanningFlowJoinRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .fetch_all(pool)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_response(r)?);
    }

    Ok(out)
}

#[utoipa::path(
    post,
    path = "/v1/planning/flows",
    tag = "planning",
    request_body = CreatePlanningFlowBody,
    responses(
        (status = 201, description = "Created", body = PlanningFlowResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreatePlanningFlowBody>,
) -> Result<(axum::http::StatusCode, Json<PlanningFlowResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_planning_flow_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_planning_flow`.
/// Invalidación FULL post-insert dentro (los planning flows son inputs del engine).
/// Cota superior de `due_date`. El código va como literal completo a propósito: ver la nota en
/// `handlers::max_user_settable_future_date`.
async fn validate_due_date_range(
    pool: &sqlx::PgPool,
    iid: Uuid,
    due_date: Option<NaiveDate>,
) -> Result<(), ApiError> {
    let Some(d) = due_date else { return Ok(()) };
    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    if d > crate::handlers::max_user_settable_future_date(today) {
        return Err(ApiError::BadRequest(
            "due_date_out_of_range: due_date must not be more than 100 years in the future".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn create_planning_flow_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreatePlanningFlowBody,
) -> Result<PlanningFlowResponse, ApiError> {
    assert_budget_category(&state.pool, iid, body.category_id).await?;

    if body.expected_amount <= Decimal::ZERO {
        return Err(ApiError::BadRequest(
            "amount_not_positive: expected_amount must be greater than zero".into(),
        ));
    }
    let basis = match &body.amount_basis {
        Some(s) => PlanningAmountBasis::parse(s)?,
        None => PlanningAmountBasis::OneOff,
    };
    validate_flow_shape(basis, body.due_date, body.window_start_date, body.window_end_date)?;
    validate_due_date_range(&state.pool, iid, body.due_date).await?;
    validate_window_date_range(
        &state.pool,
        iid,
        [body.window_start_date, body.window_end_date],
    )
    .await?;

    let title = normalize_title(&body.title)?;
    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    // Invariante (#148 amplía el histórico): solo un puntual CON fecha produce un escalón
    // dibujable — un `per_month` es una rampa, la misma razón por la que los sin-fecha nunca
    // entraron al chart.
    let show_in_chart = body.show_in_chart.unwrap_or(false)
        && basis == PlanningAmountBasis::OneOff
        && body.due_date.is_some();

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO planning_flows (
               installation_id, category_id, title, expected_amount, amount_basis, due_date,
               window_start_date, window_end_date, notes, sort_index, owner_user_id,
               show_in_chart
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&title)
    .bind(body.expected_amount)
    .bind(basis.as_str())
    .bind(body.due_date)
    .bind(body.window_start_date)
    .bind(body.window_end_date)
    .bind(&notes)
    .bind(sort_index)
    .bind(user_id)
    .bind(show_in_chart)
    .fetch_one(&state.pool)
    .await?;

    let row: PlanningFlowJoinRow = sqlx::query_as(
        r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                  p.expected_amount, p.amount_basis, p.due_date,
                  p.window_start_date, p.window_end_date, p.notes, p.sort_index,
                  p.show_in_chart
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE p.id = $1"#,
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    row_to_response(row)
}

#[utoipa::path(
    patch,
    path = "/v1/planning/flows/{id}",
    tag = "planning",
    request_body = PatchPlanningFlowBody,
    params(
        ("id" = Uuid, Path, description = "Planning flow id"),
    ),
    responses(
        (status = 200, description = "Updated", body = PlanningFlowResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Flow missing"),
    )
)]
pub async fn patch_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchPlanningFlowBody>,
) -> Result<Json<PlanningFlowResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_planning_flow_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_planning_flow`.
/// `due_date` es tri-state (`patch_due_date_from_json`); invalidación FULL dentro.
pub(crate) async fn patch_planning_flow_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchPlanningFlowBody,
) -> Result<PlanningFlowResponse, ApiError> {
    if body.category_id.is_none()
        && body.title.is_none()
        && body.expected_amount.is_none()
        && body.due_date.is_none()
        && body.amount_basis.is_none()
        && body.window_start_date.is_none()
        && body.window_end_date.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
        && body.show_in_chart.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one field to update".into(),
        ));
    }

    let row: Option<PlanningFlowJoinRow> = sqlx::query_as(
        r#"SELECT p.id, p.category_id, c.scope AS scope, p.title,
                  p.expected_amount, p.amount_basis, p.due_date,
                  p.window_start_date, p.window_end_date, p.notes, p.sort_index,
                  p.show_in_chart
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE p.id = $1 AND p.installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };

    let new_cat = body.category_id.unwrap_or(current.category_id);
    if new_cat != current.category_id {
        assert_budget_category(&state.pool, iid, new_cat).await?;
    }

    let new_title = match &body.title {
        Some(t) => normalize_title(t)?,
        None => current.title.clone(),
    };

    let new_amount = match body.expected_amount {
        Some(a) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "amount_not_positive: expected_amount must be greater than zero".into(),
                ));
            }
            a
        }
        None => current.expected_amount,
    };

    let new_due = match &body.due_date {
        Some(v) => patch_due_date_from_json(v)?,
        None => current.due_date,
    };
    // Solo se valida lo que el patch INTRODUCE: una fila antigua fuera de cota se puede seguir
    // editando en otros campos (y arreglando), que es lo contrario de dejarla intocable.
    if body.due_date.is_some() {
        validate_due_date_range(&state.pool, iid, new_due).await?;
    }

    let new_basis = match &body.amount_basis {
        Some(s) => PlanningAmountBasis::parse(s)?,
        None => PlanningAmountBasis::parse(current.amount_basis.as_str())?,
    };
    let new_window_start = match &body.window_start_date {
        Some(v) => patch_window_date_from_json(v)?,
        None => current.window_start_date,
    };
    let new_window_end = match &body.window_end_date {
        Some(v) => patch_window_date_from_json(v)?,
        None => current.window_end_date,
    };
    // El estado RESULTANTE entero (#148): cambiar de base exige dejar coherentes fecha y
    // ventana en el mismo request — nada se auto-borra en silencio.
    validate_flow_shape(new_basis, new_due, new_window_start, new_window_end)?;
    validate_window_date_range(
        &state.pool,
        iid,
        [
            body.window_start_date.as_ref().and(new_window_start),
            body.window_end_date.as_ref().and(new_window_end),
        ],
    )
    .await?;

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let new_show_in_chart_raw = body.show_in_chart.unwrap_or(current.show_in_chart);
    // Invariante: solo un puntual con due_date tras el patch puede marcarse (#148 amplía la
    // regla histórica: un per_month es una rampa, no un escalón).
    let new_show_in_chart = new_show_in_chart_raw
        && new_basis == PlanningAmountBasis::OneOff
        && new_due.is_some();

    let updated: PlanningFlowJoinRow = sqlx::query_as(
        r#"UPDATE planning_flows
           SET category_id = $1,
               title = $2,
               expected_amount = $3,
               amount_basis = $4,
               due_date = $5,
               window_start_date = $6,
               window_end_date = $7,
               notes = $8,
               sort_index = $9,
               show_in_chart = $10,
               updated_at = now()
           WHERE id = $11 AND installation_id = $12
           RETURNING planning_flows.id,
                     planning_flows.category_id,
                     (
                         SELECT c.scope
                         FROM categories c
                         WHERE c.id = planning_flows.category_id
                     ) AS scope,
                     planning_flows.title,
                     planning_flows.expected_amount,
                     planning_flows.amount_basis,
                     planning_flows.due_date,
                     planning_flows.window_start_date,
                     planning_flows.window_end_date,
                     planning_flows.notes,
                     planning_flows.sort_index,
                     planning_flows.show_in_chart"#,
    )
    .bind(new_cat)
    .bind(&new_title)
    .bind(new_amount)
    .bind(new_basis.as_str())
    .bind(new_due)
    .bind(new_window_start)
    .bind(new_window_end)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(new_show_in_chart)
    .bind(id)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    row_to_response(updated)
}

#[utoipa::path(
    delete,
    path = "/v1/planning/flows/{id}",
    tag = "planning",
    params(
        ("id" = Uuid, Path, description = "Planning flow id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Flow missing"),
    )
)]
pub async fn delete_planning_flow(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    delete_planning_flow_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_planning_flow`.
pub(crate) async fn delete_planning_flow_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res =
        sqlx::query(r#"DELETE FROM planning_flows WHERE id = $1 AND installation_id = $2"#)
            .bind(id)
            .bind(iid)
            .execute(&state.pool)
            .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(())
}

pub fn planning_router() -> Router {
    Router::new()
        .route("/flows", get(list_planning_flows).post(create_planning_flow))
        .route(
            "/flows/{id}",
            patch(patch_planning_flow).delete(delete_planning_flow),
        )
}
