/**
 * Mini-proyección compacta — comparte lenguaje visual con
 * ProjectionNetWorthChart (línea NW + opcional target FIRE + marcador
 * jubilación + áreas apiladas por activo) pero sin ejes, leyenda ni
 * interacción. Se reutiliza en Resumen y Jubilación.
 *
 * Recibe la serie del endpoint /v1/projection/series ya cargada por App.tsx
 * (no hace fetch propio). Si `series` es null o no tiene puntos, renderiza
 * un placeholder vacío.
 *
 * **5.0.0 · rediseño UX U1b (decisión U5 de #207) — este componente absorbe el abanico.**
 * Jubilación pasa de DOS gráficos (el determinista arriba y el fan de percentiles en la sección
 * «Riesgo») a UNO: la misma curva, con el objetivo FIRE, la banda p10–p90 opcional encima y las
 * marcas de los hitos del plan. Tres props nuevas y opcionales lo hacen posible —`band`,
 * `markers` y `deflator`—, y las tres son no-ops cuando no se pasan: el Resumen sigue pintando
 * byte a byte lo de 4.15.x.
 *
 * La objeción que documentaba `RiskFanChart` («dos fuentes con rejillas distintas no caben en un
 * componente que dibuja UNA serie») sigue siendo cierta y por eso la banda **no** se empareja por
 * posición: entra como una lista de `{month, p10, p90}` y se posiciona con `xAtMonth`, la misma
 * escala temporal que ya usaban la tira de fases y el eje X. Lo que ha cambiado no es la
 * dificultad, es la pregunta: con dos charts el usuario tenía que emparejar a ojo dos ejes X que
 * ni siquiera empezaban en el mismo sitio.
 *
 * **La deflactación ocurre AQUÍ y en un solo sitio** (`deflator`): patrimonio, objetivo y banda
 * pasan por el mismo factor mes a mes. Si la banda llegara ya deflactada y la curva no, el
 * abanico dejaría de contener a la línea que dice contener — y el chart seguiría pareciendo
 * correcto.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ProjectionSeriesApi } from "../../api/types";
import {
  ASSET_LINE_COLORS,
  lastPointIndexAtOrBeforeMonth,
  projectionXTickLabel,
} from "../../lib/projection-chart";
import { buildPhaseSegments } from "../../lib/phase-strip";
import {
  placeMarkerLabels,
  type RetirementChartMarker,
} from "../../lib/retirement-chart";

export type MiniProjectionXAxisOpts = {
  ageUiMode: "dates" | "ages";
  birthDateIso?: string | null;
  anchorDateYmd?: string | null;
  calendarTz: string;
};

/** Un punto de la banda de percentiles, **en euros NOMINALES** y con su MES de la rejilla: la
 *  deflactación la aplica el chart, para que sea la misma que la de la curva. */
export type MiniProjectionBandPoint = {
  month: number;
  p10: number;
  p90: number;
};

export function MiniProjection({
  series,
  months,
  height = 120,
  showFire = false,
  showJub = true,
  showPhases = false,
  showAreas = true,
  zoomY = false,
  clampToMonth,
  xAxis,
  band,
  markers,
  deflator,
}: {
  series: ProjectionSeriesApi | null;
  /** Número de meses a mostrar; recorta si la serie es más larga. */
  months?: number;
  height?: number;
  showFire?: boolean;
  showJub?: boolean;
  /**
   * Versión REDUCIDA de la tira de fases del chart grande (D29): una banda de 6px bajo el plot,
   * sin rótulos ni marcas — a este ancho no cabe texto, y el detalle vive en Proyección. Sigue
   * siendo la misma geometría (`buildPhaseSegments`, todo por `month_index`), así que el corte
   * cae en el mismo mes en los dos charts. Opt-in: sin ella la geometría es idéntica a 4.15.x.
   */
  showPhases?: boolean;
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
  /**
   * Banda p10–p90 de los escenarios con volatilidad (U5). En euros NOMINALES y **por MES**: la
   * rejilla de la banda (siempre `hybrid`) no tiene por qué coincidir con la de `points[]`, así
   * que emparejarlas por posición desplazaría el abanico décadas sin que nada fallara.
   * Ausente ⇒ no se dibuja y no entra en el dominio del eje Y.
   */
  band?: readonly MiniProjectionBandPoint[] | null;
  /** Hitos del plan (jubilación, coast, media jornada, pensión). Sus rótulos se ceden por
   *  prioridad cuando no caben — `lib/retirement-chart.ts`, con test. */
  markers?: readonly RetirementChartMarker[] | null;
  /**
   * Factor por el que se multiplica cada importe NOMINAL del mes: `deflationFactorAt(mi, pct)`
   * para leer «en dinero de hoy», ausente (o `() => 1`) para leer en euros corrientes. Se aplica
   * a patrimonio, objetivo FIRE y banda **por igual**; las áreas de activo se escalan al
   * patrimonio y por tanto lo heredan.
   */
  deflator?: ((monthIndex: number) => number) | null;
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

    // Un deflactor ausente es la identidad: el Resumen no pasa ninguno y su chart no cambia.
    const df = deflator ?? (() => 1);
    const nw = points.map((p) => p.net_worth * df(p.month_index));
    const fire =
      showFire && series.fire_target_series && series.fire_target_series.length > 0
        ? series.fire_target_series
            .slice(0, total)
            .map((v, i) => v * df(monthAt(i)))
        : null;

    const assetSeries =
      showAreas && series.asset_series && series.asset_series.length > 0
        ? series.asset_series.map((a) => a.values.slice(0, total))
        : null;

    // La banda se recorta a la VENTANA por mes (nunca por longitud: las dos rejillas difieren) y
    // se deflacta con el mismo factor que la curva. Menos de dos puntos dibujables no es media
    // banda: es ninguna, y una banda degenerada se leería como certeza.
    const bandPoints = band
      ? band
          .filter(
            (b) =>
              Number.isFinite(b.month) &&
              b.month >= monthStart &&
              b.month <= monthEnd &&
              Number.isFinite(b.p10) &&
              Number.isFinite(b.p90),
          )
          .slice()
          .sort((a, b) => a.month - b.month)
          .map((b) => ({
            month: b.month,
            p10: b.p10 * df(b.month),
            p90: b.p90 * df(b.month),
          }))
      : [];
    const bandVisible = bandPoints.length >= 2;

    const allValues = [
      ...nw,
      ...(fire ?? []),
      ...(bandVisible
        ? bandPoints.flatMap((b) => [b.p10, b.p90])
        : []),
    ];
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
    const phaseSegments = showPhases
      ? buildPhaseSegments(series.phase_transitions, {
          startMonth: monthStart,
          endMonth: monthEnd,
        })
      : [];
    // La banda sale del alto del plot, como el eje: el SVG mide `height` exacto.
    const phaseH = phaseSegments.length > 0 ? 8 : 0;
    const pw = W - padX * 2;
    const ph = H - padY * 2 - axisH - phaseH;

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

    // Área entre percentiles: p90 de ida y p10 de vuelta, en UN solo `path` cerrado — dos
    // polígonos dejarían una costura de 1 px visible en oscuro.
    const bandPath = bandVisible
      ? `M ${bandPoints
          .map((b) => `${xAtMonth(b.month).toFixed(1)},${yAt(b.p90).toFixed(1)}`)
          .join(" L ")} L ${bandPoints
          .slice()
          .reverse()
          .map((b) => `${xAtMonth(b.month).toFixed(1)},${yAt(b.p10).toFixed(1)}`)
          .join(" L ")} Z`
      : null;

    const placedMarkers =
      markers && markers.length > 0
        ? placeMarkerLabels({
            markers: markers.filter(
              (m) => m.month >= monthStart && m.month <= monthEnd,
            ),
            xAtMonth,
            width: W,
          })
        : [];

    return {
      total,
      monthStart,
      monthSpan,
      visibleMonths,
      nw,
      fire,
      bandPath,
      placedMarkers,
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
      phaseSegments,
      phaseH,
    };
  }, [
    series,
    months,
    height,
    showFire,
    showJub,
    showPhases,
    showAreas,
    zoomY,
    clampToMonth,
    xAxis,
    band,
    markers,
    deflator,
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
    bandPath,
    placedMarkers,
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
    phaseSegments,
    phaseH,
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

      {/* Banda 10–90 % de los escenarios (U5). Relleno tenue del acento —es la lectura del
          plan, la misma familia que el objetivo FIRE— con la opacidad en el atributo y no
          dentro del color, para que el mismo token resuelva en claro y en oscuro. Va DEBAJO de
          todo: es el contexto sobre el que se leen la curva y el objetivo, no una serie más. */}
      {bandPath ? (
        <path
          d={bandPath}
          fill="var(--ff-accent)"
          fillOpacity={0.16}
          stroke="var(--ff-accent)"
          strokeOpacity={0.3}
          strokeWidth={0.8}
        />
      ) : null}

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

      {/* Hitos del plan (U5): jubilación, coast, media jornada y pensión sobre el MISMO eje.
          La línea se pinta siempre; el rótulo solo cuando cabe (`placeMarkerLabels`), y el que
          nunca se cede es el de la jubilación. Las secundarias van discontinuas y en el gris de
          meta del chart grande: son contexto, no el hito que la página contesta. */}
      {placedMarkers.map((m) => {
        const primary = m.emphasis === "primary";
        return (
          <g key={`mini-marker-${m.key}`}>
            <line
              x1={m.x}
              x2={m.x}
              y1={padY}
              y2={padY + ph}
              stroke={primary ? "var(--ff-accent)" : "var(--proj-meta)"}
              strokeWidth={primary ? 1.5 : 1}
              strokeDasharray={primary ? undefined : "3 3"}
            />
            {m.showLabel ? (
              <text
                x={m.x}
                y={padY + 11}
                textAnchor={m.anchor}
                className="proj-mini-marker-label"
                fill={primary ? "var(--ff-accent)" : "var(--proj-meta)"}
                fontSize="9.5"
              >
                {m.label}
              </text>
            ) : null}
          </g>
        );
      })}

      {/* Tira de fases reducida (D29): banda sin rótulos bajo el plot. Las posiciones salen de
          `xAtMonth` (meses), nunca de `xAt` (posiciones del array): con `density=hybrid` los
          puntos no son equidistantes y el corte caería en el año equivocado. */}
      {phaseH > 0
        ? phaseSegments.map((seg) => {
            const left = xAtMonth(seg.startMonth);
            const right = xAtMonth(
              seg.endMonth >= monthStart + monthSpan
                ? seg.endMonth
                : seg.endMonth + 1,
            );
            const w = Math.max(0, right - left);
            if (w <= 0) return null;
            return (
              <rect
                key={`mini-phase-${seg.phase}-${seg.transitionMonth}`}
                x={left}
                y={padY + ph + 2}
                width={w}
                height={phaseH - 2}
                rx={1.5}
                className={`projection-phase-band projection-phase-band--${seg.phase}`}
              />
            );
          })
        : null}

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
          const yBase = padY + ph + phaseH + 12;
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

