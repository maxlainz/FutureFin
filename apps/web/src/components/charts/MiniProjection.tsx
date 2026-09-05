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
 *
 * **5.0.0 · tercera vuelta de UX (V2/V5) — cuatro props más, todas opt-in.** El owner leyó el
 * chart de riesgo y dijo que «no deja nada claro qué representa cada cosa» (F6). Las cuatro
 * atacan esa frase y ninguna cambia nada si no se pasa:
 *
 *  - **`yAxis`** — importes en el eje. Sin él no había forma de saber si la banda vale 200.000 €
 *    o dos millones. Reserva una canaleta a la izquierda (`padLeft`) y dibuja la rejilla; el
 *    resto de la geometría sale de ella, así que el chart sin eje es idéntico al de 4.15.x.
 *  - **`bandGradient`** — el relleno de la banda dice la probabilidad de haber agotado el capital
 *    a esa edad (`lib/risk-gradient.ts`). Es lo que jubila la tabla «agotar a los 65/70/…».
 *  - **`bandEdgeLabels`** — qué es cada borde («optimista (p90)» / «pesimista (p10)»).
 *  - **`hoverLabel`** — el porcentaje exacto por edad, que un degradado solo puede aproximar.
 *
 * **La invariante que las gobierna: sin ellas, la geometría es byte a byte la de antes.** El
 * Resumen usa este mismo componente y su chart no puede moverse un píxel por un cambio que solo
 * pedía Jubilación. Lo que la sostiene es que `padLeft` degrada a `padX` y que cada bloque nuevo
 * está dentro de un `prop ? … : null`: sin prop no se emite ni un nodo.
 */

import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ProjectionSeriesApi } from "../../api/types";
import {
  ASSET_LINE_COLORS,
  lastPointIndexAtOrBeforeMonth,
  niceYTicks,
  projectionXTickLabel,
} from "../../lib/projection-chart";
import { formatAxisMoney } from "../../lib/ledger";
import { buildPhaseSegments } from "../../lib/phase-strip";
import {
  placeMarkerLabels,
  type RetirementChartMarker,
} from "../../lib/retirement-chart";
import type { RiskGradientStop } from "../../lib/risk-gradient";

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
  yAxis,
  band,
  bandGradient,
  bandEdgeLabels,
  hoverLabel,
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
   * Eje Y con importes (5.0.0, V2). Cuando se pasa, el plot reserva una canaleta a la izquierda y
   * dibuja ~4 líneas de rejilla rotuladas con `formatAxisMoney` — el mismo formateador compacto
   * del chart grande, que es una de las dos excepciones sancionadas a los helpers canónicos.
   *
   * Los valores que se rotulan son los que el chart ya tiene, o sea **ya deflactados**: el toggle
   * «En dinero de hoy» mueve el eje entero, como debe.
   */
  yAxis?: { currencyIso: string } | null;
  /**
   * Banda p10–p90 de los escenarios con volatilidad (U5). En euros NOMINALES y **por MES**: la
   * rejilla de la banda (siempre `hybrid`) no tiene por qué coincidir con la de `points[]`, así
   * que emparejarlas por posición desplazaría el abanico décadas sin que nada fallara.
   * Ausente ⇒ no se dibuja y no entra en el dominio del eje Y.
   */
  band?: readonly MiniProjectionBandPoint[] | null;
  /** Hitos del plan (jubilación, coast, media jornada, pensión). Sus rótulos se ceden por
   *  prioridad cuando no caben — `lib/retirement-chart.ts`, con test. */
  /**
   * Paradas del degradado que tiñe la banda por probabilidad de agotar el capital (V2/V5,
   * `lib/risk-gradient.ts`). **Sus `offset` tienen que estar calculados sobre los MISMOS
   * `monthStart`/`monthEnd` que la ventana visible de este chart**: el `<linearGradient>` se
   * declara con `gradientUnits="userSpaceOnUse"` entre esos dos meses, así que un llamante que
   * usara otros extremos desplazaría el color sin que nada fallara. Hoy el único consumidor no
   * recorta ventana (`months`/`clampToMonth` ausentes), y por eso los extremos coinciden.
   *
   * Menos de dos paradas ⇒ la banda vuelve al acento plano de siempre.
   */
  bandGradient?: readonly RiskGradientStop[] | null;
  /** Rótulos de los dos bordes de la banda en su extremo derecho («optimista (p90)» arriba,
   *  «pesimista (p10)» abajo). Se omiten si los dos bordes quedan a menos de 14 px: dos textos
   *  pisándose dicen menos que ninguno. */
  bandEdgeLabels?: { p10: string; p90: string } | null;
  /** Rótulo del hover, construido por el llamante a partir del MES bajo el cursor. Devolver
   *  `null` para un mes sin nada que decir. Ausente ⇒ el chart no captura el puntero. */
  hoverLabel?: ((monthIndex: number) => string | null) | null;
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
    // Canaleta del eje Y. **Sin `yAxis` vale `padX`**, así que `padLeft`, `pw` y `xAtMonth` dan
    // exactamente los mismos números que antes de V2: la geometría del Resumen no se mueve un
    // píxel. Los dos anchos son los del chart grande en su modo estrecho.
    const padLeft = yAxis ? (W < 420 ? 34 : 46) : padX;
    const axisH = xAxis ? 18 : 0;
    const phaseSegments = showPhases
      ? buildPhaseSegments(series.phase_transitions, {
          startMonth: monthStart,
          endMonth: monthEnd,
        })
      : [];
    // La banda sale del alto del plot, como el eje: el SVG mide `height` exacto.
    const phaseH = phaseSegments.length > 0 ? 8 : 0;
    const pw = W - padLeft - padX;
    const ph = H - padY * 2 - axisH - phaseH;

    /** X de un MES concreto (no de una posición): el reparto es temporal, no posicional. */
    const xAtMonth = (m: number) =>
      padLeft + (monthSpan <= 0 ? pw / 2 : ((m - monthStart) / monthSpan) * pw);
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

    // Ticks del eje Y, recortados al rango REALMENTE pintado: `niceYTicks` redondea hacia fuera
    // y una etiqueta por encima de `vmax` caería fuera del plot, rotulando una línea que no está.
    const yTicks = yAxis
      ? niceYTicks(vmin, vmax, 4).filter((v) => v >= vmin && v <= vmax)
      : [];

    // Los dos bordes de la banda en su extremo DERECHO: es donde la dispersión es máxima y donde
    // hay sitio, porque la curva ya no sube por ahí.
    const bandEdge =
      bandVisible && bandEdgeLabels
        ? (() => {
            const last = bandPoints[bandPoints.length - 1]!;
            return {
              x: xAtMonth(last.month),
              yTop: yAt(last.p90),
              yBottom: yAt(last.p10),
            };
          })()
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
      monthEnd,
      monthSpan,
      visibleMonths,
      nw,
      fire,
      bandPath,
      bandEdge,
      yTicks,
      placedMarkers,
      W,
      H,
      padLeft,
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
    yAxis,
    band,
    bandEdgeLabels,
    markers,
    deflator,
    containerW,
  ]);

  /** Mes bajo el cursor, o `null`. Solo se arma cuando el llamante pide `hoverLabel`: sin él, el
   *  chart no captura el puntero y su árbol de nodos es el de siempre. */
  const [hoverMonth, setHoverMonth] = useState<number | null>(null);
  /** Id único del degradado. `useId` trae dos puntos en su valor, que no valen en un selector
   *  CSS pero sí en `url(#…)`; el prefijo `ff-risk-` deja claro de quién es y —regla de la
   *  casa— que un `#` en el código no es un color hardcoded. */
  const gradientId = `ff-risk-${useId().replace(/:/g, "")}`;

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
    monthEnd,
    monthSpan,
    visibleMonths,
    nw,
    fire,
    bandPath,
    bandEdge,
    yTicks,
    placedMarkers,
    W,
    H,
    padLeft,
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

  /** Con menos de dos paradas no hay degradado que pintar y la banda vuelve al acento plano:
   *  media escala de color se leería como una escala entera mal calculada. */
  const useGradient = bandGradient != null && bandGradient.length >= 2;
  const hoverProbe = hoverLabel ?? null;
  const hoverText = hoverProbe && hoverMonth != null ? hoverProbe(hoverMonth) : null;
  /** X del cursor, ya cuantizada al mes: la línea cae donde cae el número, no donde el ratón. */
  const hoverX = hoverMonth != null ? xAtMonth(hoverMonth) : null;

  /** Punto del puntero → MES de la rejilla. El `viewBox` mide en píxeles reales, pero el SVG se
   *  estira al 100 % del contenedor: sin reescalar por el ancho medido, el mes saldría corrido en
   *  cuanto el `ResizeObserver` fuera un frame por detrás. */
  const monthAtPointer = (clientX: number, rect: DOMRect): number | null => {
    if (rect.width <= 0 || pw <= 0) return null;
    const x = ((clientX - rect.left) * W) / rect.width;
    const frac = (x - padLeft) / pw;
    if (frac < 0 || frac > 1) return null;
    return Math.round(monthStart + frac * monthSpan);
  };

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
        x={padLeft}
        y={padY}
        width={pw}
        height={ph}
        rx={8}
        ry={8}
        fill="var(--proj-plot-bg)"
        stroke="var(--proj-grid)"
        strokeWidth={1}
      />

      {/* Eje Y (V2): rejilla + importes. Va DEBAJO de todo lo demás — es la referencia sobre la
          que se leen las series, no una serie. Sin `yAxis` no se emite ni un nodo. */}
      {yAxis && yTicks.length > 0
        ? yTicks.map((v) => (
            <g key={`mini-y-${v}`}>
              <line
                x1={padLeft}
                x2={padLeft + pw}
                y1={yAt(v)}
                y2={yAt(v)}
                stroke="var(--proj-grid)"
                strokeWidth={1}
              />
              <text
                x={padLeft - 6}
                y={yAt(v)}
                textAnchor="end"
                dominantBaseline="middle"
                className="proj-mini-tick"
                fill="var(--proj-tick)"
                fontSize="9.5"
              >
                {formatAxisMoney(v, yAxis.currencyIso)}
              </text>
            </g>
          ))
        : null}

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
        useGradient ? (
          <>
            {/* `gradientUnits="userSpaceOnUse"` es OBLIGATORIO: con el default
                (`objectBoundingBox`) el degradado se estiraría a la caja del `path`, que no
                empieza en `monthStart` ni acaba en `monthEnd`, y el mapeo mes→color se
                desplazaría en silencio — el fallo que ni se ve ni falla. */}
            <defs>
              <linearGradient
                id={gradientId}
                gradientUnits="userSpaceOnUse"
                x1={xAtMonth(monthStart)}
                x2={xAtMonth(monthEnd)}
                y1={0}
                y2={0}
              >
                {bandGradient!.map((stop, i) => (
                  // `style` y no el atributo `stop-color`: el valor es un `color-mix()` y el
                  // atributo de presentación de SVG 1.1 no lo acepta. Es la única excepción
                  // sancionada a «cero estilos inline» de la casa, y vive aquí.
                  <stop
                    key={`${gradientId}-${i}`}
                    offset={stop.offset}
                    style={{ stopColor: stop.color }}
                  />
                ))}
              </linearGradient>
            </defs>
            <path
              d={bandPath}
              fill={`url(#${gradientId})`}
              fillOpacity={0.28}
              stroke={`url(#${gradientId})`}
              strokeOpacity={0.55}
              strokeWidth={0.8}
            />
          </>
        ) : (
          <path
            d={bandPath}
            fill="var(--ff-accent)"
            fillOpacity={0.16}
            stroke="var(--ff-accent)"
            strokeOpacity={0.3}
            strokeWidth={0.8}
          />
        )
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

      {/* Rótulos de los dos bordes de la banda (V2). Dentro del plot y con halo del color del
          fondo (`.proj-mini-band-label`, `paint-order: stroke`) para que se lean sobre la banda
          teñida sin abrirles un hueco. Si los dos bordes están a menos de 14 px, no caben dos
          renglones y se omiten LOS DOS: media etiqueta rotularía el borde equivocado. */}
      {bandEdge && bandEdgeLabels && Math.abs(bandEdge.yBottom - bandEdge.yTop) >= 14 ? (
        <g>
          <text
            x={bandEdge.x - 4}
            y={bandEdge.yTop + 9}
            textAnchor="end"
            className="proj-mini-band-label"
            fill="var(--proj-meta)"
            fontSize="9.5"
          >
            {bandEdgeLabels.p90}
          </text>
          <text
            x={bandEdge.x - 4}
            y={bandEdge.yBottom - 3}
            textAnchor="end"
            className="proj-mini-band-label"
            fill="var(--proj-meta)"
            fontSize="9.5"
          >
            {bandEdgeLabels.p10}
          </text>
        </g>
      ) : null}

      {/* Hover (V5): el porcentaje exacto por edad, que es lo que el color solo puede aproximar.
          El rect captor va el ÚLTIMO para quedar por encima de todo lo dibujado, y es
          `pointer-events: all` con `fill="none"` — sin relleno no taparía nada aunque quisiera. */}
      {hoverProbe ? (
        <g>
          {hoverX != null && hoverText ? (
            <>
              <line
                x1={hoverX}
                x2={hoverX}
                y1={padY}
                y2={padY + ph}
                stroke="var(--proj-crosshair)"
                strokeWidth={1}
              />
              <text
                x={Math.min(Math.max(hoverX, padLeft + 4), padLeft + pw - 4)}
                y={padY + ph - 6}
                textAnchor={
                  hoverX > padLeft + pw * 0.66
                    ? "end"
                    : hoverX < padLeft + pw * 0.33
                      ? "start"
                      : "middle"
                }
                className="proj-mini-hover-label"
                fill="var(--ff-ink)"
                fontSize="9.5"
              >
                {hoverText}
              </text>
            </>
          ) : null}
          <rect
            x={padLeft}
            y={padY}
            width={pw}
            height={ph}
            fill="none"
            pointerEvents="all"
            onMouseMove={(e) =>
              setHoverMonth(
                monthAtPointer(e.clientX, e.currentTarget.getBoundingClientRect()),
              )
            }
            onMouseLeave={() => setHoverMonth(null)}
          />
        </g>
      ) : null}
    </svg>
   </div>
  );
}

