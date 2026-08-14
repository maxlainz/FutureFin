import type {
  InstallationAccess,
  ProjectionSeriesApi,
  SummaryResponse,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import {
  SummaryBreakdownBlock,
  SummaryDonutChart,
} from "../components/charts/summary";
import {
  MiniProjection,
  MiniProjectionLegend,
} from "../components/charts/MiniProjection";
import { ASSET_LINE_COLORS } from "../lib/projection-chart";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatDebtToAssetsPct,
  formatFractionAsPercent,
  formatPercentDisplay,
  formatRunwayValue,
  isZeroFractionMetric,
  isZeroMoneyMetric,
  parseDisplayDecimal,
} from "../lib/format";
import { savingsAvgParenthetical } from "../lib/fire";

type LedgerPersonScope = "household" | "mine";

export function SummaryView({
  installation,
  loading,
  hasMembership,
  ledgerPersonScope,
  summary,
  summaryBusy,
  projectionSeries,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
  projectionSeries: ProjectionSeriesApi | null;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const showMetrics =
    hasMembership && !loading && !summaryBusy && summary !== null;
  const currencyIso = installation?.installation.base_currency ?? "";

  // Valor de patrimonio al inicio y al final de la ventana de 12 meses (misma
  // que `months={12}` de la mini-gráfica). net_worth es f64 → formatCurrencyNumber.
  const projectionEnds =
    projectionSeries && projectionSeries.points.length > 0
      ? (() => {
          const pts = projectionSeries.points;
          const total = Math.min(12, pts.length);
          return {
            first: formatCurrencyNumber(pts[0]!.net_worth, currencyIso),
            last: formatCurrencyNumber(pts[total - 1]!.net_worth, currencyIso),
          };
        })()
      : null;

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

  // Modos B y C (promedio): el ahorro y la tasa vienen ya en base promedio ponderado desde
  // el servidor. En esos modos los paréntesis «sin deuda» pierden sentido (la base ya es
  // real) → los sustituimos por «promedio de N meses». En modo presupuesto: sin cambios.
  // La fuente efectiva vive dentro de `financial_health` (no en la raíz del summary).
  const avgParenthetical = showMetrics
    ? savingsAvgParenthetical(
        fh?.savings_source,
        fh?.savings_source_months_with_data,
      )
    : undefined;
  const isAvgMode = avgParenthetical !== undefined;

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
  const savingsMoneyParenthetical = isAvgMode
    ? avgParenthetical
    : showSavingsMoneyTile &&
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
        savingsRateParenthetical = isAvgMode
          ? avgParenthetical
          : srx !== METRIC_DASH && srx !== sr
            ? srx
            : undefined;
      } else {
        savingsRatePrimary = srx;
        if (isAvgMode) savingsRateParenthetical = avgParenthetical;
      }
    }
  }
  const showSavingsRateTile =
    showMetrics && fh && savingsRatePrimary !== METRIC_DASH;

  // Runway: con retorno suficiente el servidor manda `runway_is_indefinite` y `runway_months`
  // a null — la tarjeta sigue siendo relevante (es la mejor noticia posible).
  const runwayIsIndefinite = fh?.runway_is_indefinite === true;
  const showRunwayTile =
    showMetrics &&
    fh &&
    (runwayIsIndefinite || !isZeroMoneyMetric(fh.runway_months));
  const runwayParenthetical = runwayIsIndefinite
    ? "más de 100 años"
    : avgParenthetical;

  const financialHealthHasAnyTile =
    showMetrics &&
    fh &&
    (showSavingsMoneyTile ||
      showSavingsRateTile ||
      !isZeroMoneyMetric(fh.liquid_assets_total) ||
      showRunwayTile ||
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
              {showRunwayTile ? (
                <MetricCard
                  label="Runway"
                  value={formatRunwayValue(
                    summary.financial_health.runway_months,
                    summary.financial_health.runway_is_indefinite,
                  )}
                  parenthetical={runwayParenthetical}
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

      {hasMembership ? (
        <section className="panel">
          <div className="panel-head-row">
            <h3 className="panel-title">Proyección · 12 meses</h3>
            {projectionEnds ? (
              <span
                className="muted"
                style={{
                  fontSize: "0.78rem",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {projectionEnds.first} → {projectionEnds.last}
              </span>
            ) : null}
          </div>
          {projectionSeries && projectionSeries.points.length > 0 ? (
            <>
              <MiniProjection
                series={projectionSeries}
                months={12}
                height={170}
                showFire={false}
                showAreas={true}
                zoomY
              />
              <MiniProjectionLegend
                items={[
                  { label: "Patrimonio neto", color: "var(--proj-nw)" },
                  ...(projectionSeries.asset_series ?? []).map((a, idx) => ({
                    label: a.asset_name,
                    color: ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length]!,
                  })),
                ]}
              />
            </>
          ) : (
            <div className="ff-chart-skeleton ff-chart-skeleton--mini" aria-hidden />
          )}
        </section>
      ) : null}

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
        ) : hasMembership ? (
          <div className="summary-donuts-row bordered-top">
            <div className="ff-chart-skeleton ff-chart-skeleton--donut" aria-hidden />
            <div className="ff-chart-skeleton ff-chart-skeleton--donut" aria-hidden />
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
