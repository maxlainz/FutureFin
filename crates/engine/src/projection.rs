//! Monthly projection: regular budget (no derived liability rows) + active debt service +
//! asset contributions / drain / compound growth. Ajustes opcionales por mes desde «Próximos»
//! (`planning_monthly_cash_adjustment`) suman al flujo de caja recurrente del mes (ingreso (+)
//! / gasto (−)) antes del reparto a activos o el drenaje.
//!
//! El reparto del sobrante mensual se hace mediante una **cascada de reglas**
//! ([`AllocationRule`]) ejecutadas en orden ascendente. Cada regla consume parte del sobrante
//! para un activo destino hasta su tope opcional; lo que queda pasa a la siguiente regla.

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
    #[error("allocation_rules contains an out-of-bounds target_index")]
    InvalidAllocationRuleTarget,
    #[error("history timeline dates must be strictly ascending")]
    InvalidHistoryTimeline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimAsset {
    pub id: Uuid,
    pub value: Decimal,
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    /// Expected annual return % (e.g. 7 for 7%). None → no compound growth (factor 1).
    pub expected_annual_return_percent: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AllocationKind {
    /// Fixed € amount per month (capped at remaining surplus).
    Fixed,
    /// Percentage (0..=100) of the surplus *remaining at this step* of the cascade.
    Percent,
    /// Whatever surplus remains. Exactly one such rule is enforced by the handler.
    Remainder,
}

/// Optional ceiling on the destination asset. When the asset's current value reaches the
/// resolved ceiling, the rule contributes 0 and the cascade continues with the next rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AllocationCap {
    /// Absolute € target value for the destination asset.
    Amount(Decimal),
    /// Cap = N × (monthly expense + debt service). N is stored here.
    MonthsExpense(Decimal),
    /// Cap = N × monthly income. N is stored here.
    IncomeMultiple(Decimal),
}

#[derive(Debug, Clone)]
pub struct AllocationRule {
    /// Index into [`ProjectionInput::assets`]. The handler resolves UUIDs into indices.
    pub target_index: usize,
    pub kind: AllocationKind,
    /// `Fixed` → €/mes. `Percent` → 0..=100. `Remainder` → ignored (use `None`).
    pub amount: Option<Decimal>,
    pub cap: Option<AllocationCap>,
}

#[derive(Debug, Clone)]
pub struct ProjectionLiabilityInput {
    pub principal: Decimal,
    pub monthly_payment: Decimal,
    pub payment_end: Option<NaiveDate>,
}

/// Target FIRE evaluado mes a mes. `base_amount` es el patrimonio necesario en euros de hoy
/// (gross-up de impuestos ya aplicado); el target del mes `k` es
/// `base_amount × (1 + annual_inflation_percent/100)^((k-1)/12)`, lo que preserva el poder
/// adquisitivo del usuario en el momento de la jubilación. `annual_inflation_percent = 0`
/// degenera a un target plano (mismo valor en todos los meses).
#[derive(Debug, Clone)]
pub struct FireTarget {
    pub base_amount: Decimal,
    pub annual_inflation_percent: Decimal,
}

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    /// Civil "today" de la instalación (inicio del mes simulado para el índice 1).
    pub ref_date: NaiveDate,
    pub horizon_months: u32,
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    /// Cascada de reglas, en orden ascendente de prioridad (índice 0 = primera).
    pub allocation_rules: Vec<AllocationRule>,
    pub liabilities: Vec<ProjectionLiabilityInput>,
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
    /// Expenses from sources that continue after retirement (i.e. entries where
    /// `ends_at_retirement = false`). Replaces `expense_regular_monthly` from
    /// `retirement_start_month` onward. Equal to `expense_regular_monthly` when
    /// no expense ends at retirement.
    pub expense_retirement_monthly: Decimal,
    /// Optional additional monthly draw from assets on top of the income/expense budget.
    /// The handler typically passes `0` — income reduction via `income_retirement_monthly`
    /// is the primary drain mechanism in the new model.
    pub retirement_monthly_withdrawal: Decimal,
    /// Target FIRE móvil. Cuando está presente, las aportaciones se detienen en el primer mes
    /// donde el patrimonio cruza el target inflado correspondiente al mes en curso.
    pub fire_target: Option<FireTarget>,
}

#[derive(Debug, Clone)]
pub struct ProjectionOutput {
    /// Month index 0..=horizon_months inclusive. Serie nominal: el patrimonio se expresa en euros
    /// del momento. La comparación con la inflación se hace contra el target FIRE móvil
    /// (`FireTarget`), no deflactando esta serie.
    pub net_worth: Vec<Decimal>,
    /// Cumulative contributed basis (nominal).
    pub contributed_capital: Vec<Decimal>,
    /// Per-asset value series: `per_asset_series[asset_index][month_index]`.
    /// Length == `assets.len()`; inner length == `horizon_months + 1` (months 0..=horizon).
    /// Nominal, igual que `net_worth`.
    pub per_asset_series: Vec<Vec<Decimal>>,
}

/// Primero-de-mes de una fecha (día 1 del mismo mes). Compartido con `history.rs`.
pub(crate) fn month_first_calendar(d: NaiveDate) -> NaiveDate {
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
    (Decimal::ONE + p / Decimal::from(100)).powd(Decimal::ONE / Decimal::from(12))
}

/// Target FIRE en el `month_index` indicado (0 = punto de partida, 12 = un año después, etc.),
/// con inflación anual compuesta capitalizada en pasos de 1/12 de año. `month_index = 0` devuelve
/// el `base_amount`. Devuelve `None` cuando no hay target o su base es ≤ 0.
///
/// Es la **única fuente de verdad**: tanto el motor (para decidir `fire_reached`) como el
/// handler de la API (para construir `fire_target_series`) la consumen, evitando off-by-one
/// entre la serie y el cruce.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal> {
    let ft = ft?;
    if ft.base_amount <= Decimal::ZERO {
        return None;
    }
    if ft.annual_inflation_percent <= Decimal::ZERO || month_index == 0 {
        return Some(ft.base_amount);
    }
    let years = Decimal::from(month_index) / Decimal::from(12u32);
    let factor = (Decimal::ONE + ft.annual_inflation_percent / Decimal::from(100u32)).powd(years);
    Some(ft.base_amount * factor)
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

/// Resolve a rule's cap into an absolute € ceiling for the destination asset.
/// Returns `None` for an uncapped rule.
fn resolve_cap_ceiling(
    cap: Option<AllocationCap>,
    monthly_expense_with_debt: Decimal,
    monthly_income: Decimal,
) -> Option<Decimal> {
    match cap {
        None => None,
        Some(AllocationCap::Amount(v)) => Some(v.max(Decimal::ZERO)),
        Some(AllocationCap::MonthsExpense(n)) => {
            Some((n.max(Decimal::ZERO) * monthly_expense_with_debt).max(Decimal::ZERO))
        }
        Some(AllocationCap::IncomeMultiple(n)) => {
            Some((n.max(Decimal::ZERO) * monthly_income).max(Decimal::ZERO))
        }
    }
}

/// Cascade-distribute a positive `pool` across assets following the ordered `rules`.
///
/// For each rule (in order):
/// - resolve the destination asset's cap_room (`ceiling − current_value`); rule is skipped if 0.
/// - compute the rule's intent:
///   - `Fixed`     → `min(amount, remaining)`
///   - `Percent`   → `remaining × amount / 100` (over what's left at this step)
///   - `Remainder` → `remaining`
/// - take `min(intent, cap_room?, remaining)`, add to `alloc[target]`, subtract from `remaining`.
///
/// Returns `(alloc, leftover)`: `alloc[i]` ≥ 0 added to asset `i`; `leftover` is the pool that
/// no rule absorbed (caller routes it to `surplus_cash`).
fn distribute_contributions(
    pool: Decimal,
    rules: &[AllocationRule],
    values: &[Decimal],
    monthly_expense_with_debt: Decimal,
    monthly_income: Decimal,
) -> (Vec<Decimal>, Decimal) {
    let n = values.len();
    let mut alloc = vec![Decimal::ZERO; n];
    if pool <= Decimal::ZERO || n == 0 {
        return (alloc, pool.max(Decimal::ZERO));
    }
    let mut remaining = pool;
    // Live view of asset values for cap calculations as the cascade progresses (so multiple
    // rules into the same asset respect a shared ceiling).
    let mut live_values: Vec<Decimal> = values.to_vec();

    for rule in rules {
        if remaining <= Decimal::ZERO {
            break;
        }
        let target = rule.target_index;
        if target >= n {
            continue;
        }
        let ceiling = resolve_cap_ceiling(rule.cap, monthly_expense_with_debt, monthly_income);
        let cap_room = match ceiling {
            None => None,
            Some(c) => Some((c - live_values[target]).max(Decimal::ZERO)),
        };
        if let Some(room) = cap_room {
            if room <= Decimal::ZERO {
                continue;
            }
        }
        let intent = match rule.kind {
            AllocationKind::Fixed => rule.amount.unwrap_or(Decimal::ZERO).max(Decimal::ZERO),
            AllocationKind::Percent => {
                let pct = rule.amount.unwrap_or(Decimal::ZERO).max(Decimal::ZERO);
                (remaining * pct) / Decimal::from(100)
            }
            AllocationKind::Remainder => remaining,
        };
        let mut take = intent.min(remaining);
        if let Some(room) = cap_room {
            take = take.min(room);
        }
        if take <= Decimal::ZERO {
            continue;
        }
        alloc[target] += take;
        live_values[target] += take;
        remaining -= take;
    }

    (alloc, remaining.max(Decimal::ZERO))
}

/// Nominal contributions routed to each asset in the **first simulated month** (calendar month de
/// `ref_date`): cascada de reglas sobre el sobrante recurrente del mes. Cero si el superávit
/// es ≤ 0.
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
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }

    let values: Vec<Decimal> = input.assets.iter().map(|a| a.value).collect();
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
        Some(start) if 1 >= start => input.retirement_monthly_withdrawal,
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

    let (alloc, _leftover) = distribute_contributions(
        net_cash_month,
        &input.allocation_rules,
        &values,
        input.expense_regular_monthly + debt_service,
        input.income_regular_monthly,
    );
    for i in 0..n {
        out[i] = alloc[i];
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
    let n = input.assets.len();
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }

    let mut values: Vec<Decimal> = input.assets.iter().map(|a| a.value).collect();
    let liquid: Vec<bool> = input.assets.iter().map(|a| a.is_liquid).collect();
    let rates: Vec<Option<Decimal>> = input
        .assets
        .iter()
        .map(|a| a.expected_annual_return_percent)
        .collect();

    let mut principals: Vec<Decimal> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(Decimal::ZERO))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);

    let mut net_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut contrib_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut per_asset_series: Vec<Vec<Decimal>> = input
        .assets
        .iter()
        .map(|_| Vec::with_capacity(input.horizon_months as usize + 1))
        .collect();

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
    for (i, s) in per_asset_series.iter_mut().enumerate() {
        s.push(values[i]);
    }

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
        // `nw_prev` es el patrimonio al cierre del mes k-1; lo comparamos contra el target
        // correspondiente a ese mismo punto del eje temporal.
        let fire_reached = fire_target_at_month_index(input.fire_target.as_ref(), k - 1)
            .map_or(false, |t| nw_prev >= t);
        let in_retirement =
            fire_reached || input.retirement_start_month.map_or(false, |s| k >= s);
        let income = if in_retirement {
            input.income_retirement_monthly
        } else {
            input.income_regular_monthly
        };
        let expense = if in_retirement {
            input.expense_retirement_monthly
        } else {
            input.expense_regular_monthly
        };

        let retirement_withdrawal = if in_retirement {
            input.retirement_monthly_withdrawal
        } else {
            Decimal::ZERO
        };

        let net_cash_month = income
            - expense
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
            let (alloc, leftover) = distribute_contributions(
                net_cash_month,
                &input.allocation_rules,
                &values,
                expense + debt_service,
                income,
            );
            for i in 0..values.len() {
                values[i] += alloc[i];
                contributed_cumulative += alloc[i];
            }
            if leftover > Decimal::ZERO {
                surplus_cash += leftover;
                contributed_cumulative += leftover;
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

        let nw = nw_fn(
            &values,
            &principals,
            undrained_cumulative,
            surplus_cash,
        );
        net_series.push(nw);
        contrib_series.push(contributed_cumulative);
        for (i, s) in per_asset_series.iter_mut().enumerate() {
            s.push(values[i]);
        }
    }

    Ok(ProjectionOutput {
        net_worth: net_series,
        contributed_capital: contrib_series,
        per_asset_series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn mk_asset(id: u128, value: Decimal, liquid: bool, rate: Option<Decimal>) -> SimAsset {
        SimAsset {
            id: Uuid::from_u128(id),
            value,
            purchase_price: None,
            is_liquid: liquid,
            expected_annual_return_percent: rate,
        }
    }

    fn rule_remainder(target: usize) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Remainder,
            amount: None,
            cap: None,
        }
    }

    fn rule_fixed(target: usize, amount: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Fixed,
            amount: Some(amount),
            cap,
        }
    }

    fn rule_percent(target: usize, pct: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Percent,
            amount: Some(pct),
            cap,
        }
    }

    fn base_input(
        horizon: u32,
        income: Decimal,
        expense: Decimal,
        assets: Vec<SimAsset>,
        rules: Vec<AllocationRule>,
    ) -> ProjectionInput {
        ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: horizon,
            income_regular_monthly: income,
            expense_regular_monthly: expense,
            assets,
            allocation_rules: rules,
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: expense,
            retirement_monthly_withdrawal: Decimal::ZERO,
            fire_target: None,
        }
    }

    #[test]
    fn no_rules_routes_surplus_to_cash() {
        // Sin reglas: el sobrante queda como caja (surplus_cash) y entra al NW.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(3, Decimal::from(3000), Decimal::from(1000), vec![a], vec![]);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::ZERO);
        assert_eq!(out.net_worth[1], Decimal::from(2000));
        assert_eq!(out.net_worth[2], Decimal::from(4000));
        assert_eq!(out.net_worth[3], Decimal::from(6000));
    }

    #[test]
    fn single_remainder_rule_routes_full_surplus_to_target() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(0)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.per_asset_series[0][1], Decimal::from(1000));
    }

    #[test]
    fn fixed_amount_greater_than_surplus_clips_to_remaining() {
        // Regla pide 1500€ pero solo hay 1000€ → recibe 1000€ y resto = 0.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_fixed(0, Decimal::from(1500), None)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
    }

    #[test]
    fn cascade_fixed_then_remainder_splits_correctly() {
        // 1: fija 200 a A; 2: resto a B. Sobrante 1000 → A=200, B=800.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_fixed(0, Decimal::from(200), None),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(200));
        assert_eq!(nom[1], Decimal::from(800));
    }

    #[test]
    fn percent_rule_applies_to_remaining_at_step() {
        // Sobrante 1000. R1 fija 200 → quedan 800. R2 percent 50% → 400 a B. R3 remainder → 400 a C.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let c = mk_asset(3, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b, c],
            vec![
                rule_fixed(0, Decimal::from(200), None),
                rule_percent(1, Decimal::from(50), None),
                rule_remainder(2),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(200));
        assert_eq!(nom[1], Decimal::from(400));
        assert_eq!(nom[2], Decimal::from(400));
    }

    #[test]
    fn cap_amount_skips_rule_when_asset_at_ceiling() {
        // A ya está a 1000 con cap 1000 → su regla aporta 0. Toda la pasta va a B.
        let a = mk_asset(1, Decimal::from(1000), true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::Amount(Decimal::from(1000)))),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::ZERO);
        assert_eq!(nom[1], Decimal::from(1000));
    }

    #[test]
    fn cap_amount_clips_partial_room() {
        // A a 500, cap 1000 → 500 de room. Regla pide todo (remainder) pero clipa a 500. B remainder.
        let a = mk_asset(1, Decimal::from(500), true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::Amount(Decimal::from(1000)))),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(500));
        assert_eq!(nom[1], Decimal::from(1500));
    }

    #[test]
    fn cap_months_expense_resolves_against_monthly_total() {
        // Gasto 600€/mes. Cap = 2 meses = 1200. A vacío → 1200 de room.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::from(600),
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::MonthsExpense(Decimal::from(2)))),
                rule_remainder(1),
            ],
        );
        // Sobrante = 2000 - 600 = 1400. A se llena a 1200 (cap), B recibe 200.
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1200));
        assert_eq!(nom[1], Decimal::from(200));
    }

    #[test]
    fn cap_income_multiple_resolves_against_monthly_income() {
        // Ingreso 2000€/mes. Cap = 0.5× = 1000.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(
                    0,
                    Decimal::from(100),
                    Some(AllocationCap::IncomeMultiple(Decimal::new(5, 1))), // 0.5
                ),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
        assert_eq!(nom[1], Decimal::from(1000));
    }

    #[test]
    fn multi_rules_same_asset_share_ceiling() {
        // R1: fija 300 a A (cap 500). R2: remainder a A (cap 500).
        // Sobrante 1000. R1 pone 300, A=300, room=200. R2 quiere todo (700) pero room=200 → 200.
        // Quedan 500 sin asignar → surplus_cash. Sin regla remainder a otro, ese 500 va a cash.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![
                rule_fixed(0, Decimal::from(300), Some(AllocationCap::Amount(Decimal::from(500)))),
                AllocationRule {
                    target_index: 0,
                    kind: AllocationKind::Remainder,
                    amount: None,
                    cap: Some(AllocationCap::Amount(Decimal::from(500))),
                },
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(500));
        let out = project_net_worth_series(&inp).unwrap();
        // A llega a 500. Sobran 500 → surplus_cash. NW = 500 + 500 = 1000.
        assert_eq!(out.net_worth[1], Decimal::from(1000));
    }

    #[test]
    fn surplus_zero_yields_zero_contributions() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::from(1000),
            vec![a],
            vec![rule_remainder(0)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::ZERO);
    }

    #[test]
    fn invalid_target_index_errors() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(5)], // out of bounds
        );
        match project_net_worth_series(&inp) {
            Err(EngineError::InvalidAllocationRuleTarget) => {}
            other => panic!("expected InvalidAllocationRuleTarget, got {other:?}"),
        }
    }

    #[test]
    fn planning_adjustment_boosts_first_month_contribution_nominals() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let mut inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(0)],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::from(250)];
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1250));
    }

    #[test]
    fn contributed_capital_month_zero_includes_purchase_prices() {
        let mut inp = base_input(
            2,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(99),
                value: Decimal::from(100_000),
                purchase_price: Some(Decimal::from(80_000)),
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 2];
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[1], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[2], Decimal::from(80_000));
    }

    #[test]
    fn contributed_capital_tracks_purchase_basis_plus_routed_surplus() {
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(11),
                value: Decimal::ZERO,
                purchase_price: Some(Decimal::from(1200)),
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![rule_remainder(0)],
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(1200));
        assert_eq!(out.contributed_capital[1], Decimal::from(2200));
    }

    #[test]
    fn retirement_withdrawal_drains_assets_after_start_month() {
        let mut inp = base_input(
            4,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(10),
                value: Decimal::from(12_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![],
        );
        inp.retirement_start_month = Some(3);
        inp.retirement_monthly_withdrawal = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(12_000));
        assert_eq!(out.net_worth[1], Decimal::from(12_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(11_000));
        assert_eq!(out.net_worth[4], Decimal::from(10_000));
    }

    #[test]
    fn retirement_income_drops_to_income_retirement_monthly_after_start_month() {
        let mut inp = base_input(
            5,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![SimAsset {
                id: Uuid::from_u128(40),
                value: Decimal::from(10_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![rule_remainder(0)],
        );
        inp.retirement_start_month = Some(3);
        inp.income_retirement_monthly = Decimal::from(500);
        inp.expense_retirement_monthly = Decimal::from(2_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(10_000));
        assert_eq!(out.net_worth[1], Decimal::from(11_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(10_500));
        assert_eq!(out.net_worth[4], Decimal::from(9_000));
        assert_eq!(out.net_worth[5], Decimal::from(7_500));
    }

    #[test]
    fn fire_target_with_inflation_does_not_trigger_early_drain() {
        let mut inp = base_input(
            240,
            Decimal::from(1_000),
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(99),
                value: Decimal::ZERO,
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(7)),
            }],
            vec![rule_remainder(0)],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 240];
        inp.fire_target = Some(FireTarget {
            base_amount: Decimal::from(50_000),
            annual_inflation_percent: Decimal::from(10),
        });
        let out = project_net_worth_series(&inp).unwrap();
        let cross_idx = out
            .net_worth
            .iter()
            .enumerate()
            .find(|(k, v)| {
                let target = fire_target_at_month_index(inp.fire_target.as_ref(), *k as u32)
                    .unwrap_or(Decimal::ZERO);
                **v >= target && target > Decimal::ZERO
            })
            .map(|(k, _)| k)
            .expect("debe cruzar el target móvil en el horizonte");
        for i in 1..=cross_idx {
            assert!(
                out.net_worth[i] >= out.net_worth[i - 1] - Decimal::new(1, 2),
                "serie cayó en mes {i} (prev={}, curr={}) — fire_reached prematuro",
                out.net_worth[i - 1],
                out.net_worth[i]
            );
        }
    }

    #[test]
    fn asset_growth_is_nominal_independent_of_currency_value() {
        let inp = base_input(
            12,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(40),
                value: Decimal::from(50_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(5)),
            }],
            vec![],
        );
        let out = project_net_worth_series(&inp).unwrap();
        let expected = Decimal::from(52_500);
        let diff = (out.net_worth[12] - expected).abs();
        assert!(
            diff < Decimal::new(50, 2),
            "esperado ≈ {expected}, obtenido {} (diff {diff})",
            out.net_worth[12]
        );
    }

    #[test]
    fn fire_target_grows_with_inflation_each_month() {
        let ft = FireTarget {
            base_amount: Decimal::from(750_000),
            annual_inflation_percent: Decimal::from(3),
        };
        let t0 = fire_target_at_month_index(Some(&ft), 0).unwrap();
        assert_eq!(t0, Decimal::from(750_000));
        let t20y = fire_target_at_month_index(Some(&ft), 240).unwrap();
        // 750_000 × 1.03^20 ≈ 1_354_583. Comprobamos con tolerancia ≤ 1€.
        let factor = (Decimal::ONE + Decimal::from(3) / Decimal::from(100u32))
            .powd(Decimal::from(20u32));
        let expected = Decimal::from(750_000) * factor;
        let diff = (t20y - expected).abs();
        assert!(
            diff < Decimal::ONE,
            "esperado ≈ {expected}, obtenido {t20y} (diff {diff})"
        );
        // El target a 10 años está estrictamente entre el base y el de 20 años.
        let t10y = fire_target_at_month_index(Some(&ft), 120).unwrap();
        assert!(t10y > Decimal::from(750_000));
        assert!(t10y < t20y);
    }

    #[test]
    fn fire_target_with_zero_inflation_is_flat() {
        let ft = FireTarget {
            base_amount: Decimal::from(500_000),
            annual_inflation_percent: Decimal::ZERO,
        };
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 0).unwrap(),
            Decimal::from(500_000)
        );
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 600).unwrap(),
            Decimal::from(500_000)
        );
    }

    /// Regresión Fase 1.5: con el helper nuevo, el target en month_index=12 corresponde a 1 año
    /// completo de inflación. Antes, el handler usaba `month_index/12` y el motor `(k-1)/12` —
    /// había una diferencia de un mes entre la serie que se devolvía al cliente y el cruce que
    /// disparaba `fire_reached`. Ahora ambos consumen `fire_target_at_month_index`.
    #[test]
    fn fire_target_helper_matches_compound_factor_at_year_boundaries() {
        let ft = FireTarget {
            base_amount: Decimal::from(100_000),
            annual_inflation_percent: Decimal::from(5),
        };
        let r = Decimal::ONE + Decimal::from(5) / Decimal::from(100u32);

        let t1y = fire_target_at_month_index(Some(&ft), 12).unwrap();
        let expected_1y = Decimal::from(100_000) * r;
        assert!(
            (t1y - expected_1y).abs() < Decimal::new(1, 4),
            "month_index=12 → 1 año compuesto"
        );

        let t5y = fire_target_at_month_index(Some(&ft), 60).unwrap();
        let expected_5y = Decimal::from(100_000) * r.powd(Decimal::from(5u32));
        assert!(
            (t5y - expected_5y).abs() < Decimal::new(1, 4),
            "month_index=60 → 5 años compuesto"
        );
    }
}
