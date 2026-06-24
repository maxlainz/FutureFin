import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import type {
  PlanningFlowApiRow,
  PlanningFlowDirectionApi,
  ProjectionMilestoneApi,
  ProjectionSeriesApi,
} from "../api/types";
import { parseYmdComponents } from "../lib/dates";
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
  formatProjectionChartHorizonLine,
  niceYTicks,
  projectionHoverTitle,
  projectionXTickLabel,
  projectionXTicks,
} from "../lib/projection-chart";
import {
  formatAxisMoney,
  formatProjectionMilestoneCompactLabel,
  type LedgerPersonScope,
} from "../lib/ledger";
import { chartPerf } from "../lib/perf";

export function ProjectionNetWorthChart({
  series,
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
}: {
  series: ProjectionSeriesApi;
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
}) {
  // `mount-start` se mide en el primer render (cuando el chart aparece en
  // pantalla), separado de `fetch-start` que ocurre cuando llega la serie.
  // Esto distingue el tiempo de cómputo del chart del delay entre fetch y
  // navegación (que puede ser "tiempo del usuario", no del sistema).
  chartPerf.mark("mount-start");
  const pts = series.points;
  const gid = useId().replace(/:/g, "");
  const svgRef = useRef<SVGSVGElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const yAxisAnimRef = useRef<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [tipOffset, setTipOffset] = useState({ x: 0, y: 0 });
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
  // viewWindow está en **meses reales** (no en índices de array). Soporta
  // densidades mixtas: con `density=monthly` los puntos van 0..months con paso
  // 1; con `density=hybrid` van 0..12 con paso 1 y luego 24, 36, ... La
  // ventana se mueve siempre sobre el horizonte temporal completo
  // (`series.months`), independiente de cuántos puntos haya serializados.
  const [viewWindow, setViewWindow] = useState({
    startMonth: 0,
    monthSpan: series.months,
  });
  const [animatedYDomain, setAnimatedYDomain] = useState<{
    min: number;
    max: number;
  } | null>(null);
  const animatedYDomainRef = useRef<{ min: number; max: number } | null>(null);

  useLayoutEffect(() => {
    const node = wrapRef.current;
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
    const clampedEnd = Math.max(0, Math.min(series.months - 1, focusEnd));
    return { startMonth: 0, monthSpan: clampedEnd + 1 };
  }, [milestones, pts.length, series.months]);

  useEffect(() => {
    setViewWindow((prev) => {
      if (pts.length <= 0) return { startMonth: 0, monthSpan: 0 };
      const next = focusMode && focusWindow
        ? focusWindow
        : { startMonth: 0, monthSpan: series.months };
      if (prev.startMonth === next.startMonth && prev.monthSpan === next.monthSpan) {
        return prev;
      }
      return next;
    });
  }, [focusMode, focusWindow, pts.length, series.months]);

  const hasFireTargetSeries = useMemo(() => {
    const f = series.fire_target_series;
    return Array.isArray(f) && f.length === series.points.length && f.length > 0;
  }, [series.fire_target_series, series.points.length]);

  const legendLabels = useMemo(() => {
    const assetNames = (series.asset_series ?? []).map((as) => as.asset_name);
    const labels: string[] = ["Patrimonio neto", "Capital aportado"];
    if (hasFireTargetSeries) labels.push("Target FIRE");
    labels.push(...assetNames);
    return labels;
  }, [series.asset_series, hasFireTargetSeries]);

  const layoutDims = useMemo(
    () =>
      buildProjectionChartLayout(
        containerSize.width > 0 ? containerSize.width : 1040,
        containerSize.height > 0 ? containerSize.height : undefined,
        legendLabels,
      ),
    [containerSize.height, containerSize.width, legendLabels],
  );

  // Series base: deflactación, orden de activos, stacking acumulado completo.
  // No depende de viewWindow → pan/zoom no recalculan este bloque.
  const baseSeries = useMemo(() => {
    chartPerf.mark("baseSeries-start");
    if (pts.length < 2) {
      chartPerf.mark("baseSeries-end");
      return null;
    }
    const deflate = inflationAdjusted && installationInflationPct > 0;
    // El deflactor debe usar el `month_index` real del punto, no su posición en el array: con
    // densidad `hybrid` los puntos no son equidistantes (mes 0..12 y luego 24, 36, …), así que el
    // índice del array subestimaría los años transcurridos y deflactaría de menos. Esto mantiene la
    // curva alineada con los `milestones_real` del backend (calculados sobre `month_index`).
    const deflator = (monthIndex: number) =>
      deflate
        ? 1 / Math.pow(1 + installationInflationPct / 100, monthIndex / 12)
        : 1;
    const miAt = (i: number) => pts[i]?.month_index ?? i;
    const nw = pts.map((p) => p.net_worth * deflator(p.month_index));
    const cc = pts.map((p) => p.contributed_capital * deflator(p.month_index));
    const fireRaw = series.fire_target_series;
    const fireTarget =
      Array.isArray(fireRaw) && fireRaw.length === pts.length
        ? fireRaw.map((v, i) => v * deflator(miAt(i)))
        : null;
    const assetSeries = (series.asset_series ?? [])
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
    const allowNegativeAxis = startNw < 0;
    chartPerf.mark("baseSeries-end");
    return { nw, cc, fireTarget, assetSeries, assetStacks, allowNegativeAxis };
  }, [
    pts,
    series.starting_net_worth,
    series.asset_series,
    series.fire_target_series,
    inflationAdjusted,
    installationInflationPct,
  ]);

  // xTicks completos del horizonte. No dependen de viewWindow.
  const xTicksAll = useMemo(() => {
    chartPerf.mark("xTicks-start");
    const t = projectionXTicks(
      series.months,
      { ageUiMode, birthDateIso: userBirthDate, anchorDateYmd, calendarTz },
      { plotWidthPx: layoutDims.pw },
    );
    chartPerf.mark("xTicks-end");
    return t;
  }, [series.months, ageUiMode, userBirthDate, anchorDateYmd, calendarTz, layoutDims.pw]);

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
    const minMonthSpan = Math.min(totalMonths, 12);
    const monthSpan = Math.max(
      minMonthSpan,
      Math.min(totalMonths, Math.round(viewWindow.monthSpan)),
    );
    const maxStartMonth = Math.max(0, totalMonths - monthSpan);
    const visibleMonthStart = Math.max(
      0,
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
    const ccVisible = visibleIndices.map((i) => cc[i] ?? 0);
    const fireTargetVisible = fireTarget
      ? visibleIndices.map((i) => fireTarget[i] ?? 0)
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

    const xTicks = xTicksAll.filter(
      (tick) =>
        tick.monthIndex >= visibleMonthStart && tick.monthIndex <= visibleMonthEnd,
    );
    if (xTicks.length === 0 && visibleMonthEnd > visibleMonthStart) {
      xTicks.push({
        monthIndex: visibleMonthEnd,
        label: projectionXTickLabel(visibleMonthEnd, series.months, {
          ageUiMode,
          birthDateIso: userBirthDate,
          anchorDateYmd,
          calendarTz,
        }),
      });
    }

    const tickSpanPx =
      xTicks.length > 1 ? layoutDims.pw / (xTicks.length - 1) : layoutDims.pw;
    const rotateXLabels =
      xTicks.length > 11 || (xTicks.length > 5 && tickSpanPx < 46);
    const xAxisExtraBottom = rotateXLabels ? 38 : 0;
    const viewHeight = layoutDims.H + xAxisExtraBottom;

    const { W, H, ml, mr, mt, mb, pw, ph } = layoutDims;

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
        if (mi < visibleMonthStart || mi > visibleMonthEnd) continue;
        out.push({ id: f.id, mi, title: f.title, direction: f.direction });
      }
      return out;
    })();
    chartPerf.mark("model-end");
    return {
      nw,
      cc,
      fireTargetVisible,
      assetSeries,
      assetStacks,
      nwVisible,
      ccVisible,
      allowNegativeAxis,
      targetYMin: yMin,
      targetYMax: yMax,
      xTicks,
      xScale,
      compoundOutpaceMonth,
      showCompoundOutpaceMarker,
      visibleMilestones,
      chartPlanningMarkers,
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
    ccVisible,
    allowNegativeAxis,
    xTicks,
    xScale,
    compoundOutpaceMonth,
    showCompoundOutpaceMarker,
    visibleMilestones,
    chartPlanningMarkers,
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
  // Cada punto visible se posiciona en X según su `month_index` real (no por
  // índice del array). Esto soporta puntos no equidistantes (hybrid).
  const nwPoints = nwVisible
    .map((v, j) => `${xScale(pts[visibleIndices[j]!]!.month_index)},${yScale(v)}`)
    .join(" ");
  const ccPoints = ccVisible
    .map((v, j) => `${xScale(pts[visibleIndices[j]!]!.month_index)},${yScale(v)}`)
    .join(" ");
  const firePoints = fireTargetVisible
    ? fireTargetVisible
        .map((v, j) => `${xScale(pts[visibleIndices[j]!]!.month_index)},${yScale(v)}`)
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

  function onPointerMove(e: PointerEvent<SVGSVGElement>) {
    const i = pointerToIndex(e.clientX);
    setHover(i);
    const wrap = wrapRef.current;
    if (wrap) {
      const r = wrap.getBoundingClientRect();
      setTipOffset({ x: e.clientX - r.left, y: e.clientY - r.top });
    }
  }

  function onPointerLeave() {
    setHover(null);
  }

  function onWheel(e: WheelEvent<SVGSVGElement>) {
    e.preventDefault();
    if (pts.length < 2) return;
    const panInput = e.shiftKey ? e.deltaY : e.deltaX;
    const isPan = Math.abs(panInput) > Math.abs(e.deltaY) || e.shiftKey;
    const totalMonths = series.months;
    const minSpan = Math.min(totalMonths, 12);
    if (isPan && monthSpan < totalMonths) {
      const step = Math.max(1, Math.round(monthSpan * 0.08));
      const direction = panInput > 0 ? 1 : -1;
      const maxStart = Math.max(0, totalMonths - monthSpan);
      const nextStart = Math.max(
        0,
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
    const nextSpan = Math.max(minSpan, Math.min(totalMonths, nextSpanRaw));
    if (nextSpan === monthSpan) return;
    const anchorMonth = pointerToMonth(e.clientX);
    const ratio = (anchorMonth - visibleMonthStart) / Math.max(1, monthSpan - 1);
    const nextStartRaw = Math.round(anchorMonth - ratio * (nextSpan - 1));
    const maxStart = Math.max(0, totalMonths - nextSpan);
    const nextStart = Math.max(0, Math.min(maxStart, nextStartRaw));
    setViewWindow({ startMonth: nextStart, monthSpan: nextSpan });
  }

  const horizonLine = formatProjectionChartHorizonLine(series);
  const deltaStr = formatCurrencyAmount(series.monthly_delta_assumption, currencyIso);
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
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
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
        <text x={ml} y={layoutDims.headlineBlockTopY + 40} className="projection-chart-meta">
          {inflationShort} · Δ regular presup. {deltaStr}/mes
        </text>

        <g
          transform={`translate(${layoutDims.legend.x}, ${layoutDims.legend.y})`}
          className="projection-chart-legend"
        >
          {(() => {
            const p = layoutDims.legendPlacements;
            const items: ReactNode[] = [];
            if (p[0]) {
              items.push(
                <g key="legend-nw" transform={`translate(${p[0].x}, ${p[0].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="var(--proj-nw)" strokeWidth={3} strokeLinecap="round" />
                  <text x={28} y={15}>Patrimonio neto</text>
                </g>,
              );
            }
            if (p[1]) {
              items.push(
                <g key="legend-cc" transform={`translate(${p[1].x}, ${p[1].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="var(--proj-cc)" strokeWidth={2.25} strokeDasharray="6 5" strokeLinecap="round" />
                  <text x={28} y={15}>Capital aportado</text>
                </g>,
              );
            }
            const fireOffset = hasFireTargetSeries ? 1 : 0;
            if (hasFireTargetSeries && p[2]) {
              items.push(
                <g key="legend-fire" transform={`translate(${p[2].x}, ${p[2].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="var(--proj-fire)" strokeWidth={1.5} strokeDasharray="3 4" strokeLinecap="round" opacity={0.5} />
                  <text x={28} y={15}>Target FIRE</text>
                </g>,
              );
            }
            assetSeries.forEach((as, idx) => {
              const pos = p[2 + fireOffset + idx];
              if (!pos) return;
              const color = ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length];
              items.push(
                <g key={`legend-${as.id}`} transform={`translate(${pos.x}, ${pos.y})`}>
                  <rect
                    x={0}
                    y={6}
                    width={20}
                    height={11}
                    rx={2}
                    fill={color}
                    fillOpacity={0.14}
                    stroke={color}
                    strokeOpacity={0.4}
                    strokeWidth={0.8}
                  />
                  <text x={26} y={15}>{as.name}</text>
                </g>,
              );
            });
            return items;
          })()}
        </g>

        {yTicks.map((yt) => (
          <g key={`gy-${yt}`}>
            <line
              x1={ml}
              y1={yScale(yt)}
              x2={ml + pw}
              y2={yScale(yt)}
              className="projection-chart-grid"
            />
            <text
              x={ml - 10}
              y={yScale(yt)}
              textAnchor="end"
              dominantBaseline="middle"
              className="projection-chart-tick"
            >
              {formatAxisMoney(yt, currencyIso)}
            </text>
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
          <polyline
            points={nwPoints}
            fill="none"
            stroke="var(--proj-nw)"
            strokeWidth={2.85}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
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
            const y1Floor = mt + 12;
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
            const nwAtMi = nw[m.mi] ?? null;
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
              <circle cx={xScale(pts[hover]!.month_index)} cy={yScale(cc[hover] ?? 0)} r={5} className="projection-chart-dot-cc" />
            </>
          ) : null}
        </g>

        <text
          transform={`translate(${Math.min(30, ml * 0.32)}, ${mt + ph / 2}) rotate(-90)`}
          textAnchor="middle"
          className="projection-chart-axis-caption"
        >
          {normalizeCurrencyIso(currencyIso) ?? "Importe"}
        </text>
      </svg>

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
            {projectionHoverTitle(
              hover,
              ageUiMode,
              userBirthDate,
              calendarTz,
              anchorDateYmd,
            )}
          </div>
          <div>
            Patrimonio neto —{" "}
            {formatCurrencyOrDashNumber(nw[hover], currencyIso)}
          </div>
          <div>
            Capital aportado —{" "}
            {formatCurrencyOrDashNumber(cc[hover], currencyIso)}
          </div>
          {assetSeries.map((as) => (
            <div key={as.id}>
              {as.name} —{" "}
              {formatCurrencyOrDashNumber(as.values[hover], currencyIso)}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
