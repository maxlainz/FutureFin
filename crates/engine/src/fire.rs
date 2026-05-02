//! FIRE milestone: compare projected portfolio / net worth to phase targets derived from
//! withdrawal rates, safety multiplier, and capital-gains tax on the gain slice of withdrawals.

use rust_decimal::Decimal;

/// Progressive band: tax `rate_percent` applies to taxable income in `[band_start, next_band_start)`.
#[derive(Debug, Clone)]
pub struct TaxBand {
    pub band_start: Decimal,
    pub rate_percent: Decimal,
}

/// Income tax on taxable income using contiguous ascending bands (same shape as CG tranches on gains).
pub fn progressive_tax_on_amount(income: Decimal, bands: &[TaxBand]) -> Decimal {
    if income <= Decimal::ZERO || bands.is_empty() {
        return Decimal::ZERO;
    }
    let mut sorted: Vec<&TaxBand> = bands.iter().collect();
    sorted.sort_by(|a, b| a.band_start.cmp(&b.band_start));
    let mut tax = Decimal::ZERO;
    for i in 0..sorted.len() {
        let start = sorted[i].band_start;
        let taxable_in_band = match sorted.get(i + 1) {
            Some(next) => (income.min(next.band_start) - start).max(Decimal::ZERO),
            None => (income - start).max(Decimal::ZERO),
        };
        if taxable_in_band > Decimal::ZERO {
            tax += taxable_in_band * sorted[i].rate_percent / Decimal::from(100);
        }
    }
    tax
}

/// Annual net cash flow after taxing only the gain portion of a gross portfolio withdrawal.
#[inline]
pub fn net_after_capital_gains_on_withdrawal(
    gross_withdrawal: Decimal,
    taxable_gain_ratio: Decimal,
    cgt_bands: &[TaxBand],
) -> Decimal {
    let taxable = gross_withdrawal * taxable_gain_ratio;
    let tax = progressive_tax_on_amount(taxable, cgt_bands);
    gross_withdrawal - tax
}

/// Classic rule: annual expense / SWR × safety multiplier (no taxes).
pub fn portfolio_target_simple(
    annual_expense: Decimal,
    withdrawal_rate_percent: Decimal,
    safety_multiplier: Decimal,
) -> Decimal {
    let wr = withdrawal_rate_percent / Decimal::from(100);
    if wr <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (annual_expense / wr) * safety_multiplier
}

/// Gross annual withdrawal (from portfolio) needed so net after CGT meets `net_annual_need`.
pub fn gross_withdrawal_for_net_after_cgt(
    net_annual_need: Decimal,
    taxable_gain_ratio: Decimal,
    cgt_bands: &[TaxBand],
) -> Decimal {
    if net_annual_need <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let mut lo = Decimal::ZERO;
    let mut hi = net_annual_need * Decimal::from(10);
    for _ in 0..96 {
        let mid = (lo + hi) / Decimal::from(2);
        let net = net_after_capital_gains_on_withdrawal(mid, taxable_gain_ratio, cgt_bands);
        if net >= net_annual_need {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Portfolio principal needed so that `withdrawal_rate_percent × portfolio` produces enough gross
/// withdrawal (after CGT on the gain fraction) to cover `net_annual_need`, times `safety_multiplier`.
pub fn portfolio_target_for_net_annual_need(
    net_annual_need: Decimal,
    withdrawal_rate_percent: Decimal,
    taxable_gain_ratio: Decimal,
    cgt_bands: &[TaxBand],
    safety_multiplier: Decimal,
) -> Decimal {
    let wr = withdrawal_rate_percent / Decimal::from(100);
    if wr <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let gross = gross_withdrawal_for_net_after_cgt(net_annual_need, taxable_gain_ratio, cgt_bands);
    (gross / wr) * safety_multiplier
}

#[derive(Debug, Clone)]
pub struct FireMilestoneInput {
    pub net_worth_series: Vec<Decimal>,
    pub required_portfolio_series: Vec<Decimal>,
}

#[derive(Debug, Clone)]
pub struct FireMilestoneOutput {
    pub reached: bool,
    pub reach_month_index: Option<u32>,
}

/// First month index where projected NW meets or exceeds the required portfolio for that month.
pub fn fire_milestone_month(input: &FireMilestoneInput) -> FireMilestoneOutput {
    let n = input.net_worth_series.len().min(input.required_portfolio_series.len());
    for i in 0..n {
        if input.net_worth_series[i] >= input.required_portfolio_series[i] {
            return FireMilestoneOutput {
                reached: true,
                reach_month_index: Some(i as u32),
            };
        }
    }
    FireMilestoneOutput {
        reached: false,
        reach_month_index: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn es_flat_cgt(rate: Decimal) -> Vec<TaxBand> {
        vec![TaxBand {
            band_start: Decimal::ZERO,
            rate_percent: rate,
        }]
    }

    #[test]
    fn portfolio_target_simple_four_percent_rule() {
        let t = portfolio_target_simple(Decimal::from(40_000), Decimal::from(4), Decimal::ONE);
        assert_eq!(t, Decimal::from(1_000_000));
    }

    #[test]
    fn taxes_only_gain_portion_flat_rate() {
        let bands = es_flat_cgt(Decimal::from(20));
        let ratio = Decimal::new(5, 1); // 0.5
        let gross = Decimal::from(1000);
        let net = net_after_capital_gains_on_withdrawal(gross, ratio, &bands);
        // Tax on 500 at 20% = 100 → net 900
        assert_eq!(net, Decimal::from(900));
    }

    #[test]
    fn higher_cgt_raises_required_portfolio() {
        let bands_low = es_flat_cgt(Decimal::from(10));
        let bands_high = es_flat_cgt(Decimal::from(25));
        let target_low = portfolio_target_for_net_annual_need(
            Decimal::from(40_000),
            Decimal::from(4),
            Decimal::new(5, 1),
            &bands_low,
            Decimal::ONE,
        );
        let target_high = portfolio_target_for_net_annual_need(
            Decimal::from(40_000),
            Decimal::from(4),
            Decimal::new(5, 1),
            &bands_high,
            Decimal::ONE,
        );
        assert!(target_high > target_low);
    }

    #[test]
    fn fire_milestone_first_crossing() {
        let out = fire_milestone_month(&FireMilestoneInput {
            net_worth_series: vec![
                Decimal::from(100),
                Decimal::from(200),
                Decimal::from(500),
            ],
            required_portfolio_series: vec![
                Decimal::from(1000),
                Decimal::from(400),
                Decimal::from(400),
            ],
        });
        assert!(out.reached);
        assert_eq!(out.reach_month_index, Some(2));
    }

    #[test]
    fn fire_milestone_not_reached() {
        let out = fire_milestone_month(&FireMilestoneInput {
            net_worth_series: vec![Decimal::from(100), Decimal::from(200)],
            required_portfolio_series: vec![Decimal::from(300), Decimal::from(400)],
        });
        assert!(!out.reached);
        assert_eq!(out.reach_month_index, None);
    }
}
