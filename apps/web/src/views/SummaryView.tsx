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
  isAbsentMetric,
  isZeroFractionMetric,
  isZeroMoneyMetric,
  parseDisplayDecimal,
} from "../lib/format";
import { runwaySwrParenthetical } from "../lib/fire";
import { formatDeltaCurrency } from "../lib/expenses";

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

  // 3.9.0 — UNA sola cifra de ahorro por modo: la que usa la proyección.
  //
  // Antes convivían tres (el neto del modo, el neto del presupuesto y el promedio real bruto) y
  // en modo C ninguna era derivable de las otras. Ahora la tarjeta enseña el neto EFECTIVO como
  // valor, su tasa como detalle —misma base SIEMPRE, así que no pueden contradecirse— y el
  // contraste con el plan como tendencia. Cada base concreta se explica en el popup de ayuda.
  const savingsPrimary =
    showMetrics && fh
      ? formatCurrencyAmount(fh.net_monthly_equivalent, currencyIso)
      : METRIC_DASH;
  const showSavingsTile =
    showMetrics && fh !== undefined && !isZeroMoneyMetric(fh.net_monthly_equivalent);

  // Detalle: la tasa de ahorro, que por construcción comparte numerador y denominador con la
  // cifra de arriba (`net / income` del MISMO modo). El bug que abrió este rediseño era
  // justamente una tasa que mezclaba el neto híbrido con el ingreso del presupuesto.
  const savingsRateDetail = (() => {
    if (!showMetrics || !fh) return undefined;
    const pct = formatFractionAsPercent(fh.savings_rate);
    return pct === METRIC_DASH ? undefined : `${pct} de tus ingresos`;
  })();

  // Tendencia vs plan: solo cuando el neto efectivo NO es ya el del presupuesto (en modo A son
  // el mismo número y compararlo consigo mismo no dice nada).
  const savingsPlanTrend = (() => {
    if (!showMetrics || !fh?.savings_expected_monthly_equivalent) return undefined;
    const net = parseDisplayDecimal(fh.net_monthly_equivalent);
    const plan = parseDisplayDecimal(fh.savings_expected_monthly_equivalent);
    if (net === null || plan === null) return undefined;
    const delta = net - plan;
    if (Math.abs(delta) < 0.5) return undefined;
    const tone = delta > 0 ? "num-pos" : "num-neg";
    return (
      <span className="metric-trend">
        <span className={`metric-trend-arrow ${tone}`} aria-hidden>
          {delta > 0 ? "\u25B2" : "\u25BC"}
        </span>
        <span className={`metric-trend-delta ${tone}`}>
          {formatDeltaCurrency(delta, currencyIso)}
        </span>
        <span className="metric-trend-label">vs plan</span>
      </span>
    );
  })();

  // Runway: el servidor marca `runway_is_indefinite` (y `runway_months` a null) cuando la
  // retirada anual cabe en el SWR de `fire_settings` (pestaña Jubilación) — la tarjeta sigue
  // siendo relevante (es la mejor noticia posible) y el paréntesis explica el porqué.
  // El guard mira AUSENCIA, no cero: un runway de 0 meses es información (el peor caso posible).
  const runwayIsIndefinite = fh?.runway_is_indefinite === true;
  const showRunwayTile =
    showMetrics && fh && (runwayIsIndefinite || !isAbsentMetric(fh.runway_months));
  const runwayParenthetical = runwayIsIndefinite
    ? runwaySwrParenthetical(installation?.installation.fire_settings)
    : undefined;

  const financialHealthHasAnyTile =
    showMetrics &&
    fh &&
    (showSavingsTile ||
      !isZeroMoneyMetric(fh.liquid_assets_total) ||
      showRunwayTile);

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
            <MetricCard label="Patrimonio neto"
                  helpId="summary.net_worth" value={nw} />
            <MetricCard label="Activos totales" value={ta} />
            <MetricCard label="Pasivos totales" value={tl} />
            <MetricCard label="Ratio deuda / activos" value={dta} />
          </>
        ) : (
          <>
            {!isZeroMoneyMetric(summary.net_worth) ? (
              <MetricCard label="Patrimonio neto"
                  helpId="summary.net_worth" value={nw} />
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
              {showSavingsTile ? (
                <MetricCard
                  label="Ahorro mensual"
                  helpId="summary.savings"
                  value={savingsPrimary}
                  trend={savingsPlanTrend}
                  detail={savingsRateDetail}
                />
              ) : null}
              {!isZeroMoneyMetric(summary.financial_health.liquid_assets_total) ? (
                <MetricCard
                  label="Activos líquidos"
                  helpId="summary.liquid_assets"
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
                  label="Autonomía"
                  helpId="summary.runway"
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
