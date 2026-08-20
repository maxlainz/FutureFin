//! Monthly projection via `futurefin-engine`: presupuesto regular, cuotas de pasivos activos,
//! aportaciones a activos / drenaje / crecimiento. Los «Próximos» ajustan la caja por mes (fechas
//! explícitas en su mes civil; sin fecha repartidas en 90 días desde la fecha de referencia).

use crate::error::ApiError;
use crate::handlers::budget::ledger_regular_monthly_income_and_expense;
use crate::handlers::installation::{
    load_fire_settings, naive_date_in_calendar_tz, require_installation_member,
    resolve_fire_settings, FireNumberMode, FireSettings, SavingsSource, TaxBracket,
};
use crate::handlers::person_view::LedgerView;
use crate::handlers::transactions::summary::transactions_12m_avg;
use crate::handlers::session::require_session_user;
use crate::state::{AppState, Density, ProjectionCacheKey};
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, Duration, Months, NaiveDate};
use futurefin_engine::{
    fire_target_at_month_index, first_month_per_asset_contribution_nominals,
    project_net_worth_series, AllocationCap, AllocationKind, AllocationRule, EngineError,
    FireTarget, ProjectionInput, ProjectionLiabilityInput, SimAsset,
};
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
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
    /// Meses a proyectar (12–840). Si se omite: horizonte derivado de la instalación (véase `horizon_basis`).
    pub months: Option<u32>,
    /// `monthly` (default) o `hybrid` (mes 0..12 mensual + anual desde 24). Reduce el JSON ~5× con `hybrid`.
    #[serde(default)]
    pub density: Option<String>,
}

fn resolve_density(q: &ProjectionSeriesQuery) -> Density {
    match q.density.as_deref().map(str::trim) {
        Some("hybrid") => Density::Hybrid,
        _ => Density::Monthly,
    }
}

/// Indices a incluir en el response según la densidad. Para `Hybrid`: mes 0..12
/// mensual + mes 24, 36, ..., months.
fn density_month_indices(density: Density, months: u32) -> Vec<u32> {
    match density {
        Density::Monthly => (0..months).collect(),
        Density::Hybrid => {
            let cap = months.saturating_sub(1);
            let mut v: Vec<u32> = (0..=12u32.min(cap)).collect();
            let mut k = 24u32;
            while k <= cap {
                v.push(k);
                k += 12;
            }
            v
        }
    }
}

#[cfg(test)]
fn tax_on_gross_capital_annual(gross: Decimal, brackets: &[TaxBracket]) -> Decimal {
    if gross <= Decimal::ZERO || brackets.is_empty() {
        return Decimal::ZERO;
    }
    let mut prev_ceiling = Decimal::ZERO;
    let mut tax = Decimal::ZERO;
    for b in brackets {
        let r = b.pct / Decimal::from(100u32);
        match b.up_to {
            None => {
                let taxable = (gross - prev_ceiling).max(Decimal::ZERO);
                tax += taxable * r;
                break;
            }
            Some(ceiling) => {
                let slice_end = gross.min(ceiling);
                let taxable = (slice_end - prev_ceiling).max(Decimal::ZERO);
                tax += taxable * r;
                prev_ceiling = ceiling;
                if gross <= ceiling {
                    break;
                }
            }
        }
    }
    tax
}

/// Devuelve el `gross` tal que `gross − tax(gross) == net_annual`, sin búsqueda binaria.
///
/// La función `tax(·)` es lineal por tramos: dentro del tramo i con tipo `r_i` y umbral
/// inferior `prev_i`, `after(g) = g·(1 − r_i) + (r_i·prev_i − K_i)`, donde `K_i` es el impuesto
/// acumulado de los tramos anteriores. Despejando `g = (net − r_i·prev_i + K_i) / (1 − r_i)` se
/// obtiene un candidato; si cae dentro del tramo (≤ `ceiling_i`), es la solución; si no, se
/// avanza al siguiente y se actualiza `K_i`.
pub(crate) fn gross_up_net_annual_fire(net_annual: Decimal, brackets: &[TaxBracket], taxes_enabled: bool) -> Decimal {
    if !taxes_enabled || net_annual <= Decimal::ZERO {
        return net_annual.max(Decimal::ZERO);
    }
    let hundred = Decimal::from(100u32);
    let mut prev_ceiling = Decimal::ZERO;
    let mut k_cumulative = Decimal::ZERO;
    for b in brackets {
        let r = b.pct / hundred;
        let denom = Decimal::ONE - r;
        if denom <= Decimal::ZERO {
            // Tipo del 100% (o superior): imposible recuperar `net` positivo; degeneración.
            return prev_ceiling;
        }
        let gross = (net_annual + k_cumulative - r * prev_ceiling) / denom;
        match b.up_to {
            None => return gross,
            Some(ceiling) => {
                if gross <= ceiling {
                    return gross;
                }
                let width = ceiling - prev_ceiling;
                k_cumulative += r * width;
                prev_ceiling = ceiling;
            }
        }
    }
    // Inalcanzable: `validate_tax_brackets` exige que el último tramo tenga `up_to = None`.
    net_annual
}

fn compute_fire_target_nw(
    fire: &FireSettings,
    income_monthly: Decimal,
    income_retirement_monthly: Decimal,
    expense_monthly: Decimal,
) -> Option<Decimal> {
    let need_annual = match fire.fire_number_mode {
        FireNumberMode::Manual => {
            let amt = fire.fire_number_manual_amount?;
            if amt <= Decimal::ZERO { return None; }
            amt
        }
        FireNumberMode::AnnualExpense => {
            let net = expense_monthly - income_retirement_monthly;
            if net <= Decimal::ZERO { return None; }
            net * Decimal::from(12u32)
        }
        FireNumberMode::CurrentIncome => {
            let net = income_monthly - income_retirement_monthly;
            if net <= Decimal::ZERO { return None; }
            net * Decimal::from(12u32)
        }
    };
    let swr = fire.swr_pct;
    if swr <= Decimal::ZERO { return None; }
    let gross = gross_up_net_annual_fire(need_annual, &fire.tax_brackets, fire.taxes_enabled);
    Some(gross / (swr / Decimal::from(100u32)))
}

fn resolve_ledger_view(q: &ProjectionSeriesQuery) -> LedgerView {
    match q.view.as_deref().map(str::trim) {
        Some("mine") => LedgerView::Mine,
        _ => LedgerView::Household,
    }
}

/// Serializa un Decimal como f64 (~15 dígitos de precisión, suficiente para
/// display de horizontes de 70 años). Reduce ~30 KB JSON y elimina ~5.000
/// llamadas a parseDisplayDecimal en el cliente. Los KPIs/totales escalares
/// que requieren precisión decimal siguen usando `rust_decimal::serde::str`.
/// `pub(crate)`: la ÚNICA otra superficie autorizada a usarla son los arrays
/// por punto de `/v1/history/series` (`handlers/history.rs`) — misma
/// justificación chart-only (excepción D4/I3 del architecture contract).
pub(crate) fn serialize_decimal_as_f64<S: serde::Serializer>(d: &Decimal, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(d.to_f64().unwrap_or(0.0))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionPoint {
    pub month_index: u32,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub contributed_capital: Decimal,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AssetSeries {
    pub asset_id: Uuid,
    pub asset_name: String,
    /// Decimal values serializados como f64 (paralelo a `points`).
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionSeriesResponse {
    pub points: Vec<ProjectionPoint>,
    pub months: u32,
    /// Años de horizonte efectivos (`months / 12`).
    pub horizon_years: u32,
    /// `lifespan_90` | `fallback_no_demographics` | `months_override`
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
    /// Próximos hitos de patrimonio en euros **nominales**: umbrales 1/2.5/5*10^n, deduplicados por año.
    /// La web los usa cuando el toggle «Inflation Adjusted» está apagado.
    pub milestones: Vec<ProjectionMilestone>,
    /// Mismos umbrales que `milestones` pero cruzados sobre el patrimonio **deflactado** (euros de
    /// hoy): el hito de 1.000.000 € se alcanza cuando el patrimonio nominal vale 1.000.000 € *en
    /// poder adquisitivo de hoy*, no en euros del futuro. La web los usa cuando el toggle
    /// «Inflation Adjusted» está encendido. Vacío cuando la inflación es 0 (coincide con `milestones`).
    pub milestones_real: Vec<ProjectionMilestone>,
    /// Primer mes en que el componente de interés/mercado supera el ahorro mensual base (sin Próximos ni plan de pagos de deudas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compound_outpaces_true_savings_month_index: Option<u32>,
    /// Primer mes en que el patrimonio neto ≥ objetivo FIRE móvil del mes en curso. `null` si no hay objetivo o no se alcanza.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jubilacion_month_index: Option<u32>,
    /// Objetivo FIRE base en euros de hoy (gross-up de impuestos aplicado). Sirve como referencia
    /// y como anclaje del target móvil — el target en cada mes es este valor × `(1 + inflación%)^(meses/12)`.
    /// `null` cuando no hay configuración FIRE válida.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jubilacion_target_net_worth: Option<String>,
    /// Serie mensual del target FIRE ajustado por inflación, paralela a `points`. Cada valor =
    /// `target_base × (1 + inflación%)^(month_index/12)`. Vacío cuando no hay FIRE configurado.
    /// Serializado como f64 (ver `serialize_decimal_as_f64`).
    pub fire_target_series: Vec<f64>,
    /// Valor de cada activo mes a mes (paralelo a `points`). Un elemento por activo, en el mismo orden que la consulta de activos.
    pub asset_series: Vec<AssetSeries>,
    /// Densidad de los puntos serializados: `monthly` (default) o `hybrid`.
    pub density: String,
    /// Fuente del ahorro **efectiva** que produjo `monthly_delta_assumption` (tras el fallback: en
    /// modo `transactions_avg` / `budget_income_real_expense` sin meses reales cae a `budget`).
    /// Mismo naming y semántica que el campo homónimo de `/v1/summary`.
    pub savings_source: SavingsSource,
    /// Meses reales usados por el promedio cuando `savings_source` deriva de transacciones; `0` en
    /// modo `budget` (configurado o por fallback).
    pub savings_source_months_with_data: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionMilestone {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub target: Decimal,
    pub reached_month_index: u32,
    pub reached_date_ymd: String,
}

#[derive(Debug, FromRow)]
struct AssetEngineRow {
    id: Uuid,
    name: String,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    expected_annual_return_percent: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct AllocationRuleEngineRow {
    target_asset_id: Uuid,
    kind: String,
    amount: Option<Decimal>,
    cap_kind: Option<String>,
    cap_value: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct LiabEngineRow {
    principal: Decimal,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
pub(crate) struct PlanningFlowProjRow {
    pub scope: String,
    pub expected_amount: Decimal,
    pub due_date: Option<NaiveDate>,
}

/// Días civiles: reparto equitativo del total entre `ref_date` y `ref_date + 89` (90 días inclusive).
const PLANNING_UNDATED_SPREAD_DAYS: i64 = 90;
const PROJECTION_MILESTONE_MINIMUM: i64 = 1_000;
const PROJECTION_MILESTONE_SEARCH_COUNT: usize = 64;
const PROJECTION_MILESTONE_LIMIT: usize = 3;

fn proj_month_first(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

fn proj_add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n)).unwrap_or(d)
}

fn proj_month_last(m_first: NaiveDate) -> NaiveDate {
    proj_add_months(m_first, 1)
        .pred_opt()
        .unwrap_or(m_first)
}

fn overlap_inclusive_days(
    a_start: NaiveDate,
    a_end: NaiveDate,
    b_start: NaiveDate,
    b_end: NaiveDate,
) -> i64 {
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    if start > end {
        return 0;
    }
    end.signed_duration_since(start).num_days() + 1
}

fn planning_monthly_cash_adjustments_from_flows(
    ref_date: NaiveDate,
    horizon_months: u32,
    flows: &[PlanningFlowProjRow],
) -> Vec<Decimal> {
    let mut adj = vec![Decimal::ZERO; horizon_months as usize];
    let anchor_month_first = proj_month_first(ref_date);

    let undated_win_first = ref_date;
    let undated_win_last = ref_date
        .checked_add_signed(Duration::days(PLANNING_UNDATED_SPREAD_DAYS - 1))
        .unwrap_or(ref_date);

    for flow in flows {
        let signed = match flow.scope.as_str() {
            "income" => flow.expected_amount,
            "expense" => -flow.expected_amount,
            _ => continue,
        };

        match flow.due_date {
            Some(due) => {
                if due < anchor_month_first {
                    continue;
                }
                for idx in 0..horizon_months as usize {
                    let m_first = proj_add_months(anchor_month_first, idx as u32);
                    let m_last = proj_month_last(m_first);
                    if due >= m_first && due <= m_last {
                        adj[idx] += signed;
                        break;
                    }
                }
            }
            None => {
                let daily = signed / Decimal::from(PLANNING_UNDATED_SPREAD_DAYS);
                for idx in 0..horizon_months as usize {
                    let m_first = proj_add_months(anchor_month_first, idx as u32);
                    let m_last = proj_month_last(m_first);
                    let days = overlap_inclusive_days(
                        m_first,
                        m_last,
                        undated_win_first,
                        undated_win_last,
                    );
                    if days > 0 {
                        adj[idx] += daily * Decimal::from(days);
                    }
                }
            }
        }
    }
    adj
}

fn expense_end_date_monthly_adjustments(
    today: NaiveDate,
    horizon_months: u32,
    entries: &[(Decimal, NaiveDate)],
) -> Vec<Decimal> {
    let mut adj = vec![Decimal::ZERO; horizon_months as usize];
    let anchor = proj_month_first(today);
    for (amount, end_date) in entries {
        for idx in 0..horizon_months as usize {
            let m_first = proj_add_months(anchor, idx as u32);
            if m_first > *end_date {
                // Cancel the expense from this month onwards (base rate already deducts it).
                for i in idx..horizon_months as usize {
                    adj[i] += amount;
                }
                break;
            }
        }
    }
    adj
}

fn planning_upcoming_net_for_milestone_baseline(
    ref_date: NaiveDate,
    flows: &[PlanningFlowProjRow],
) -> Decimal {
    let window_last = ref_date
        .checked_add_signed(Duration::days(PLANNING_UNDATED_SPREAD_DAYS - 1))
        .unwrap_or(ref_date);
    let mut baseline = Decimal::ZERO;
    for flow in flows {
        let signed = match flow.scope.as_str() {
            "income" => flow.expected_amount,
            "expense" => -flow.expected_amount,
            _ => continue,
        };
        match flow.due_date {
            Some(due) => {
                if due >= ref_date && due <= window_last {
                    baseline += signed;
                }
            }
            None => baseline += signed,
        }
    }
    baseline
}

fn projection_next_milestone(after: Decimal) -> Decimal {
    let steps = [Decimal::ONE, Decimal::new(25, 1), Decimal::from(5i64)];
    let minimum = Decimal::from(PROJECTION_MILESTONE_MINIMUM);
    let safe_value = after.max(minimum);
    let safe_f64 = safe_value.to_f64().unwrap_or(PROJECTION_MILESTONE_MINIMUM as f64);
    let power = safe_f64.log10().floor() as i32;
    let magnitude = Decimal::from(10i64).powi(power.into());
    for step in steps {
        let candidate = step * magnitude;
        if candidate > safe_value {
            return candidate;
        }
    }
    Decimal::from(10i64).powi((power + 1).into())
}

fn projection_next_milestones(from: Decimal, count: usize) -> Vec<Decimal> {
    let mut out = Vec::with_capacity(count);
    let mut current = from;
    for _ in 0..count {
        let next = projection_next_milestone(current);
        out.push(next);
        current = next;
    }
    out
}

/// Deflacta una serie de puntos a euros de hoy: el patrimonio del mes `k` se divide por
/// `(1 + inflación%)^(k/12)`. Es la versión a resolución mensual completa de la deflactación
/// **visual** que hace el chart de la web (`ProjectionNetWorthChart.baseSeries`); calcularla aquí
/// preserva la precisión del `reached_month_index` de los milestones bajo densidad `hybrid`, donde
/// el cliente solo recibe puntos anuales. Con inflación 0 devuelve una copia sin cambios.
fn deflate_points_to_today(
    points: &[ProjectionPoint],
    annual_inflation_percent: Decimal,
) -> Vec<ProjectionPoint> {
    if annual_inflation_percent <= Decimal::ZERO {
        return points.to_vec();
    }
    let infl_factor = Decimal::ONE + annual_inflation_percent / Decimal::from(100u32);
    points
        .iter()
        .map(|p| {
            let years = Decimal::from(p.month_index) / Decimal::from(12u32);
            let deflator = Decimal::ONE / infl_factor.powd(years);
            ProjectionPoint {
                month_index: p.month_index,
                net_worth: p.net_worth * deflator,
                contributed_capital: p.contributed_capital * deflator,
            }
        })
        .collect()
}

fn projection_unique_reached_milestones(
    points: &[ProjectionPoint],
    anchor_date: NaiveDate,
    baseline_adjustment: Decimal,
    limit: usize,
    search_count: usize,
) -> Vec<ProjectionMilestone> {
    if points.is_empty() || limit == 0 {
        return vec![];
    }
    let baseline = points[0].net_worth + baseline_adjustment;
    let generated = projection_next_milestones(baseline, search_count.max(limit));
    let mut events: Vec<ProjectionMilestone> = Vec::with_capacity(limit);
    let mut last_year: Option<i32> = None;

    for milestone in generated {
        let Some(reached_idx) = points.iter().position(|p| p.net_worth >= milestone) else {
            break;
        };
        let reached_month_index = points[reached_idx].month_index;
        let reached_date = proj_add_months(proj_month_first(anchor_date), reached_month_index);
        let event = ProjectionMilestone {
            target: milestone,
            reached_month_index,
            reached_date_ymd: reached_date.format("%Y-%m-%d").to_string(),
        };

        if let Some(prev_year) = last_year {
            if prev_year == reached_date.year() {
                let replace_index = events.len() - 1;
                events[replace_index] = event;
            } else {
                events.push(event);
                last_year = Some(reached_date.year());
            }
        } else {
            events.push(event);
            last_year = Some(reached_date.year());
        }

        if events.len() >= limit {
            break;
        }
    }

    events
}

fn compound_outpaces_true_savings_month(
    input: &ProjectionInput,
    monthly_delta_assumption: Decimal,
) -> Result<Option<u32>, EngineError> {
    if monthly_delta_assumption <= Decimal::ZERO {
        return Ok(None);
    }
    let mut neutral = input.clone();
    neutral.planning_monthly_cash_adjustment =
        vec![Decimal::ZERO; input.horizon_months as usize];
    for liab in neutral.liabilities.iter_mut() {
        liab.monthly_payment = Decimal::ZERO;
    }
    let out = project_net_worth_series(&neutral)?;
    let mut consecutive = 0u32;
    const REQUIRED_CONSECUTIVE_MONTHS: u32 = 3;
    for k in 1..out.net_worth.len() {
        let nw_delta = out.net_worth[k] - out.net_worth[k - 1];
        let savings_delta = out.contributed_capital[k] - out.contributed_capital[k - 1];
        if savings_delta <= Decimal::ZERO {
            consecutive = 0;
            continue;
        }
        let market_delta = nw_delta - savings_delta;
        if market_delta > savings_delta {
            consecutive += 1;
            if consecutive >= REQUIRED_CONSECUTIVE_MONTHS {
                return Ok(Some(k as u32 + 1 - REQUIRED_CONSECUTIVE_MONTHS));
            }
        } else {
            consecutive = 0;
        }
    }
    Ok(None)
}

/// Cuota mensual equivalente de un importe periódico: `weekly → ×52/12`, cualquier otra
/// frecuencia (incluida `monthly`) → el importe tal cual. Fuente única de esta conversión;
/// también la usa `history.rs` al construir [`futurefin_engine::LoanTerms`].
pub(crate) fn monthly_payment_from(amount: Decimal, frequency: Option<&str>) -> Decimal {
    match frequency {
        Some("weekly") => (amount * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amount,
    }
}

/// Cuota mensual equivalente de un pasivo a partir de sus campos crudos (`payment_amount`,
/// `payment_frequency`). Sin importe → `0`. Fuente única compartida por `projection.rs` (filas
/// `LiabEngineRow` ya cargadas para el engine) y `summary.rs` (filas de una SELECT dedicada).
pub(crate) fn liability_monthly_payment(
    amount: Option<Decimal>,
    frequency: Option<&str>,
) -> Decimal {
    match amount {
        Some(amt) => monthly_payment_from(amt, frequency),
        None => Decimal::ZERO,
    }
}

/// Predicado de "pasivo activo" (misma semántica que el `WHERE payment_end_date IS NULL OR >= today`
/// de las lecturas SQL): un pasivo sin fecha de fin de pago o cuya fecha aún no ha pasado sigue vivo.
pub(crate) fn liability_is_active(payment_end_date: Option<NaiveDate>, today: NaiveDate) -> bool {
    payment_end_date.map_or(true, |d| d >= today)
}

/// Completed calendar age in years (`today` inclusive), used for horizon.
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

/// Máximo años hasta los 90 años de edad por fecha de nacimiento; acotado [5, 70]; sin DOB → 30 años.
pub(crate) fn projection_horizon_months(
    today: NaiveDate,
    birth_dates: &[Option<NaiveDate>],
) -> (u32, &'static str) {
    const LIFESPAN_AGE: i32 = 90;
    const MIN_YEARS: u32 = 5;
    const MAX_YEARS: u32 = 70;
    const FALLBACK_YEARS: u32 = 30;

    let mut max_remaining: Option<i32> = None;
    let mut any_birth = false;
    for bd in birth_dates {
        let Some(birth) = *bd else {
            continue;
        };
        any_birth = true;
        let age = age_completed_years(today, birth);
        let rem = (LIFESPAN_AGE - age).max(0);
        max_remaining = Some(max_remaining.map_or(rem, |m| m.max(rem)));
    }

    if !any_birth {
        return (FALLBACK_YEARS * 12, "fallback_no_demographics");
    }

    let years_raw = max_remaining.unwrap_or(0).max(0) as u32;
    let clamped_years = years_raw.clamp(MIN_YEARS, MAX_YEARS);
    (clamped_years * 12, "lifespan_90")
}

fn map_engine_err(e: EngineError) -> ApiError {
    ApiError::BadRequest(e.to_string())
}

/// Primer mes cuyo patrimonio cruza el target FIRE (inflado mes a mes). Se evalúa sobre TODOS
/// los meses de la serie — nunca sobre puntos decimados — para no perder precisión. Compartido
/// por `compute_projection_series_response` y `simulate_projection_core`.
fn fire_crossover_month(
    ft: Option<&futurefin_engine::FireTarget>,
    net_worth: &[Decimal],
) -> Option<u32> {
    let ft = ft.filter(|f| f.base_amount > Decimal::ZERO)?;
    for (i, nw) in net_worth.iter().enumerate() {
        let target = fire_target_at_month_index(Some(ft), i as u32).unwrap_or(Decimal::ZERO);
        if target > Decimal::ZERO && *nw >= target {
            return Some(i as u32);
        }
    }
    None
}

pub(crate) struct BuiltProjection {
    pub input: ProjectionInput,
    pub monthly_net_regular: Decimal,
    /// `(id, name)` por activo en el mismo orden que `input.assets` — evita un segundo SELECT.
    pub asset_id_name: Vec<(Uuid, String)>,
    /// Flujos de planificación crudos (scope + amount + due_date) — los reusa el handler para
    /// calcular el baseline de milestones sin tener que volver a la BD.
    pub planning_rows: Vec<PlanningFlowProjRow>,
    /// Fuente del ahorro **efectiva** (tras el fallback: modo B/C sin meses reales → `budget`).
    /// Es la que produjo `input.income_regular_monthly` / `input.expense_regular_monthly`.
    pub effective_savings_source: SavingsSource,
    /// Meses reales que alimentaron el promedio cuando la fuente efectiva usa transacciones; `0` en
    /// modo A y en el fallback.
    pub savings_source_months_with_data: u32,
    /// Servicio de deuda mensual de los pasivos **activos** (`payment_end_date` nula o ≥ hoy), con
    /// la cuota nominal normalizada a mensual. No entra en `expense_regular_monthly` (el engine
    /// amortiza los pasivos aparte); lo consumen los caps `months_expense` de `assets.rs`. En modo
    /// real (B/C con datos) es **0 por contrato**: la cuota ya vive dentro del promedio de gasto.
    pub debt_service_monthly: Decimal,
}

/// Overrides what-if de `simulate_projection` que deben aplicarse DENTRO del ensamblado, en el
/// punto semántico correcto (antes de derivar target FIRE y bases de caps). Los overrides
/// post-build (ajustes de caja, one-off, tasas por activo) NO van aquí: se aplican mutando el
/// `ProjectionInput` clonado (patrón `compound_outpaces_true_savings_month`).
#[derive(Debug, Default, Clone)]
pub(crate) struct SimOverrides {
    /// Gasto mensual extra «real»: se suma al gasto efectivo y al gasto de jubilación ANTES de
    /// derivar el target FIRE y las bases de caps — mueve el objetivo en todos los modos.
    pub extra_monthly_expense: Decimal,
    /// Gasto MENSUAL de jubilación (el caller ya dividió el anual entre 12): sustituye por
    /// completo la base de gasto del target FIRE y el gasto post-jubilación (gana sobre
    /// `extra_monthly_expense` en ese tramo).
    pub retirement_monthly_expense: Option<Decimal>,
}

pub(crate) async fn build_installation_projection_input(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    horizon_months: u32,
    inflation_annual_percent: Decimal,
    fire_settings: Option<&FireSettings>,
    overrides: Option<&SimOverrides>,
) -> Result<BuiltProjection, ApiError> {
    let (income_reg, income_retirement, expense_reg, expense_retirement, expense_end_entries) =
        ledger_regular_monthly_income_and_expense(pool, iid, session_user_id, view, today).await?;

    // Liabilities: una sola carga para todos los consumidores (debt service de modo A, caps de
    // assets.rs y el input del engine). En modo real (B/C) se les anula la cuota más abajo.
    // Solo pasivos ACTIVOS (mismo predicado que /v1/summary y /v1/liabilities): hasta la 3.4.0
    // esta query no filtraba y el principal de un pasivo ya vencido seguía restando net worth en
    // toda la serie — `projection.starting_net_worth` divergía de `summary.net_worth` (contra el
    // contrato D5/I5 de la arquitectura).
    let liab_scope = view.scope_where("");
    let liab_today_ph = view.next_arg_index();
    let liab_sql = format!(
        r#"SELECT principal, payment_amount, payment_frequency, payment_end_date
           FROM liabilities
           WHERE {liab_scope}
             AND (payment_end_date IS NULL OR payment_end_date >= ${liab_today_ph})"#
    );
    let mut liabs: Vec<LiabEngineRow> = view
        .bind_scope_as(sqlx::query_as(&liab_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    // Fuente del ahorro: presupuesto (modo A) vs modos que derivan de las transacciones (modo B
    // `transactions_avg`, modo C `budget_income_real_expense`). Ambos toman el gasto del promedio real
    // 12m CRUDO (las cuotas de pasivo viven dentro) y anulan `end_adj`; difieren solo en el income
    // (B → promedio real; C → presupuesto). `months_with_data == 0` → fallback silencioso a modo A.
    let savings_source = fire_settings
        .map(|fs| fs.savings_source)
        .unwrap_or_default();
    // Inputs regulares efectivos del mes 0. Con `expense_from_avg == false` (modo A o fallback) son
    // EXACTAMENTE los escalares del presupuesto (income_reg, expense_reg); en modo B/C con datos, el
    // gasto sale del promedio real (y en B también el income).
    struct EffectiveInputs {
        income: Decimal,
        expense: Decimal,
        expense_from_avg: bool,
        /// Fuente efectiva tras el fallback (modo B/C sin meses reales → `Budget`).
        effective_source: SavingsSource,
        /// Meses reales del promedio; `0` en modo A y en el fallback.
        months_with_data: u32,
    }
    let inputs: EffectiveInputs = match savings_source {
        SavingsSource::TransactionsAvg | SavingsSource::BudgetIncomeRealExpense => {
            let avg = transactions_12m_avg(pool, iid, session_user_id, view, today).await?;
            if avg.months_with_data > 0 {
                // Contrato de los modos reales (reforma 3.4.0): el promedio de gasto se usa CRUDO —
                // las cuotas de pasivo ya viven dentro de los movimientos (amortización incluida),
                // así que no se restan aquí ni se vuelven a cargar como debt service (ver el bloque
                // que anula `payment_amount` más abajo). Mismo contrato en `summary.rs`.
                // Modo C: income del presupuesto; modo B: income del promedio real.
                let income = match savings_source {
                    SavingsSource::BudgetIncomeRealExpense => income_reg,
                    _ => avg.income_avg,
                };
                EffectiveInputs {
                    income,
                    expense: avg.expense_avg,
                    expense_from_avg: true,
                    effective_source: savings_source,
                    months_with_data: avg.months_with_data,
                }
            } else {
                EffectiveInputs {
                    income: income_reg,
                    expense: expense_reg,
                    expense_from_avg: false,
                    effective_source: SavingsSource::Budget,
                    months_with_data: 0,
                }
            }
        }
        SavingsSource::Budget => EffectiveInputs {
            income: income_reg,
            expense: expense_reg,
            expense_from_avg: false,
            effective_source: SavingsSource::Budget,
            months_with_data: 0,
        },
    };
    // Overrides what-if pre-target: el gasto extra entra ANTES de derivar el target FIRE y las
    // bases de caps (semántica «gasto real»); el gasto de jubilación explícito sustituye al
    // derivado. Sin overrides (`None`, todos los callers no-simulación) nada cambia.
    let ov = overrides.cloned().unwrap_or_default();
    let mut inputs = inputs;
    inputs.expense += ov.extra_monthly_expense;
    let expense_retirement = match ov.retirement_monthly_expense {
        Some(v) => v,
        None => expense_retirement + ov.extra_monthly_expense,
    };

    let expense_from_avg = inputs.expense_from_avg;
    let effective_savings_source = inputs.effective_source;
    let savings_source_months_with_data = inputs.months_with_data;

    // Modo real (B/C con datos): la cuota ya está dentro del promedio de gasto, así que los
    // pasivos NO tocan la caja de la simulación. Se anula `payment_amount` EN MEMORIA en un único
    // punto y todo lo que cuelga de las filas hereda el contrato: `debt_service_monthly` queda a 0
    // (caps `months_expense` sin cuota), y el input del engine lleva `monthly_payment = 0` — con lo
    // que el engine ni cobra caja ni amortiza, y el principal queda como **resta constante** del
    // net worth en todo el horizonte (decisión de producto 3.4.0: sin amortización proyectada; la
    // realidad entra en cada recomputación vía promedio y principal actualizados).
    if expense_from_avg {
        for row in &mut liabs {
            row.payment_amount = None;
        }
    }

    // Servicio de deuda mensual de los pasivos activos (mismo filtro `payment_end_date` que las
    // lecturas SQL). No es un input del engine (que amortiza los pasivos por su cuenta): lo exporta
    // `BuiltProjection` para los caps `months_expense` de `assets.rs`. En modo real es 0 por el
    // bloque anterior.
    let debt_service_monthly: Decimal = liabs
        .iter()
        .filter(|r| liability_is_active(r.payment_end_date, today))
        .map(|r| liability_monthly_payment(r.payment_amount, r.payment_frequency.as_deref()))
        .filter(|p| *p > Decimal::ZERO)
        .sum();

    let monthly_net_regular = inputs.income - inputs.expense;

    // Base del FIRE number: income = income efectivo (modo C → presupuesto; modo B → promedio real);
    // expense = gasto efectivo en modo B/C (el gasto ya no es el del presupuesto), o
    // `expense_retirement` en modo A (comportamiento histórico).
    let fire_target_base = fire_settings.and_then(|fs| {
        // El override explícito de gasto de jubilación ancla el target en TODOS los modos; sin
        // él, la base histórica (gasto efectivo en B/C, gasto de jubilación en A).
        let fire_expense = match ov.retirement_monthly_expense {
            Some(v) => v,
            None if inputs.expense_from_avg => inputs.expense,
            None => expense_retirement,
        };
        compute_fire_target_nw(fs, inputs.income, income_retirement, fire_expense)
    });
    let fire_target = fire_target_base.map(|base_amount| FireTarget {
        base_amount,
        annual_inflation_percent: inflation_annual_percent.max(Decimal::ZERO),
    });

    let assets_scope = view.scope_where("");
    let assets_sql = format!(
        r#"SELECT id, name, current_value, purchase_price, is_liquid,
                  expected_annual_return_percent
           FROM assets
           WHERE {assets_scope}
           ORDER BY sort_index ASC, name ASC"#
    );
    let assets_rows: Vec<AssetEngineRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let alloc_scope = view.scope_where("");
    let alloc_sql = format!(
        r#"SELECT target_asset_id, kind, amount, cap_kind, cap_value
           FROM allocation_rules
           WHERE {alloc_scope} AND enabled = true
           ORDER BY priority ASC, id ASC"#
    );
    let alloc_rows: Vec<AllocationRuleEngineRow> = view
        .bind_scope_as(sqlx::query_as(&alloc_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let plan_scope = view.scope_where("p");
    let plan_sql = format!(
        r#"SELECT c.scope AS scope, p.expected_amount, p.due_date
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {plan_scope}"#
    );
    let planning_rows: Vec<PlanningFlowProjRow> = view
        .bind_scope_as(sqlx::query_as(&plan_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let flow_adj =
        planning_monthly_cash_adjustments_from_flows(today, horizon_months, &planning_rows);
    // Modo B/C con datos: el gasto ya no viene del presupuesto → los ajustes por end-date de partidas
    // de presupuesto se anulan (los `planning_flows` son ortogonales y se mantienen).
    let end_adj = if expense_from_avg {
        vec![Decimal::ZERO; horizon_months as usize]
    } else {
        expense_end_date_monthly_adjustments(today, horizon_months, &expense_end_entries)
    };
    let planning_monthly_cash_adjustment: Vec<Decimal> = flow_adj
        .iter()
        .zip(end_adj.iter())
        .map(|(a, b)| a + b)
        .collect();

    let mut asset_id_name: Vec<(Uuid, String)> = Vec::with_capacity(assets_rows.len());
    let assets: Vec<SimAsset> = assets_rows
        .into_iter()
        .map(|r| {
            asset_id_name.push((r.id, r.name));
            SimAsset {
                id: r.id,
                value: r.current_value,
                purchase_price: r.purchase_price,
                is_liquid: r.is_liquid,
                expected_annual_return_percent: r.expected_annual_return_percent,
            }
        })
        .collect();

    // Build allocation rules in priority order; resolve target_asset_id → index in assets[].
    let asset_index_by_id: HashMap<Uuid, usize> = assets
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();
    let allocation_rules: Vec<AllocationRule> = alloc_rows
        .into_iter()
        .filter_map(|r| {
            let target_index = *asset_index_by_id.get(&r.target_asset_id)?;
            let kind = match r.kind.as_str() {
                "fixed" => AllocationKind::Fixed,
                "percent" => AllocationKind::Percent,
                "remainder" => AllocationKind::Remainder,
                _ => return None,
            };
            let amount = r.amount;
            let cap = match (r.cap_kind.as_deref(), r.cap_value) {
                (Some("amount"), Some(v)) => Some(AllocationCap::Amount(v.max(Decimal::ZERO))),
                (Some("months_expense"), Some(v)) => {
                    Some(AllocationCap::MonthsExpense(v.max(Decimal::ZERO)))
                }
                (Some("income_multiple"), Some(v)) => {
                    Some(AllocationCap::IncomeMultiple(v.max(Decimal::ZERO)))
                }
                _ => None,
            };
            Some(AllocationRule {
                target_index,
                kind,
                amount,
                cap,
            })
        })
        .collect();

    let liabilities: Vec<ProjectionLiabilityInput> = liabs
        .into_iter()
        .map(|r| ProjectionLiabilityInput {
            principal: r.principal.max(Decimal::ZERO),
            monthly_payment: liability_monthly_payment(r.payment_amount, r.payment_frequency.as_deref()),
            payment_end: r.payment_end_date,
        })
        .collect();

    let input = ProjectionInput {
        ref_date: today,
        horizon_months,
        income_regular_monthly: inputs.income,
        expense_regular_monthly: inputs.expense,
        assets,
        allocation_rules,
        liabilities,
        planning_monthly_cash_adjustment,
        retirement_start_month: None,
        income_retirement_monthly: income_retirement,
        expense_retirement_monthly: expense_retirement,
        retirement_monthly_withdrawal: Decimal::ZERO,
        fire_target,
    };

    Ok(BuiltProjection {
        input,
        monthly_net_regular,
        asset_id_name,
        planning_rows,
        effective_savings_source,
        savings_source_months_with_data,
        debt_service_monthly,
    })
}

/// Todo lo que `assets.rs` necesita del motor para una vista, con **un solo**
/// `build_installation_projection_input` (horizonte 1 mes: basta para el reparto del mes 1 y para
/// los escalares del mes 0).
pub(crate) struct AssetsProjectionContext {
    /// Asset id → € nominales encaminados en el mes 1 (fixed escalado + parte del remanente).
    /// **Incluye el tramo de planning flows del mes en curso**, así que varía día a día.
    pub nominals: HashMap<Uuid, Decimal>,
    /// Asset id → € que la MISMA cascada encamina sobre el neto **recurrente**
    /// (`income − expense − debt_service`, sin el tramo de planning). Estable y reproducible.
    pub recurring_nominals: HashMap<Uuid, Decimal>,
    /// Income mensual **efectivo** del mes 0 (presupuesto en modo A/fallback; promedio real en B).
    pub income_monthly: Decimal,
    /// Gasto mensual **efectivo** + servicio de deuda de los pasivos activos.
    pub expense_with_debt: Decimal,
}

/// Contexto de proyección para el listado/edición de activos: aportación del primer mes por activo
/// más los escalares que resuelven los caps de las reglas de asignación.
///
/// Los caps `months_expense` / `income_multiple` se resuelven con **los mismos escalares que usa el
/// engine** (fix B4): en modo B/C con datos el ahorro sale del promedio real 12m, así que antes —
/// cuando estos escalares venían siempre del presupuesto — el objetivo mostrado en Activos no
/// casaba ni con la aportación del mes 1 ni con la simulación. El resto de campos de `fire_settings`
/// (p.ej. `fire_target`) no altera ni las aportaciones del mes 1 ni los escalares.
pub(crate) async fn assets_projection_context(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<AssetsProjectionContext, ApiError> {
    let fire_settings = load_fire_settings(pool, iid).await?;
    let built = build_installation_projection_input(
        pool,
        iid,
        session_user_id,
        view,
        today,
        1,
        Decimal::ZERO,
        Some(&fire_settings),
        None,
    )
    .await?;
    let nominals_vec =
        first_month_per_asset_contribution_nominals(&built.input).map_err(map_engine_err)?;
    let nominals: HashMap<Uuid, Decimal> = built
        .input
        .assets
        .iter()
        .zip(nominals_vec.into_iter())
        .map(|(a, n)| (a.id, n))
        .collect();

    // Segunda pasada de la MISMA cascada sobre el neto recurrente: se anula el ajuste de planning
    // del mes en curso y se vuelve a repartir. Reutilizar el engine (en vez de aproximar el
    // reparto a mano) es lo que garantiza que los caps y la precedencia sean idénticos; el coste
    // son microsegundos de aritmética Decimal, sin ningún SELECT extra.
    let mut recurring_input = built.input.clone();
    if let Some(first) = recurring_input.planning_monthly_cash_adjustment.first_mut() {
        *first = Decimal::ZERO;
    }
    let recurring_vec = first_month_per_asset_contribution_nominals(&recurring_input)
        .map_err(map_engine_err)?;
    let recurring_nominals: HashMap<Uuid, Decimal> = built
        .input
        .assets
        .iter()
        .zip(recurring_vec.into_iter())
        .map(|(a, n)| (a.id, n))
        .collect();

    Ok(AssetsProjectionContext {
        nominals,
        recurring_nominals,
        income_monthly: built.input.income_regular_monthly,
        // Misma semántica que el helper anterior (`expense_reg + debt_service`), pero con la base de
        // gasto EFECTIVA en vez de la del presupuesto.
        expense_with_debt: built.input.expense_regular_monthly + built.debt_service_monthly,
    })
}

#[utoipa::path(
    get,
    path = "/v1/projection/series",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` = vista titular"),
        ("months" = Option<u32>, Query, description = "Horizonte en meses (12–840); omitir = edad objetivo + DOB, 5–70 años o fallback 30"),
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

    let view = resolve_ledger_view(&q);
    let density = resolve_density(&q);
    let response =
        projection_series_cached(&state, user.id.0, iid, view, q.months, density).await?;
    Ok(Json(response))
}

/// Core sin HTTP con la política de cache intacta: lo comparten el handler GET y la
/// tool MCP `get_projection`. Cache hot path solo sin `months_override` (caso por
/// defecto y 99% del tráfico); con horizonte custom se recomputa sin cachear.
pub(crate) async fn projection_series_cached(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    view: LedgerView,
    months_override: Option<u32>,
    density: Density,
) -> Result<ProjectionSeriesResponse, ApiError> {
    if months_override.is_none() {
        let key = ProjectionCacheKey {
            installation_id: iid,
            view,
            owner_user_id: if matches!(view, LedgerView::Mine) {
                Some(user_id)
            } else {
                None
            },
            density,
        };
        if let Some(cached) = state.projection_cache_get(&key).await {
            tracing::info!(installation_id = %iid, view = ?view, density = ?density, "projection cache HIT");
            return Ok((*cached).clone());
        }
        tracing::info!(installation_id = %iid, view = ?view, density = ?density, "projection cache MISS, computing");
        let t0 = std::time::Instant::now();
        let response =
            compute_projection_series_response(state, user_id, iid, view, None, density).await?;
        tracing::info!(
            installation_id = %iid,
            density = ?density,
            ms = t0.elapsed().as_millis() as u64,
            "projection compute done, inserting in cache"
        );
        state
            .projection_cache_insert(key, Arc::new(response.clone()))
            .await;
        return Ok(response);
    }

    compute_projection_series_response(state, user_id, iid, view, months_override, density).await
}

/// Calcula la respuesta de proyección sin tocar el cache. Es la unidad de
/// recompute reusada por: (a) cache miss en el handler, (b) warm-up post-login,
/// (c) warm-up post-mutación. `density` solo afecta a la serialización (qué
/// puntos incluir en `points`/`fire_target_series`/`asset_series.values`);
/// el compute interno del engine siempre es el horizonte mensual completo.
/// Contexto resuelto de una proyección: `today` en la TZ de la instalación, inflación efectiva,
/// `show_age_mode`, `fire_settings` resueltos, DOB para demografía y la regla de horizonte.
/// Extraído de `compute_projection_series_response` para que la tool MCP `simulate_projection`
/// resuelva EXACTAMENTE el mismo contexto (mismas queries, misma regla de clamp) sin duplicarlo.
pub(crate) struct ProjectionContext {
    pub today: NaiveDate,
    pub inflation_annual_percent: Decimal,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
    /// DOB de demografía: la del usuario de la sesión, o la del primer miembro del hogar.
    pub birth_date: Option<NaiveDate>,
    pub months: u32,
    pub horizon_basis: String,
}

/// Resuelve el [`ProjectionContext`]. 1 query consolidada a `installation` (calendar_tz +
/// inflación + show_age_mode + fire_settings) y las DOB del usuario y del primer miembro del
/// hogar en paralelo con `try_join!`.
pub(crate) async fn resolve_projection_context(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    months_override: Option<u32>,
) -> Result<ProjectionContext, ApiError> {
    type InstallationRow = (
        String, // calendar_tz
        Decimal,
        String,
        Option<sqlx::types::Json<FireSettings>>,
    );
    let inst_q = sqlx::query_as::<_, InstallationRow>(
        r#"SELECT calendar_tz,
                  annual_inflation_assumption_percent,
                  show_age_mode,
                  fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool);
    let session_birth_q = sqlx::query_scalar::<_, Option<NaiveDate>>(
        r#"SELECT birth_date FROM users WHERE id = $1"#,
    )
    .bind(user_id)
    .fetch_one(pool);
    let household_birth_q = sqlx::query_scalar::<_, NaiveDate>(
        r#"SELECT birth_date FROM persons
           WHERE installation_id = $1 AND birth_date IS NOT NULL
           ORDER BY is_primary DESC, sort_index ASC
           LIMIT 1"#,
    )
    .bind(iid)
    .fetch_optional(pool);

    let (inst_row, session_birth, household_member_birth) =
        tokio::try_join!(inst_q, session_birth_q, household_birth_q)?;

    let today = naive_date_in_calendar_tz(&inst_row.0)?;
    let inflation_annual_percent = inst_row.1.max(Decimal::ZERO);
    let show_age_mode = inst_row.2;
    let fire_settings = resolve_fire_settings(inst_row.3.map(|j| j.0));

    let birth_date = session_birth.or(household_member_birth);
    let birth_dates: Vec<Option<NaiveDate>> = vec![birth_date];

    let (months, horizon_basis): (u32, String) = match months_override {
        Some(m) => (m.clamp(12, 840), "months_override".into()),
        None => {
            let (m, b) = projection_horizon_months(today, &birth_dates);
            (m, b.into())
        }
    };

    Ok(ProjectionContext {
        today,
        inflation_annual_percent,
        show_age_mode,
        fire_settings,
        birth_date,
        months,
        horizon_basis,
    })
}

pub async fn compute_projection_series_response(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    view: LedgerView,
    months_override: Option<u32>,
    density: Density,
) -> Result<ProjectionSeriesResponse, ApiError> {
    let ProjectionContext {
        today,
        inflation_annual_percent,
        show_age_mode,
        fire_settings,
        birth_date: resolved_birth_for_demographics,
        months,
        horizon_basis,
    } = resolve_projection_context(&state.pool, iid, user_id, months_override).await?;

    let horizon_years = months / 12;

    let built = build_installation_projection_input(
        &state.pool,
        iid,
        user_id,
        view,
        today,
        months,
        inflation_annual_percent,
        Some(&fire_settings),
        None,
    )
    .await?;
    let BuiltProjection {
        input: projection_input,
        monthly_net_regular: monthly_delta_assumption,
        asset_id_name,
        planning_rows,
        effective_savings_source,
        savings_source_months_with_data,
        debt_service_monthly: _,
    } = built;

    // Las dos simulaciones (principal + marker «compound supera ahorro») son CPU-bound y se
    // ejecutan en el pool blocking de Tokio. `tokio::join!` arranca ambas en paralelo, así que
    // un horizonte de 70 años con N activos no bloquea el reactor y aprovecha 2 cores.
    let main_input = projection_input.clone();
    let marker_input = projection_input.clone();
    let assumption = monthly_delta_assumption;
    let (main_join, marker_join) = tokio::join!(
        tokio::task::spawn_blocking(move || project_net_worth_series(&main_input)),
        tokio::task::spawn_blocking(move || {
            compound_outpaces_true_savings_month(&marker_input, assumption)
        }),
    );
    let output = main_join
        .map_err(|e| ApiError::BadRequest(format!("projection task panic: {e}")))?
        .map_err(map_engine_err)?;
    let compound_outpaces_true_savings_month_index = marker_join
        .map_err(|e| ApiError::BadRequest(format!("compound marker task panic: {e}")))?
        .map_err(map_engine_err)?;

    let starting_net_worth = output
        .net_worth
        .first()
        .copied()
        .unwrap_or(Decimal::ZERO);

    // Indices a serializar según la densidad solicitada. Para `Hybrid`
    // (mes 0..12 mensual + anual desde 24) el JSON pesa ~5× menos.
    let kept_indices = density_month_indices(density, output.net_worth.len() as u32);

    let points: Vec<ProjectionPoint> = kept_indices
        .iter()
        .filter_map(|&i| {
            let idx = i as usize;
            let nw = output.net_worth.get(idx)?;
            let cc = output.contributed_capital.get(idx)?;
            Some(ProjectionPoint {
                month_index: i,
                net_worth: *nw,
                contributed_capital: *cc,
            })
        })
        .collect();

    // Milestones se computan sobre TODOS los meses (no sobre los serializados),
    // si no, con `density=hybrid` se perderían milestones que caen entre dos
    // puntos anuales.
    let points_full: Vec<ProjectionPoint> = output
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

    // `asset_id_name` y `planning_rows` se reusan de `build_installation_projection_input` —
    // antes el handler hacía 2 SELECTs adicionales redundantes contra `assets` y `planning_flows`.
    let asset_series: Vec<AssetSeries> = asset_id_name
        .iter()
        .zip(output.per_asset_series.iter())
        .map(|((id, name), series)| AssetSeries {
            asset_id: *id,
            asset_name: name.clone(),
            values: kept_indices
                .iter()
                .filter_map(|&i| series.get(i as usize))
                .map(|v| v.to_f64().unwrap_or(0.0))
                .collect(),
        })
        .collect();

    let milestone_baseline_adjustment =
        planning_upcoming_net_for_milestone_baseline(today, &planning_rows);
    let milestones = projection_unique_reached_milestones(
        &points_full,
        today,
        milestone_baseline_adjustment,
        PROJECTION_MILESTONE_LIMIT,
        PROJECTION_MILESTONE_SEARCH_COUNT,
    );

    // Milestones en euros de hoy: mismos umbrales sobre el patrimonio deflactado. Con inflación 0 el
    // deflactor es 1 y serían idénticos a `milestones`, así que dejamos el vector vacío y la web
    // reusa `milestones`. Se computa sobre `points_full` (resolución mensual) por la misma razón
    // que `milestones`: no perder hitos que caen entre dos puntos anuales con densidad `hybrid`.
    let milestones_real = if inflation_annual_percent > Decimal::ZERO {
        let points_full_real = deflate_points_to_today(&points_full, inflation_annual_percent);
        projection_unique_reached_milestones(
            &points_full_real,
            today,
            milestone_baseline_adjustment,
            PROJECTION_MILESTONE_LIMIT,
            PROJECTION_MILESTONE_SEARCH_COUNT,
        )
    } else {
        Vec::new()
    };

    let fire_target_ref = projection_input.fire_target.as_ref();
    let (fire_target_series, jubilacion_month_index, jubilacion_target_net_worth) =
        match fire_target_ref {
            Some(ft) if ft.base_amount > Decimal::ZERO => {
                let crossed_at = fire_crossover_month(Some(ft), &output.net_worth);
                // Serializar el target solo en los puntos retenidos por la
                // densidad. Paralelo a `points`.
                let series: Vec<f64> = kept_indices
                    .iter()
                    .map(|&i| {
                        fire_target_at_month_index(Some(ft), i)
                            .unwrap_or(Decimal::ZERO)
                            .to_f64()
                            .unwrap_or(0.0)
                    })
                    .collect();
                (series, crossed_at, Some(ft.base_amount.to_string()))
            }
            _ => (Vec::new(), None, None),
        };

    Ok(ProjectionSeriesResponse {
        points,
        months,
        horizon_years,
        horizon_basis,
        starting_net_worth,
        monthly_delta_assumption,
        model_note:
            "Motor mensual: presupuesto regular sin cuotas derivadas de pasivos, servicio de deuda activo por mes, ajustes por Próximos, caja mensual consolidada (ingresos/gastos/aportaciones constantes en euros nominales), crecimiento compuesto por activo en términos nominales. El target FIRE se evalúa mes a mes ajustado por inflación para preservar el poder adquisitivo del usuario."
                .into(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        show_age_mode: show_age_mode.clone(),
        use_age_on_x_axis: show_age_mode.trim() == "ages"
            && resolved_birth_for_demographics.is_some(),
        viewer_birth_date: resolved_birth_for_demographics
            .map(|d| d.format("%Y-%m-%d").to_string()),
        milestones,
        milestones_real,
        compound_outpaces_true_savings_month_index,
        jubilacion_month_index,
        jubilacion_target_net_worth,
        fire_target_series,
        asset_series,
        density: match density {
            Density::Monthly => "monthly".into(),
            Density::Hybrid => "hybrid".into(),
        },
        savings_source: effective_savings_source,
        savings_source_months_with_data,
    })
}

// ---------------------------------------------------------------------------
// simulate_projection (tool MCP, what-if puro sin persistir)
// ---------------------------------------------------------------------------

/// Overrides parseados de la tool `simulate_projection` (los strings decimales/UUID/fecha ya
/// convertidos por la capa MCP; las COTAS de dominio se validan aquí, en el core).
#[derive(Debug, Default)]
pub(crate) struct SimulationSpec {
    pub months: Option<u32>,
    /// Gasto puntual: importe > 0 + exactamente uno de (`month_index` 1..=horizonte, `date`).
    pub one_off_amount: Option<Decimal>,
    pub one_off_month_index: Option<u32>,
    pub one_off_date: Option<NaiveDate>,
    pub extra_monthly_expense: Option<Decimal>,
    pub extra_monthly_cash_adjustment: Option<Decimal>,
    pub extra_monthly_savings: Option<Decimal>,
    pub swr_pct: Option<Decimal>,
    pub annual_inflation_percent: Option<Decimal>,
    pub retirement_annual_expense: Option<Decimal>,
    pub asset_return_overrides: Vec<(Uuid, Decimal)>,
    pub include_series: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimKpis {
    /// Mes del cruce con el target FIRE (None = no se alcanza en el horizonte).
    pub jubilacion_month_index: Option<u32>,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_net_worth: Decimal,
    /// Base del target FIRE (euros de hoy; el target servido crece con la inflación).
    #[serde(with = "rust_decimal::serde::str_option")]
    pub fire_target_base: Option<Decimal>,
    /// Runway de líquidos con la misma fórmula que `/v1/summary`, sobre los inputs del lado.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub runway_months: Option<Decimal>,
    pub runway_is_indefinite: bool,
    /// Ingreso mensual efectivo del lado (`ProjectionInput::income_regular_monthly`): presupuesto
    /// en modos A y C, promedio real 12m en modo B.
    #[serde(with = "rust_decimal::serde::str")]
    pub income_monthly: Decimal,
    /// Gasto mensual **total** = gasto regular + servicio de deuda. Es la MISMA base que alimenta
    /// el runway y el target FIRE de este lado, y la que cuadra con
    /// `expense_total_monthly_equivalent` de `/v1/summary` en los tres modos. Ojo: no es
    /// `input.expense_regular_monthly` a secas — en modo A la cuota de pasivo vive deliberadamente
    /// fuera de esa base (`budget.rs`, para no contarla dos veces en la proyección) y entra aquí
    /// por `debt_service_monthly`.
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_total_monthly: Decimal,
    /// Cuota mensual de los pasivos activos. 0 en los modos B y C por contrato (ahí las cuotas ya
    /// son movimientos reales dentro del promedio de gasto).
    #[serde(with = "rust_decimal::serde::str")]
    pub debt_service_monthly: Decimal,
    /// `income_monthly − expense_total_monthly`. Es el neto recurrente, NO el `net_cash_month` que
    /// reparte la cascada: ese incluye además el tramo de planning flows del mes en curso.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_monthly: Decimal,
    /// `net_monthly / income_monthly`, redondeado a 6 decimales igual que en `/v1/summary` (misma
    /// precisión en las dos superficies). `None` si no hay ingreso.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub savings_rate: Option<Decimal>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimDeltas {
    /// `scenario − baseline` en meses; None si alguno de los dos lados no alcanza el target.
    pub jubilacion_months_delta: Option<i64>,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_net_worth_delta: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub fire_target_base_delta: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub runway_months_delta: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub income_monthly_delta: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_total_monthly_delta: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub net_monthly_delta: Decimal,
    /// Diferencia de tasas calculada sobre los ratios **sin redondear** y redondeada al final:
    /// restar dos valores ya recortados a 6 dp propagaría el error de presentación al delta.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub savings_rate_delta: Option<Decimal>,
}

/// Series decimadas (hybrid) opt-in — números f64 como el resto de series de chart.
#[derive(Debug, Serialize)]
pub(crate) struct SimSeries {
    pub month_indices: Vec<u32>,
    pub baseline_net_worth: Vec<f64>,
    pub scenario_net_worth: Vec<f64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimulateProjectionResponse {
    pub horizon_months: u32,
    pub view: String,
    pub baseline: SimKpis,
    pub scenario: SimKpis,
    pub deltas: SimDeltas,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<SimSeries>,
}

fn require_non_negative(name: &str, v: Option<Decimal>) -> Result<Decimal, ApiError> {
    let v = v.unwrap_or(Decimal::ZERO);
    if v < Decimal::ZERO {
        return Err(ApiError::BadRequest(format!("{name} must be >= 0")));
    }
    Ok(v)
}

/// KPIs de un lado de la simulación (baseline o escenario).
fn sim_kpis(
    input: &ProjectionInput,
    output: &futurefin_engine::ProjectionOutput,
    debt_service_monthly: Decimal,
    inflation_annual_percent: Decimal,
    fs: &FireSettings,
) -> SimKpis {
    let jubilacion_month_index = fire_crossover_month(input.fire_target.as_ref(), &output.net_worth);
    let final_net_worth = output.net_worth.last().copied().unwrap_or(Decimal::ZERO);

    // Runway: misma fórmula que `/v1/summary` (gasto total = regular + servicio de deuda;
    // infinito ⟺ SWR sobre el gasto anual grosseado), evaluada sobre los inputs del lado.
    let liquid_rows: Vec<(Decimal, Option<Decimal>)> = input
        .assets
        .iter()
        .filter(|a| a.is_liquid)
        .map(|a| (a.value.max(Decimal::ZERO), a.expected_annual_return_percent))
        .collect();
    let monthly_expense = input.expense_regular_monthly + debt_service_monthly;
    let annual_expense_gross = gross_up_net_annual_fire(
        monthly_expense * Decimal::from(12u32),
        &fs.tax_brackets,
        fs.taxes_enabled,
    );
    let (runway_months, runway_is_indefinite) = match futurefin_engine::liquid_runway_months(
        &liquid_rows,
        monthly_expense,
        inflation_annual_percent,
        fs.swr_pct,
        annual_expense_gross,
    ) {
        futurefin_engine::RunwayOutcome::Months(m) => (Some(m.round_dp(1)), false),
        futurefin_engine::RunwayOutcome::Indefinite => (None, true),
        futurefin_engine::RunwayOutcome::NoExpenseBase => (None, false),
    };

    let income_monthly = input.income_regular_monthly;
    let net_monthly = income_monthly - monthly_expense;
    let savings_rate = (income_monthly > Decimal::ZERO)
        .then(|| (net_monthly / income_monthly).round_dp(SIM_RATIO_DP));

    SimKpis {
        jubilacion_month_index,
        final_net_worth,
        fire_target_base: input.fire_target.as_ref().map(|ft| ft.base_amount),
        runway_months,
        runway_is_indefinite,
        income_monthly,
        expense_total_monthly: monthly_expense,
        debt_service_monthly,
        net_monthly,
        savings_rate,
    }
}

/// Decimales de `SimKpis::savings_rate`. Debe seguir siendo el mismo que el `RATIO_DP` de
/// `handlers/summary.rs`: si divergen, se reabre la incoherencia de precisión entre superficies que
/// el redondeo de 3.8.0 vino a cerrar (le pasó a `runway_months` durante dos versiones).
const SIM_RATIO_DP: u32 = 6;

/// What-if de proyección/FIRE sin persistir nada. Ensambla el baseline y el escenario con el
/// MISMO contexto (`resolve_projection_context`: today, horizonte, inflación, fire_settings) y
/// re-simula ambos en `spawn_blocking` (patrón del marker de `compute_projection_series_response`).
///
/// **Cache-neutral por construcción**: jamás pasa por `projection_series_cached` (que insertaría
/// el what-if en la cache del hogar) — regresión pinneada en `mcp_simulate.rs`.
///
/// Dos ensamblados (una tanda de SELECTs por lado) a propósito: los overrides pre-target
/// (`SimOverrides`) se aplican DENTRO de `build_installation_projection_input`, en el mismo punto
/// donde se derivan target y caps, para que la semántica del escenario no pueda divergir de la de
/// una proyección real con esos valores guardados. El resto de overrides muta el input clonado.
pub(crate) async fn simulate_projection_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    spec: SimulationSpec,
) -> Result<SimulateProjectionResponse, ApiError> {
    // ---- Cotas de dominio (mismas que sus ejes de settings reales) --------------------------
    if let Some(m) = spec.months {
        if !(12..=840).contains(&m) {
            return Err(ApiError::BadRequest("months must be between 12 and 840".into()));
        }
    }
    let extra_expense = require_non_negative("extra_monthly_expense", spec.extra_monthly_expense)?;
    let extra_cash_adj = require_non_negative(
        "extra_monthly_cash_adjustment",
        spec.extra_monthly_cash_adjustment,
    )?;
    let extra_savings = require_non_negative("extra_monthly_savings", spec.extra_monthly_savings)?;
    if let Some(p) = spec.annual_inflation_percent {
        crate::handlers::installation::validate_annual_inflation_assumption(p)?;
    }
    if let Some(v) = spec.retirement_annual_expense {
        if v <= Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "retirement_annual_expense must be > 0".into(),
            ));
        }
    }
    for (asset_id, pct) in &spec.asset_return_overrides {
        // −100 no tiene raíz 12ª real (el engine clamparía a pérdida total): se rechaza.
        if *pct <= Decimal::from(-100) {
            return Err(ApiError::BadRequest(format!(
                "expected_annual_return_percent for asset {asset_id} must be greater than -100"
            )));
        }
    }
    match (
        spec.one_off_amount,
        spec.one_off_month_index,
        spec.one_off_date,
    ) {
        (None, None, None) => {}
        (Some(a), mi, d) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest("one_off_expense.amount must be > 0".into()));
            }
            if mi.is_some() == d.is_some() {
                return Err(ApiError::BadRequest(
                    "one_off_expense requires exactly one of month_index or date".into(),
                ));
            }
        }
        _ => {
            return Err(ApiError::BadRequest(
                "one_off_expense.amount is required with month_index/date".into(),
            ))
        }
    }

    // ---- Contexto compartido (mismas queries y regla de horizonte que el GET) ----------------
    let ctx = resolve_projection_context(pool, iid, user_id, spec.months).await?;
    let months = ctx.months;

    // Settings efectivos del escenario, re-validados con las cotas del PATCH real.
    let mut fs_eff = ctx.fire_settings.clone();
    if let Some(swr) = spec.swr_pct {
        fs_eff.swr_pct = swr;
    }
    crate::handlers::installation::validate_fire_settings(&fs_eff)?;
    let inflation_eff = spec
        .annual_inflation_percent
        .unwrap_or(ctx.inflation_annual_percent);

    let sim_ov = SimOverrides {
        extra_monthly_expense: extra_expense,
        retirement_monthly_expense: spec
            .retirement_annual_expense
            .map(|v| v / Decimal::from(12u32)),
    };

    let baseline_built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        ctx.today,
        months,
        ctx.inflation_annual_percent,
        Some(&ctx.fire_settings),
        None,
    )
    .await?;
    let scenario_built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        ctx.today,
        months,
        inflation_eff,
        Some(&fs_eff),
        Some(&sim_ov),
    )
    .await?;

    // ---- Overrides post-build sobre el input clonado del escenario ---------------------------
    let mut scenario_input = scenario_built.input.clone();

    // Ajustes de caja constantes: mecanismo planning adjustment (entran en el net_cash mensual y
    // pasan por la cascada real, sin mover target ni caps).
    let monthly_adj = extra_savings - extra_cash_adj;
    if !monthly_adj.is_zero() {
        for slot in scenario_input.planning_monthly_cash_adjustment.iter_mut() {
            *slot += monthly_adj;
        }
    }

    // Gasto puntual.
    if let Some(amount) = spec.one_off_amount {
        match (spec.one_off_month_index, spec.one_off_date) {
            (Some(k), None) => {
                if !(1..=months).contains(&k) {
                    return Err(ApiError::BadRequest(format!(
                        "one_off_expense.month_index must be between 1 and {months}"
                    )));
                }
                scenario_input.planning_monthly_cash_adjustment[(k - 1) as usize] -= amount;
            }
            (None, Some(date)) => {
                // Mismo mapeo fecha→mes que un planning flow real con due_date.
                let synthetic = PlanningFlowProjRow {
                    scope: "expense".into(),
                    expected_amount: amount,
                    due_date: Some(date),
                };
                let adj = planning_monthly_cash_adjustments_from_flows(
                    ctx.today,
                    months,
                    std::slice::from_ref(&synthetic),
                );
                if adj.iter().all(|v| v.is_zero()) {
                    return Err(ApiError::BadRequest(
                        "one_off_expense.date is outside the projection horizon".into(),
                    ));
                }
                for (slot, extra) in scenario_input
                    .planning_monthly_cash_adjustment
                    .iter_mut()
                    .zip(adj.iter())
                {
                    *slot += *extra;
                }
            }
            _ => unreachable!("validated above"),
        }
    }

    // Tasas por activo (los negativos > −100 son válidos desde el fix de `monthly_multiplier`).
    for (asset_id, pct) in &spec.asset_return_overrides {
        let Some(idx) = scenario_built
            .asset_id_name
            .iter()
            .position(|(id, _)| id == asset_id)
        else {
            return Err(ApiError::BadRequest(format!(
                "unknown asset_id {asset_id} (not in scope for this view)"
            )));
        };
        scenario_input.assets[idx].expected_annual_return_percent = Some(*pct);
    }

    // ---- Doble simulación en el pool blocking (CPU-bound; patrón del marker) -----------------
    let baseline_input = baseline_built.input.clone();
    let scenario_sim_input = scenario_input.clone();
    let (baseline_join, scenario_join) = tokio::join!(
        tokio::task::spawn_blocking(move || project_net_worth_series(&baseline_input)),
        tokio::task::spawn_blocking(move || project_net_worth_series(&scenario_sim_input)),
    );
    let baseline_out = baseline_join
        .map_err(|e| ApiError::BadRequest(format!("projection task panic: {e}")))?
        .map_err(map_engine_err)?;
    let scenario_out = scenario_join
        .map_err(|e| ApiError::BadRequest(format!("projection task panic: {e}")))?
        .map_err(map_engine_err)?;

    let baseline = sim_kpis(
        &baseline_built.input,
        &baseline_out,
        baseline_built.debt_service_monthly,
        ctx.inflation_annual_percent,
        &ctx.fire_settings,
    );
    let scenario = sim_kpis(
        &scenario_input,
        &scenario_out,
        scenario_built.debt_service_monthly,
        inflation_eff,
        &fs_eff,
    );

    let deltas = SimDeltas {
        jubilacion_months_delta: match (baseline.jubilacion_month_index, scenario.jubilacion_month_index)
        {
            (Some(b), Some(s)) => Some(s as i64 - b as i64),
            _ => None,
        },
        final_net_worth_delta: scenario.final_net_worth - baseline.final_net_worth,
        fire_target_base_delta: match (baseline.fire_target_base, scenario.fire_target_base) {
            (Some(b), Some(s)) => Some(s - b),
            _ => None,
        },
        runway_months_delta: match (baseline.runway_months, scenario.runway_months) {
            (Some(b), Some(s)) => Some(s - b),
            _ => None,
        },
        income_monthly_delta: scenario.income_monthly - baseline.income_monthly,
        expense_total_monthly_delta: scenario.expense_total_monthly - baseline.expense_total_monthly,
        net_monthly_delta: scenario.net_monthly - baseline.net_monthly,
        // Se recalcula desde `net`/`income` (que viajan EXACTOS) en vez de restar los dos
        // `savings_rate` ya redondeados.
        savings_rate_delta: {
            let raw = |k: &SimKpis| {
                (k.income_monthly > Decimal::ZERO).then(|| k.net_monthly / k.income_monthly)
            };
            match (raw(&baseline), raw(&scenario)) {
                (Some(b), Some(s)) => Some((s - b).round_dp(SIM_RATIO_DP)),
                _ => None,
            }
        },
    };

    let series = if spec.include_series {
        let kept = density_month_indices(Density::Hybrid, baseline_out.net_worth.len() as u32);
        let pick = |serie: &[Decimal]| -> Vec<f64> {
            kept.iter()
                .filter_map(|&i| serie.get(i as usize))
                .map(|v| v.to_f64().unwrap_or(0.0))
                .collect()
        };
        Some(SimSeries {
            baseline_net_worth: pick(&baseline_out.net_worth),
            scenario_net_worth: pick(&scenario_out.net_worth),
            month_indices: kept,
        })
    } else {
        None
    };

    Ok(SimulateProjectionResponse {
        horizon_months: months,
        view: if view == LedgerView::Mine {
            "mine".into()
        } else {
            "household".into()
        },
        baseline,
        scenario,
        deltas,
        series,
    })
}

pub fn projection_router() -> Router {
    Router::new().route("/series", get(get_projection_series))
}

/// Recompute de la proyección `view=household` (ambas densidades) y guardado
/// en cache. Pensado para `tokio::spawn` tras login. Si falla, no propaga el
/// error: solo deja el cache vacío para que el próximo GET haga el compute
/// sincronamente.
pub async fn warm_up_household_projection(
    state: Arc<AppState>,
    installation_id: Uuid,
    user_id: Uuid,
) {
    for density in [Density::Hybrid, Density::Monthly] {
        tracing::info!(installation_id = %installation_id, density = ?density, "warm-up household projection start");
        let t0 = std::time::Instant::now();
        let key = ProjectionCacheKey {
            installation_id,
            view: LedgerView::Household,
            owner_user_id: None,
            density,
        };
        match compute_projection_series_response(
            &state,
            user_id,
            installation_id,
            LedgerView::Household,
            None,
            density,
        )
        .await
        {
            Ok(response) => {
                state.projection_cache_insert(key, Arc::new(response)).await;
                tracing::info!(
                    installation_id = %installation_id,
                    density = ?density,
                    ms = t0.elapsed().as_millis() as u64,
                    "warm-up done"
                );
            }
            Err(e) => {
                tracing::warn!(installation_id = %installation_id, density = ?density, error = ?e, "warm-up failed");
            }
        }
    }
}

/// Helper para handlers de mutación. Invalida todas las entries del
/// installation. **No** dispara warm-up tras mutación para evitar una race
/// condition: dos mutaciones consecutivas (M1, M2) podrían generar dos
/// warm-ups concurrentes y el de M1 (con datos pre-M2) puede terminar
/// después del de M2, dejando el cache stale. El próximo GET (cache miss)
/// hace compute on-demand — paga ~500 ms una vez tras una mutación, luego
/// cache. El warm-up proactivo se mantiene solo en login (sin
/// invalidaciones concurrentes).
pub fn refresh_projection_after_mutation(
    state: Arc<AppState>,
    installation_id: Uuid,
    _user_id: Uuid,
) {
    tokio::spawn(async move {
        state
            .invalidate_projection_by_installation(installation_id)
            .await;
    });
}

#[cfg(test)]
mod horizon_tests {
    use super::*;

    #[test]
    fn horizon_fallback_without_birth_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let (m, basis) = projection_horizon_months(today, &[None]);
        assert_eq!(m, 30 * 12);
        assert_eq!(basis, "fallback_no_demographics");
    }

    #[test]
    fn horizon_uses_lifespan_90_from_birth_date() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![
            Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap()), // age 36 → 54y to 90
            Some(NaiveDate::from_ymd_opt(1985, 1, 1).unwrap()), // age 41 → 49y to 90
        ];
        let (m, basis) = projection_horizon_months(today, &bd);
        assert_eq!(m, 54 * 12); // max of 54 and 49, not clamped (54 < 70)
        assert_eq!(basis, "lifespan_90");
    }

    #[test]
    fn horizon_minimum_five_years_when_already_near_lifespan() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![Some(NaiveDate::from_ymd_opt(1940, 1, 1).unwrap())]; // age 86 → 4y to 90, clamped to 5
        let (m, basis) = projection_horizon_months(today, &bd);
        assert_eq!(basis, "lifespan_90");
        assert_eq!(m, 5 * 12);
    }
}

#[cfg(test)]
mod planning_distribution_tests {
    use super::*;

    #[test]
    fn dated_planning_hits_single_calendar_month() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        let flows = vec![PlanningFlowProjRow {
            scope: "expense".into(),
            expected_amount: Decimal::from(500),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()),
        }];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 4, &flows);
        assert_eq!(adj[2], Decimal::from(-500));
        assert_eq!(adj[0] + adj[1] + adj[3], Decimal::ZERO);
    }

    #[test]
    fn dated_before_anchor_month_is_ignored() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        let flows = vec![PlanningFlowProjRow {
            scope: "income".into(),
            expected_amount: Decimal::from(9999),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()),
        }];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 3, &flows);
        assert!(adj.iter().all(|x| *x == Decimal::ZERO));
    }

    #[test]
    fn undated_splits_over_ninety_days_from_ref_date() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let flows = vec![PlanningFlowProjRow {
            scope: "expense".into(),
            expected_amount: Decimal::from(900),
            due_date: None,
        }];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 3, &flows);
        assert_eq!(adj.iter().sum::<Decimal>(), Decimal::from(-900));
        assert_eq!(adj[0], Decimal::from(-310));
        assert_eq!(adj[1], Decimal::from(-280));
        assert_eq!(adj[2], Decimal::from(-310));
    }
}

#[cfg(test)]
mod milestone_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn baseline_adjustment_uses_dated_ninety_days_and_all_undated() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let flows = vec![
            PlanningFlowProjRow {
                scope: "income".into(),
                expected_amount: Decimal::from(1200),
                due_date: Some(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()),
            },
            PlanningFlowProjRow {
                scope: "expense".into(),
                expected_amount: Decimal::from(300),
                due_date: Some(NaiveDate::from_ymd_opt(2026, 4, 5).unwrap()),
            },
            PlanningFlowProjRow {
                scope: "expense".into(),
                expected_amount: Decimal::from(100),
                due_date: None,
            },
        ];
        // 1200 (dated within 90d) -100 (undated expense); April expense fuera ventana.
        assert_eq!(
            planning_upcoming_net_for_milestone_baseline(ref_d, &flows),
            Decimal::from(1100)
        );
    }

    #[test]
    fn milestones_deduplicate_by_year_keeping_highest_target() {
        let points = vec![
            ProjectionPoint {
                month_index: 0,
                net_worth: Decimal::from(900),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 1,
                net_worth: Decimal::from(1200),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 3,
                net_worth: Decimal::from(2700),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 9,
                net_worth: Decimal::from(6000),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 15,
                net_worth: Decimal::from(11000),
                contributed_capital: Decimal::ZERO,
            },
        ];
        let out = projection_unique_reached_milestones(
            &points,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            Decimal::ZERO,
            3,
            16,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].target, Decimal::from(5000));
        assert_eq!(out[1].target, Decimal::from(10_000));
    }

    #[test]
    fn deflate_points_to_today_is_identity_with_zero_inflation() {
        let points = vec![
            ProjectionPoint {
                month_index: 0,
                net_worth: Decimal::from(1000),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 12,
                net_worth: Decimal::from(2000),
                contributed_capital: Decimal::from(100),
            },
        ];
        let out = deflate_points_to_today(&points, Decimal::ZERO);
        assert_eq!(out[0].net_worth, Decimal::from(1000));
        assert_eq!(out[1].net_worth, Decimal::from(2000));
        assert_eq!(out[1].contributed_capital, Decimal::from(100));
    }

    #[test]
    fn deflate_points_to_today_discounts_future_to_present() {
        // Con 10% anual, 1.100 € dentro de un año equivalen a 1.000 € de hoy.
        let points = vec![
            ProjectionPoint {
                month_index: 0,
                net_worth: Decimal::from(1000),
                contributed_capital: Decimal::ZERO,
            },
            ProjectionPoint {
                month_index: 12,
                net_worth: Decimal::from(1100),
                contributed_capital: Decimal::ZERO,
            },
        ];
        let out = deflate_points_to_today(&points, Decimal::from(10));
        assert_eq!(out[0].net_worth, Decimal::from(1000)); // mes 0 intacto
        let diff = (out[1].net_worth - Decimal::from(1000)).abs();
        assert!(
            diff < Decimal::new(1, 6),
            "expected ~1000 € de hoy, got {}",
            out[1].net_worth
        );
    }

    #[test]
    fn real_milestones_are_reached_later_than_nominal() {
        // Patrimonio nominal que alcanza 10.000 € al final del horizonte. Deflactado al 8% anual,
        // el mismo umbral en euros de hoy se cruza más tarde (o no se cruza dentro del horizonte).
        let points: Vec<ProjectionPoint> = (0u32..=120)
            .map(|m| ProjectionPoint {
                month_index: m,
                net_worth: Decimal::from(1000) + Decimal::from(75) * Decimal::from(m),
                contributed_capital: Decimal::ZERO,
            })
            .collect();
        let anchor = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let nominal = projection_unique_reached_milestones(&points, anchor, Decimal::ZERO, 3, 64);
        let real_points = deflate_points_to_today(&points, Decimal::from(8));
        let real = projection_unique_reached_milestones(&real_points, anchor, Decimal::ZERO, 3, 64);
        // Para cualquier umbral común, en euros de hoy se cruza más tarde (nunca antes); al menos uno
        // estrictamente posterior, porque el deflactor < 1 en cualquier mes > 0.
        let mut found_strictly_later = false;
        for n in &nominal {
            if let Some(r) = real.iter().find(|r| r.target == n.target) {
                assert!(
                    r.reached_month_index >= n.reached_month_index,
                    "umbral {} se cruzó antes en euros de hoy ({} < {})",
                    n.target,
                    r.reached_month_index,
                    n.reached_month_index
                );
                if r.reached_month_index > n.reached_month_index {
                    found_strictly_later = true;
                }
            }
        }
        assert!(
            found_strictly_later,
            "algún umbral común debería cruzarse más tarde en euros de hoy"
        );
    }

    #[test]
    fn compound_marker_ignores_planning_and_liability_payments() {
        let input = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 24,
            income_regular_monthly: Decimal::from(3000),
            expense_regular_monthly: Decimal::from(2500),
            assets: vec![SimAsset {
                id: Uuid::from_u128(1),
                value: Decimal::from(10_000),
                purchase_price: Some(Decimal::from(10_000)),
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(6)),
            }],
            allocation_rules: vec![AllocationRule {
                target_index: 0,
                kind: AllocationKind::Remainder,
                amount: None,
                cap: None,
            }],
            liabilities: vec![ProjectionLiabilityInput {
                principal: Decimal::from(50_000),
                monthly_payment: Decimal::from(1200),
                payment_end: None,
            }],
            planning_monthly_cash_adjustment: vec![Decimal::from(5_000); 24],
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: Decimal::from(2500),
            retirement_monthly_withdrawal: Decimal::ZERO,
            fire_target: None,
        };
        let month = compound_outpaces_true_savings_month(&input, Decimal::from(500)).unwrap();
        assert!(month.is_none());
    }

    #[test]
    fn compound_marker_requires_persistent_crossover() {
        let input = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 24,
            income_regular_monthly: Decimal::from(1200),
            expense_regular_monthly: Decimal::from(1000),
            assets: vec![SimAsset {
                id: Uuid::from_u128(2),
                value: Decimal::from(50_000),
                purchase_price: Some(Decimal::from(50_000)),
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(18)),
            }],
            allocation_rules: vec![AllocationRule {
                target_index: 0,
                kind: AllocationKind::Remainder,
                amount: None,
                cap: None,
            }],
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 24],
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: Decimal::from(1000),
            retirement_monthly_withdrawal: Decimal::ZERO,
            fire_target: None,
        };
        let month = compound_outpaces_true_savings_month(&input, Decimal::from(200)).unwrap();
        assert!(month.is_some());
        assert!(month.unwrap() >= 1);
    }
}

#[cfg(test)]
mod gross_up_tests {
    use super::*;

    fn es_brackets() -> Vec<TaxBracket> {
        vec![
            TaxBracket { up_to: Some(Decimal::from(6_000u32)),   pct: Decimal::from(19u32) },
            TaxBracket { up_to: Some(Decimal::from(50_000u32)),  pct: Decimal::from(21u32) },
            TaxBracket { up_to: Some(Decimal::from(200_000u32)), pct: Decimal::from(23u32) },
            TaxBracket { up_to: Some(Decimal::from(300_000u32)), pct: Decimal::from(27u32) },
            TaxBracket { up_to: None,                            pct: Decimal::from(30u32) },
        ]
    }

    /// Versión binaria de referencia (la que tenía el handler antes de Fase 2.4). Sirve para
    /// confirmar que la forma cerrada es numéricamente equivalente a ≤ 0.01 €.
    fn gross_up_binary_reference(net_annual: Decimal, brackets: &[TaxBracket]) -> Decimal {
        if net_annual <= Decimal::ZERO { return net_annual.max(Decimal::ZERO); }
        let mut lo = net_annual;
        let mut hi = (net_annual * Decimal::from(4u32))
            .max(net_annual + Decimal::from(200_000u32));
        for _ in 0..90 {
            let mid = (lo + hi) / Decimal::from(2u32);
            let after = mid - tax_on_gross_capital_annual(mid, brackets);
            if after < net_annual { lo = mid; } else { hi = mid; }
        }
        hi
    }

    #[test]
    fn closed_form_matches_binary_search_across_es_brackets() {
        let brackets = es_brackets();
        let nets = [
            Decimal::from(1_000u32),
            Decimal::from(5_000u32),
            Decimal::from(20_000u32),
            Decimal::from(40_000u32),
            Decimal::from(80_000u32),
            Decimal::from(150_000u32),
            Decimal::from(250_000u32),
            Decimal::from(400_000u32),
            Decimal::from(1_000_000u32),
        ];
        let tol = Decimal::new(1, 2); // 0.01 €
        for net in nets {
            let g_closed = gross_up_net_annual_fire(net, &brackets, true);
            let g_binary = gross_up_binary_reference(net, &brackets);
            let diff = (g_closed - g_binary).abs();
            assert!(
                diff <= tol,
                "diff {diff} excede tolerancia para net={net}: closed={g_closed}, binary={g_binary}"
            );
            // Y verifica que el gross resultante deja después-de-tax ≈ net.
            let after = g_closed - tax_on_gross_capital_annual(g_closed, &brackets);
            assert!(
                (after - net).abs() <= tol,
                "after-tax({g_closed}) = {after} no recupera net={net}"
            );
        }
    }

    #[test]
    fn closed_form_handles_taxes_disabled_and_zero_net() {
        let brackets = es_brackets();
        assert_eq!(gross_up_net_annual_fire(Decimal::from(50_000u32), &brackets, false), Decimal::from(50_000u32));
        assert_eq!(gross_up_net_annual_fire(Decimal::ZERO, &brackets, true), Decimal::ZERO);
        assert_eq!(
            gross_up_net_annual_fire(-Decimal::from(100u32), &brackets, true),
            Decimal::ZERO,
            "net negativo se clipea a 0"
        );
    }
}
