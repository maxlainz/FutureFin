//! Monthly projection: regular budget (no derived liability rows) + active debt service +
//! asset contributions / drain / compound growth. Ajustes opcionales por mes desde «Próximos»
//! (`planning_monthly_cash_adjustment`) suman al flujo de caja recurrente del mes (ingreso (+)
//! / gasto (−)) antes del reparto a activos o el drenaje.

use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("horizon_months must be >= 1")]
    InvalidHorizon,
    #[error("planning_monthly_cash_adjustment must have length horizon_months")]
    InvalidPlanningAdjustments,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimAsset {
    pub id: Uuid,
    pub value: Decimal,
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    /// Expected annual return % (e.g. 7 for 7%). None → no compound growth (factor 1).
    pub expected_annual_return_percent: Option<Decimal>,
    pub monthly_contribution_fixed: Decimal,
    /// Non-negative weight for splitting remainder after fixed contributions (normalized).
    pub contribution_remainder_weight: Decimal,
}

#[derive(Debug, Clone)]
pub struct ProjectionLiabilityInput {
    pub principal: Decimal,
    pub monthly_payment: Decimal,
    pub payment_end: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    /// Civil "today" de la instalación (inicio del mes simulado para el índice 1).
    pub ref_date: NaiveDate,
    pub horizon_months: u32,
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    pub liabilities: Vec<ProjectionLiabilityInput>,
    /// Annual inflation % for deflating nominal series to “money of today”; None → nominal.
    pub inflation_annual_percent: Option<Decimal>,
    /// Signed cash from planning flows per simulated month (`len == horizon_months`): index `i`
    /// pairs with month `i+1` (calendar month `add_months(month_first_calendar(ref_date), i)`).
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
}

#[derive(Debug, Clone)]
pub struct ProjectionOutput {
    /// Month index 0..=horizon_months inclusive.
    pub net_worth: Vec<Decimal>,
    /// Cumulative contributed basis, deflated when inflation assumption is active.
    pub contributed_capital: Vec<Decimal>,
}

fn month_first_calendar(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n))
        .unwrap_or(d)
}

fn month_window(month_first: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = month_first;
    let next_first = add_months(month_first, 1);
    let end = next_first.pred_opt().unwrap_or(start);
    (start, end)
}

fn monthly_multiplier(annual_percent: Option<Decimal>) -> Decimal {
    let Some(p) = annual_percent else {
        return Decimal::ONE;
    };
    if p <= Decimal::ZERO {
        return Decimal::ONE;
    }
    let r = p / Decimal::from(100);
    let base = Decimal::ONE + r;
    base.powd(Decimal::ONE / Decimal::from(12))
}

fn drain_from_assets(
    values: &mut [Decimal],
    liquid: &[bool],
    rates: &[Option<Decimal>],
    mut need: Decimal,
) -> Decimal {
    if need <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        let li = liquid[i];
        let lj = liquid[j];
        match (li, lj) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => rates[i]
                .unwrap_or(Decimal::ZERO)
                .cmp(&rates[j].unwrap_or(Decimal::ZERO))
                .then_with(|| i.cmp(&j)),
        }
    });
    for idx in order {
        if need <= Decimal::ZERO {
            break;
        }
        let take = values[idx].min(need);
        values[idx] -= take;
        need -= take;
    }
    need
}

/// Nominal contributions routed to each asset in the **first simulated month** (calendar month de
/// `ref_date`): cuotas fijas escaladas más remanente por pesos — mismas reglas que la primera
/// iteración de [`project_net_worth_series`]. Cero si el superávit recurrente del mes es ≤ 0.
pub fn first_month_per_asset_contribution_nominals(
    input: &ProjectionInput,
) -> Result<Vec<Decimal>, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }

    let n = input.assets.len();
    let mut out = vec![Decimal::ZERO; n];
    if n == 0 {
        return Ok(out);
    }

    let fixed: Vec<Decimal> = input
        .assets
        .iter()
        .map(|a| a.monthly_contribution_fixed.max(Decimal::ZERO))
        .collect();
    let weights: Vec<Decimal> = input
        .assets
        .iter()
        .map(|a| a.contribution_remainder_weight.max(Decimal::ZERO))
        .collect();

    let principals: Vec<Decimal> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(Decimal::ZERO))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);
    let month_first = add_months(start_month_first, 0);
    let (m_start, _m_end) = month_window(month_first);

    let mut debt_service = Decimal::ZERO;
    for (i, liab) in input.liabilities.iter().enumerate() {
        let active = match liab.payment_end {
            None => liab.monthly_payment > Decimal::ZERO,
            Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
        };
        if !active {
            continue;
        }
        let pay = liab
            .monthly_payment
            .min(principals.get(i).copied().unwrap_or(Decimal::ZERO));
        debt_service += pay;
    }

    let planning_adj = input.planning_monthly_cash_adjustment[0];

    let scheduled_savings = input.income_regular_monthly
        - input.expense_regular_monthly
        - debt_service
        + planning_adj;

    let net_cash_month = scheduled_savings;
    if net_cash_month <= Decimal::ZERO {
        return Ok(out);
    }

    let pool = net_cash_month;
    let sum_fixed: Decimal = fixed.iter().copied().sum();
    let scale = if sum_fixed > pool && sum_fixed > Decimal::ZERO {
        pool / sum_fixed
    } else {
        Decimal::ONE
    };

    for (i, fx) in fixed.iter().enumerate() {
        let add = (*fx * scale).max(Decimal::ZERO);
        out[i] += add;
    }

    let applied: Decimal = out.iter().copied().sum();
    let remainder_pool = pool - applied;
    if remainder_pool <= Decimal::ZERO {
        return Ok(out);
    }

    let wsum: Decimal = weights.iter().copied().sum();
    if wsum <= Decimal::ZERO {
        return Ok(out);
    }

    for (i, w) in weights.iter().enumerate() {
        let share = remainder_pool * (*w) / wsum;
        out[i] += share;
    }

    Ok(out)
}

pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }

    let mut values: Vec<Decimal> = input.assets.iter().map(|a| a.value).collect();
    let liquid: Vec<bool> = input.assets.iter().map(|a| a.is_liquid).collect();
    let rates: Vec<Option<Decimal>> = input
        .assets
        .iter()
        .map(|a| a.expected_annual_return_percent)
        .collect();
    let fixed: Vec<Decimal> = input
        .assets
        .iter()
        .map(|a| a.monthly_contribution_fixed.max(Decimal::ZERO))
        .collect();
    let weights: Vec<Decimal> = input
        .assets
        .iter()
        .map(|a| a.contribution_remainder_weight.max(Decimal::ZERO))
        .collect();

    let mut principals: Vec<Decimal> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(Decimal::ZERO))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);

    let mut net_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut contrib_series = Vec::with_capacity(input.horizon_months as usize + 1);

    // Coste histórico ya invertido (precio de compra) antes del primer mes simulado.
    let initial_contributed_basis: Decimal = input
        .assets
        .iter()
        .filter_map(|a| a.purchase_price)
        .filter(|p| *p > Decimal::ZERO)
        .sum();

    let mut contributed_cumulative = initial_contributed_basis;
    let mut undrained_cumulative = Decimal::ZERO;
    // Monthly savings not routed when remainder weights sum to zero.
    let mut surplus_cash = Decimal::ZERO;

    let nw_fn = |vals: &[Decimal], pr: &[Decimal], und: Decimal, surplus: Decimal| -> Decimal {
        let ta: Decimal = vals.iter().copied().sum();
        let tl: Decimal = pr.iter().copied().sum();
        ta + surplus - tl - und
    };

    net_series.push(nw_fn(
        &values,
        &principals,
        undrained_cumulative,
        surplus_cash,
    ));
    contrib_series.push(contributed_cumulative);

    for k in 1..=input.horizon_months {
        let month_first = add_months(start_month_first, k - 1);
        let (m_start, _m_end) = month_window(month_first);

        let mut debt_service = Decimal::ZERO;
        for (i, liab) in input.liabilities.iter().enumerate() {
            let active = match liab.payment_end {
                None => liab.monthly_payment > Decimal::ZERO,
                Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
            };
            if !active {
                continue;
            }
            let pay = liab
                .monthly_payment
                .min(principals.get(i).copied().unwrap_or(Decimal::ZERO));
            debt_service += pay;
        }

        let planning_adj = input.planning_monthly_cash_adjustment[(k - 1) as usize];

        let scheduled_savings = input.income_regular_monthly
            - input.expense_regular_monthly
            - debt_service
            + planning_adj;

        let net_cash_month = scheduled_savings;

        if net_cash_month <= Decimal::ZERO {
            let mut need = -net_cash_month;
            let from_surplus = surplus_cash.min(need);
            surplus_cash -= from_surplus;
            need -= from_surplus;
            if need > Decimal::ZERO {
                let und = drain_from_assets(&mut values, &liquid, &rates, need);
                undrained_cumulative += und;
            }
        } else {
            let pool = net_cash_month;
            let sum_fixed: Decimal = fixed.iter().copied().sum();
            let scale = if sum_fixed > pool && sum_fixed > Decimal::ZERO {
                pool / sum_fixed
            } else {
                Decimal::ONE
            };
            let mut applied = Decimal::ZERO;
            for (i, fx) in fixed.iter().enumerate() {
                let add = (*fx * scale).max(Decimal::ZERO);
                values[i] += add;
                contributed_cumulative += add;
                applied += add;
            }
            let remainder_pool = pool - applied;
            if remainder_pool > Decimal::ZERO {
                let wsum: Decimal = weights.iter().copied().sum();
                if wsum > Decimal::ZERO {
                    for (i, w) in weights.iter().enumerate() {
                        let share = remainder_pool * (*w) / wsum;
                        values[i] += share;
                        contributed_cumulative += share;
                    }
                } else {
                    surplus_cash += remainder_pool;
                    contributed_cumulative += remainder_pool;
                }
            }
        }

        for i in 0..values.len() {
            let m = monthly_multiplier(rates[i]);
            values[i] *= m;
        }

        for (i, liab) in input.liabilities.iter().enumerate() {
            if i >= principals.len() {
                break;
            }
            let active = match liab.payment_end {
                None => liab.monthly_payment > Decimal::ZERO,
                Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
            };
            if active {
                let pay = liab
                    .monthly_payment
                    .min(principals[i])
                    .max(Decimal::ZERO);
                principals[i] -= pay;
            }
        }

        let mut nw = nw_fn(
            &values,
            &principals,
            undrained_cumulative,
            surplus_cash,
        );
        let mut contributed_display = contributed_cumulative;
        if let Some(inf) = input.inflation_annual_percent {
            if inf > Decimal::ZERO {
                let months_k = Decimal::from(k);
                let denom = (Decimal::ONE + inf / Decimal::from(100))
                    .powd(months_k / Decimal::from(12));
                if denom > Decimal::ZERO {
                    nw /= denom;
                    contributed_display /= denom;
                }
            }
        }
        net_series.push(nw);
        contrib_series.push(contributed_display);
    }

    Ok(ProjectionOutput {
        net_worth: net_series,
        contributed_capital: contrib_series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn linear_savings_no_assets_matches_accumulation() {
        let id = Uuid::nil();
        let a = SimAsset {
            id,
            value: Decimal::ZERO,
            purchase_price: Some(Decimal::ZERO),
            is_liquid: true,
            expected_annual_return_percent: None,
            monthly_contribution_fixed: Decimal::ZERO,
            contribution_remainder_weight: Decimal::ZERO,
        };
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 3,
            income_regular_monthly: Decimal::from(3000),
            expense_regular_monthly: Decimal::from(1000),
            assets: vec![a],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 3],
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth.len(), 4);
        assert_eq!(out.net_worth[0], Decimal::ZERO);
        assert_eq!(out.net_worth[1], Decimal::from(2000));
        assert_eq!(out.net_worth[2], Decimal::from(4000));
        assert_eq!(out.net_worth[3], Decimal::from(6000));
    }

    #[test]
    fn first_month_nominals_split_remainder_by_weights() {
        let id_a = Uuid::from_u128(1);
        let id_b = Uuid::from_u128(2);
        let a = SimAsset {
            id: id_a,
            value: Decimal::ZERO,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: None,
            monthly_contribution_fixed: Decimal::ZERO,
            contribution_remainder_weight: Decimal::from(50),
        };
        let b = SimAsset {
            id: id_b,
            value: Decimal::ZERO,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: None,
            monthly_contribution_fixed: Decimal::ZERO,
            contribution_remainder_weight: Decimal::from(50),
        };
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 3,
            income_regular_monthly: Decimal::from(4000),
            expense_regular_monthly: Decimal::from(3000),
            assets: vec![a, b],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 3],
        };
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom.len(), 2);
        assert_eq!(nom[0], Decimal::from(500));
        assert_eq!(nom[1], Decimal::from(500));
    }

    #[test]
    fn planning_adjustment_boosts_first_month_contribution_nominals() {
        let id_a = Uuid::from_u128(1);
        let a = SimAsset {
            id: id_a,
            value: Decimal::ZERO,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: None,
            monthly_contribution_fixed: Decimal::ZERO,
            contribution_remainder_weight: Decimal::ONE,
        };
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 1,
            income_regular_monthly: Decimal::from(1000),
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![a],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::from(250)],
        };
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom.len(), 1);
        assert_eq!(nom[0], Decimal::from(1250));
    }

    #[test]
    fn contributed_capital_month_zero_includes_purchase_prices() {
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 2,
            income_regular_monthly: Decimal::ZERO,
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![SimAsset {
                id: Uuid::from_u128(99),
                value: Decimal::from(100_000),
                purchase_price: Some(Decimal::from(80_000)),
                is_liquid: true,
                expected_annual_return_percent: None,
                monthly_contribution_fixed: Decimal::ZERO,
                contribution_remainder_weight: Decimal::ZERO,
            }],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 2],
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[1], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[2], Decimal::from(80_000));
    }

    #[test]
    fn contributed_capital_is_deflated_when_inflation_is_active() {
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 1,
            income_regular_monthly: Decimal::from(1000),
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![SimAsset {
                id: Uuid::from_u128(11),
                value: Decimal::ZERO,
                purchase_price: Some(Decimal::from(1200)),
                is_liquid: true,
                expected_annual_return_percent: None,
                monthly_contribution_fixed: Decimal::ZERO,
                contribution_remainder_weight: Decimal::ONE,
            }],
            liabilities: vec![],
            inflation_annual_percent: Some(Decimal::from(12)),
            planning_monthly_cash_adjustment: vec![Decimal::ZERO],
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(1200));
        assert!(out.contributed_capital[1] < Decimal::from(2200));
    }
}
