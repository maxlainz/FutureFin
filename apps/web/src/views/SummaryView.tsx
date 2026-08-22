import type {
  InstallationAccess,
  ProjectionSeriesApi,
  SummaryResponse,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
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
  onAddFirstAsset,
  onAddFirstBudgetEntry,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
  projectionSeries: ProjectionSeriesApi | null;
  /**
   * Lleva al formulario de nuevo activo (pestaña Activos con el modal ya abierto). El Resumen
   * no tiene formularios propios, así que su estado vacío delega en el de Activos — pero cae
   * directamente en el formulario, no en «vete a otra pestaña».
   */
  onAddFirstAsset?: () => void;
  /** Ídem con el formulario de nueva línea de presupuesto (pestaña Presupuesto). */
  onAddFirstBudgetEntry?: () => void;
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

  // ¿Hay ledger que resumir? Cero filas en ambos desgloses = la instalación no tiene ni un
  // activo ni un pasivo (no es «vale 0 €»: es que no hay nada registrado).
  const ledgerEmpty =
    showMetrics &&
    summary.assets_by_category.length === 0 &&
    summary.liabilities_by_category.length === 0;

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

  // ¿Tiene el bloque «Salud financiera» de dónde salir? Sus dos entradas son el ahorro del
  // modo activo (presupuesto o movimientos) y los activos líquidos. Si ambas son cero no hay
  // salud que medir: el bloque se sustituye por su estado vacío en vez de enseñar dos ceros.
  // La autonomía NO entra en la cuenta: con todo a cero el servidor la marca indefinida
  // (0 € de gasto caben de sobra en cualquier SWR) y eso no es información, es un artefacto.
  const healthEmpty =
    showMetrics &&
    (!fh ||
      (isZeroMoneyMetric(fh.net_monthly_equivalent) &&
        isZeroMoneyMetric(fh.liquid_assets_total)));

  // Portada de un hogar recién creado: ni ledger ni salud. Una sola llamada a la acción, sin
  // rejillas de ceros ni tres paneles diciendo «Sin datos.».
  const nothingYet = ledgerEmpty && healthEmpty;

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

      {nothingYet ? (
        <EmptyState
          title="Empieza por aquí"
          description="El Resumen reúne tu patrimonio, tu ahorro y tu proyección, pero aún no hay nada que resumir. Añade tu primer activo (una cuenta, un fondo, tu casa) y las cifras aparecen solas."
          actionLabel="Añadir tu primer activo"
          onAction={onAddFirstAsset}
          secondaryLabel="Crear presupuesto"
          onSecondary={onAddFirstBudgetEntry}
        />
      ) : null}

      {/* Política de ceros — la decisión es de BLOQUE, no de tarjeta (ver App.css §Empty
          states). Antes esta banda escondía una a una las KPIs que valían cero: con datos
          eso oculta información real (un 0 € de pasivos es una buena noticia, no un hueco)
          y con la instalación vacía dejaba la portada en blanco. Ahora, o se pintan las
          cuatro, o el bloque entero cede su sitio al estado vacío de arriba. */}
      {!nothingYet ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Patrimonio neto"
            helpId="summary.net_worth"
            value={nw}
          />
          <MetricCard label="Activos totales" value={ta} />
          <MetricCard label="Pasivos totales" value={tl} />
          <MetricCard label="Ratio deuda / activos" value={dta} />
        </div>
      ) : null}

      {!nothingYet ? (
        <section className="panel">
          <h3 className="panel-title">Salud financiera</h3>
          {showMetrics ? (
            !healthEmpty ? (
              <div className="metric-grid bordered-top">
                {/* Bloque con datos ⇒ se pintan sus tarjetas, valgan lo que valgan. */}
                <MetricCard
                  label="Ahorro mensual"
                  helpId="summary.savings"
                  value={savingsPrimary}
                  trend={savingsPlanTrend}
                  detail={savingsRateDetail}
                />
                <MetricCard
                  label="Activos líquidos"
                  helpId="summary.liquid_assets"
                  value={formatCurrencyAmount(
                    summary.financial_health.liquid_assets_total,
                    currencyIso,
                  )}
                  parenthetical={liquidAssetsPctOfTotalAssets ?? undefined}
                />
                {/* Única tarjeta condicional del bloque, y NO por valer cero: `showRunwayTile`
                    mira AUSENCIA de dato del servidor. Un runway de 0 meses sí se pinta. */}
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
              <EmptyState
                embedded
                title="Sin ahorro que medir"
                description="Esta sección mide cuánto ahorras al mes y cuánto tiempo aguantarías con lo que tienes a mano. Necesita tu presupuesto o tus movimientos para calcularlo."
                actionLabel="Crear presupuesto"
                onAction={onAddFirstBudgetEntry}
              />
            )
          ) : (
            <p className="muted bordered-top">Sin acceso.</p>
          )}
        </section>
      ) : null}

      {hasMembership && !nothingYet ? (
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

      {!nothingYet ? (
        <section className="panel">
          <h3 className="panel-title">Desglose</h3>
          {showMetrics && ledgerEmpty ? (
            <EmptyState
              embedded
              title="Sin activos ni pasivos"
              description="Cuando registres tus cuentas, fondos o deudas verás aquí cómo se reparte tu patrimonio por categoría."
              actionLabel="Añadir activo"
              onAction={onAddFirstAsset}
            />
          ) : null}
          {showMetrics && summary && !ledgerEmpty ? (
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
          ) : hasMembership && !showMetrics ? (
            <div className="summary-donuts-row bordered-top">
              <div className="ff-chart-skeleton ff-chart-skeleton--donut" aria-hidden />
              <div className="ff-chart-skeleton ff-chart-skeleton--donut" aria-hidden />
            </div>
          ) : null}
          {showMetrics && summary && !ledgerEmpty ? (
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
          ) : !hasMembership ? (
            <p className="muted bordered-top">Sin acceso.</p>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}
