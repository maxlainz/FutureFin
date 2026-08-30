/**
 * Mini-proyección compacta — comparte lenguaje visual con
 * ProjectionNetWorthChart (línea NW + opcional target FIRE + marcador
 * jubilación + áreas apiladas por activo) pero sin ejes, leyenda ni
 * interacción. Se reutiliza en Resumen y Jubilación.
 *
 * Recibe la serie del endpoint /v1/projection/series ya cargada por App.tsx
 * (no hace fetch propio). Si `series` es null o no tiene puntos, renderiza
 * un placeholder vacío.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ProjectionSeriesApi } from "../../api/types";
import {
  ASSET_LINE_COLORS,
  lastPointIndexAtOrBeforeMonth,
  projectionXTickLabel,
} from "../../lib/projection-chart";

export type MiniProjectionXAxisOpts = {
  ageUiMode: "dates" | "ages";
  birthDateIso?: string | null;
  anchorDateYmd?: string | null;
  calendarTz: string;
};

export function MiniProjection({
  series,
  months,
  height = 120,
  showFire = false,
  showJub = true,
  showAreas = true,
  zoomY = false,
  clampToMonth,
  xAxis,
}: {
  series: ProjectionSeriesApi | null;
  /** Número de meses a mostrar; recorta si la serie es más larga. */
  months?: number;
  height?: number;
  showFire?: boolean;
  showJub?: boolean;
  showAreas?: boolean;
  /**
   * Si es true, el eje Y se ajusta a `[min, max]` de las series visibles
   * (con un pequeño padding), no empieza en 0. Las áreas apiladas siguen
   * funcionando: están escaladas al NW y se anclan al suelo del rango.
   */
  zoomY?: boolean;
  /**
   * Último MES (`month_index`, no posición del array) incluido en la ventana
   * visible. Si está fuera de rango se ignora. Tiene prioridad sobre `months`
   * cuando ambas se pasan.
   */
  clampToMonth?: number | null;
  /**
   * Cuando se pasa, se renderiza un eje X compacto con ticks de
   * edad/fecha siguiendo la misma config que ProjectionNetWorthChart.
   */
  xAxis?: MiniProjectionXAxisOpts | null;
}) {
  // Medimos el ancho real del contenedor para que el viewBox del SVG use
  // unidades = px reales y los marcadores `<circle>` salgan redondos
  // (con preserveAspectRatio="none" sobre viewBox=320×h sale ovalado).
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [containerW, setContainerW] = useState(320);
  useLayoutEffect(() => {
    if (!wrapperRef.current) return;
    const el = wrapperRef.current;
    const update = () => {
      const w = el.getBoundingClientRect().width;
      if (w > 0) setContainerW(Math.round(w));
    };
    update();
  }, []);
  useEffect(() => {
    if (!wrapperRef.current) return;
    const el = wrapperRef.current;
    const ro = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr && cr.width > 0) setContainerW(Math.round(cr.width));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Todo el compute (parseo de series, escalas, stacks, jubPos y funciones
  // de proyección xAt/yAt) se memoiza en una sola pasada. Se recalcula solo
  // si cambian props o el ancho medido. Sin esto, recomputaba O(assets ×
  // months) en cada render del padre (Resumen / Jubilación).
  const computed = useMemo(() => {
    if (!series || series.points.length === 0) return null;

    // Con `density=hybrid` el servidor DIEZMA la serie (meses 0..12, 24, 36…), así que la
    // POSICIÓN en el array deja de ser el mes: el mes real vive en `point.month_index`. Tratar
    // una por otro recortaba la ventana por el sitio equivocado (`clampToMonth` es un mes, no un
    // índice: con 82 puntos, «jubilación + 12» no recortaba nada) y repartía el eje X a
    // distancias iguales, comprimiendo los 12 primeros meses y estirando las décadas.
    const monthAt = (i: number) => series.points[i]?.month_index ?? i;

    let total: number;
    if (clampToMonth != null && Number.isFinite(clampToMonth)) {
      total = lastPointIndexAtOrBeforeMonth(series.points, clampToMonth) + 1;
    } else if (months) {
      // `months` es un número de MESES visibles (12 = el primer año), no de puntos.
      total = lastPointIndexAtOrBeforeMonth(series.points, months - 1) + 1;
    } else {
      total = series.points.length;
    }
    total = Math.max(2, Math.min(total, series.points.length));
    const points = series.points.slice(0, total);
    const monthStart = points[0]?.month_index ?? 0;
    const monthEnd = points[points.length - 1]?.month_index ?? monthStart;
    const monthSpan = monthEnd - monthStart;
    /** Meses que abarca la ventana visible (para las etiquetas del eje). */
    const visibleMonths = monthSpan + 1;

    const nw = points.map((p) => p.net_worth);
    const fire =
      showFire && series.fire_target_series && series.fire_target_series.length > 0
        ? series.fire_target_series.slice(0, total)
        : null;

    const assetSeries =
      showAreas && series.asset_series && series.asset_series.length > 0
        ? series.asset_series.map((a) => a.values.slice(0, total))
        : null;

    const allValues = [...nw, ...(fire ?? [])];
    let vmin: number;
    let vmax: number;
    if (zoomY) {
      const lo = Math.min(...allValues);
      const hi = Math.max(...allValues);
      const pad = (hi - lo) * 0.08 || Math.abs(hi) * 0.02 || 1;
      vmin = lo - pad;
      vmax = hi + pad;
    } else {
      vmin = 0;
      vmax = Math.max(1, ...allValues) * 1.05;
    }

    const W = containerW;
    const H = height;
    const padX = 4;
    const padY = 4;
    const axisH = xAxis ? 18 : 0;
    const pw = W - padX * 2;
    const ph = H - padY * 2 - axisH;

    /** X de un MES concreto (no de una posición): el reparto es temporal, no posicional. */
    const xAtMonth = (m: number) =>
      padX + (monthSpan <= 0 ? pw / 2 : ((m - monthStart) / monthSpan) * pw);
    const xAt = (i: number) => xAtMonth(monthAt(i));
    const yAt = (v: number) => {
      const range = vmax - vmin || 1;
      const clamped = zoomY ? v : Math.max(0, v);
      return padY + ph - ((clamped - vmin) / range) * ph;
    };

    const pointsStr = (arr: number[]) =>
      arr.map((v, i) => `${xAt(i).toFixed(1)},${yAt(v).toFixed(1)}`).join(" ");

    const stackPath = (low: number[], high: number[]): string => {
      const top = high
        .map((v, i) => `${xAt(i).toFixed(1)},${yAt(v).toFixed(1)}`)
        .join(" L ");
      const bot = low
        .map(
          (_v, i) =>
            `${xAt(low.length - 1 - i).toFixed(1)},${yAt(
              low[low.length - 1 - i]!,
            ).toFixed(1)}`,
        )
        .join(" L ");
      return `M ${top} L ${bot} Z`;
    };

    let cumulative: number[][] = [];
    let stackBase: number[] = new Array(total).fill(0);
    if (assetSeries && assetSeries.length > 0) {
      cumulative = [];
      const sums: number[] = new Array(total).fill(0);
      for (let i = 0; i < total; i++) {
        let s = 0;
        for (const a of assetSeries) s += Math.max(0, a[i] ?? 0);
        sums[i] = s;
      }
      if (zoomY) {
        stackBase = nw.map((v) => Math.min(Math.max(0, v), vmin));
      }
      let prev = [...stackBase];
      for (const s of assetSeries) {
        const next = s.map((v, i) => {
          const total_i = sums[i] ?? 0;
          const nwI = Math.max(0, nw[i] ?? 0);
          const visibleNw = Math.max(0, nwI - (stackBase[i] ?? 0));
          const scaled =
            total_i > 0 ? visibleNw * (Math.max(0, v) / total_i) : 0;
          return (prev[i] ?? 0) + scaled;
        });
        cumulative.push(next);
        prev = next;
      }
    }

    // POSICIÓN del cruce — publicada por el servidor (`jubilacion_series_position`), ya NO se
    // escanea la rejilla local. Escanear `nw[i] >= fire[i]` sobre una serie diezmada
    // (`density=hybrid`) encontraba el primer punto SERVIDO que ya superaba el target, que no es
    // el mes exacto que calculó el motor sobre la simulación mensual completa. El guard
    // `pos < total` hace que el marcador desaparezca cuando `clampToMonth` deja el cruce fuera de
    // la ventana visible (misma convención que usa el pie del panel en RetirementView).
    let jubPos: number | null = null;
    if (showJub) {
      const pos = series.jubilacion_series_position;
      if (typeof pos === "number" && pos < total) {
        jubPos = pos;
      }
    }

    return {
      total,
      monthStart,
      monthSpan,
      visibleMonths,
      nw,
      fire,
      W,
      H,
      padX,
      padY,
      pw,
      ph,
      xAt,
      xAtMonth,
      yAt,
      pointsStr,
      stackPath,
      cumulative,
      stackBase,
      jubPos,
    };
  }, [
    series,
    months,
    height,
    showFire,
    showJub,
    showAreas,
    zoomY,
    clampToMonth,
    xAxis,
    containerW,
  ]);

  if (!computed) {
    return (
      <div ref={wrapperRef} style={{ width: "100%", height }}>
        <svg
          viewBox={`0 0 ${containerW} ${height}`}
          className="proj-mini"
          preserveAspectRatio="none"
          style={{ width: "100%", height, display: "block" }}
          aria-hidden
        />
      </div>
    );
  }

  const {
    total,
    monthStart,
    monthSpan,
    visibleMonths,
    nw,
    fire,
    W,
    H,
    padX,
    padY,
    pw,
    ph,
    xAt,
    xAtMonth,
    yAt,
    pointsStr,
    stackPath,
    cumulative,
    stackBase,
    jubPos,
  } = computed;

  return (
   <div ref={wrapperRef} style={{ width: "100%", height }}>
    <svg
      viewBox={`0 0 ${W} ${H}`}
      className="proj-mini"
      preserveAspectRatio="none"
      style={{ width: "100%", height, display: "block" }}
      role="img"
      aria-label="Proyección"
    >
      <rect
        x={padX}
        y={padY}
        width={pw}
        height={ph}
        rx={8}
        ry={8}
        fill="var(--proj-plot-bg)"
        stroke="var(--proj-grid)"
        strokeWidth={1}
      />

      {/* Áreas apiladas por activo — paleta policroma compartida con
          ProjectionNetWorthChart para que ambos charts se sientan del
          mismo mundo visual. La base inicial es `stackBase` (0 cuando
          zoomY está apagado, vmin cuando está activo). */}
      {cumulative.length > 0
        ? cumulative.map((stack, idx) => {
            const lower = idx === 0 ? stackBase : cumulative[idx - 1]!;
            const color =
              ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length]!;
            return (
              <path
                key={idx}
                d={stackPath(lower, stack)}
                fill={color}
                fillOpacity={0.18}
                stroke={color}
                strokeOpacity={0.45}
                strokeWidth={0.8}
              />
            );
          })
        : null}

      {/* Target FIRE (acento, dash) */}
      {fire ? (
        <polyline
          points={pointsStr(fire)}
          fill="none"
          stroke="var(--proj-fire)"
          strokeWidth={1.5}
          strokeDasharray="5 3"
        />
      ) : null}

      {/* Patrimonio neto — línea principal */}
      <polyline
        points={pointsStr(nw)}
        fill="none"
        stroke="var(--proj-nw)"
        strokeWidth={2}
        strokeLinejoin="round"
        strokeLinecap="round"
      />

      {/* Marcador jubilación: línea vertical + punto en NW */}
      {jubPos != null ? (
        <g>
          <line
            x1={xAt(jubPos)}
            x2={xAt(jubPos)}
            y1={padY}
            y2={padY + ph}
            stroke="var(--ff-accent)"
            strokeWidth={1.5}
          />
          <circle
            cx={xAt(jubPos)}
            cy={yAt(nw[jubPos] ?? 0)}
            r={3.5}
            fill="var(--ff-accent)"
            stroke="var(--ff-frame)"
            strokeWidth={1.2}
          />
        </g>
      ) : null}

      {/* Eje X: ~5 ticks equidistantes en MESES (no en posiciones del array: con `hybrid` la
          quinta posición es el mes 4 y la última el mes 840, y las etiquetas mentían). */}
      {xAxis ? (
        (() => {
          const tickCount = Math.min(5, total);
          const ticks: number[] = [];
          for (let i = 0; i < tickCount; i++) {
            const frac = tickCount - 1 === 0 ? 0 : i / (tickCount - 1);
            ticks.push(Math.round(monthStart + frac * monthSpan));
          }
          const yBase = padY + ph + 12;
          return (
            <g>
              {ticks.map((m, i) => (
                <text
                  key={i}
                  x={xAtMonth(m)}
                  y={yBase}
                  textAnchor={
                    i === 0 ? "start" : i === ticks.length - 1 ? "end" : "middle"
                  }
                  className="proj-mini-tick"
                  fill="var(--proj-tick)"
                  fontSize="10"
                >
                  {projectionXTickLabel(m, visibleMonths, xAxis)}
                </text>
              ))}
            </g>
          );
        })()
      ) : null}
    </svg>
   </div>
  );
}

