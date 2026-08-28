import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent,
  type WheelEvent,
} from "react";
import type {
  CashflowFineApi,
  HistoryCashflowApi,
  HistorySeriesApi,
  PlanningFlowApiRow,
  PlanningFlowDirectionApi,
  ProjectionMilestoneApi,
  ProjectionSeriesApi,
} from "../api/types";
import { mergeProjectionWithHistory } from "../lib/history-merge";
import { formatDateDmy, parseYmdComponents } from "../lib/dates";
import {
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatCurrencyOrDashNumber,
  normalizeCurrencyIso,
  parseDisplayDecimal,
} from "../lib/format";
import {
  ASSET_LINE_COLORS,
  buildProjectionChartLayout,
  deflationFactorAt,
  formatProjectionChartHorizonLine,
  niceYTicks,
  projectionHoverTitle,
  projectionMaxXTicks,
  projectionXTickLabel,
  projectionXTicks,
  thinTicksFromEnd,
} from "../lib/projection-chart";
import {
  buildAssetLegendItems,
  buildStructuralLegendItems,
  collapsedAssetLegendCap,
  legendOrderByPeakDesc,
  topAssetTooltipRows,
} from "../lib/chart-legend";
import { ChartLegend } from "../components/charts/ChartLegend";
import {
  formatAxisMoney,
  formatProjectionMilestoneCompactLabel,
  type LedgerPersonScope,
} from "../lib/ledger";
import { savingsSourceUsesTransactions } from "../lib/fire";
import { useIsMobile } from "../lib/responsive";
import { chartPerf } from "../lib/perf";
import { panWindow, pinchWindow, type ChartDomain } from "../lib/chart-gestures";

// ── Umbrales de la máquina de gestos táctiles ──
// Un gesto arranca en `maybe`; cruzar SLOP_PX decide pan (horizontal) o cede al
// scroll de página (vertical, vía `touch-action: pan-y` → pointercancel). Un
// `maybe` que se suelta bajo SLOP_PX y TAP_MAX_MS es un TAP → tooltip.
const SLOP_PX = 8;
const TAP_MAX_MS = 300;
// El tooltip táctil se auto-cierra si no hay nueva interacción.
const TIP_CLOSE_MS = 3500;
// En coarse el tooltip se eleva un extra sobre el dedo para no quedar tapado.
const COARSE_TIP_LIFT_PX = 22;

type GesturePhase = "idle" | "maybe" | "pan" | "pinch" | "yield";

interface GestureState {
  phase: GesturePhase;
  /** Punteros táctiles activos, con su posición actual. Orden de inserción. */
  pointers: Map<number, { x: number; y: number }>;
  /** Origen del gesto (para slop de tap y delta de pan). Se resetea al comprometer pan. */
  startClientX: number;
  startClientY: number;
  startTimeMs: number;
  /** Snapshot de la ventana al comprometer pan/pinch (meses reales). */
  windowStart: number;
  windowSpan: number;
  /** Meses por píxel-cliente, capturado 1 vez al comprometer el pan. */
  monthsPerPx: number;
  /** Distancia inicial entre los dos dedos y mes-ancla (punto medio) del pinch. */
  pinchStartDist: number;
  pinchAnchorMonth: number;
  /** Captura PEREZOSA del puntero: solo se activa al comprometer un pan. */
  captured: boolean;
  capturedPointerId: number | null;
  /** Ventana pendiente de commit, coalescida por requestAnimationFrame. */
  pendingWindow: { startMonth: number; monthSpan: number } | null;
  rafId: number | null;
  /** Timeout de auto-cierre del tooltip táctil. */
  tipTimeout: number | null;
}

export function ProjectionNetWorthChart({
  series,
  history,
  cashflow,
  cashflowDaily,
  onRequestDailyCashflow,
  milestones,
  focusMode,
  inflationAdjusted,
  installationInflationPct,
  currencyIso,
  ledgerPersonScope,
  inflationPctDisplay,
  ageUiMode,
  userBirthDate,
  anchorDateYmd,
  calendarTz,
  planningFlows,
  assetOwnerNames,
}: {
  series: ProjectionSeriesApi;
  /** Serie histórica (snapshots pasados) ya validada contra el anchor de la proyección, o
   *  `null` cuando no hay histórico usable → el merge es la identidad (render solo-futuro). */
  history: HistorySeriesApi | null;
  /** Cash-flow histórico weekly (ventana 24m), ya validado contra el anchor. Su campo `fine`
   *  alimenta el overlay fino de la zona pasada; `null` → el pasado se pinta solo mensual. */
  cashflow: HistoryCashflowApi | null;
  /** Detalle diario (ventana 6m), fetcheado lazy al hacer zoom histórico reciente. */
  cashflowDaily: HistoryCashflowApi | null;
  onRequestDailyCashflow?: () => void;
  milestones: ProjectionMilestoneApi[];
  focusMode: boolean;
  /** Cuando true (default), las series del chart se deflactan visualmente — la matemática del
   *  motor sigue siendo nominal, pero el eje Y muestra "dinero de hoy" y el target FIRE base es plano. */
  inflationAdjusted: boolean;
  installationInflationPct: number;
  currencyIso: string;
  ledgerPersonScope: LedgerPersonScope;
  inflationPctDisplay: string | null;
  ageUiMode: "dates" | "ages";
  userBirthDate: string | null;
  /** Mes 0 del motor (YYYY-MM-DD); prioridad sobre reloj cliente. */
  anchorDateYmd: string | null;
  calendarTz: string;
  planningFlows: PlanningFlowApiRow[];
  /** asset_id → nombre de owner (join /v1/assets + /v1/installation/members hecho en App.tsx;
   *  `null` en el valor = activo actual sin owner resoluble). Solo se usa en vista hogar para
   *  desambiguar nombres duplicados en leyenda y tooltip; `null`/vacío degrada a sin sufijo. */
  assetOwnerNames: Readonly<Record<string, string | null>> | null;
}) {
  // `mount-start` se mide en el primer render (cuando el chart aparece en
  // pantalla), separado de `fetch-start` que ocurre cuando llega la serie.
  // Esto distingue el tiempo de cómputo del chart del delay entre fetch y
  // navegación (que puede ser "tiempo del usuario", no del sistema).
  chartPerf.mark("mount-start");
  // Fusión pasado (histórico) + futuro (proyección). Identidad por referencia cuando no hay
  // histórico usable → `pts`/`historyStartMonth` degradan al comportamiento solo-futuro actual.
  const merged = useMemo(
    () => mergeProjectionWithHistory(series, history),
    [series, history],
  );
  const pts = merged.pts;
  const historyStartMonth = merged.historyStartMonth;
  const gid = useId().replace(/:/g, "");
  const svgRef = useRef<SVGSVGElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  // Caja del plot (solo el SVG): es la que mide el ResizeObserver. `wrapRef` (la raíz,
  // que además contiene la leyenda HTML) sigue siendo la base del tooltip absoluto.
  const plotRef = useRef<HTMLDivElement>(null);
  const yAxisAnimRef = useRef<number | null>(null);
  // ── Máquina de gestos táctiles (Pointer Events) ──
  // TODO el estado del gesto vive en este ref (sin re-render): puntos activos,
  // snapshot de la ventana al comprometer un gesto, monthsPerPx capturado 1 vez,
  // ventana pendiente coalescida por rAF, y el timeout de auto-cierre del
  // tooltip. La ruta ratón (wheel/hover) NUNCA entra aquí (guards `pointerType`).
  const gestureRef = useRef<GestureState>({
    phase: "idle",
    pointers: new Map(),
    startClientX: 0,
    startClientY: 0,
    startTimeMs: 0,
    windowStart: 0,
    windowSpan: 0,
    monthsPerPx: 0,
    pinchStartDist: 0,
    pinchAnchorMonth: 0,
    captured: false,
    capturedPointerId: null,
    pendingWindow: null,
    rafId: null,
    tipTimeout: null,
  });
  const [hover, setHover] = useState<number | null>(null);
  const [tipOffset, setTipOffset] = useState({ x: 0, y: 0 });
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
  const isMobile = useIsMobile();
  // viewWindow está en **meses reales** (no en índices de array). Soporta
  // densidades mixtas: con `density=monthly` los puntos van 0..months con paso
  // 1; con `density=hybrid` van 0..12 con paso 1 y luego 24, 36, ... La
  // ventana se mueve siempre sobre el horizonte temporal completo
  // (`series.months`), independiente de cuántos puntos haya serializados.
  const [viewWindow, setViewWindow] = useState({
    startMonth: historyStartMonth,
    monthSpan: series.months - historyStartMonth,
  });
  const [animatedYDomain, setAnimatedYDomain] = useState<{
    min: number;
    max: number;
  } | null>(null);
  const animatedYDomainRef = useRef<{ min: number; max: number } | null>(null);

  useLayoutEffect(() => {
    // Mide el plot, NO la raíz: la raíz incluye la leyenda HTML y el viewBox saldría
    // más alto que la caja del SVG (letterboxing + hover desalineado).
    const node = plotRef.current;
    if (!node) return;
    const measure = () => {
      const rect = node.getBoundingClientRect();
      if (rect.width > 0) {
        setContainerSize((prev) => {
          const nextW = Math.round(rect.width);
          const nextH = Math.max(0, Math.round(rect.height));
          return prev.width === nextW && prev.height === nextH
            ? prev
            : { width: nextW, height: nextH };
        });
      }
    };
    measure();
    const ro = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect && rect.width > 0) {
        setContainerSize((prev) => {
          const nextW = Math.round(rect.width);
          const nextH = Math.max(0, Math.round(rect.height));
          return prev.width === nextW && prev.height === nextH
            ? prev
            : { width: nextW, height: nextH };
        });
      }
    });
    ro.observe(node);
    return () => ro.disconnect();
  }, []);

  const focusWindow = useMemo(() => {
    if (pts.length <= 0) return null;
    const nextMonetaryMilestones = milestones
      .filter((m) => m.reached_month_index >= 0)
      .sort((a, b) => a.reached_month_index - b.reached_month_index)
      .slice(0, 3);
    const focusEnd = nextMonetaryMilestones.at(-1)?.reached_month_index;
    if (focusEnd == null) return null;
    // Margen tras el último hito: sin él, el marcador (p. ej. «Jubilación») cae en el
    // borde derecho y su etiqueta centrada queda pisada por el recorte del plot.
    const padded = focusEnd + Math.max(9, Math.round(focusEnd * 0.12));
    const clampedEnd = Math.max(0, Math.min(series.months - 1, padded));
    // Focus mantiene startMonth 0: es una lente de planificación futura; el pasado histórico
    // queda deliberadamente oculto (no arranca en historyStartMonth).
    return { startMonth: 0, monthSpan: clampedEnd + 1 };
  }, [milestones, pts.length, series.months]);

  useEffect(() => {
    setViewWindow((prev) => {
      if (pts.length <= 0) return { startMonth: 0, monthSpan: 0 };
      // La vista completa arranca en historyStartMonth (≤ 0) para incluir el pasado; focus sigue
      // en 0 (ver focusWindow).
      const next = focusMode && focusWindow
        ? focusWindow
        : {
            startMonth: historyStartMonth,
            monthSpan: series.months - historyStartMonth,
          };
      if (prev.startMonth === next.startMonth && prev.monthSpan === next.monthSpan) {
        return prev;
      }
      return next;
    });
  }, [focusMode, focusWindow, pts.length, series.months, historyStartMonth]);

  const hasFireTargetSeries = useMemo(() => {
    const f = series.fire_target_series;
    return Array.isArray(f) && f.length === series.points.length && f.length > 0;
  }, [series.fire_target_series, series.points.length]);

  const layoutDims = useMemo(
    () =>
      buildProjectionChartLayout(
        containerSize.width > 0 ? containerSize.width : 1040,
        containerSize.height > 0 ? containerSize.height : undefined,
        { hideYAxisLabels: isMobile, compactHeader: isMobile },
      ),
    [containerSize.height, containerSize.width, isMobile],
  );

  // Series base: deflactación, orden de activos, stacking acumulado completo.
  // No depende de viewWindow → pan/zoom no recalculan este bloque.
  const baseSeries = useMemo(() => {
    chartPerf.mark("baseSeries-start");
    if (pts.length < 2) {
      chartPerf.mark("baseSeries-end");
      return null;
    }
    // El deflactor usa el `month_index` real del punto (nunca la posición en el array): con
    // densidad `hybrid` los puntos no son equidistantes, y con histórico los `month_index` son
    // negativos (el deflactor los AMPLIFICA automáticamente, ×(1+inf)^(−k/12)). `deflationFactorAt`
    // devuelve 1 cuando el pct efectivo es 0. Alineado con `milestones_real` del backend.
    const effectivePct =
      inflationAdjusted && installationInflationPct > 0
        ? installationInflationPct
        : 0;
    const deflator = (monthIndex: number) =>
      deflationFactorAt(monthIndex, effectivePct);
    const miAt = (i: number) => pts[i]?.month_index ?? i;
    const nw = pts.map((p) => p.net_worth * deflator(p.month_index));
    const cc = pts.map((p) => p.contributed_capital * deflator(p.month_index));
    // `fire_target_series` es paralelo a `series.points` (SOLO futuro): se re-mapea a un array de
    // longitud combinada `(number | null)[]`, null en el pasado (k < 0). Solo los vértices no-null
    // se dibujan → la línea FIRE arranca en el mes 0.
    const fireRaw = series.fire_target_series;
    const fireTarget: (number | null)[] | null =
      Array.isArray(fireRaw) && fireRaw.length === series.points.length
        ? pts.map((p, i) =>
            i >= merged.futureOffset
              ? (fireRaw[i - merged.futureOffset] ?? 0) * deflator(p.month_index)
              : null,
          )
        : null;
    const assetSeries = merged.assetSeries
      .map((as) => {
        const values = as.values.map((v, i) => v * deflator(miAt(i)));
        return {
          id: as.asset_id,
          name: as.asset_name,
          values,
          peak: values.length > 0 ? Math.max(0, ...values) : 0,
        };
      })
      .sort((a, b) => {
        if (a.peak !== b.peak) return a.peak - b.peak;
        return a.name.localeCompare(b.name);
      });
    const monthCount = pts.length;
    const assetSums: number[] = new Array(monthCount).fill(0);
    for (let k = 0; k < monthCount; k++) {
      let s = 0;
      for (const as of assetSeries) s += Math.max(0, as.values[k] ?? 0);
      assetSums[k] = s;
    }
    const assetStacks = assetSeries.map(() => ({
      bottoms: new Array(monthCount).fill(0),
      tops: new Array(monthCount).fill(0),
    }));
    for (let k = 0; k < monthCount; k++) {
      const baseTotal = Math.max(0, nw[k] ?? 0);
      const sum = assetSums[k];
      let cursor = 0;
      for (let i = 0; i < assetSeries.length; i++) {
        const v = Math.max(0, assetSeries[i].values[k] ?? 0);
        const height = sum > 0 ? baseTotal * (v / sum) : 0;
        assetStacks[i].bottoms[k] = cursor;
        cursor += height;
        assetStacks[i].tops[k] = cursor;
      }
    }
    const startNwParsed = parseDisplayDecimal(series.starting_net_worth);
    const startNw = startNwParsed !== null ? startNwParsed : nw[0] ?? 0;
    // El histórico puede bajar el NW por debajo de 0 en el pasado: el eje debe permitir negativos
    // aunque el patrimonio de partida (mes 0) sea positivo.
    const allowNegativeAxis = startNw < 0 || merged.minNetWorth < 0;
    chartPerf.mark("baseSeries-end");
    return { nw, cc, fireTarget, assetSeries, assetStacks, allowNegativeAxis };
  }, [
    pts,
    series.starting_net_worth,
    series.points.length,
    series.fire_target_series,
    merged.assetSeries,
    merged.futureOffset,
    merged.minNetWorth,
    inflationAdjusted,
    installationInflationPct,
  ]);

  // ── Modelo de la leyenda (HTML, fuera del SVG) ──
  const legendStructural = useMemo(
    () =>
      buildStructuralLegendItems({
        hasFire: hasFireTargetSeries,
        hasHistory: historyStartMonth < 0,
        historyIsAssetsOnly: merged.pastIsAssetsOnly,
      }),
    [hasFireTargetSeries, historyStartMonth, merged.pastIsAssetsOnly],
  );

  // Activos por peak DESC para la leyenda, conservando el color del orden de
  // pintado (peak ASC de baseSeries). Sufijo de owner solo en vista hogar.
  const legendAssets = useMemo(() => {
    const painted = baseSeries?.assetSeries ?? [];
    const ordered = legendOrderByPeakDesc(painted);
    return buildAssetLegendItems(
      ordered.map(({ item, colorIndex }) => ({
        id: item.id,
        name: item.name,
        colorIndex,
      })),
      ledgerPersonScope === "mine" ? null : assetOwnerNames,
    );
  }, [baseSeries, ledgerPersonScope, assetOwnerNames]);

  // asset_id → etiqueta con sufijo: misma fuente que la leyenda, la consume el tooltip.
  const assetLabelById = useMemo(() => {
    const out: Record<string, string> = {};
    for (const it of legendAssets) out[it.key] = it.label;
    return out;
  }, [legendAssets]);

  // xTicks completos del horizonte. No dependen de viewWindow.
  const xTicksAll = useMemo(() => {
    chartPerf.mark("xTicks-start");
    const t = projectionXTicks(
      series.months,
      { ageUiMode, birthDateIso: userBirthDate, anchorDateYmd, calendarTz },
      { plotWidthPx: layoutDims.pw },
      historyStartMonth,
    );
    chartPerf.mark("xTicks-end");
    return t;
  }, [
    series.months,
    ageUiMode,
    userBirthDate,
    anchorDateYmd,
    calendarTz,
    layoutDims.pw,
    historyStartMonth,
  ]);

  // Modelo dependiente de viewWindow: slicing visible (por month_index, no por
  // índice de array — soporta densidades mixtas), yTicks, ticks X visibles y
  // markers. Pan/zoom solo recalculan esto.
  const model = useMemo(() => {
    chartPerf.mark("model-start");
    if (!baseSeries) {
      chartPerf.mark("model-end");
      return null;
    }
    const { nw, cc, fireTarget, assetSeries, assetStacks, allowNegativeAxis } =
      baseSeries;
    const totalMonths = series.months;
    // Dominio panorámico: incluye el pasado histórico (historyStartMonth ≤ 0). Sin histórico,
    // domainMonths === totalMonths → comportamiento idéntico al actual.
    const domainMonths = totalMonths - historyStartMonth;
    const minMonthSpan = Math.min(totalMonths, 12);
    const monthSpan = Math.max(
      minMonthSpan,
      Math.min(domainMonths, Math.round(viewWindow.monthSpan)),
    );
    const maxStartMonth = Math.max(historyStartMonth, totalMonths - monthSpan);
    const visibleMonthStart = Math.max(
      historyStartMonth,
      Math.min(maxStartMonth, Math.round(viewWindow.startMonth)),
    );
    const visibleMonthEnd = visibleMonthStart + monthSpan - 1;

    // Indices del array `pts` cuyos month_index caen dentro de la ventana
    // visible. Soporta puntos no equidistantes (densidad hybrid: mes 0..12
    // mensual + mes 24, 36, ..., 840 anual).
    const visibleIndices: number[] = [];
    for (let i = 0; i < pts.length; i++) {
      const mi = pts[i].month_index;
      if (mi >= visibleMonthStart && mi <= visibleMonthEnd) {
        visibleIndices.push(i);
      }
    }

    const nwVisible = visibleIndices.map((i) => nw[i] ?? 0);
    // El "capital aportado" solo existe en el futuro (k ≥ 0). En el pasado el valor es un centinela
    // 0 que NO debe dibujarse ni contar para el dominio Y.
    const ccVisibleIndices = visibleIndices.filter(
      (i) => pts[i]!.month_index >= 0,
    );
    const ccVisible = ccVisibleIndices.map((i) => cc[i] ?? 0);
    const fireTargetVisible = fireTarget
      ? visibleIndices.map((i) => fireTarget[i] ?? null)
      : null;
    const assetVisibleValues = assetSeries.flatMap((as) =>
      visibleIndices.map((i) => as.values[i] ?? 0),
    );

    // El target FIRE inflado puede crecer muy por encima del patrimonio en horizontes largos;
    // dejarlo fuera del rango del eje Y para no aplastar la curva del patrimonio. La línea se
    // recorta visualmente por el clipPath del plot si excede.
    const dataMin = Math.min(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const dataMax = Math.max(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const rawSpan = dataMax - dataMin;
    const padY =
      rawSpan > 0 ? rawSpan * 0.07 : Math.max(Math.abs(dataMax) * 0.06, 1);

    let plotMin = dataMin - padY;
    const plotMax = dataMax + padY;
    if (!allowNegativeAxis) plotMin = Math.max(0, plotMin);

    const yTicksRaw = niceYTicks(plotMin, plotMax, 6);
    let yTicks = allowNegativeAxis
      ? yTicksRaw
      : yTicksRaw.filter((t) => t >= 0);
    if (!allowNegativeAxis && yTicks.length < 2) {
      yTicks = niceYTicks(Math.max(0, plotMin), plotMax, 6);
    }
    let yMin = yTicks[0] ?? (allowNegativeAxis ? plotMin : Math.max(0, plotMin));
    const yMax = yTicks[yTicks.length - 1] ?? plotMax;
    if (!allowNegativeAxis && yMin < 0) yMin = 0;

    const xTicksInWindow = xTicksAll.filter(
      (tick) =>
        tick.monthIndex >= visibleMonthStart && tick.monthIndex <= visibleMonthEnd,
    );
    if (xTicksInWindow.length === 0 && visibleMonthEnd > visibleMonthStart) {
      xTicksInWindow.push({
        monthIndex: visibleMonthEnd,
        label: projectionXTickLabel(visibleMonthEnd, series.months, {
          ageUiMode,
          birthDateIso: userBirthDate,
          anchorDateYmd,
          calendarTz,
        }),
      });
    }
    // Diezmado por ancho SOBRE LOS VISIBLES (nunca sobre el horizonte completo: un
    // zoom filtraría los supervivientes y se quedaría sin etiquetas).
    const xTicksThinned = thinTicksFromEnd(
      xTicksInWindow,
      projectionMaxXTicks(layoutDims.pw, ageUiMode),
    );
    // «Hoy» vive en la fila del eje X: aparta cualquier etiqueta de año que caiga
    // a menos de ~40px del divisor (el mes 0 ya está excluido de los builders,
    // pero el primer año del horizonte puede quedar casi encima).
    const showsTodayLabel =
      historyStartMonth < 0 && visibleMonthStart <= 0 && visibleMonthEnd >= 0;
    const pxPerMonth = layoutDims.pw / Math.max(1, monthSpan - 1);
    const xTicks = xTicksThinned.filter(
      (t) => !showsTodayLabel || Math.abs(t.monthIndex) * pxPerMonth > 40,
    );

    const tickSpanPx =
      xTicks.length > 1 ? layoutDims.pw / (xTicks.length - 1) : layoutDims.pw;
    const rotateXLabels =
      xTicks.length > 11 || (xTicks.length > 5 && tickSpanPx < 46);
    // Los 38px de las etiquetas X rotadas salen del alto del plot (ph), NUNCA de
    // lienzo extra: si el viewBox fuera más alto que la caja CSS medida, `meet`
    // encogería el dibujo entero y lo centraría con bandas laterales — el chart
    // dejaba de ser ancho completo (defecto preexistente que se notaba más con la
    // leyenda HTML restando altura a la caja del plot).
    const xAxisExtraBottom = rotateXLabels ? 38 : 0;
    const viewHeight = layoutDims.H;

    const { W, H, ml, mr, mt, mb, pw } = layoutDims;
    const ph = Math.max(60, layoutDims.ph - xAxisExtraBottom);

    // xScale toma un `monthIndex` real (mes desde el inicio del horizonte),
    // no un índice de array. Esto desacopla el render de la densidad de
    // puntos serializados.
    const xScale = (monthIndex: number) => {
      const local = monthIndex - visibleMonthStart;
      return ml + (local / Math.max(1, monthSpan - 1)) * pw;
    };
    const compoundOutpaceMonth =
      series.compound_outpaces_true_savings_month_index ?? null;
    const visibleMilestones = milestones.filter(
      (m) =>
        m.reached_month_index >= visibleMonthStart &&
        m.reached_month_index <= visibleMonthEnd,
    );
    const showCompoundOutpaceMarker =
      compoundOutpaceMonth != null &&
      compoundOutpaceMonth >= visibleMonthStart &&
      compoundOutpaceMonth <= visibleMonthEnd;
    const chartPlanningMarkers = (() => {
      const anchor = anchorDateYmd ? parseYmdComponents(anchorDateYmd) : null;
      if (!anchor) return [];
      const out: Array<{
        id: string;
        mi: number;
        title: string;
        direction: PlanningFlowDirectionApi;
      }> = [];
      for (const f of planningFlows) {
        if (!f.show_in_chart || !f.due_date) continue;
        const d = parseYmdComponents(f.due_date);
        if (!d) continue;
        const mi = (d.y - anchor.y) * 12 + (d.m - anchor.m);
        // Los flujos de planificación son siempre futuros: descarta mi < 0 para que una ventana
        // extendida al pasado no genere marcadores fantasma.
        if (mi < 0) continue;
        if (mi < visibleMonthStart || mi > visibleMonthEnd) continue;
        out.push({ id: f.id, mi, title: f.title, direction: f.direction });
      }
      return out;
    })();
    // Marcadores de snapshot visibles: se posicionan por `month_index` (x), sobre el mismo vértice
    // que la polilínea NW. `month_fraction` no se usa para posicionar (el punto k=0 caería a la
    // derecha del divisor «Hoy», en zona futura, y el dot se despegaría de la línea).
    const snapshotMarkers = merged.markers.filter(
      (mk) =>
        mk.month_index >= visibleMonthStart &&
        mk.month_index <= visibleMonthEnd,
    );
    chartPerf.mark("model-end");
    return {
      nw,
      cc,
      fireTargetVisible,
      assetSeries,
      assetStacks,
      nwVisible,
      ccVisibleIndices,
      allowNegativeAxis,
      targetYMin: yMin,
      targetYMax: yMax,
      xTicks,
      xScale,
      compoundOutpaceMonth,
      showCompoundOutpaceMarker,
      visibleMilestones,
      chartPlanningMarkers,
      snapshotMarkers,
      pw,
      ph,
      ml,
      mr,
      mt,
      mb,
      W,
      H,
      rotateXLabels,
      viewHeight,
      visibleMonthStart,
      visibleMonthEnd,
      monthSpan,
      visibleIndices,
    };
  }, [
    baseSeries,
    xTicksAll,
    pts,
    historyStartMonth,
    merged.markers,
    series.months,
    series.compound_outpaces_true_savings_month_index,
    layoutDims,
    ageUiMode,
    userBirthDate,
    anchorDateYmd,
    calendarTz,
    milestones,
    planningFlows,
    viewWindow.monthSpan,
    viewWindow.startMonth,
  ]);

  // Captura el commit (post-render) y vuelca measures a consola. Solo cuando
  // cambia la serie (mount inicial o nueva data) — no en pan/zoom para no
  // ahogar la consola.
  useEffect(() => {
    if (!chartPerf.enabled) return;
    chartPerf.mark("first-commit");
    chartPerf.measure("fetch+network", "fetch-start", "fetch-response");
    chartPerf.measure("json-parse", "fetch-response", "fetch-end");
    chartPerf.measure("baseSeries-memo", "baseSeries-start", "baseSeries-end");
    chartPerf.measure("xTicks-memo", "xTicks-start", "xTicks-end");
    chartPerf.measure("model-memo", "model-start", "model-end");
    // mount-to-commit: el coste real del chart desde el primer render hasta
    // el commit post-paint. Esto es lo que importa optimizar.
    chartPerf.measure("mount-to-commit", "mount-start", "first-commit");
    // fetch-to-mount: delay entre que llegó la data y el chart empezó a
    // renderizar. Si es grande, suele ser tiempo del usuario navegando,
    // no del sistema (la medida `total-from-fetch` del plan v4 daba falsos
    // positivos por esto).
    chartPerf.measure("fetch-to-mount", "fetch-end", "mount-start");
    chartPerf.report();
  }, [series]);

  // Hoisted antes del early return para que los hooks de animación del eje Y
  // (abajo) no queden tras un `return` condicional. Cuando `model` es null el
  // componente no renderiza nada y estos valores caen a 0/0 de forma inocua.
  const targetYMin = model?.targetYMin ?? 0;
  const targetYMax = model?.targetYMax ?? 0;

  useEffect(() => {
    animatedYDomainRef.current = animatedYDomain;
  }, [animatedYDomain]);

  useEffect(() => {
    if (targetYMax <= targetYMin) {
      setAnimatedYDomain({ min: targetYMin, max: targetYMax + 1 });
      return;
    }
    if (yAxisAnimRef.current != null) {
      cancelAnimationFrame(yAxisAnimRef.current);
      yAxisAnimRef.current = null;
    }
    const from = animatedYDomainRef.current ?? { min: targetYMin, max: targetYMax };
    const to = { min: targetYMin, max: targetYMax };
    const start = performance.now();
    const durationMs = 170;
    const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;
    const tick = (now: number) => {
      const t = Math.max(0, Math.min(1, (now - start) / durationMs));
      const eased = easeOutCubic(t);
      setAnimatedYDomain({
        min: from.min + (to.min - from.min) * eased,
        max: from.max + (to.max - from.max) * eased,
      });
      if (t < 1) {
        yAxisAnimRef.current = requestAnimationFrame(tick);
      } else {
        yAxisAnimRef.current = null;
      }
    };
    yAxisAnimRef.current = requestAnimationFrame(tick);
    return () => {
      if (yAxisAnimRef.current != null) {
        cancelAnimationFrame(yAxisAnimRef.current);
        yAxisAnimRef.current = null;
      }
    };
  }, [targetYMax, targetYMin]);

  // Fetch lazy del detalle diario: se pide una sola vez (App lo desduplica) cuando la vista es un
  // zoom histórico reciente — ventana corta (el span mínimo del chart es 12 meses) que arranca en
  // el pasado y termina cerca de hoy. Sin serie fina weekly no habrá daily útil (mismo gating que
  // el backend: sin transacciones vinculadas no hay `fine`). Hoisted antes del early return por
  // las reglas de hooks (mismo patrón que la animación del eje Y).
  const dailyVisStart = model?.visibleMonthStart ?? 0;
  const dailyVisEnd = model?.visibleMonthEnd ?? 0;
  const dailySpan = model?.monthSpan ?? 0;
  useEffect(() => {
    if (!onRequestDailyCashflow || cashflowDaily || cashflow?.fine == null) {
      return;
    }
    if (
      dailyVisStart < 0 &&
      dailyVisStart >= -13 &&
      dailyVisEnd <= 2 &&
      dailySpan <= 14
    ) {
      onRequestDailyCashflow();
    }
  }, [
    dailyVisStart,
    dailyVisEnd,
    dailySpan,
    cashflow,
    cashflowDaily,
    onRequestDailyCashflow,
  ]);

  // Cleanup de la máquina de gestos al desmontar: cancela cualquier rAF de commit
  // y el timeout de auto-cierre del tooltip pendientes (evita callbacks huérfanos).
  useEffect(() => {
    const g = gestureRef.current;
    return () => {
      if (g.rafId != null) cancelAnimationFrame(g.rafId);
      if (g.tipTimeout != null) window.clearTimeout(g.tipTimeout);
    };
  }, []);

  if (!model) {
    return null;
  }

  const {
    nw,
    cc,
    fireTargetVisible,
    assetSeries,
    assetStacks,
    nwVisible,
    ccVisibleIndices,
    allowNegativeAxis,
    xTicks,
    xScale,
    compoundOutpaceMonth,
    showCompoundOutpaceMarker,
    visibleMilestones,
    chartPlanningMarkers,
    snapshotMarkers,
    pw,
    ph,
    ml,
    mt,
    W,
    rotateXLabels,
    viewHeight,
    visibleMonthStart,
    visibleMonthEnd,
    monthSpan,
    visibleIndices,
  } = model;

  /** Devuelve el valor del array `values` correspondiente al `month` dado.
   * Como `pts` puede ser no equidistante (densidad hybrid), busca el index
   * cuyo `month_index` coincide; si no hay coincidencia exacta, devuelve el
   * más cercano. Para milestones / compound markers / hover en zonas sin
   * punto exacto. */
  const valueAtMonth = (month: number, values: number[]): number | null => {
    if (pts.length === 0) return null;
    let bestIdx = 0;
    let bestDist = Number.POSITIVE_INFINITY;
    for (let i = 0; i < pts.length; i++) {
      const d = Math.abs(pts[i].month_index - month);
      if (d < bestDist) {
        bestDist = d;
        bestIdx = i;
        if (d === 0) break;
      }
    }
    return values[bestIdx] ?? null;
  };

  const yMin = animatedYDomain?.min ?? targetYMin;
  const yMax = animatedYDomain?.max ?? targetYMax;
  const spanY = Math.max(1, yMax - yMin);
  const yScale = (v: number) => mt + ph - ((v - yMin) / spanY) * ph;
  const yTicksRaw = niceYTicks(yMin, yMax, 6);
  let yTicks = allowNegativeAxis ? yTicksRaw : yTicksRaw.filter((t) => t >= 0);
  if (!allowNegativeAxis && yTicks.length < 2) {
    yTicks = niceYTicks(Math.max(0, yMin), yMax, 6);
  }
  // ── Overlay fino histórico (cash-flow anclado a snapshots) ──
  // Curva semanal/diaria del pasado, posicionada por `month_fraction` real (xScale es lineal en
  // meses y admite fracciones) y deflactada con el MISMO deflator fraccional que el resto del
  // chart. La curva pasa exacta por los snapshots (anclaje del engine), así que "abraza" los
  // markers existentes. Es display-only: el hover sigue snapeando a los vértices mensuales.
  // Donde hay daily cargado se usa para su ventana y weekly para lo anterior (stitch contiguo).
  const fineOverlay = (() => {
    if (historyStartMonth >= 0 || visibleMonthStart >= 0) return null;
    const valid = (src: HistoryCashflowApi | null): CashflowFineApi | null => {
      const f = src?.fine;
      // `net_worth` puede ser null legítimamente (sin el pasivo del scope fotografiado entero):
      // en ese caso la curva se dibuja desde `asset_series`, así que null NO invalida el fine.
      if (!f || f.grid.length < 2) return null;
      if (f.net_worth !== null && f.net_worth.length !== f.grid.length) return null;
      return f;
    };
    const weekly = valid(cashflow);
    const daily = valid(cashflowDaily);
    const primary = weekly ?? daily;
    if (!primary) return null;
    const effectivePct =
      inflationAdjusted && installationInflationPct > 0
        ? installationInflationPct
        : 0;
    const visEnd = Math.min(0, visibleMonthEnd);
    // La curva fina tiene que medir LO MISMO que la mensual que continúa. Cuando el pasado son
    // activos (`pastIsAssetsOnly`) el servidor manda `fine.net_worth: null` — precisamente porque
    // ahí no hay patrimonio neto — y la curva se construye con `Σ asset_series`, que es la parte
    // de activos de esa misma cifra (el backend la arma desde `acc.asset_values`) evaluada en el
    // mismo grid fino. Con el pasivo fotografiado entero, `net_worth` existe y es lo correcto.
    const valueAt = (f: CashflowFineApi, i: number): number => {
      if (!merged.pastIsAssetsOnly) return f.net_worth?.[i] ?? 0;
      let sum = 0;
      for (const as of f.asset_series) sum += as.values[i] ?? 0;
      return sum;
    };
    const pointsOf = (f: CashflowFineApi, from: number, to: number): string[] => {
      const parts: string[] = [];
      for (let i = 0; i < f.grid.length; i++) {
        const g = f.grid[i]!;
        if (g.month_fraction < from || g.month_fraction > to) continue;
        const v = valueAt(f, i) * deflationFactorAt(g.month_fraction, effectivePct);
        parts.push(`${xScale(g.month_fraction)},${yScale(v)}`);
      }
      return parts;
    };
    const dailyStart = daily ? daily.grid[0]!.month_fraction : null;
    let parts: string[];
    if (daily && dailyStart != null) {
      const weeklyBefore =
        weekly && weekly.grid[0]!.month_fraction < dailyStart
          ? pointsOf(weekly, visibleMonthStart, Math.min(dailyStart, visEnd))
          : [];
      parts = [
        ...weeklyBefore,
        ...pointsOf(daily, Math.max(visibleMonthStart, dailyStart), visEnd),
      ];
    } else {
      parts = pointsOf(weekly!, visibleMonthStart, visEnd);
    }
    if (parts.length < 2) return null;
    const coverageStart = Math.min(
      weekly ? weekly.grid[0]!.month_fraction : Infinity,
      dailyStart ?? Infinity,
    );
    // Puente con la polilínea mensual: el vértice mensual inmediatamente anterior a la cobertura
    // fina se antepone para unir ambas curvas sin hueco. La mensual se recorta a ese mes (abajo).
    const bridgeMonth = Math.floor(coverageStart);
    if (bridgeMonth >= visibleMonthStart && bridgeMonth < coverageStart) {
      const v = valueAtMonth(bridgeMonth, nw);
      if (v != null && Number.isFinite(v)) {
        parts.unshift(`${xScale(bridgeMonth)},${yScale(v)}`);
      }
    }
    return { points: parts.join(" "), bridgeMonth };
  })();

  // Cada punto visible se posiciona en X según su `month_index` real (no por
  // índice del array). Esto soporta puntos no equidistantes (hybrid).
  const nwPoints = nwVisible
    .map((v, j) => `${xScale(pts[visibleIndices[j]!]!.month_index)},${yScale(v)}`)
    .join(" ");
  // NW partido en el mes 0: el tramo pasado (month_index ≤ 0) y el futuro (≥ 0) comparten el
  // vértice del mes 0 para unirse sin hueco. Solo se usan cuando hay histórico (historyStartMonth < 0).
  // Con overlay fino activo, la mensual se recorta a los meses ANTERIORES a su cobertura (el
  // tramo cubierto lo dibuja la curva fina — dibujar ambas mostraría cuerda + arco divergentes).
  const nwPastPoints = visibleIndices
    .filter(
      (i) =>
        pts[i]!.month_index <= (fineOverlay ? fineOverlay.bridgeMonth : 0),
    )
    .map((i) => `${xScale(pts[i]!.month_index)},${yScale(nw[i] ?? 0)}`)
    .join(" ");
  const nwFuturePoints = visibleIndices
    .filter((i) => pts[i]!.month_index >= 0)
    .map((i) => `${xScale(pts[i]!.month_index)},${yScale(nw[i] ?? 0)}`)
    .join(" ");
  const ccPoints = ccVisibleIndices
    .map((i) => `${xScale(pts[i]!.month_index)},${yScale(cc[i] ?? 0)}`)
    .join(" ");
  const firePoints = fireTargetVisible
    ? fireTargetVisible
        .map((v, j) =>
          v == null
            ? null
            : `${xScale(pts[visibleIndices[j]!]!.month_index)},${yScale(v)}`,
        )
        .filter((s): s is string => s !== null)
        .join(" ")
    : null;
  let areaD = "";
  if (nwVisible.length > 0) {
    const parts: string[] = [];
    nwVisible.forEach((v, j) => {
      const mi = pts[visibleIndices[j]!]!.month_index;
      parts.push(`${j === 0 ? "M" : "L"} ${xScale(mi)} ${yScale(v)}`);
    });
    const xLast = xScale(visibleMonthEnd);
    const x0 = xScale(visibleMonthStart);
    const yBase = mt + ph;
    parts.push(`L ${xLast} ${yBase}`);
    parts.push(`L ${x0} ${yBase} Z`);
    areaD = parts.join(" ");
  }

  /** Convierte px clientX → mes real, clamped al rango visible. */
  function pointerToMonth(clientX: number): number {
    const svg = svgRef.current;
    if (!svg) return visibleMonthStart;
    const rect = svg.getBoundingClientRect();
    const vb = svg.viewBox.baseVal;
    if (!vb || rect.width === 0) return visibleMonthStart;
    const xSvg = ((clientX - rect.left) / rect.width) * vb.width;
    const local = xSvg - ml;
    const t = local / Math.max(1, pw);
    const mi = visibleMonthStart + t * Math.max(1, monthSpan - 1);
    return Math.max(
      visibleMonthStart,
      Math.min(visibleMonthEnd, Math.round(mi)),
    );
  }

  /** Devuelve el índice del array `pts` cuyo `month_index` está más cerca
   * del clientX. Usado por el hover (que necesita acceder a `nw[i]`/`cc[i]`). */
  function pointerToIndex(clientX: number): number {
    const month = pointerToMonth(clientX);
    let best = 0;
    let bestDist = Number.POSITIVE_INFINITY;
    for (let i = 0; i < pts.length; i++) {
      const d = Math.abs(pts[i].month_index - month);
      if (d < bestDist) {
        bestDist = d;
        best = i;
        if (d === 0) break;
      }
    }
    return best;
  }

  // Dominio panorámico compartido por la máquina de gestos (espejo de onWheel).
  const gestureDomain: ChartDomain = {
    totalMonths: series.months,
    historyStartMonth,
  };

  /** Cuerpo compartido por el hover de ratón y el tap táctil: fija el índice
   * `hover` y la posición del tooltip. `coarse` (tap) eleva más el tooltip sobre
   * el dedo y clampa su centro horizontal para no recortarse contra el
   * `overflow:hidden` del chart. Para ratón (`coarse=false`) el comportamiento es
   * byte-idéntico al original. */
  function showTooltipAt(clientX: number, clientY: number, coarse = false) {
    const i = pointerToIndex(clientX);
    setHover(i);
    const wrap = wrapRef.current;
    if (!wrap) return;
    const r = wrap.getBoundingClientRect();
    let x = clientX - r.left;
    let y = clientY - r.top;
    if (coarse) {
      y -= COARSE_TIP_LIFT_PX;
      // El tooltip se centra vía translate(-50%): mantener su centro dentro del
      // wrap (tipHalf estima la mitad del max-width min(16rem, 100vw−2rem)).
      const tipHalf = Math.min(128, Math.max(0, (r.width - 32) / 2));
      x = Math.max(tipHalf, Math.min(r.width - tipHalf, x));
    }
    setTipOffset({ x, y });
  }

  function onPointerMove(e: PointerEvent<SVGSVGElement>) {
    // La ruta táctil NO ejecuta el hover de ratón: la conduce la máquina de gestos.
    if (e.pointerType === "touch") {
      handleTouchMove(e);
      return;
    }
    showTooltipAt(e.clientX, e.clientY);
  }

  function onPointerLeave(e: PointerEvent<SVGSVGElement>) {
    // En táctil el tooltip persiste hasta timeout/nuevo gesto; no lo mata el leave.
    if (e.pointerType === "touch") return;
    setHover(null);
  }

  // ── Máquina de gestos táctiles (Pointer Events) ──
  // Estados: idle → maybe → (tap | pan | pinch | yield) → idle.
  // pan-y cede el swipe vertical al scroll de página (→ pointercancel aborta).

  /** Meses por píxel-CLIENTE del plot, para el pan 1:1. Convierte los px del dedo
   * a meses respetando el escalado del SVG (viewBox → CSS). Se captura una vez. */
  function computeMonthsPerClientPx(): number {
    const svg = svgRef.current;
    if (!svg) return 0;
    const rect = svg.getBoundingClientRect();
    const vb = svg.viewBox.baseVal;
    if (!vb || rect.width === 0 || vb.width === 0) return 0;
    const clientPerUserX = rect.width / vb.width;
    const plotWidthClientPx = pw * clientPerUserX;
    if (plotWidthClientPx <= 0) return 0;
    return Math.max(1, monthSpan - 1) / plotWidthClientPx;
  }

  /** Commit de la ventana coalescido por rAF: muchos pointermove por frame → un
   * solo setViewWindow por frame. */
  function commitWindow(next: { startMonth: number; monthSpan: number }) {
    const g = gestureRef.current;
    g.pendingWindow = next;
    if (g.rafId == null) {
      g.rafId = requestAnimationFrame(() => {
        g.rafId = null;
        const w = g.pendingWindow;
        g.pendingWindow = null;
        if (w) {
          setViewWindow((prev) =>
            prev.startMonth === w.startMonth && prev.monthSpan === w.monthSpan
              ? prev
              : w,
          );
        }
      });
    }
  }

  /** Vuelca de inmediato cualquier ventana pendiente (al soltar el pan). */
  function flushPendingWindow() {
    const g = gestureRef.current;
    if (g.rafId != null) {
      cancelAnimationFrame(g.rafId);
      g.rafId = null;
    }
    const w = g.pendingWindow;
    g.pendingWindow = null;
    if (w) {
      setViewWindow((prev) =>
        prev.startMonth === w.startMonth && prev.monthSpan === w.monthSpan
          ? prev
          : w,
      );
    }
  }

  function scheduleTipClose() {
    const g = gestureRef.current;
    if (g.tipTimeout != null) window.clearTimeout(g.tipTimeout);
    g.tipTimeout = window.setTimeout(() => {
      g.tipTimeout = null;
      setHover(null);
    }, TIP_CLOSE_MS);
  }

  function clearTipTimeout() {
    const g = gestureRef.current;
    if (g.tipTimeout != null) {
      window.clearTimeout(g.tipTimeout);
      g.tipTimeout = null;
    }
  }

  function releasePointerCaptureSafe(pointerId: number) {
    const svg = svgRef.current;
    const g = gestureRef.current;
    if (svg && g.captured && g.capturedPointerId === pointerId) {
      try {
        svg.releasePointerCapture(pointerId);
      } catch {
        /* ya liberado / puntero desaparecido */
      }
      g.captured = false;
      g.capturedPointerId = null;
    }
  }

  /** Resetea el estado del gesto (no toca el tooltip: sobrevive tras un tap). */
  function resetGesture() {
    const g = gestureRef.current;
    if (g.rafId != null) {
      cancelAnimationFrame(g.rafId);
      g.rafId = null;
    }
    g.pendingWindow = null;
    g.pointers.clear();
    g.phase = "idle";
    g.captured = false;
    g.capturedPointerId = null;
  }

  /** Compromete un pan: fija snapshot de ventana + monthsPerPx, resetea el origen
   * al punto actual (pan 1:1 desde aquí, sin salto de SLOP) y CAPTURA el puntero
   * (perezosamente — solo ahora) para no interferir con la decisión del compositor. */
  function beginPan(e: PointerEvent<SVGSVGElement>) {
    const g = gestureRef.current;
    g.phase = "pan";
    g.windowStart = visibleMonthStart;
    g.windowSpan = monthSpan;
    g.monthsPerPx = computeMonthsPerClientPx();
    g.startClientX = e.clientX;
    g.startClientY = e.clientY;
    const svg = svgRef.current;
    if (svg && !g.captured) {
      try {
        svg.setPointerCapture(e.pointerId);
        g.captured = true;
        g.capturedPointerId = e.pointerId;
      } catch {
        /* captura no disponible: el pan sigue funcionando sin ella */
      }
    }
  }

  function handleTouchMove(e: PointerEvent<SVGSVGElement>) {
    const g = gestureRef.current;
    const p = g.pointers.get(e.pointerId);
    if (!p) return;
    p.x = e.clientX;
    p.y = e.clientY;

    if (g.phase === "pinch" && g.pointers.size >= 2) {
      const it = g.pointers.values();
      const a = it.next().value!;
      const b = it.next().value!;
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      if (g.pinchStartDist > 0) {
        const scale = dist / g.pinchStartDist;
        commitWindow(
          pinchWindow(
            g.windowStart,
            g.windowSpan,
            g.pinchAnchorMonth,
            scale,
            gestureDomain,
          ),
        );
      }
      return;
    }

    if (g.phase === "pan") {
      const dxPx = e.clientX - g.startClientX;
      commitWindow({
        startMonth: panWindow(
          g.windowStart,
          g.windowSpan,
          dxPx,
          g.monthsPerPx,
          gestureDomain,
        ),
        monthSpan: g.windowSpan,
      });
      return;
    }

    if (g.phase === "maybe") {
      const dx = e.clientX - g.startClientX;
      const dy = e.clientY - g.startClientY;
      if (Math.hypot(dx, dy) > SLOP_PX) {
        if (Math.abs(dx) > Math.abs(dy)) {
          // Intención horizontal → pan JS (pan-y no reserva el eje horizontal).
          beginPan(e);
        } else {
          // Intención vertical → ceder al scroll de página. El navegador toma el
          // puntero (pan-y) y emitirá pointercancel; no volvemos a panear.
          g.phase = "yield";
        }
      }
      return;
    }
    // phase === "yield": ignoramos moves; esperamos up/cancel.
  }

  function onPointerDown(e: PointerEvent<SVGSVGElement>) {
    if (e.pointerType !== "touch") return;
    const g = gestureRef.current;
    // Cualquier gesto nuevo cierra el tooltip abierto (y su timeout).
    clearTipTimeout();
    setHover(null);
    g.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });

    if (g.pointers.size >= 2) {
      // Segundo dedo → pinch anclado al punto medio, con snapshot de la ventana.
      const it = g.pointers.values();
      const a = it.next().value!;
      const b = it.next().value!;
      const midX = (a.x + b.x) / 2;
      g.pinchStartDist = Math.hypot(a.x - b.x, a.y - b.y);
      g.windowStart = visibleMonthStart;
      g.windowSpan = monthSpan;
      g.pinchAnchorMonth = pointerToMonth(midX);
      // Un pan en curso deja de estar capturado: pasamos a pinch (2 dedos).
      if (g.capturedPointerId != null) {
        releasePointerCaptureSafe(g.capturedPointerId);
      }
      g.captured = false;
      g.capturedPointerId = null;
      g.phase = "pinch";
      return;
    }

    // Primer dedo → maybe (aún indeciso entre tap / pan / scroll vertical).
    g.phase = "maybe";
    g.startClientX = e.clientX;
    g.startClientY = e.clientY;
    g.startTimeMs = performance.now();
  }

  function onPointerUp(e: PointerEvent<SVGSVGElement>) {
    if (e.pointerType !== "touch") return;
    const g = gestureRef.current;
    const phase = g.phase;
    g.pointers.delete(e.pointerId);
    releasePointerCaptureSafe(e.pointerId);

    if (phase === "maybe") {
      // ¿Tap? Movimiento ≤ SLOP y duración ≤ TAP_MAX_MS → tooltip.
      const dx = e.clientX - g.startClientX;
      const dy = e.clientY - g.startClientY;
      const dt = performance.now() - g.startTimeMs;
      if (Math.hypot(dx, dy) <= SLOP_PX && dt <= TAP_MAX_MS) {
        showTooltipAt(e.clientX, e.clientY, true);
        scheduleTipClose();
      }
      resetGesture();
      return;
    }

    if (phase === "pan") {
      flushPendingWindow();
      resetGesture();
      return;
    }

    // pinch / yield / idle: al soltar un dedo cerramos el gesto entero.
    resetGesture();
  }

  function onPointerCancel(e: PointerEvent<SVGSVGElement>) {
    if (e.pointerType !== "touch") return;
    // El navegador reclamó el puntero (scroll vertical bajo pan-y): abortar.
    releasePointerCaptureSafe(e.pointerId);
    resetGesture();
  }

  function onWheel(e: WheelEvent<SVGSVGElement>) {
    e.preventDefault();
    if (pts.length < 2) return;
    const panInput = e.shiftKey ? e.deltaY : e.deltaX;
    const isPan = Math.abs(panInput) > Math.abs(e.deltaY) || e.shiftKey;
    const totalMonths = series.months;
    // El dominio panorámico incluye el pasado histórico (historyStartMonth ≤ 0): el span máximo y
    // el borde izquierdo del pan/zoom deben respetarlo, o una sola rueda expulsaría el histórico.
    // Espeja la aritmética del memo `model` (domainMonths / lower bound = historyStartMonth).
    const domainMonths = totalMonths - historyStartMonth;
    const minSpan = Math.min(totalMonths, 12);
    if (isPan && monthSpan < domainMonths) {
      const step = Math.max(1, Math.round(monthSpan * 0.08));
      const direction = panInput > 0 ? 1 : -1;
      const maxStart = Math.max(historyStartMonth, totalMonths - monthSpan);
      const nextStart = Math.max(
        historyStartMonth,
        Math.min(maxStart, visibleMonthStart + direction * step),
      );
      if (nextStart !== visibleMonthStart) {
        setViewWindow({ startMonth: nextStart, monthSpan });
      }
      return;
    }
    if (e.deltaY === 0) return;
    const zoomFactor = e.deltaY < 0 ? 0.88 : 1.14;
    const nextSpanRaw = Math.round(monthSpan * zoomFactor);
    const nextSpan = Math.max(minSpan, Math.min(domainMonths, nextSpanRaw));
    if (nextSpan === monthSpan) return;
    const anchorMonth = pointerToMonth(e.clientX);
    const ratio = (anchorMonth - visibleMonthStart) / Math.max(1, monthSpan - 1);
    const nextStartRaw = Math.round(anchorMonth - ratio * (nextSpan - 1));
    const maxStart = Math.max(historyStartMonth, totalMonths - nextSpan);
    const nextStart = Math.max(historyStartMonth, Math.min(maxStart, nextStartRaw));
    setViewWindow({ startMonth: nextStart, monthSpan: nextSpan });
  }

  const horizonLine = formatProjectionChartHorizonLine(series);
  const deltaStr = formatCurrencyAmount(series.monthly_delta_assumption, currencyIso);
  // Base del Δ mensual: presupuesto (modo A) o promedio de movimientos (modos B y C, ya con
  // el fallback del servidor aplicado). Se mantiene corto: comparte línea con más metadatos.
  //
  // Sale de los dos `*_basis` porque el escalar `savings_source_months_with_data` que se leía aquí
  // dejó de enviarse en 3.9.0, al hacerse las ventanas configurables por lado — y esto llevaba
  // desde entonces pintando «prom. 0 meses» en los modos B y C. Con dos ventanas puede haber dos
  // denominadores distintos (un lado 3 meses, el otro 12): si difieren se dicen los dos, porque
  // enseñar uno solo mal-etiqueta la mitad de la cifra.
  const incomeMonths = series.savings_income_basis?.avg_months ?? 0;
  const expenseMonths = series.savings_expense_basis?.avg_months ?? 0;
  const monthsLabel = (n: number) => `${n} ${n === 1 ? "mes" : "meses"}`;
  const deltaBaseLabel = savingsSourceUsesTransactions(series.savings_source)
    ? incomeMonths === expenseMonths
      ? `prom. ${monthsLabel(incomeMonths)}`
      : `prom. ${monthsLabel(incomeMonths)} ingreso / ${monthsLabel(expenseMonths)} gasto`
    : "presup.";
  const scopeShort = ledgerPersonScope === "mine" ? "Mi vista" : "Hogar";
  const inflationShort =
    installationInflationPct > 0
      ? inflationAdjusted
        ? `Dinero de hoy (deflactado ~${inflationPctDisplay ?? `${installationInflationPct}%`} anual)`
        : `Patrimonio nominal · target FIRE +${inflationPctDisplay ?? `${installationInflationPct}%`} anual`
      : "Sin inflación · target FIRE plano";

  return (
    <div
      ref={wrapRef}
      className="projection-chart-root projection-chart-root--fullbleed bordered-top"
    >
      <div ref={plotRef} className="projection-chart-plot">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${viewHeight}`}
        preserveAspectRatio="xMidYMid meet"
        className="projection-chart-svg"
        style={{
          aspectRatio: `${W} / ${viewHeight}`,
        }}
        role="application"
        aria-label="Proyección de patrimonio neto y capital aportado acumulado"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
        onWheel={onWheel}
      >
        <title>Patrimonio neto y capital aportado en el tiempo</title>
        <defs>
          <linearGradient id={`nwFill-${gid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--proj-nw)" stopOpacity="0.18" />
            <stop offset="100%" stopColor="var(--proj-nw)" stopOpacity="0.02" />
          </linearGradient>
          <clipPath id={`projectionPlotClip-${gid}`}>
            <rect x={ml} y={mt} width={pw} height={ph} rx={6} />
          </clipPath>
        </defs>

        <text x={ml} y={layoutDims.headlineBlockTopY} className="projection-chart-headline">
          {scopeShort}
        </text>
        <text x={ml} y={layoutDims.headlineBlockTopY + 22} className="projection-chart-meta">
          {horizonLine}
        </text>
        {isMobile ? (
          // La línea combinada no cabe en un plot de ~380px y rozaba el borde
          // derecho sin padding: en móvil se parte en dos (mt lo compensa vía
          // compactHeader).
          <>
            <text
              x={ml}
              y={layoutDims.headlineBlockTopY + 40}
              className="projection-chart-meta"
            >
              {inflationShort}
            </text>
            <text
              x={ml}
              y={layoutDims.headlineBlockTopY + 58}
              className="projection-chart-meta"
            >
              Δ regular {deltaBaseLabel} {deltaStr}/mes
            </text>
          </>
        ) : (
          <text
            x={ml}
            y={layoutDims.headlineBlockTopY + 40}
            className="projection-chart-meta"
          >
            {inflationShort} · Δ regular {deltaBaseLabel} {deltaStr}/mes
          </text>
        )}

        {yTicks.map((yt) => (
          <g key={`gy-${yt}`}>
            <line
              x1={ml}
              y1={yScale(yt)}
              x2={ml + pw}
              y2={yScale(yt)}
              className="projection-chart-grid"
            />
            {/* Móvil: sin etiquetas del eje Y (estilo MiniProjection) — el valor
                exacto vive en el tooltip; el plot gana todo el margen izquierdo. */}
            {!isMobile ? (
              <text
                x={ml - 10}
                y={yScale(yt)}
                textAnchor="end"
                dominantBaseline="middle"
                className="projection-chart-tick"
              >
                {formatAxisMoney(yt, currencyIso)}
              </text>
            ) : null}
          </g>
        ))}

        {xTicks.map(({ monthIndex, label }) => {
          const cx = xScale(monthIndex);
          const tickY =
            mt + ph + (layoutDims.narrow ? 12 : 14) + (rotateXLabels ? 8 : 0);
          return (
            <text
              key={`gx-${monthIndex}`}
              transform={
                rotateXLabels
                  ? `rotate(38 ${cx.toFixed(2)} ${tickY.toFixed(2)})`
                  : undefined
              }
              x={cx}
              y={tickY}
              textAnchor="start"
              dominantBaseline={rotateXLabels ? "middle" : "auto"}
              className={`projection-chart-tick${rotateXLabels ? " projection-chart-tick--xrot" : ""}`}
            >
              {label}
            </text>
          );
        })}

        <rect
          x={ml}
          y={mt}
          width={pw}
          height={ph}
          fillOpacity={0.35}
          rx={6}
          className="projection-chart-plot-bg"
        />

        {visibleMonthStart < 0 ? (
          <rect
            x={xScale(visibleMonthStart)}
            y={mt}
            width={Math.max(
              0,
              xScale(Math.min(0, visibleMonthEnd)) - xScale(visibleMonthStart),
            )}
            height={ph}
            className="projection-chart-past-bg"
          />
        ) : null}

        {historyStartMonth < 0 &&
        visibleMonthStart <= 0 &&
        visibleMonthEnd >= 0 ? (
          <>
            <line
              x1={xScale(0)}
              x2={xScale(0)}
              y1={mt}
              y2={mt + ph}
              className="projection-chart-today-divider"
            />
            {/* «Hoy» se alinea con la fila de etiquetas del eje X (misma baseline y
                rotación que los años) en vez de flotar sobre el plot pegado al
                subtítulo. Es texto, no controla nada: la navegación no cambia. */}
            {(() => {
              const cx = xScale(0);
              const tickY =
                mt + ph + (layoutDims.narrow ? 12 : 14) + (rotateXLabels ? 8 : 0);
              return (
                <text
                  transform={
                    rotateXLabels
                      ? `rotate(38 ${cx.toFixed(2)} ${tickY.toFixed(2)})`
                      : undefined
                  }
                  x={cx}
                  y={tickY}
                  textAnchor="start"
                  dominantBaseline={rotateXLabels ? "middle" : "auto"}
                  className="projection-chart-today-label"
                >
                  Hoy
                </text>
              );
            })()}
          </>
        ) : null}

        <g clipPath={`url(#projectionPlotClip-${gid})`}>
          {assetSeries.length === 0 ? (
            <path d={areaD} fill={`url(#nwFill-${gid})`} stroke="none" />
          ) : (
            assetSeries.map((as, idx) => {
              const color = ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length];
              const stack = assetStacks[idx];
              const topParts: string[] = [];
              const botParts: string[] = [];
              for (const k of visibleIndices) {
                const mi = pts[k]!.month_index;
                topParts.push(`${xScale(mi)},${yScale(stack.tops[k] ?? 0)}`);
              }
              for (let j = visibleIndices.length - 1; j >= 0; j--) {
                const k = visibleIndices[j]!;
                const mi = pts[k]!.month_index;
                botParts.push(`${xScale(mi)},${yScale(stack.bottoms[k] ?? 0)}`);
              }
              const d = `M ${topParts.join(" L ")} L ${botParts.join(" L ")} Z`;
              return (
                <path
                  key={as.id}
                  d={d}
                  fill={color}
                  fillOpacity={0.14}
                  stroke={color}
                  strokeWidth={0.8}
                  strokeOpacity={0.4}
                />
              );
            })
          )}
          {historyStartMonth < 0 ? (
            <>
              <polyline
                points={nwPastPoints}
                fill="none"
                stroke="var(--proj-nw-past)"
                strokeWidth={2.25}
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              {fineOverlay ? (
                <polyline
                  points={fineOverlay.points}
                  fill="none"
                  stroke="var(--proj-nw-past)"
                  strokeWidth={2.25}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              ) : null}
              <polyline
                points={nwFuturePoints}
                fill="none"
                stroke="var(--proj-nw)"
                strokeWidth={2.85}
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </>
          ) : (
            <polyline
              points={nwPoints}
              fill="none"
              stroke="var(--proj-nw)"
              strokeWidth={2.85}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          )}
          <polyline
            points={ccPoints}
            fill="none"
            stroke="var(--proj-cc)"
            strokeWidth={2.1}
            strokeDasharray="7 5"
            strokeLinecap="round"
            strokeLinejoin="round"
            opacity={0.92}
          />
          {firePoints ? (
            <polyline
              points={firePoints}
              fill="none"
              stroke="var(--proj-fire)"
              strokeWidth={1.2}
              strokeDasharray="3 4"
              strokeLinecap="round"
              strokeLinejoin="round"
              opacity={0.225}
            />
          ) : null}
          {historyStartMonth < 0
            ? snapshotMarkers.map((mk, idx) => {
                const val = valueAtMonth(mk.month_index, nw);
                if (val == null || !Number.isFinite(val)) return null;
                const isAsset = mk.kind === "asset";
                return (
                  <circle
                    key={`snap-${mk.kind}-${mk.owner_user_id}-${mk.month_index}-${idx}`}
                    cx={xScale(mk.month_index)}
                    cy={yScale(val)}
                    r={4}
                    fill={isAsset ? "var(--proj-snapshot)" : "var(--proj-plot-bg)"}
                    stroke={isAsset ? "var(--proj-plot-bg)" : "var(--proj-snapshot)"}
                    strokeWidth={1.5}
                  />
                );
              })
            : null}
          {(() => {
            // Calcula la posición inicial (siguiendo la curva NW) y luego
            // empuja hacia arriba los milestones que se solapan horizontalmente
            // colocándolos en "carriles" sucesivos. La línea punteada se
            // estira para acompañar la nueva y2, manteniendo continuidad.
            type MS = {
              m: typeof visibleMilestones[number];
              x: number;
              y0: number;
              y1Base: number;
              y1: number;
              label: string;
              halfW: number;
              isJubilacion: boolean;
            };
            // 22 y no 12: la etiqueta se pinta en y1−6 con ~10px de glifo por encima
            // de la baseline; con el suelo antiguo, un milestone cuyo NW roza el techo
            // del plot (p. ej. «Jubilación» en la vista cercana móvil) quedaba con la
            // mitad superior recortada por el clip del plot.
            const y1Floor = mt + 22;
            const items: MS[] = visibleMilestones
              .map((m) => {
                const x = xScale(m.reached_month_index);
                const y0 = mt + ph;
                const nwAtMilestone = valueAtMonth(m.reached_month_index, nw);
                const nwY =
                  nwAtMilestone != null && Number.isFinite(nwAtMilestone)
                    ? yScale(nwAtMilestone)
                    : null;
                const y1FromNetWorth =
                  nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
                const y1Base = Math.max(
                  y1Floor,
                  Math.min(y0 - 8, y1FromNetWorth),
                );
                const isJubilacion = m.target === "jubilacion";
                const targetNum = parseDisplayDecimal(m.target);
                const label =
                  targetNum != null
                    ? formatCurrencyNumber(targetNum, currencyIso)
                    : formatProjectionMilestoneCompactLabel(m.target);
                // 5.5px por carácter aprox. + 4 de padding por lado.
                const halfW = (label.length * 5.5) / 2 + 4;
                return {
                  m,
                  x,
                  y0,
                  y1Base,
                  y1: y1Base,
                  label,
                  halfW,
                  isJubilacion,
                };
              })
              .sort((a, b) => a.x - b.x);

            type LaneSlot = { right: number };
            const lanes: LaneSlot[] = [];
            const rowHeight = 14;
            for (const it of items) {
              const left = it.x - it.halfW;
              let lane = 0;
              while (lane < lanes.length && lanes[lane]!.right > left) lane++;
              if (lane === lanes.length) {
                lanes.push({ right: it.x + it.halfW });
              } else {
                lanes[lane] = { right: it.x + it.halfW };
              }
              it.y1 = Math.max(y1Floor - 4, it.y1Base - lane * rowHeight);
            }

            return items.map((it) => (
              <g key={`ms-${it.m.target}-${it.m.reached_month_index}`}>
                <line
                  x1={it.x}
                  x2={it.x}
                  y1={it.y0}
                  y2={it.y1}
                  className={
                    it.isJubilacion
                      ? "projection-chart-jubilacion-line"
                      : "projection-chart-milestone-line"
                  }
                />
                <text
                  x={it.x}
                  y={it.y1 - 6}
                  textAnchor="middle"
                  className={
                    it.isJubilacion
                      ? "projection-chart-jubilacion-label"
                      : "projection-chart-milestone-label"
                  }
                >
                  {it.label}
                </text>
              </g>
            ));
          })()}
          {chartPlanningMarkers.map((m) => {
            const x = xScale(m.mi);
            const y0 = mt + ph;
            // `m.mi` es un month_index real, no un índice de array: con densidad hybrid o ventana
            // extendida al pasado, `nw[m.mi]` leería la posición equivocada. Buscar por mes.
            const nwAtMi = valueAtMonth(m.mi, nw);
            const nwY =
              nwAtMi != null && Number.isFinite(nwAtMi) ? yScale(nwAtMi) : null;
            const y1Floor = mt + 12;
            const y1FromNetWorth =
              nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
            const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
            const isInflow = m.direction === "inflow";
            const label =
              m.title.length > 18 ? m.title.slice(0, 17) + "…" : m.title;
            return (
              <g key={`pf-${m.id}`}>
                <line
                  x1={x}
                  x2={x}
                  y1={y0}
                  y2={y1}
                  className={
                    isInflow
                      ? "projection-chart-planning-inflow-line"
                      : "projection-chart-planning-outflow-line"
                  }
                />
                <text
                  x={x}
                  y={y1 - 6}
                  textAnchor="middle"
                  className={
                    isInflow
                      ? "projection-chart-planning-inflow-label"
                      : "projection-chart-planning-outflow-label"
                  }
                >
                  {label}
                </text>
              </g>
            );
          })}
          {showCompoundOutpaceMarker && compoundOutpaceMonth != null
            ? (() => {
                const x = xScale(compoundOutpaceMonth);
                const y0 = mt + ph;
                const nwAtMs = valueAtMonth(compoundOutpaceMonth, nw);
                const nwY =
                  nwAtMs != null && Number.isFinite(nwAtMs)
                    ? yScale(nwAtMs)
                    : null;
                const y1Floor = mt + 12;
                const y1FromNetWorth =
                  nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
                const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
                return (
                  <g>
                    <line
                      x1={x}
                      x2={x}
                      y1={y0}
                      y2={y1}
                      className="projection-chart-milestone-line"
                    />
                    <text
                      x={x}
                      y={y1 - 6}
                      textAnchor="middle"
                      className="projection-chart-milestone-label"
                    >
                      Interés &gt; ahorro
                    </text>
                  </g>
                );
              })()
            : null}

          {hover !== null &&
          pts[hover] != null &&
          pts[hover]!.month_index >= visibleMonthStart &&
          pts[hover]!.month_index <= visibleMonthEnd ? (
            <line
              x1={xScale(pts[hover]!.month_index)}
              x2={xScale(pts[hover]!.month_index)}
              y1={mt}
              y2={mt + ph}
              className="projection-chart-crosshair"
            />
          ) : null}
          {hover !== null &&
          pts[hover] != null &&
          pts[hover]!.month_index >= visibleMonthStart &&
          pts[hover]!.month_index <= visibleMonthEnd ? (
            <>
              <circle cx={xScale(pts[hover]!.month_index)} cy={yScale(nw[hover] ?? 0)} r={6} className="projection-chart-dot-nw" />
              {pts[hover]!.month_index >= 0 ? (
                <circle cx={xScale(pts[hover]!.month_index)} cy={yScale(cc[hover] ?? 0)} r={5} className="projection-chart-dot-cc" />
              ) : null}
            </>
          ) : null}
        </g>

        {!isMobile ? (
          <text
            transform={`translate(${Math.min(30, ml * 0.32)}, ${mt + ph / 2}) rotate(-90)`}
            textAnchor="middle"
            className="projection-chart-axis-caption"
          >
            {normalizeCurrencyIso(currencyIso) ?? "Importe"}
          </text>
        ) : null}
      </svg>
      </div>

      <ChartLegend
        structural={legendStructural}
        assets={legendAssets}
        collapsedCap={collapsedAssetLegendCap(
          containerSize.width > 0 ? containerSize.width : 1040,
        )}
        size="md"
        ariaLabel="Series del gráfico de proyección"
      />

      {hover !== null &&
      pts[hover] != null &&
      pts[hover]!.month_index >= visibleMonthStart &&
      pts[hover]!.month_index <= visibleMonthEnd ? (
        <div
          className="projection-chart-tooltip"
          style={{
            left: tipOffset.x,
            top: tipOffset.y,
          }}
        >
          <div className="projection-chart-tooltip-title">
            {/* Fix: el título debe usar el `month_index` real del punto, no su posición en el
                array (con densidad hybrid o histórico difieren). */}
            {projectionHoverTitle(
              pts[hover]!.month_index,
              ageUiMode,
              userBirthDate,
              calendarTz,
              anchorDateYmd,
            )}
            {pts[hover]!.month_index < 0 ? " · histórico" : ""}
          </div>
          <div>
            {/* En el pasado sin pasivo fotografiado, la curva son ACTIVOS: llamarla «patrimonio
                neto» aquí sería repetir en el tooltip el número que el servidor ya se niega a
                publicar con ese nombre. El futuro sale de la proyección y sí es neto. */}
            {merged.pastIsAssetsOnly && pts[hover]!.month_index < 0
              ? "Activos"
              : "Patrimonio neto"}{" "}
            — {formatCurrencyOrDashNumber(nw[hover], currencyIso)}
          </div>
          {pts[hover]!.month_index >= 0 ? (
            <div>
              Capital aportado —{" "}
              {formatCurrencyOrDashNumber(cc[hover], currencyIso)}
            </div>
          ) : null}
          {(() => {
            // Top-N por |valor| en el mes hovered + agregado «Otros»: el tooltip no
            // escala listando N activos (misma disciplina que la leyenda).
            const { shown, hiddenCount, hiddenTotal } = topAssetTooltipRows(
              assetSeries.map((as) => ({
                id: as.id,
                label: assetLabelById[as.id] ?? as.name,
                value: as.values[hover!],
              })),
            );
            return (
              <>
                {shown.map((r) => (
                  <div key={r.id}>
                    {r.label} —{" "}
                    {formatCurrencyOrDashNumber(r.value, currencyIso)}
                  </div>
                ))}
                {hiddenCount > 0 ? (
                  <div className="projection-chart-tooltip-rest">
                    Otros ({hiddenCount}) —{" "}
                    {formatCurrencyOrDashNumber(hiddenTotal, currencyIso)}
                  </div>
                ) : null}
              </>
            );
          })()}
          {pts[hover]!.month_index < 0
            ? merged.markers
                .filter((mk) => mk.month_index === pts[hover]!.month_index)
                .map((mk, idx) => (
                  <div key={`hovmk-${mk.kind}-${mk.owner_user_id}-${idx}`}>
                    {mk.kind === "asset"
                      ? "Snapshot activos"
                      : "Snapshot pasivos"}{" "}
                    — {formatDateDmy(mk.date_ymd)}
                  </div>
                ))
            : null}
        </div>
      ) : null}
    </div>
  );
}
