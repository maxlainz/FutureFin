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
  formatCurrencyOrDash,
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
  const pts = series.points;
  const gid = useId().replace(/:/g, "");
  const svgRef = useRef<SVGSVGElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const yAxisAnimRef = useRef<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [tipOffset, setTipOffset] = useState({ x: 0, y: 0 });
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
  const [viewWindow, setViewWindow] = useState({ start: 0, count: pts.length });
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
    const clampedEnd = Math.max(0, Math.min(pts.length - 1, focusEnd));
    return { start: 0, count: clampedEnd + 1 };
  }, [milestones, pts.length]);

  useEffect(() => {
    setViewWindow((prev) => {
      if (pts.length <= 0) return { start: 0, count: 0 };
      const next = focusMode && focusWindow
        ? focusWindow
        : { start: 0, count: pts.length };
      if (prev.start === next.start && prev.count === next.count) {
        return prev;
      }
      return next;
    });
  }, [focusMode, focusWindow, pts.length]);

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

  const model = useMemo(() => {
    if (pts.length < 2) return null;
    const deflate = inflationAdjusted && installationInflationPct > 0;
    const deflator = (monthIndex: number) =>
      deflate
        ? 1 / Math.pow(1 + installationInflationPct / 100, monthIndex / 12)
        : 1;
    const nw = pts.map(
      (p, i) => (parseDisplayDecimal(p.net_worth) ?? 0) * deflator(i),
    );
    const cc = pts.map(
      (p, i) => (parseDisplayDecimal(p.contributed_capital) ?? 0) * deflator(i),
    );
    const fireRaw = series.fire_target_series;
    const fireTarget =
      Array.isArray(fireRaw) && fireRaw.length === pts.length
        ? fireRaw.map((v, i) => (parseDisplayDecimal(v) ?? 0) * deflator(i))
        : null;
    const assetSeries = (series.asset_series ?? [])
      .map((as) => {
        const values = as.values.map(
          (v, i) => (parseDisplayDecimal(v) ?? 0) * deflator(i),
        );
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
    const minVisiblePoints = Math.min(pts.length, 12);
    const visibleCount = Math.max(
      minVisiblePoints,
      Math.min(pts.length, Math.round(viewWindow.count)),
    );
    const maxStart = Math.max(0, pts.length - visibleCount);
    const visibleStart = Math.max(
      0,
      Math.min(maxStart, Math.round(viewWindow.start)),
    );
    const visibleEnd = visibleStart + visibleCount - 1;
    const nwVisible = nw.slice(visibleStart, visibleEnd + 1);
    const ccVisible = cc.slice(visibleStart, visibleEnd + 1);
    const fireTargetVisible = fireTarget
      ? fireTarget.slice(visibleStart, visibleEnd + 1)
      : null;
    const assetVisibleValues = assetSeries.flatMap((as) =>
      as.values.slice(visibleStart, visibleEnd + 1),
    );
    const startNwParsed = parseDisplayDecimal(series.starting_net_worth);
    const startNw =
      startNwParsed !== null ? startNwParsed : nw[0] ?? 0;
    const allowNegativeAxis = startNw < 0;

    // El target FIRE inflado puede crecer muy por encima del patrimonio en horizontes largos;
    // dejarlo fuera del rango del eje Y para no aplastar la curva del patrimonio. La línea se
    // recorta visualmente por el clipPath del plot si excede.
    const dataMin = Math.min(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const dataMax = Math.max(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const rawSpan = dataMax - dataMin;
    const padY =
      rawSpan > 0
        ? rawSpan * 0.07
        : Math.max(Math.abs(dataMax) * 0.06, 1);

    let plotMin = dataMin - padY;
    let plotMax = dataMax + padY;
    if (!allowNegativeAxis) {
      plotMin = Math.max(0, plotMin);
    }

    let yTicksRaw = niceYTicks(plotMin, plotMax, 6);
    let yTicks = allowNegativeAxis
      ? yTicksRaw
      : yTicksRaw.filter((t) => t >= 0);
    if (!allowNegativeAxis && yTicks.length < 2) {
      yTicks = niceYTicks(Math.max(0, plotMin), plotMax, 6);
    }
    let yMin = yTicks[0] ?? (allowNegativeAxis ? plotMin : Math.max(0, plotMin));
    let yMax = yTicks[yTicks.length - 1] ?? plotMax;
    if (!allowNegativeAxis && yMin < 0) {
      yMin = 0;
    }
    const xTicksAll = projectionXTicks(
      series.months,
      {
        ageUiMode,
        birthDateIso: userBirthDate,
        anchorDateYmd,
        calendarTz,
      },
      { plotWidthPx: layoutDims.pw },
    );
    const xTicks = xTicksAll.filter(
      (tick) => tick.monthIndex >= visibleStart && tick.monthIndex <= visibleEnd,
    );
    if (xTicks.length === 0 && visibleEnd > visibleStart) {
      xTicks.push({
        monthIndex: visibleEnd,
        label: projectionXTickLabel(visibleEnd, series.months, {
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

    const xScale = (i: number) => {
      const local = i - visibleStart;
      return ml + (local / Math.max(1, visibleCount - 1)) * pw;
    };
    const compoundOutpaceMonth =
      series.compound_outpaces_true_savings_month_index ?? null;
    const visibleMilestones = milestones.filter(
      (m) =>
        m.reached_month_index >= visibleStart && m.reached_month_index <= visibleEnd,
    );
    const showCompoundOutpaceMarker =
      compoundOutpaceMonth != null &&
      compoundOutpaceMonth >= visibleStart &&
      compoundOutpaceMonth <= visibleEnd;
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
        if (mi < visibleStart || mi > visibleEnd) continue;
        out.push({ id: f.id, mi, title: f.title, direction: f.direction });
      }
      return out;
    })();
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
      visibleStart,
      visibleEnd,
      visibleCount,
    };
  }, [
    pts,
    series.months,
    series.starting_net_worth,
    series.asset_series,
    series.fire_target_series,
    layoutDims,
    ageUiMode,
    userBirthDate,
    anchorDateYmd,
    calendarTz,
    milestones,
    focusMode,
    inflationAdjusted,
    installationInflationPct,
    viewWindow.count,
    viewWindow.start,
    planningFlows,
  ]);

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
    targetYMin,
    targetYMax,
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
    visibleStart,
    visibleEnd,
    visibleCount,
  } = model;

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

  const yMin = animatedYDomain?.min ?? targetYMin;
  const yMax = animatedYDomain?.max ?? targetYMax;
  const spanY = Math.max(1, yMax - yMin);
  const yScale = (v: number) => mt + ph - ((v - yMin) / spanY) * ph;
  const yTicksRaw = niceYTicks(yMin, yMax, 6);
  let yTicks = allowNegativeAxis ? yTicksRaw : yTicksRaw.filter((t) => t >= 0);
  if (!allowNegativeAxis && yTicks.length < 2) {
    yTicks = niceYTicks(Math.max(0, yMin), yMax, 6);
  }
  const nwPoints = nwVisible
    .map((v, i) =>
      `${xScale(visibleStart + i)},${yScale(v)}`,
    )
    .join(" ");
  const ccPoints = ccVisible
    .map((v, i) =>
      `${xScale(visibleStart + i)},${yScale(v)}`,
    )
    .join(" ");
  const firePoints = fireTargetVisible
    ? fireTargetVisible
        .map((v, i) => `${xScale(visibleStart + i)},${yScale(v)}`)
        .join(" ")
    : null;
  let areaD = "";
  if (nwVisible.length > 0) {
    const parts: string[] = [];
    nwVisible.forEach((v, i) => {
      const globalIndex = visibleStart + i;
      parts.push(
        `${i === 0 ? "M" : "L"} ${xScale(globalIndex)} ${yScale(v)}`,
      );
    });
    const xLast = xScale(visibleEnd);
    const x0 = xScale(visibleStart);
    const yBase = mt + ph;
    parts.push(`L ${xLast} ${yBase}`);
    parts.push(`L ${x0} ${yBase} Z`);
    areaD = parts.join(" ");
  }

  function pointerToIndex(clientX: number): number {
    const svg = svgRef.current;
    if (!svg) return 0;
    const rect = svg.getBoundingClientRect();
    const vb = svg.viewBox.baseVal;
    const xSvg = ((clientX - rect.left) / Math.max(rect.width, 1)) * vb.width;
    const local = xSvg - ml;
    const idx = Math.round((local / pw) * Math.max(1, visibleCount - 1)) + visibleStart;
    return Math.max(0, Math.min(pts.length - 1, idx));
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
    const minVisiblePoints = Math.min(pts.length, 12);
    if (isPan && visibleCount < pts.length) {
      const step = Math.max(1, Math.round(visibleCount * 0.08));
      const direction = panInput > 0 ? 1 : -1;
      const maxStart = Math.max(0, pts.length - visibleCount);
      const nextStart = Math.max(
        0,
        Math.min(maxStart, visibleStart + direction * step),
      );
      if (nextStart !== visibleStart) {
        setViewWindow({ start: nextStart, count: visibleCount });
      }
      return;
    }
    if (e.deltaY === 0) return;
    const zoomFactor = e.deltaY < 0 ? 0.88 : 1.14;
    const nextCountRaw = Math.round(visibleCount * zoomFactor);
    const nextCount = Math.max(
      minVisiblePoints,
      Math.min(pts.length, nextCountRaw),
    );
    if (nextCount === visibleCount) return;
    const anchorIndex = pointerToIndex(e.clientX);
    const ratioWithinWindow =
      (anchorIndex - visibleStart) / Math.max(1, visibleCount - 1);
    const nextStartRaw = Math.round(
      anchorIndex - ratioWithinWindow * (nextCount - 1),
    );
    const maxStart = Math.max(0, pts.length - nextCount);
    const nextStart = Math.max(0, Math.min(maxStart, nextStartRaw));
    setViewWindow({ start: nextStart, count: nextCount });
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
            <stop offset="0%" stopColor="#10b981" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#10b981" stopOpacity="0.03" />
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
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#047857" strokeWidth={3} strokeLinecap="round" />
                  <text x={28} y={15}>Patrimonio neto</text>
                </g>,
              );
            }
            if (p[1]) {
              items.push(
                <g key="legend-cc" transform={`translate(${p[1].x}, ${p[1].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#b45309" strokeWidth={2.25} strokeDasharray="6 5" strokeLinecap="round" />
                  <text x={28} y={15}>Capital aportado</text>
                </g>,
              );
            }
            const fireOffset = hasFireTargetSeries ? 1 : 0;
            if (hasFireTargetSeries && p[2]) {
              items.push(
                <g key="legend-fire" transform={`translate(${p[2].x}, ${p[2].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#7c3aed" strokeWidth={1.5} strokeDasharray="3 4" strokeLinecap="round" opacity={0.5} />
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
          fill="#ffffff"
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
              for (let k = visibleStart; k <= visibleEnd; k++) {
                topParts.push(`${xScale(k)},${yScale(stack.tops[k])}`);
              }
              for (let k = visibleEnd; k >= visibleStart; k--) {
                botParts.push(`${xScale(k)},${yScale(stack.bottoms[k])}`);
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
            stroke="#047857"
            strokeWidth={2.85}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <polyline
            points={ccPoints}
            fill="none"
            stroke="#b45309"
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
              stroke="#7c3aed"
              strokeWidth={1.2}
              strokeDasharray="3 4"
              strokeLinecap="round"
              strokeLinejoin="round"
              opacity={0.225}
            />
          ) : null}
          {visibleMilestones.map((m) => {
            const x = xScale(m.reached_month_index);
            const y0 = mt + ph;
            const nwAtMilestone = nw[m.reached_month_index] ?? null;
            const nwY =
              nwAtMilestone != null && Number.isFinite(nwAtMilestone)
                ? yScale(nwAtMilestone)
                : null;
            // Mantiene la marca siempre por encima de la curva de patrimonio neto.
            const y1Floor = mt + 12;
            const y1FromNetWorth = nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
            const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
            const isJubilacion = m.target === "jubilacion";
            const targetNum = parseDisplayDecimal(m.target);
            const label =
              targetNum != null
                ? formatCurrencyNumber(targetNum, currencyIso)
                : formatProjectionMilestoneCompactLabel(m.target);
            return (
              <g key={`ms-${m.target}-${m.reached_month_index}`}>
                <line
                  x1={x}
                  x2={x}
                  y1={y0}
                  y2={y1}
                  className={isJubilacion ? "projection-chart-jubilacion-line" : "projection-chart-milestone-line"}
                />
                <text
                  x={x}
                  y={y1 - 6}
                  textAnchor="middle"
                  className={isJubilacion ? "projection-chart-jubilacion-label" : "projection-chart-milestone-label"}
                >
                  {label}
                </text>
              </g>
            );
          })}
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
                const nwAtMs = nw[compoundOutpaceMonth] ?? null;
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

          {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
            <line
              x1={xScale(hover)}
              x2={xScale(hover)}
              y1={mt}
              y2={mt + ph}
              className="projection-chart-crosshair"
            />
          ) : null}
          {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
            <>
              <circle cx={xScale(hover)} cy={yScale(nw[hover])} r={6} className="projection-chart-dot-nw" />
              <circle cx={xScale(hover)} cy={yScale(cc[hover])} r={5} className="projection-chart-dot-cc" />
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

      {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
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
            {formatCurrencyOrDash(pts[hover]?.net_worth, currencyIso)}
          </div>
          <div>
            Capital aportado —{" "}
            {formatCurrencyOrDash(
              pts[hover]?.contributed_capital,
              currencyIso,
            )}
          </div>
          {assetSeries.map((as) => (
            <div key={as.id}>
              {as.name} —{" "}
              {formatCurrencyOrDash(
                series.asset_series?.find((s) => s.asset_id === as.id)?.values[hover] ?? undefined,
                currencyIso,
              )}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
