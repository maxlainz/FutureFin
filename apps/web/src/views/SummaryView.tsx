import type { InstallationAccess, SummaryResponse } from "../api/types";
import { MetricCard } from "../components/MetricCard";
import {
  SummaryBreakdownBlock,
  SummaryDonutChart,
} from "../components/charts/summary";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatDebtToAssetsPct,
  formatFractionAsPercent,
  formatMonthsRough,
  formatPercentDisplay,
  isZeroFractionMetric,
  isZeroMoneyMetric,
  parseDisplayDecimal,
} from "../lib/format";

type LedgerPersonScope = "household" | "mine";

export function SummaryView({
  installation,
  loading,
  hasMembership,
  ledgerPersonScope,
  summary,
  summaryBusy,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const showMetrics =
    hasMembership && !loading && !summaryBusy && summary !== null;
  const currencyIso = installation?.installation.base_currency ?? "";

  const nw = showMetrics
    ? formatCurrencyAmount(summary.net_worth, currencyIso)
    : METRIC_DASH;
  const ta = showMetrics
    ? formatCurrencyAmount(summary.total_assets, currencyIso)
    : METRIC_DASH;
  const tl = showMetrics
    ? formatCurrencyAmount(summary.total_liabilities, currencyIso)
    : METRIC_DASH;
  const dta = showMetrics
    ? formatDebtToAssetsPct(summary.debt_to_assets_ratio)
    : METRIC_DASH;

  const fh = summary?.financial_health;
  const savingsMoneyParen =
    showMetrics && fh
      ? formatCurrencyAmount(fh.monthly_net_excluding_derived_debt, currencyIso)
      : "";
  const savingsMoneyPrimary =
    showMetrics && fh
      ? formatCurrencyAmount(fh.net_monthly_equivalent, currencyIso)
      : METRIC_DASH;
  const showSavingsMoneyTile =
    showMetrics &&
    fh &&
    (!isZeroMoneyMetric(fh.net_monthly_equivalent) ||
      !isZeroMoneyMetric(fh.monthly_net_excluding_derived_debt));
  const savingsMoneyParenthetical =
    showSavingsMoneyTile &&
    savingsMoneyPrimary !== savingsMoneyParen &&
    savingsMoneyParen !== ""
      ? savingsMoneyParen
      : undefined;

  let savingsRatePrimary = METRIC_DASH;
  let savingsRateParenthetical: string | undefined;
  if (showMetrics && fh) {
    const sr = formatFractionAsPercent(fh.savings_rate);
    const srx = formatFractionAsPercent(fh.savings_rate_excluding_derived_debt);
    const showPctTile =
      !isZeroFractionMetric(fh.savings_rate) ||
      !isZeroFractionMetric(fh.savings_rate_excluding_derived_debt);
    if (showPctTile) {
      if (sr !== METRIC_DASH) {
        savingsRatePrimary = sr;
        savingsRateParenthetical =
          srx !== METRIC_DASH && srx !== sr ? srx : undefined;
      } else {
        savingsRatePrimary = srx;
      }
    }
  }
  const showSavingsRateTile =
    showMetrics && fh && savingsRatePrimary !== METRIC_DASH;

  const financialHealthHasAnyTile =
    showMetrics &&
    fh &&
    (showSavingsMoneyTile ||
      showSavingsRateTile ||
      !isZeroMoneyMetric(fh.liquid_assets_total) ||
      !isZeroMoneyMetric(fh.runway_months) ||
      !isZeroFractionMetric(fh.upcoming_coverage_ratio));

  const liquidAssetsPctOfTotalAssets =
    showMetrics && summary && fh
      ? (() => {
          const liq = parseDisplayDecimal(fh.liquid_assets_total);
          const tot = parseDisplayDecimal(summary.total_assets);
          if (liq === null || tot === null || tot <= 0) return null;
          return formatPercentDisplay((liq / tot) * 100);
        })()
      : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Resumen</h2>
        <p className="workspace-sub">
          {loading
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      <div className="metric-grid workspace-kpi-strip">
        {!showMetrics ? (
          <>
            <MetricCard label="Patrimonio neto" value={nw} />
            <MetricCard label="Activos totales" value={ta} />
            <MetricCard label="Pasivos totales" value={tl} />
            <MetricCard label="Ratio deuda / activos" value={dta} />
          </>
        ) : (
          <>
            {!isZeroMoneyMetric(summary.net_worth) ? (
              <MetricCard label="Patrimonio neto" value={nw} />
            ) : null}
            {!isZeroMoneyMetric(summary.total_assets) ? (
              <MetricCard label="Activos totales" value={ta} />
            ) : null}
            {!isZeroMoneyMetric(summary.total_liabilities) ? (
              <MetricCard label="Pasivos totales" value={tl} />
            ) : null}
            {!isZeroFractionMetric(summary.debt_to_assets_ratio) ? (
              <MetricCard label="Ratio deuda / activos" value={dta} />
            ) : null}
          </>
        )}
      </div>

      <section className="panel">
        <h3 className="panel-title">Salud financiera</h3>
        {showMetrics ? (
          financialHealthHasAnyTile ? (
            <div className="metric-grid bordered-top">
              {showSavingsMoneyTile ? (
                <MetricCard
                  label="Ahorro mensual neto"
                  value={savingsMoneyPrimary}
                  parenthetical={savingsMoneyParenthetical}
                />
              ) : null}
              {showSavingsRateTile ? (
                <MetricCard
                  label="Tasa de ahorro"
                  value={savingsRatePrimary}
                  parenthetical={savingsRateParenthetical}
                />
              ) : null}
              {!isZeroMoneyMetric(summary.financial_health.liquid_assets_total) ? (
                <MetricCard
                  label="Activos líquidos"
                  value={formatCurrencyAmount(
                    summary.financial_health.liquid_assets_total,
                    currencyIso,
                  )}
                  parenthetical={
                    liquidAssetsPctOfTotalAssets ?? undefined
                  }
                />
              ) : null}
              {!isZeroMoneyMetric(summary.financial_health.runway_months) ? (
                <MetricCard
                  label="Runway"
                  value={formatMonthsRough(
                    summary.financial_health.runway_months,
                  )}
                />
              ) : null}
            </div>
          ) : (
            <p className="muted bordered-top">Sin datos.</p>
          )
        ) : (
          <p className="muted bordered-top">Sin acceso.</p>
        )}
      </section>

      <section className="panel">
        <h3 className="panel-title">Desglose</h3>
        {showMetrics && summary ? (
          <div className="summary-donuts-row bordered-top">
            <SummaryDonutChart
              title="Activos por categoría"
              currencyIso={currencyIso}
              chartScope="asset"
              totalWhole={summary.total_assets}
              rows={summary.assets_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
            <SummaryDonutChart
              title="Pasivos por categoría"
              currencyIso={currencyIso}
              chartScope="liability"
              totalWhole={summary.total_liabilities}
              rows={summary.liabilities_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
          </div>
        ) : null}
        {showMetrics && summary ? (
          <div className="breakdown-grid">
            <SummaryBreakdownBlock
              title="Activos por categoría"
              labelColumn="Categoría"
              chartScope="asset"
              totalWhole={summary.total_assets}
              currencyIso={currencyIso}
              rows={summary.assets_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
            <SummaryBreakdownBlock
              title="Pasivos por categoría"
              labelColumn="Categoría"
              chartScope="liability"
              totalWhole={summary.total_liabilities}
              currencyIso={currencyIso}
              rows={summary.liabilities_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
          </div>
        ) : (
          <p className="muted bordered-top">Sin acceso.</p>
        )}
      </section>
    </div>
  );
}
