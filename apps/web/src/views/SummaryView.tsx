import type {
  InstallationAccess,
  ProjectionSeriesApi,
  RetirementProfileApi,
  SummaryResponse,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
import { HelpPopover } from "../components/HelpPopover";
import { HELP_TEXTS } from "../lib/helpTexts";
import { appUrl } from "../lib/basePath";
import { MetricCard } from "../components/MetricCard";
import {
  SummaryBreakdownBlock,
  SummaryDonutChart,
} from "../components/charts/summary";
import { MiniProjection } from "../components/charts/MiniProjection";
import { ChartLegend } from "../components/charts/ChartLegend";
import { buildAssetLegendItems, householdMemberColor } from "../lib/chart-legend";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatDebtToAssetsPct,
  formatFractionAsPercent,
  formatPercentDisplay,
  formatPercentDisplaySigned,
  formatRunwayValue,
  isAbsentMetric,
  isZeroMoneyMetric,
  parseDisplayDecimal,
} from "../lib/format";
import { runwaySwrParenthetical } from "../lib/fire";
import { formatDeltaCurrency } from "../lib/expenses";
import { formatDateDmy } from "../lib/dates";
import { planCardV2, resolvePlanMilestoneCivil } from "../lib/plan-card";
import { householdPlanLines } from "../lib/household-plan-lines";
import { summarySuccessTile } from "../lib/risk-bands";
import { TAB_PATH, settingsSubTabPath } from "../lib/navigation";

type LedgerPersonScope = "household" | "mine";

export function SummaryView({
  installation,
  retirementProfile,
  loading,
  hasMembership,
  ledgerPersonScope,
  summary,
  summaryBusy,
  projectionSeries,
  navigate,
  onAddFirstAsset,
  onAddFirstBudgetEntry,
}: {
  installation: InstallationAccess | null;
  /** Perfil de jubilación del usuario (5.0.0): de aquí sale el SWR de la tarjeta de Autonomía. */
  retirementProfile: RetirementProfileApi | null;
  loading: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
  projectionSeries: ProjectionSeriesApi | null;
  /** Router de la app: la tarjeta «Tu plan» enlaza a «Tu cuenta» y a «Jubilación» cuando falta
   *  un dato del perfil. */
  navigate: (path: string, replace?: boolean) => void;
  /**
   * Lleva al formulario de nuevo activo (pestaña Activos con el modal ya abierto). El Resumen
   * no tiene formularios propios, así que su estado vacío delega en el de Activos — pero cae
   * directamente en el formulario, no en «vete a otra pestaña».
   */
  onAddFirstAsset?: () => void;
  /** Ídem con el formulario de nueva línea de presupuesto (pestaña Presupuesto). */
  onAddFirstBudgetEntry?: () => void;
}) {
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
    ? runwaySwrParenthetical(retirementProfile)
    : undefined;

  // Rendimiento neto: la cifra grande es la REAL (descontada la inflación) y el paréntesis el
  // nominal. Los dos llegan ya en porcentaje, no en fracción — de ahí `formatPercentDisplaySigned`
  // directo y no `formatFractionAsPercent`. Mismo criterio de ausencia que la Autonomía: el
  // servidor omite ambos campos cuando el patrimonio neto no es positivo (no es un cero, es que
  // la métrica no existe), así que la tarjeta se oculta en vez de pintar un «—».
  const netReturn = (() => {
    if (!showMetrics || !fh) return null;
    if (isAbsentMetric(fh.net_return_real_annual_pct)) return null;
    const real = parseDisplayDecimal(String(fh.net_return_real_annual_pct));
    if (real === null) return null;
    const nominal = isAbsentMetric(fh.net_return_nominal_annual_pct)
      ? null
      : parseDisplayDecimal(String(fh.net_return_nominal_annual_pct));
    return {
      value: formatPercentDisplaySigned(real),
      parenthetical:
        nominal === null
          ? undefined
          : `${formatPercentDisplaySigned(nominal)} nominal`,
    };
  })();

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

  // ── Tarjeta «Tu plan» (D27/U9) y frases del hogar (D32/U10) ───────────────────────────────
  //
  // En «Yo» es UNA tarjeta ancha con la ORACIÓN del plan (`planCardV2`, `lib/plan-card.ts`); en
  // «Hogar» el agregado no tiene plan propio y lo que hay son N frases, una por miembro
  // (`householdPlanLines`, `lib/household-plan-lines.ts`) — sin cifras ni tarjetas por persona.
  //
  // `planMonthLabel` fecha un índice de mes con el mismo ancla que usa el chart (mes 0 = hoy):
  // sin ancla cae a «mes N» en vez de inventarse una fecha.
  const planMonthLabel = (monthIndex: number): string => {
    const civil = resolvePlanMilestoneCivil({
      monthIndex,
      anchorDateYmd: projectionSeries?.anchor_date_ymd ?? null,
    });
    return civil.ymd ? formatDateDmy(civil.ymd) : `mes ${monthIndex}`;
  };

  const cardV2 =
    hasMembership && ledgerPersonScope === "mine" && (summary?.plan != null || projectionSeries != null)
      ? planCardV2({
          plan: summary?.plan,
          series: projectionSeries,
          monthLabel: planMonthLabel,
          targetRetirementAge: retirementProfile?.target_retirement_age ?? null,
        })
      : null;

  const householdLines =
    hasMembership && ledgerPersonScope === "household"
      ? householdPlanLines(projectionSeries?.members, planMonthLabel)
      : [];

  const showPlanPanel =
    hasMembership &&
    (ledgerPersonScope === "household" ? householdLines.length > 0 : cardV2 != null);

  /**
   * KPI «Éxito del plan» (D28). Sale de `summary.plan.success_*` — el MISMO sorteo de Monte
   * Carlo que dibuja la sección «Riesgo» de Jubilación, servido desde su cache: aquí no se pide
   * `/v1/projection/bands` ni se recalcula nada. Si hubiera un segundo sorteo, el tile y el
   * abanico contarían dos éxitos distintos del mismo plan.
   *
   * `null` = el backend no publica el bloque; entonces no se pinta tarjeta ninguna (un guion
   * mudo se leería como «tu plan no tiene éxito medible»).
   */
  const successTile = summarySuccessTile(summary?.plan);

  const goToPlanAction = (target: "account" | "retirement") => {
    navigate(target === "account" ? settingsSubTabPath("general") : TAB_PATH.retirement);
  };

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
        {/* S7 — sin «Moneda EUR»: no dice nada que el usuario no sepa ya. Los estados de
            carga/sin-acceso se conservan; con datos, el bloque no tiene nada que decir y no
            se pinta nada (en vez de un subtítulo vacío ocupando la línea). */}
        {loading ? (
          <p className="workspace-sub">Cargando…</p>
        ) : !hasMembership ? (
          <p className="workspace-sub">Sin acceso hasta aprobación.</p>
        ) : null}
      </div>

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
          <MetricCard label="Ratio deuda / activos" helpId="summary.debt_to_assets_ratio" value={dta} />
        </div>
      ) : null}

      {/* U9/U10 (issue #207) — «Tu plan» pasa de dos tarjetas apiladas a UNA tarjeta ancha en
          una fila: la ORACIÓN del plan (título, coloreada por su tono), la estrategia + hito
          secundario (subtítulo) y, a la derecha, el KPI «Éxito del plan». El aviso, si lo hay,
          va debajo. En Hogar el agregado no tiene plan propio: la tarjeta se sustituye por una
          lista de FRASES, una por miembro, con el punto de color de su línea del chart — sin
          cifras ni tarjetas por persona (las KPI agregadas de arriba no cambian). */}
      {!nothingYet && showPlanPanel ? (
        <section className="panel">
          <div className="panel-head-row">
            <h3 className="panel-title">
              {ledgerPersonScope === "household" ? "Planes del hogar" : "Tu plan"}
            </h3>
            <HelpPopover
              title={HELP_TEXTS["summary.plan"].title}
              body={HELP_TEXTS["summary.plan"].body}
            />
          </div>
          {ledgerPersonScope === "household" ? (
            <>
              <ul className="plan-sentence-list bordered-top">
                {householdLines.map((line, idx) => (
                  <li key={line.userId} className="plan-sentence-item">
                    <span
                      className="plan-sentence-dot"
                      style={{ background: householdMemberColor(idx) }}
                      aria-hidden
                    />
                    {line.text}
                  </li>
                ))}
              </ul>
              {/* El hogar no tiene plan propio: el KPI se conserva con guion + su razón
                  («solo en tu vista «Yo»»), nunca oculto — un hueco mudo se leería como
                  «no medible» en vez de «esta vista no tiene sujeto». */}
              {successTile ? (
                <div className="metric-grid summary-success-grid">
                  <MetricCard
                    label="Éxito del plan"
                    helpId="summary.success"
                    value={successTile.value}
                    parenthetical={successTile.parenthetical}
                    detail={successTile.detail}
                    tone={successTile.tone}
                  />
                </div>
              ) : null}
            </>
          ) : cardV2 ? (
            <>
              <div className="plan-card-wide bordered-top">
                <div className="plan-card-wide-main">
                  <div
                    className={`plan-card-wide-title${
                      cardV2.tone === "danger" ? " plan-card-wide-title--danger" : ""
                    }`}
                  >
                    {cardV2.title}
                  </div>
                  <div className="plan-card-wide-subtitle">{cardV2.subtitle}</div>
                </div>
                {cardV2.success ? (
                  <div className="plan-card-wide-kpi">
                    <MetricCard
                      label={cardV2.success.label}
                      helpId="summary.success"
                      value={cardV2.success.value}
                      parenthetical={cardV2.success.parenthetical}
                      detail={cardV2.success.detail}
                      tone={cardV2.success.tone}
                    />
                  </div>
                ) : null}
              </div>
              {cardV2.warning ? (
                <div
                  className={`plan-card-wide-warning${
                    cardV2.tone === "danger" ? " plan-card-wide-warning--danger" : ""
                  }`}
                >
                  {cardV2.warning.text}
                  {" · "}
                  <a
                    href={appUrl(
                      cardV2.warning.target === "account"
                        ? settingsSubTabPath("general")
                        : TAB_PATH.retirement,
                    )}
                    onClick={(e) => {
                      if (
                        e.button !== 0 ||
                        e.metaKey ||
                        e.altKey ||
                        e.ctrlKey ||
                        e.shiftKey
                      )
                        return;
                      e.preventDefault();
                      goToPlanAction(cardV2.warning!.target);
                    }}
                  >
                    {cardV2.warning.actionLabel}
                  </a>
                </div>
              ) : null}
            </>
          ) : null}
        </section>
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
                {netReturn ? (
                  <MetricCard
                    label="Rendimiento neto"
                    helpId="summary.net_return"
                    value={netReturn.value}
                    parenthetical={netReturn.parenthetical}
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
              <ChartLegend
                size="sm"
                structural={[
                  {
                    key: "nw",
                    label: "Patrimonio neto",
                    color: "var(--proj-nw)",
                    swatch: "line",
                  },
                ]}
                // MiniProjection apila en el orden CRUDO de la API → colorIndex = i
                // (reordenar aquí rompería la correspondencia color-área).
                assets={buildAssetLegendItems(
                  (projectionSeries.asset_series ?? []).map((a, idx) => ({
                    id: a.asset_id,
                    name: a.asset_name,
                    colorIndex: idx,
                  })),
                )}
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
