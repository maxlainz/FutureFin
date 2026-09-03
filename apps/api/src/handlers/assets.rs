use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{require_row_owner, LedgerView, LedgerViewQuery};
use crate::handlers::projection::{assets_projection_context, refresh_projection_after_mutation};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct AssetResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub current_value: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
    /// **Plusvalía latente**: `current_value − purchase_price`, en euros. Puede ser negativa.
    ///
    /// **NO es rentabilidad.** No anualiza, y no descuenta las aportaciones posteriores a la
    /// compra — que en este modelo son mensuales, así que en un activo con reglas de reparto
    /// activas el número está inflado por todo lo que se ha ido metiendo: leerlo como retorno
    /// engaña. Para retorno hay `expected_annual_return_percent` (supuesto, hacia adelante) y
    /// `summary.financial_health.net_return_*` (realizado, a nivel de hogar).
    ///
    /// `null` ⟺ `unrealized_pnl_absent_reason == "no_purchase_price"`: sin coste declarado no hay
    /// resta que hacer, y un `0` ahí significaría «no has ganado ni perdido», que es una
    /// afirmación distinta de «no sé lo que te costó».
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub unrealized_pnl: Option<Decimal>,
    /// Por qué falta `unrealized_pnl`. Hoy un único valor: `no_purchase_price`. `null` ⟺ la cifra
    /// viaja.
    #[schema(value_type = Option<String>)]
    pub unrealized_pnl_absent_reason: Option<&'static str>,
    /// La plusvalía latente como porcentaje del coste: `(current_value − purchase_price) /
    /// purchase_price × 100`, un decimal. Mismos avisos que `unrealized_pnl`: **no es una TAE**.
    ///
    /// `null` en dos casos distintos, y por eso hay un motivo aparte: sin coste declarado
    /// (`no_purchase_price`) o con coste declarado **cero** (`zero_purchase_price`), donde el
    /// porcentaje no está definido (dividir entre 0) aunque la plusvalía en euros sí lo esté.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub unrealized_pnl_pct: Option<Decimal>,
    /// `no_purchase_price` | `zero_purchase_price`. `null` ⟺ `unrealized_pnl_pct` viaja.
    #[schema(value_type = Option<String>)]
    pub unrealized_pnl_pct_absent_reason: Option<&'static str>,
    pub is_liquid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
    /// **Volatilidad anual** de los retornos de este activo, en puntos porcentuales
    /// (`"17"` = 17 %/año). Es la desviación típica ANUAL, no un rango ni un peor caso.
    ///
    /// `null` o `0` = activo determinista (cuenta corriente, depósito). El camino Decimal del
    /// motor la **ignora siempre** (D12): solo la lee el Monte Carlo, así que declararla no
    /// mueve ni un euro de la proyección determinista. Cota [0, 100].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub annual_volatility_percent: Option<Decimal>,
    /// Aporte del **primer mes** resuelto por la cascada de reglas de asignación. Ojo al nombre:
    /// NO es un importe mensual estable — la cascada reparte `net_cash_month`, que incluye el
    /// tramo de los planning flows sin fecha del mes en curso (repartidos a `importe/90` por día
    /// natural), así que el valor **decrece cada día** y **salta el día 1 de cada mes**. Para el
    /// número estable —el que una persona quiere decir con «mi aportación mensual»— usa
    /// `contribution_recurring_monthly`. Se sirve redondeado a 4 decimales (política monetaria).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub contribution_nominal_monthly: Decimal,
    /// La MISMA cascada evaluada sobre el neto **recurrente** (`income − expense − debt_service`),
    /// sin el tramo transitorio de los planning flows. Es el número **estable** — el que una
    /// persona quiere decir cuando dice «mi aportación mensual» — y el único con el que tiene
    /// sentido hacer aritmética: no cambia de un día para otro.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub contribution_recurring_monthly: Decimal,
    /// Tope absoluto en € si alguna regla de asignación apunta a este activo con
    /// `cap_kind='amount'` (el más restrictivo si hay varias). Solo se devuelve cuando es
    /// un tope concreto en euros — los topes relativos (`months_expense`, `income_multiple`)
    /// no aparecen aquí porque varían con el presupuesto.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub contribution_target_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
    /// Usuario dueño de la fila. Desde 5.0.0 la columna es `NOT NULL` (D14), así que el campo
    /// viaja SIEMPRE — el `Option` se conserva por compatibilidad del contrato publicado.
    ///
    /// Es dato de display para la UI (el trigger del modal de snapshot) **y**, desde D21, el
    /// que decide quién puede editar la fila: una mutación sobre un activo ajeno devuelve 403
    /// `not_row_owner`. La lectura sigue siendo del hogar (`?view` no es autorización).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub owner_user_id: Option<Uuid>,
    /// #150: si este create SEMBRÓ la regla `remainder` (primer activo de un scope sin cascada),
    /// aquí viaja su id — ninguna escritura implícita queda silenciosa (política S2). Solo en la
    /// respuesta del POST/tool `create_asset`; omitido en GET y PATCH.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub seeded_allocation_rule_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAssetBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub current_value: Decimal,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
    #[serde(default)]
    pub is_liquid: Option<bool>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
    /// Volatilidad anual en % (0–100). Omitir o `0` = activo determinista.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub annual_volatility_percent: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchAssetBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub name: Option<String>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub current_value: Option<Decimal>,
    /// Omitir sin cambio; `null` borra el precio de compra.
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub purchase_price: Option<serde_json::Value>,
    pub is_liquid: Option<bool>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
    /// Volatilidad anual en % (0–100). Omitir = sin cambio.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub annual_volatility_percent: Option<Decimal>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct AssetRow {
    id: Uuid,
    category_id: Uuid,
    name: String,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    expected_annual_return_percent: Option<Decimal>,
    annual_volatility_percent: Option<Decimal>,
    notes: Option<String>,
    sort_index: i32,
    /// `NOT NULL` desde la migración 5.0.0 (D14): toda fila del ledger tiene dueño.
    owner_user_id: Uuid,
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

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("amount_negative: {field} must be >= 0")));
    }
    Ok(())
}

/// PATCH: clave ausente → conservar `current`; `null` JSON → `None` en BD; valor → sustituir.
fn merge_optional_decimal_patch(
    patch: &Option<serde_json::Value>,
    current: Option<Decimal>,
    field: &'static str,
) -> Result<Option<Decimal>, ApiError> {
    match patch {
        None => Ok(current),
        Some(v) => {
            if v.is_null() {
                return Ok(None);
            }
            let d: Decimal = if let serde_json::Value::String(s) = v {
                s.trim().parse().map_err(|_| {
                    ApiError::BadRequest(format!("decimal_invalid: {field} must be a valid decimal string"))
                })?
            } else {
                serde_json::from_value(v.clone()).map_err(|_| {
                    ApiError::BadRequest(format!("decimal_invalid: {field} must be a valid decimal"))
                })?
            };
            assert_non_negative(d, field)?;
            Ok(Some(d))
        }
    }
}

async fn assert_asset_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM categories
            WHERE
                id = $1
                AND installation_id = $2
                AND scope = 'asset'
        )"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;

    if !ok {
        return Err(ApiError::BadRequest(
            "category_wrong_scope: category_id must reference an asset category in this installation".into(),
        ));
    }
    Ok(())
}

/// Plusvalía latente de un activo: `(euros, motivo_ausencia, porcentaje, motivo_ausencia_pct)`.
///
/// Un solo sitio porque los dos campos y sus dos motivos tienen que decidirse juntos: el caso
/// `purchase_price = 0` da euros SÍ y porcentaje NO, y separarlos invita a que uno de los dos
/// publique `null` sin motivo (o un `0` que se lee como «ni ganancia ni pérdida»).
fn unrealized_pnl_of(
    current_value: Decimal,
    purchase_price: Option<Decimal>,
) -> (
    Option<Decimal>,
    Option<&'static str>,
    Option<Decimal>,
    Option<&'static str>,
) {
    match purchase_price {
        None => (None, Some("no_purchase_price"), None, Some("no_purchase_price")),
        Some(pp) if pp.is_zero() => (
            Some(current_value),
            None,
            None,
            Some("zero_purchase_price"),
        ),
        Some(pp) => {
            let pnl = current_value - pp;
            let pct = (pnl / pp * Decimal::from(100)).round_dp(1);
            (Some(pnl), None, Some(pct), None)
        }
    }
}

fn row_to_response(
    r: AssetRow,
    contribution_nominal_monthly: Decimal,
    contribution_recurring_monthly: Decimal,
    contribution_target_amount: Option<Decimal>,
) -> AssetResponse {
    let (unrealized_pnl, unrealized_pnl_absent_reason, unrealized_pnl_pct, unrealized_pnl_pct_absent_reason) =
        unrealized_pnl_of(r.current_value, r.purchase_price);
    AssetResponse {
        id: r.id,
        category_id: r.category_id,
        name: r.name,
        current_value: r.current_value,
        purchase_price: r.purchase_price,
        unrealized_pnl,
        unrealized_pnl_absent_reason,
        unrealized_pnl_pct,
        unrealized_pnl_pct_absent_reason,
        is_liquid: r.is_liquid,
        expected_annual_return_percent: r.expected_annual_return_percent,
        annual_volatility_percent: r.annual_volatility_percent,
        // Presentación: la cascada trabaja con la precisión completa, aquí solo se publica.
        contribution_nominal_monthly: contribution_nominal_monthly.round_dp(4),
        contribution_recurring_monthly: contribution_recurring_monthly.round_dp(4),
        contribution_target_amount,
        notes: r.notes,
        sort_index: r.sort_index,
        owner_user_id: Some(r.owner_user_id),
        seeded_allocation_rule_id: None,
    }
}

/// Build `asset_id → target_€` from the **first** allocation rule (lowest `priority`) that
/// targets each asset and has a cap. Caps in `months_expense` / `income_multiple` are resolved
/// to absolute € using the scope's **effective** monthly income / expense + debt_service baseline
/// (`assets_projection_context`), es decir los mismos escalares con los que simula el engine.
async fn fetch_asset_resolved_targets(
    pool: &sqlx::PgPool,
    iid: Uuid,
    view: LedgerView,
    session_user_id: Uuid,
    income_monthly: Decimal,
    expense_with_debt: Decimal,
) -> Result<std::collections::HashMap<Uuid, Decimal>, ApiError> {
    let scope = view.scope_where("");
    let sql = format!(
        r#"SELECT DISTINCT ON (target_asset_id)
                  target_asset_id, cap_kind, cap_value
           FROM allocation_rules
           WHERE {scope}
             AND cap_kind IS NOT NULL AND cap_value IS NOT NULL
           ORDER BY target_asset_id, priority ASC, id ASC"#
    );
    let rows: Vec<(Uuid, String, Decimal)> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let mut out = std::collections::HashMap::with_capacity(rows.len());
    for (asset_id, cap_kind, cap_value) in rows {
        // Una sola implementación del techo en todo el lado API (`allocation_rules.rs`): el
        // objetivo que enseña esta pantalla y el techo contra el que
        // `GET /v1/allocation-rules/goals` calcula el ETA deben ser el MISMO número.
        let Some(resolved) = crate::handlers::allocation_rules::resolve_cap_ceiling_eur(
            &cap_kind,
            cap_value,
            income_monthly,
            expense_with_debt,
        ) else {
            continue;
        };
        if resolved > Decimal::ZERO {
            out.insert(asset_id, resolved);
        }
    }
    Ok(out)
}

#[utoipa::path(
    get,
    path = "/v1/assets",
    tag = "assets",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
    ),
    responses(
        (status = 200, description = "Assets for the installation", body = [AssetResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_assets(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<AssetResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_assets_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_assets`.
pub(crate) async fn list_assets_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<AssetResponse>, ApiError> {
    let today = installation_naive_today(pool, iid).await?;
    let ctx = assets_projection_context(pool, iid, user_id, view, today).await?;
    let targets = fetch_asset_resolved_targets(
        pool,
        iid,
        view,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let nominals = ctx.nominals;
    let recurring = ctx.recurring_nominals;

    let assets_scope = view.scope_where("");
    let assets_sql = format!(
        r#"SELECT id, category_id, name, current_value, purchase_price,
                  is_liquid, expected_annual_return_percent, annual_volatility_percent,
                  notes, sort_index, owner_user_id
           FROM assets
           WHERE {assets_scope}
           ORDER BY sort_index ASC, name ASC, id ASC"#
    );
    let rows: Vec<AssetRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, user_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let n = nominals.get(&r.id).copied().unwrap_or(Decimal::ZERO);
            let rec = recurring.get(&r.id).copied().unwrap_or(Decimal::ZERO);
            let t = targets.get(&r.id).copied();
            row_to_response(r, n, rec, t)
        })
        .collect())
}

#[utoipa::path(
    post,
    path = "/v1/assets",
    tag = "assets",
    request_body = CreateAssetBody,
    responses(
        (status = 201, description = "Created", body = AssetResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_asset(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateAssetBody>,
) -> Result<(axum::http::StatusCode, Json<AssetResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_asset_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Rentabilidad esperada válida: > −100 %. El engine clampa ≤ −100 a pérdida total, pero la
/// capa API rechaza inputs nuevos absurdos (misma cota que los overrides de simulate_projection).
pub(crate) fn assert_return_percent(pct: Option<Decimal>) -> Result<(), ApiError> {
    if let Some(p) = pct {
        if p <= Decimal::from(-100) {
            return Err(ApiError::BadRequest(
                "return_percent_too_low: expected_annual_return_percent must be greater than -100".into(),
            ));
        }
    }
    Ok(())
}

/// Cota de la volatilidad anual declarada por activo: `[0, 100]` puntos porcentuales.
///
/// El `CHECK` de columna es más laxo a propósito (solo `>= 0`, para poder importar backups
/// viejos); esta es la cota de la API. 100 % de desviación típica anual ya es un activo cuyo
/// valor puede duplicarse o irse a cero en un año: por encima, el número deja de describir
/// nada que una cartera pueda tener.
pub(crate) fn assert_volatility_percent(v: Option<Decimal>) -> Result<(), ApiError> {
    if let Some(vol) = v {
        if vol < Decimal::ZERO || vol > Decimal::from(100u32) {
            return Err(ApiError::BadRequest(
                "volatility_out_of_range: annual_volatility_percent must be between 0 and 100"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_asset`.
/// Invalidación FULL dentro. El alta atribuye SIEMPRE al usuario de la sesión (D21): no hay
/// forma de crear una fila a nombre de otro miembro.
///
/// **#150 — siembra del sumidero.** Si este es el PRIMER activo de un scope sin cascada (cero
/// activos Y cero reglas del owner), tras el INSERT se crea la regla `remainder` apuntándole,
/// por la MISMA `create_allocation_rule_core` que valida la invariante — cero SQL nuevo, cero
/// invariante duplicada. **Límite transaccional conocido y deliberado**: el INSERT del activo va
/// contra el pool y la regla abre su propia transacción (el módulo de reglas tiene un único
/// punto de commit, custodiado por test estructural) — la secuencia activo→regla NO es atómica;
/// si la regla falla, queda un activo sin sumidero (el estado pre-#150, que el aviso
/// `surplus_destination: "cash"` de la resolución cubre).
pub(crate) async fn create_asset_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateAssetBody,
) -> Result<AssetResponse, ApiError> {
    assert_asset_category(&state.pool, iid, body.category_id).await?;
    assert_return_percent(body.expected_annual_return_percent)?;
    assert_volatility_percent(body.annual_volatility_percent)?;

    let name = normalize_name(&body.name)?;
    assert_non_negative(body.current_value, "current_value")?;
    if let Some(pp) = body.purchase_price {
        assert_non_negative(pp, "purchase_price")?;
    }
    let notes = normalize_notes(&body.notes)?;
    let is_liquid = body.is_liquid.unwrap_or(true);
    let sort_index = body.sort_index.unwrap_or(0);

    // #150: ¿scope virgen? Las DOS condiciones, no una — «cero reglas» sola retro-sembraría en
    // scopes antiguos con activos y sin sumidero (el owner descartó la retro-siembra); «cero
    // activos» sola sembraría en un scope que borró sus activos pero conserva reglas.
    let bootstrap: bool = sqlx::query_scalar(
        r#"SELECT NOT EXISTS (
               SELECT 1 FROM assets WHERE installation_id = $1 AND owner_user_id = $2
           ) AND NOT EXISTS (
               SELECT 1 FROM allocation_rules WHERE installation_id = $1 AND owner_user_id = $2
           )"#,
    )
    .bind(iid)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    let row: AssetRow = sqlx::query_as(
        r#"INSERT INTO assets (
               installation_id, category_id, name, current_value,
               purchase_price, is_liquid,
               expected_annual_return_percent,
               annual_volatility_percent, notes, sort_index, owner_user_id
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, expected_annual_return_percent,
                     annual_volatility_percent, notes, sort_index, owner_user_id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(&name)
    .bind(body.current_value)
    .bind(body.purchase_price)
    .bind(is_liquid)
    .bind(body.expected_annual_return_percent)
    .bind(body.annual_volatility_percent)
    .bind(&notes)
    .bind(sort_index)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    // #150: la siembra usa la MISMA función que crea y valida cualquier regla — arrastra gratis
    // assert_asset_in_scope, la colocación de prioridad, commit_with_sink_invariant y la
    // invalidación de cache. Errores aquí se propagan en voz alta (el activo ya existe: el
    // estado queda como el pre-#150 y la resolución lo declara).
    let seeded_allocation_rule_id = if bootstrap {
        Some(
            crate::handlers::allocation_rules::create_allocation_rule_core(
                state,
                iid,
                user_id,
                crate::handlers::allocation_rules::CreateAllocationRuleBody {
                    target_asset_id: row.id,
                    kind: "remainder".into(),
                    amount: None,
                    cap_kind: None,
                    cap_value: None,
                    enabled: None,
                    notes: None,
                },
                crate::handlers::allocation_rules::SinkPolicy::Allowed,
            )
            .await?
            .id,
        )
    } else {
        None
    };

    let today = installation_naive_today(&state.pool, iid).await?;
    let ctx =
        assets_projection_context(&state.pool, iid, user_id, LedgerView::Household, today).await?;
    let n = ctx.nominals.get(&row.id).copied().unwrap_or(Decimal::ZERO);
    let rec = ctx
        .recurring_nominals
        .get(&row.id)
        .copied()
        .unwrap_or(Decimal::ZERO);
    let targets = fetch_asset_resolved_targets(
        &state.pool,
        iid,
        LedgerView::Household,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let t = targets.get(&row.id).copied();

    refresh_projection_after_mutation(&state, iid, user_id).await;
    let mut resp = row_to_response(row, n, rec, t);
    resp.seeded_allocation_rule_id = seeded_allocation_rule_id;
    Ok(resp)
}

#[utoipa::path(
    patch,
    path = "/v1/assets/{id}",
    tag = "assets",
    request_body = PatchAssetBody,
    params(
        ("id" = Uuid, Path, description = "Asset id"),
    ),
    responses(
        (status = 200, description = "Updated", body = AssetResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Asset missing"),
    )
)]
pub async fn patch_asset(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssetBody>,
) -> Result<Json<AssetResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_asset_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y las tools MCP `update_asset` (body completo)
/// y `update_asset_value` (subset de valoración). Invalidación FULL dentro.
///
/// **D21 (5.0.0)**: exige que el activo sea del usuario de la sesión — 403 `not_row_owner` si es
/// de otro miembro, 404 si no existe. El comentario histórico decía «sin owner-check (contrato
/// del módulo)»; ese contrato se retiró con las proyecciones por miembro.
pub(crate) async fn patch_asset_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchAssetBody,
) -> Result<AssetResponse, ApiError> {
    assert_return_percent(body.expected_annual_return_percent)?;
    assert_volatility_percent(body.annual_volatility_percent)?;
    if body.category_id.is_none()
        && body.name.is_none()
        && body.current_value.is_none()
        && body.purchase_price.is_none()
        && body.is_liquid.is_none()
        && body.expected_annual_return_percent.is_none()
        && body.annual_volatility_percent.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one field to update".into(),
        ));
    }

    let row: Option<AssetRow> = sqlx::query_as(
        r#"SELECT id, category_id, name, current_value, purchase_price,
                  is_liquid, expected_annual_return_percent, annual_volatility_percent,
                  notes, sort_index, owner_user_id
           FROM assets
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };
    require_row_owner(current.owner_user_id, user_id)?;

    let new_cat = body.category_id.unwrap_or(current.category_id);
    if new_cat != current.category_id {
        assert_asset_category(&state.pool, iid, new_cat).await?;
    }

    let new_name = match &body.name {
        Some(s) => normalize_name(s)?,
        None => current.name.clone(),
    };

    let new_val = match body.current_value {
        Some(v) => {
            assert_non_negative(v, "current_value")?;
            v
        }
        None => current.current_value,
    };

    let new_pp = merge_optional_decimal_patch(&body.purchase_price, current.purchase_price, "purchase_price")?;

    let new_liquid = body.is_liquid.unwrap_or(current.is_liquid);

    let new_exp = if body.expected_annual_return_percent.is_some() {
        body.expected_annual_return_percent
    } else {
        current.expected_annual_return_percent
    };

    let new_vol = if body.annual_volatility_percent.is_some() {
        body.annual_volatility_percent
    } else {
        current.annual_volatility_percent
    };

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let updated: AssetRow = sqlx::query_as(
        r#"UPDATE assets
           SET category_id = $1,
               name = $2,
               current_value = $3,
               purchase_price = $4,
               is_liquid = $5,
               expected_annual_return_percent = $6,
               annual_volatility_percent = $7,
               notes = $8,
               sort_index = $9,
               updated_at = now()
           WHERE id = $10 AND installation_id = $11 AND owner_user_id = $12
           RETURNING id, category_id, name, current_value, purchase_price,
                     is_liquid, expected_annual_return_percent,
                     annual_volatility_percent, notes, sort_index, owner_user_id"#,
    )
    .bind(new_cat)
    .bind(&new_name)
    .bind(new_val)
    .bind(new_pp)
    .bind(new_liquid)
    .bind(new_exp)
    .bind(new_vol)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    let today = installation_naive_today(&state.pool, iid).await?;
    let ctx =
        assets_projection_context(&state.pool, iid, user_id, LedgerView::Household, today).await?;
    let n = ctx.nominals.get(&updated.id).copied().unwrap_or(Decimal::ZERO);
    let rec = ctx
        .recurring_nominals
        .get(&updated.id)
        .copied()
        .unwrap_or(Decimal::ZERO);
    let targets = fetch_asset_resolved_targets(
        &state.pool,
        iid,
        LedgerView::Household,
        user_id,
        ctx.income_monthly,
        ctx.expense_with_debt,
    )
    .await?;
    let t = targets.get(&updated.id).copied();

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(row_to_response(updated, n, rec, t))
}

#[utoipa::path(
    delete,
    path = "/v1/assets/{id}",
    tag = "assets",
    params(
        ("id" = Uuid, Path, description = "Asset id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Asset missing"),
    )
)]
pub async fn delete_asset(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    delete_asset_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Efectos colaterales de borrar un activo, para el preview de la tool MCP: los links de
/// transacciones (`linked_asset_id`) y lotes de import (`account_asset_id`) pasan a NULL.
#[derive(Debug, serde::Serialize)]
pub(crate) struct AssetDeleteEffects {
    pub transactions_unlinked: i64,
    pub imports_unlinked: i64,
    /// Reglas de la cascada que se BORRAN con el activo (`ON DELETE CASCADE`).
    ///
    /// Es el único efecto irreversible del borrado, y era el único que el preview no contaba:
    /// enseñaba «los movimientos quedan desvinculados» —cierto, `SET NULL`—, el usuario
    /// confirmaba, y desaparecían además las reglas de reparto que apuntaban a ese activo.
    /// A partir de ahí el ahorro mensual se reparte de otra manera, en silencio.
    pub allocation_rules_deleted: i64,
    /// De esas reglas, cuántas eran `remainder` sin tope — el sumidero de la cascada. Perderlo
    /// cambia a dónde va todo el sobrante, no solo una parte.
    pub allocation_remainder_rules_deleted: i64,
}

/// **D21 en el PREVIEW, no solo en el borrado.** El preview de `delete_asset` enseña el nombre y
/// el valor del activo, cuenta la cascada que se lleva por delante y —esto es lo que lo separa de
/// una lectura cualquiera— **emite un `confirm_token`**. Sobre un activo ajeno eso es contarle a
/// alguien el plan de otro y darle además la credencial para ejecutarlo; que la confirmación
/// fuese a fallar después no lo arregla. Falla aquí, con el mismo `not_row_owner`.
pub(crate) async fn asset_delete_effects(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    id: Uuid,
) -> Result<AssetDeleteEffects, ApiError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM assets WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(pool)
    .await?;
    require_row_owner(owner.ok_or(ApiError::NotFound)?, session_user_id)?;

    let transactions_unlinked: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM transactions
           WHERE installation_id = $1 AND linked_asset_id = $2"#,
    )
    .bind(iid)
    .bind(id)
    .fetch_one(pool)
    .await?;
    let imports_unlinked: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM transaction_imports
           WHERE installation_id = $1 AND account_asset_id = $2"#,
    )
    .bind(iid)
    .bind(id)
    .fetch_one(pool)
    .await?;
    let (allocation_rules_deleted, allocation_remainder_rules_deleted): (i64, i64) =
        sqlx::query_as(
            r#"SELECT COUNT(*)::bigint,
                      COUNT(*) FILTER (WHERE kind = 'remainder' AND cap_kind IS NULL)::bigint
               FROM allocation_rules
               WHERE installation_id = $1 AND target_asset_id = $2"#,
        )
        .bind(iid)
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(AssetDeleteEffects {
        transactions_unlinked,
        imports_unlinked,
        allocation_rules_deleted,
        allocation_remainder_rules_deleted,
    })
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_asset`.
pub(crate) async fn delete_asset_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    // D21 ANTES que nada: el borrado no tenía SELECT previo, así que hace falta uno para
    // distinguir «no existe» (404) de «es de otro miembro» (403). Va delante de la guardia del
    // sumidero a propósito — contarle a alguien que el activo ajeno es el destino de una regla
    // ya es contarle algo de un plan que no es suyo.
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM assets WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;
    require_row_owner(owner.ok_or(ApiError::NotFound)?, user_id)?;

    // 4.12.1 (#176): el sumidero es indestructible con activos vivos — si este activo es su
    // destino y quedan otros, el borrado se rechaza (mueve antes la regla). El último activo
    // del scope sí se borra.
    crate::handlers::allocation_rules::assert_asset_delete_keeps_the_sink(&state.pool, iid, id)
        .await?;
    let res = sqlx::query(
        r#"DELETE FROM assets WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(())
}

pub fn assets_router() -> Router {
    Router::new()
        .route("/", get(list_assets).post(create_asset))
        .route("/{id}", patch(patch_asset).delete(delete_asset))
}
