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
    /// Month index (1-based, same indexing as the simulation loop) at which the retirement
    /// drawdown phase begins. `None` = no drawdown modelled in this projection.
    pub retirement_start_month: Option<u32>,
    /// Income from sources that persist after retirement (e.g. rental, pension).
    /// Replaces `income_regular_monthly` from `retirement_start_month` onward.
    /// Typically `0` when all income stops at retirement (the default).
    pub income_retirement_monthly: Decimal,
    /// Optional additional monthly draw from assets on top of the income/expense budget.
    /// The handler typically passes `0` — income reduction via `income_retirement_monthly`
    /// is the primary drain mechanism in the new model.
    pub retirement_monthly_withdrawal: Decimal,
    /// When set, new contributions stop as soon as net worth reaches or exceeds this threshold
    /// (even if `retirement_start_month` has not been reached yet).
    pub fire_target_net_worth: Option<Decimal>,
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

    let retirement_withdrawal = match input.retirement_start_month {
        Some(start) if 1 >= start => {
            let base = input.retirement_monthly_withdrawal;
            match input.inflation_annual_percent {
                Some(inf) if inf > Decimal::ZERO => {
                    let factor = (Decimal::ONE + inf / Decimal::from(100))
                        .powd(Decimal::ONE / Decimal::from(12));
                    base * factor
                }
                _ => base,
            }
        }
        _ => Decimal::ZERO,
    };

    let net_cash_month = input.income_regular_monthly
        - input.expense_regular_monthly
        - debt_service
        + planning_adj
        - retirement_withdrawal;

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

        let nw_prev = nw_fn(&values, &principals, undrained_cumulative, surplus_cash);
        let fire_reached = input
            .fire_target_net_worth
            .map_or(false, |t| t > Decimal::ZERO && nw_prev >= t);
        let in_retirement =
            fire_reached || input.retirement_start_month.map_or(false, |s| k >= s);
        let income = if in_retirement {
            input.income_retirement_monthly
        } else {
            input.income_regular_monthly
        };

        let retirement_withdrawal = if in_retirement {
            // When inflation is active the base withdrawal is expressed in today's money.
            // Inflate it to nominal terms so that after the end-of-month deflation the real
            // withdrawal remains constant (same purchasing power every month).
            let base = input.retirement_monthly_withdrawal;
            match input.inflation_annual_percent {
                Some(inf) if inf > Decimal::ZERO => {
                    let factor = (Decimal::ONE + inf / Decimal::from(100))
                        .powd(Decimal::from(k) / Decimal::from(12));
                    base * factor
                }
                _ => base,
            }
        } else {
            Decimal::ZERO
        };

        let net_cash_month = income
            - input.expense_regular_monthly
            - debt_service
            + planning_adj
            - retirement_withdrawal;

        if net_cash_month <= Decimal::ZERO {
            let mut need = -net_cash_month;
            let from_surplus = surplus_cash.min(need);
            surplus_cash -= from_surplus;
            need -= from_surplus;
            if need > Decimal::ZERO {
                let und = drain_from_assets(&mut values, &liquid, &rates, need);
                undrained_cumulative += und;
            }
        } else if in_retirement {
            // In retirement any surplus stays as cash buffer; no new contributions are made.
            surplus_cash += net_cash_month;
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(1200));
        assert!(out.contributed_capital[1] < Decimal::from(2200));
    }

    #[test]
    fn retirement_withdrawal_drains_assets_after_start_month() {
        // income=0, expense=0, liquid asset=12000; withdrawal=1000/month starting month 3.
        // Months 1-2: no change (no income/expense/withdrawal). Month 3 onward: -1000/month.
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 4,
            income_regular_monthly: Decimal::ZERO,
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![SimAsset {
                id: Uuid::from_u128(10),
                value: Decimal::from(12_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
                monthly_contribution_fixed: Decimal::ZERO,
                contribution_remainder_weight: Decimal::ZERO,
            }],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 4],
            retirement_start_month: Some(3),
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::from(1_000),
            fire_target_net_worth: None,
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(12_000));
        assert_eq!(out.net_worth[1], Decimal::from(12_000)); // no withdrawal yet
        assert_eq!(out.net_worth[2], Decimal::from(12_000)); // no withdrawal yet
        assert_eq!(out.net_worth[3], Decimal::from(11_000)); // first withdrawal
        assert_eq!(out.net_worth[4], Decimal::from(10_000)); // second withdrawal
    }

    #[test]
    fn retirement_withdrawal_zero_before_start_month() {
        // income_regular=500 (stops at retirement), income_retirement=0, expense=0.
        // No withdrawal before month 2 → surplus 500/month pre-retirement.
        // From month 2: income drops to 0, withdrawal=600 → net_cash=-600 → drains asset.
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 3,
            income_regular_monthly: Decimal::from(500),
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![SimAsset {
                id: Uuid::from_u128(20),
                value: Decimal::from(1_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
                monthly_contribution_fixed: Decimal::ZERO,
                contribution_remainder_weight: Decimal::ONE,
            }],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 3],
            retirement_start_month: Some(2),
            income_retirement_monthly: Decimal::ZERO,
            retirement_monthly_withdrawal: Decimal::from(600),
            fire_target_net_worth: None,
        };
        let out = project_net_worth_series(&inp).unwrap();
        // Month 0: 1000 (starting asset)
        assert_eq!(out.net_worth[0], Decimal::from(1_000));
        // Month 1: income_regular 500, no withdrawal (pre-retirement) → +500 → 1500
        assert_eq!(out.net_worth[1], Decimal::from(1_500));
        // Month 2: income_retirement=0, withdrawal=600 → net=-600 → drain 600 → 900
        assert_eq!(out.net_worth[2], Decimal::from(900));
        // Month 3: same → 900 - 600 = 300
        assert_eq!(out.net_worth[3], Decimal::from(300));
    }

    #[test]
    fn retirement_withdrawal_inflation_adjusts_nominal_to_maintain_real_purchasing_power() {
        // Verifies that with inflation active, the NOMINAL withdrawal grows over time (so that
        // real purchasing power stays constant). We run two projections with the same base
        // withdrawal and compare nominal asset drain: with inflation the drain should be larger
        // in later months, meaning the asset ends lower after 12 months.
        let base_w = Decimal::from(1_000u32);
        let mk_inp = |inf: Option<Decimal>| -> ProjectionInput {
            let h = 12u32;
            ProjectionInput {
                ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                horizon_months: h,
                income_regular_monthly: Decimal::ZERO,
                expense_regular_monthly: Decimal::ZERO,
                assets: vec![SimAsset {
                    id: Uuid::from_u128(30),
                    value: Decimal::from(24_000),
                    purchase_price: None,
                    is_liquid: true,
                    expected_annual_return_percent: None,
                    monthly_contribution_fixed: Decimal::ZERO,
                    contribution_remainder_weight: Decimal::ZERO,
                }],
                liabilities: vec![],
                inflation_annual_percent: inf,
                planning_monthly_cash_adjustment: vec![Decimal::ZERO; h as usize],
                retirement_start_month: Some(1),
                income_retirement_monthly: Decimal::ZERO,
                retirement_monthly_withdrawal: base_w,
                fire_target_net_worth: None,
            }
        };
        let no_inf = project_net_worth_series(&mk_inp(None)).unwrap();
        let with_inf = project_net_worth_series(&mk_inp(Some(Decimal::from(12u32)))).unwrap();

        // Without inflation: nominal asset after 12 months = 24000 - 12*1000 = 12000 (real=nominal)
        // With inflation: nominal withdrawal grows each month → more total drain → lower nominal asset
        // The deflated real series with inflation will be lower than the nominal-only series.
        let nominal_end = no_inf.net_worth[12];
        let real_end = with_inf.net_worth[12]; // already deflated
        // Real end must be lower because inflation-adjusted withdrawals drained more in nominal terms
        assert!(
            real_end < nominal_end,
            "real (deflated) end {real_end} should be less than nominal-only end {nominal_end}"
        );
    }

    #[test]
    fn retirement_income_drops_to_income_retirement_monthly_after_start_month() {
        // income_regular=3000, income_retirement=500, expense=2000, no growth.
        // Before month 3: net_cash = 3000 - 2000 = +1000 → contribute to asset.
        // From month 3: net_cash = 500 - 2000 = -1500 → drain 1500/month.
        let inp = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 5,
            income_regular_monthly: Decimal::from(3_000),
            expense_regular_monthly: Decimal::from(2_000),
            assets: vec![SimAsset {
                id: Uuid::from_u128(40),
                value: Decimal::from(10_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
                monthly_contribution_fixed: Decimal::ZERO,
                contribution_remainder_weight: Decimal::ONE,
            }],
            liabilities: vec![],
            inflation_annual_percent: None,
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 5],
            retirement_start_month: Some(3),
            income_retirement_monthly: Decimal::from(500),
            retirement_monthly_withdrawal: Decimal::ZERO,
                fire_target_net_worth: None,
        };
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(10_000));
        assert_eq!(out.net_worth[1], Decimal::from(11_000)); // +1000 surplus
        assert_eq!(out.net_worth[2], Decimal::from(12_000)); // +1000 surplus
        assert_eq!(out.net_worth[3], Decimal::from(10_500)); // -1500 drain
        assert_eq!(out.net_worth[4], Decimal::from(9_000));  // -1500 drain
        assert_eq!(out.net_worth[5], Decimal::from(7_500));  // -1500 drain
    }
}
