//! Monthly projection via `futurefin-engine`: presupuesto regular, cuotas de pasivos activos,
//! aportaciones a activos / drenaje / crecimiento. Los «Próximos» no forman parte de la caja mensual del motor.

use crate::error::ApiError;
use crate::handlers::budget::ledger_regular_monthly_income_and_expense;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::purge_expired_liabilities;
use crate::handlers::person_view::LedgerView;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, NaiveDate};
use futurefin_engine::{
    first_month_per_asset_contribution_nominals, project_net_worth_series, EngineError,
    ProjectionInput, ProjectionLiabilityInput, ProjectionOutput, SimAsset,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ProjectionSeriesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Meses a proyectar (12–840). Si se omite: horizonte tipo cliente macOS (véase `horizon_basis`).
    pub months: Option<u32>,
}

fn resolve_ledger_view(q: &ProjectionSeriesQuery) -> LedgerView {
    match q.view.as_deref().map(str::trim) {
        Some("mine") => LedgerView::Mine,
        _ => LedgerView::Household,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectionPoint {
    pub month_index: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_worth: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub contributed_capital: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectionSeriesResponse {
    pub points: Vec<ProjectionPoint>,
    pub months: u32,
    /// Años de horizonte efectivos (`months / 12`).
    pub horizon_years: u32,
    /// `mac_target_age` | `mac_fallback_no_demographics` | `months_override`
    pub horizon_basis: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub starting_net_worth: Decimal,
    /// Ingresos regulares − gastos regulares (sin líneas derivadas de deuda).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_delta_assumption: Decimal,
    pub model_note: String,
    /// Fecha civil del mes 0 de la serie (misma que `installation_naive_today`).
    pub anchor_date_ymd: String,
    /// Modo UI instalación: `dates` | `ages` (eje temporal en la app web).
    pub show_age_mode: String,
    /// `true` cuando `show_age_mode == ages` y hay fecha de nacimiento para el eje (la web no debe inferir esto sola).
    pub use_age_on_x_axis: bool,
    /// DOB usada para años cumplidos en el eje (perfil y/o personas del hogar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_birth_date: Option<String>,
}

#[derive(Debug, FromRow)]
struct AssetEngineRow {
    id: Uuid,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    expected_annual_return_percent: Option<Decimal>,
    monthly_contribution_fixed: Decimal,
    contribution_frequency: String,
    contribution_remainder_weight: Decimal,
}

#[derive(Debug, FromRow)]
struct LiabEngineRow {
    principal: Decimal,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
}

fn liability_monthly_payment(row: &LiabEngineRow) -> Decimal {
    let Some(amt) = row.payment_amount else {
        return Decimal::ZERO;
    };
    match row.payment_frequency.as_deref() {
        Some("weekly") => (amt * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amt,
    }
}

fn asset_fixed_monthly_equivalent(fixed: Decimal, contribution_frequency: &str) -> Decimal {
    match contribution_frequency {
        "weekly" => (fixed * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => fixed,
    }
}

/// Completed calendar age in years (`today` inclusive), Mac-style for horizon.
fn age_completed_years(today: NaiveDate, birth: NaiveDate) -> i32 {
    if birth > today {
        return 0;
    }
    let mut y = today.year() - birth.year();
    let bd_month = birth.month();
    let bd_day = birth.day();
    let td_month = today.month();
    let td_day = today.day();
    if (td_month, td_day) < (bd_month, bd_day) {
        y -= 1;
    }
    y
}

/// `PRODUCT_DOSSIER_PLAN.md`: máximo años hasta `projection_target_age` por fecha de nacimiento
/// (perfil de usuario), acotado [5, 70]; sin edad objetivo o sin DOB → 30 años.
pub(crate) fn mac_projection_horizon_months(
    today: NaiveDate,
    projection_target_age: Option<i16>,
    birth_dates: &[Option<NaiveDate>],
) -> (u32, &'static str) {
    const MIN_YEARS: u32 = 5;
    const MAX_YEARS: u32 = 70;
    const FALLBACK_YEARS: u32 = 30;

    let Some(target) = projection_target_age.map(|a| a as i32) else {
        return (FALLBACK_YEARS * 12, "mac_fallback_no_demographics");
    };

    let mut max_remaining: Option<i32> = None;
    let mut any_birth = false;
    for bd in birth_dates {
        let Some(birth) = *bd else {
            continue;
        };
        any_birth = true;
        let age = age_completed_years(today, birth);
        let rem = (target - age).max(0);
        max_remaining = Some(max_remaining.map_or(rem, |m| m.max(rem)));
    }

    if !any_birth {
        return (FALLBACK_YEARS * 12, "mac_fallback_no_demographics");
    }

    let years_raw = max_remaining.unwrap_or(0).max(0) as u32;
    let clamped_years = years_raw.clamp(MIN_YEARS, MAX_YEARS);
    (clamped_years * 12, "mac_target_age")
}

fn map_engine_err(e: EngineError) -> ApiError {
    ApiError::BadRequest(e.to_string())
}

pub(crate) async fn build_installation_projection_input(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    horizon_months: u32,
    inflation_annual_percent: Option<Decimal>,
) -> Result<(ProjectionInput, Decimal), ApiError> {
    let (income_reg, expense_reg) =
        ledger_regular_monthly_income_and_expense(pool, iid, session_user_id, view, today).await?;
    let monthly_net_regular = income_reg - expense_reg;

    let assets_rows: Vec<AssetEngineRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT id, current_value, purchase_price, is_liquid,
                          expected_annual_return_percent,
                          monthly_contribution_fixed, contribution_frequency,
                          contribution_remainder_weight
                   FROM assets
                   WHERE installation_id = $1
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT id, current_value, purchase_price, is_liquid,
                          expected_annual_return_percent,
                          monthly_contribution_fixed, contribution_frequency,
                          contribution_remainder_weight
                   FROM assets
                   WHERE installation_id = $1 AND owner_user_id = $2
                   ORDER BY sort_index ASC, name ASC"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let liabs: Vec<LiabEngineRow> = match view {
        LedgerView::Household => {
            sqlx::query_as(
                r#"SELECT principal, payment_amount, payment_frequency, payment_end_date
                   FROM liabilities WHERE installation_id = $1"#,
            )
            .bind(iid)
            .fetch_all(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_as(
                r#"SELECT principal, payment_amount, payment_frequency, payment_end_date
                   FROM liabilities
                   WHERE installation_id = $1 AND owner_user_id = $2"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_all(pool)
            .await?
        }
    };

    let assets: Vec<SimAsset> = assets_rows
        .into_iter()
        .map(|r| SimAsset {
            id: r.id,
            value: r.current_value,
            purchase_price: r.purchase_price,
            is_liquid: r.is_liquid,
            expected_annual_return_percent: r.expected_annual_return_percent,
            monthly_contribution_fixed: asset_fixed_monthly_equivalent(
                r.monthly_contribution_fixed.max(Decimal::ZERO),
                r.contribution_frequency.as_str(),
            ),
            contribution_remainder_weight: r.contribution_remainder_weight.max(Decimal::ZERO),
        })
        .collect();

    let liabilities: Vec<ProjectionLiabilityInput> = liabs
        .into_iter()
        .map(|r| ProjectionLiabilityInput {
            principal: r.principal.max(Decimal::ZERO),
            monthly_payment: liability_monthly_payment(&r),
            payment_end: r.payment_end_date,
        })
        .collect();

    let input = ProjectionInput {
        ref_date: today,
        horizon_months,
        income_regular_monthly: income_reg,
        expense_regular_monthly: expense_reg,
        assets,
        liabilities,
        inflation_annual_percent,
    };

    Ok((input, monthly_net_regular))
}

/// Map asset id → nominal € routed in month 1 (fixed escalado + parte del remanente). Vista alineada al listado de activos / proyección.
pub(crate) async fn first_month_asset_contribution_nominals_map(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<HashMap<Uuid, Decimal>, ApiError> {
    let (input, _) =
        build_installation_projection_input(pool, iid, session_user_id, view, today, 1, None)
            .await?;
    let nominals = first_month_per_asset_contribution_nominals(&input);
    Ok(input
        .assets
        .iter()
        .zip(nominals.into_iter())
        .map(|(a, n)| (a.id, n))
        .collect())
}

/// Suma de precios de compra (>0) en activos incluidos en la vista de proyección.
async fn sum_assets_purchase_basis(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
) -> Result<Decimal, ApiError> {
    let v: Decimal = match view {
        LedgerView::Household => {
            sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(purchase_price), 0)
                   FROM assets
                   WHERE installation_id = $1
                     AND purchase_price IS NOT NULL
                     AND purchase_price > 0"#,
            )
            .bind(iid)
            .fetch_one(pool)
            .await?
        }
        LedgerView::Mine => {
            sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(purchase_price), 0)
                   FROM assets
                   WHERE installation_id = $1
                     AND owner_user_id = $2
                     AND purchase_price IS NOT NULL
                     AND purchase_price > 0"#,
            )
            .bind(iid)
            .bind(session_user_id)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(v)
}

/// Alinea la serie de capital aportado con la base de compras en BD si el motor devolviera un mes 0 menor (p. ej. binario antiguo).
fn bump_contributed_series_with_purchase_basis(
    contributed: &mut Vec<Decimal>,
    basis_sum: Decimal,
) {
    if contributed.is_empty() || basis_sum <= Decimal::ZERO {
        return;
    }
    let first = contributed[0];
    let delta = basis_sum - first;
    if delta > Decimal::ZERO {
        for cc in contributed.iter_mut() {
            *cc += delta;
        }
    }
}

/// Runs the dossier-style projection for the installation ledger view.
pub(crate) async fn compute_installation_projection(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    horizon_months: u32,
    inflation_annual_percent: Option<Decimal>,
) -> Result<(ProjectionOutput, Decimal), ApiError> {
    let (input, monthly_net_regular) = build_installation_projection_input(
        pool,
        iid,
        session_user_id,
        view,
        today,
        horizon_months,
        inflation_annual_percent,
    )
    .await?;
    let out = project_net_worth_series(&input).map_err(map_engine_err)?;
    Ok((out, monthly_net_regular))
}

#[utoipa::path(
    get,
    path = "/v1/projection/series",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` = vista titular"),
        ("months" = Option<u32>, Query, description = "Horizonte en meses (12–840); omitir = edad objetivo + DOB (Mac), 5–70 años o fallback 30"),
    ),
    responses(
        (status = 200, description = "Serie mensual motor dossier", body = ProjectionSeriesResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_projection_series(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ProjectionSeriesQuery>,
) -> Result<Json<ProjectionSeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    purge_expired_liabilities(&state.pool, iid).await?;
    let today = installation_naive_today(&state.pool, iid).await?;
    let view = resolve_ledger_view(&q);

    let inst_row: (
        bool,
        Option<Decimal>,
        Option<i16>,
        String,
    ) = sqlx::query_as(
        r#"SELECT projection_includes_inflation, annual_inflation_assumption_percent,
                  projection_target_age, show_age_mode
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    let inflation_annual_percent =
        if inst_row.0 && inst_row.1.is_some() {
            inst_row.1
        } else {
            None
        };

    let session_birth: Option<NaiveDate> = sqlx::query_scalar(
        r#"SELECT birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    // Primera DOB en personas del hogar (primario primero), si existe.
    let household_member_birth: Option<NaiveDate> = sqlx::query_scalar(
        r#"SELECT birth_date FROM persons
           WHERE installation_id = $1 AND birth_date IS NOT NULL
           ORDER BY is_primary DESC, sort_index ASC
           LIMIT 1"#,
    )
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let resolved_birth_for_demographics = session_birth.or(household_member_birth);

    let birth_dates: Vec<Option<NaiveDate>> = vec![resolved_birth_for_demographics];

    let (months, horizon_basis): (u32, String) = match q.months {
        Some(m) => (m.clamp(12, 840), "months_override".into()),
        None => {
            let (m, b) = mac_projection_horizon_months(today, inst_row.2, &birth_dates);
            (m, b.into())
        }
    };

    let horizon_years = months / 12;

    let (mut output, monthly_delta_assumption) = compute_installation_projection(
        &state.pool,
        iid,
        user.id.0,
        view,
        today,
        months,
        inflation_annual_percent,
    )
    .await?;

    let purchase_basis = sum_assets_purchase_basis(&state.pool, iid, user.id.0, view).await?;
    bump_contributed_series_with_purchase_basis(&mut output.contributed_capital, purchase_basis);

    let starting_net_worth = output
        .net_worth
        .first()
        .copied()
        .unwrap_or(Decimal::ZERO);

    let points: Vec<ProjectionPoint> = output
        .net_worth
        .iter()
        .zip(output.contributed_capital.iter())
        .enumerate()
        .map(|(i, (nw, cc))| ProjectionPoint {
            month_index: i as u32,
            net_worth: *nw,
            contributed_capital: *cc,
        })
        .collect();

    Ok(Json(ProjectionSeriesResponse {
        points,
        months,
        horizon_years,
        horizon_basis,
        starting_net_worth,
        monthly_delta_assumption,
        model_note:
            "Motor mensual: presupuesto regular sin cuotas derivadas de pasivos, servicio de deuda activo por mes, aportaciones solo desde ese superávit recurrente (los Próximos no cuentan), drenaje líquidos primero y menor rentabilidad esperada, cuotas fijas escaladas y remanente por pesos, crecimiento compuesto por activo, serie nominal o deflactada si hay inflación."
                .into(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        show_age_mode: inst_row.3.clone(),
        use_age_on_x_axis: inst_row.3.trim() == "ages"
            && resolved_birth_for_demographics.is_some(),
        viewer_birth_date: resolved_birth_for_demographics
            .map(|d| d.format("%Y-%m-%d").to_string()),
    }))
}

pub fn projection_router() -> Router {
    Router::new().route("/series", get(get_projection_series))
}

#[cfg(test)]
mod horizon_tests {
    use super::*;
    #[test]
    fn mac_horizon_fallback_without_target_age() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap())];
        let (m, basis) = mac_projection_horizon_months(today, None, &bd);
        assert_eq!(m, 30 * 12);
        assert_eq!(basis, "mac_fallback_no_demographics");
    }

    #[test]
    fn mac_horizon_uses_max_years_to_target_clamped() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![
            Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap()),
            Some(NaiveDate::from_ymd_opt(1985, 1, 1).unwrap()),
        ];
        let (m, basis) = mac_projection_horizon_months(today, Some(65), &bd);
        // ages 36 and 41 → 29 y 24 años hasta 65 → máximo 29
        assert_eq!(basis, "mac_target_age");
        assert_eq!(m, 29 * 12);
    }

    #[test]
    fn mac_horizon_minimum_five_years_when_already_near_target() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![Some(NaiveDate::from_ymd_opt(1965, 1, 1).unwrap())];
        let (m, basis) = mac_projection_horizon_months(today, Some(65), &bd);
        assert_eq!(basis, "mac_target_age");
        assert_eq!(m, 5 * 12);
    }
}
