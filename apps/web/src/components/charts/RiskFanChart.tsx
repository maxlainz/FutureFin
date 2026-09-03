/**
 * Abanico de percentiles de la sección «Riesgo» de Jubilación (5.0.0, D28 / §B.5 de #207):
 * área entre p10 y p90, mediana p50 discontinua y la línea DETERMINISTA que el resto de la app
 * dibuja como dinero, sobre el mismo eje X.
 *
 * **Por qué es una primitiva propia y no una prop más de `MiniProjection`**: aquel componente
 * dibuja UNA serie (más áreas de activo apiladas) sobre la rejilla de `points[]`; aquí hay dos
 * fuentes con **rejillas distintas** (la banda es siempre `hybrid`, la serie puede ser
 * `monthly`), un área entre percentiles y un eje Y que no puede empezar en 0. Meterlo dentro
 * habría añadido cinco props y una segunda máquina de escalas al componente que ya usan el
 * Resumen y Jubilación — el clásico «una prop más» que acaba en dos charts dentro de uno.
 *
 * Todo se posiciona por **MES** (`xAtMonth`), nunca por posición de array: con dos densidades a
 * la vez, emparejar por índice desplaza el abanico décadas y el resultado sigue pareciendo un
 * chart correcto. La aritmética (alineación, deflactación, rango) vive pura en
 * `lib/risk-bands.ts` con su test; aquí solo se pinta.
 *
 * Color: dos tokens y nada más. La banda y su mediana son **acento** (es la lectura del plan,
 * la misma familia que el objetivo FIRE) y la determinista es `--proj-nw`, exactamente el color
 * con el que el patrimonio se dibuja en toda la app — para que quien mire el abanico reconozca
 * cuál de las tres curvas es la que ya conoce. Se separan además por PATRÓN (relleno / guion /
 * sólido), así que la identidad nunca es solo color.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { RiskFanModel } from "../../lib/risk-bands";
import { projectionXTickLabel } from "../../lib/projection-chart";

export type RiskFanXAxisOpts = {
  ageUiMode: "dates" | "ages";
  birthDateIso?: string | null;
  anchorDateYmd?: string | null;
  calendarTz: string;
};

export function RiskFanChart({
  model,
  height = 220,
  xAxis,
  ariaLabel = "Escenarios con volatilidad: banda 10–90 %, mediana y proyección determinista",
}: {
  model: RiskFanModel | null;
  height?: number;
  xAxis?: RiskFanXAxisOpts | null;
  ariaLabel?: string;
}) {
  // Igual que `MiniProjection`: medimos el ancho real para que el viewBox use px reales y los
  // marcadores redondos salgan redondos (con `preserveAspectRatio="none"` sobre un viewBox fijo
  // salen ovalados).
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [containerW, setContainerW] = useState(320);
  useLayoutEffect(() => {
    const el = wrapperRef.current;
    if (!el) return;
    const w = el.getBoundingClientRect().width;
    if (w > 0) setContainerW(Math.round(w));
  }, []);
  useEffect(() => {
    const el = wrapperRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr && cr.width > 0) setContainerW(Math.round(cr.width));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const geo = useMemo(() => {
    if (!model || model.band.length < 2) return null;
    const W = containerW;
    const H = height;
    const padX = 4;
    const padY = 4;
    const axisH = xAxis ? 18 : 0;
    const pw = W - padX * 2;
    const ph = H - padY * 2 - axisH;
    const monthSpan = model.monthEnd - model.monthStart;

    // Eje Y con padding: el abanico NO empieza en cero. Un fan chart anclado al 0 aplasta la
    // banda contra la línea justo donde se lee la dispersión, que es lo único que este chart
    // tiene que contar.
    const lo = model.valueMin;
    const hi = model.valueMax;
    const pad = (hi - lo) * 0.08 || Math.abs(hi) * 0.02 || 1;
    const vmin = lo - pad;
    const vmax = hi + pad;

    const xAtMonth = (m: number) =>
      padX + (monthSpan <= 0 ? pw / 2 : ((m - model.monthStart) / monthSpan) * pw);
    const yAt = (v: number) => {
      const range = vmax - vmin || 1;
      return padY + ph - ((v - vmin) / range) * ph;
    };

    const line = (pts: readonly { month: number; value: number }[]) =>
      pts
        .map((p) => `${xAtMonth(p.month).toFixed(1)},${yAt(p.value).toFixed(1)}`)
        .join(" ");

    // El área entre percentiles: p90 de izquierda a derecha, p10 de vuelta. Un solo `path`
    // cerrado — dos polígonos separados dejarían una costura de 1px visible en oscuro.
    const top = model.band
      .map((b) => `${xAtMonth(b.month).toFixed(1)},${yAt(b.p90).toFixed(1)}`)
      .join(" L ");
    const bottom = model.band
      .slice()
      .reverse()
      .map((b) => `${xAtMonth(b.month).toFixed(1)},${yAt(b.p10).toFixed(1)}`)
      .join(" L ");
    const bandPath = `M ${top} L ${bottom} Z`;

    const medianPoints = line(
      model.band.map((b) => ({ month: b.month, value: b.p50 })),
    );
    const deterministicPoints =
      model.deterministic.length >= 2 ? line(model.deterministic) : null;

    return {
      W,
      H,
      padX,
      padY,
      pw,
      ph,
      monthSpan,
      xAtMonth,
      bandPath,
      medianPoints,
      deterministicPoints,
      visibleMonths: monthSpan + 1,
    };
  }, [model, containerW, height, xAxis]);

  if (!geo) {
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
    W,
    H,
    padX,
    padY,
    pw,
    ph,
    monthSpan,
    xAtMonth,
    bandPath,
    medianPoints,
    deterministicPoints,
    visibleMonths,
  } = geo;

  return (
    <div ref={wrapperRef} style={{ width: "100%", height }}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="proj-mini"
        preserveAspectRatio="none"
        style={{ width: "100%", height, display: "block" }}
        role="img"
        aria-label={ariaLabel}
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

        {/* Banda 10–90 %: relleno tenue del acento con borde apenas visible. La opacidad va en
            el atributo y no en el color para que el mismo token resuelva en claro y en oscuro. */}
        <path
          d={bandPath}
          fill="var(--ff-accent)"
          fillOpacity={0.16}
          stroke="var(--ff-accent)"
          strokeOpacity={0.35}
          strokeWidth={0.8}
        />

        {/* Mediana: acento, DISCONTINUA — el guion es el recordatorio de que no es un camino. */}
        <polyline
          points={medianPoints}
          fill="none"
          stroke="var(--ff-accent)"
          strokeWidth={1.6}
          strokeDasharray="5 3"
          strokeLinejoin="round"
          strokeLinecap="round"
        />

        {/* Determinista: la línea que el resto de la app llama «patrimonio neto», con su color y
            sólida. Va ENCIMA de la banda a propósito: es la referencia, no una serie más. */}
        {deterministicPoints ? (
          <polyline
            points={deterministicPoints}
            fill="none"
            stroke="var(--proj-nw)"
            strokeWidth={2}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        ) : null}

        {/* Marcador de la jubilación efectiva, en el MISMO sitio que en el chart grande. */}
        {model && model.retirementMonth != null ? (
          <line
            x1={xAtMonth(model.retirementMonth)}
            x2={xAtMonth(model.retirementMonth)}
            y1={padY}
            y2={padY + ph}
            stroke="var(--proj-fire)"
            strokeWidth={1.5}
          />
        ) : null}

        {/* Eje X: ~5 ticks equidistantes en MESES (no en posiciones del array). */}
        {xAxis ? (
          (() => {
            const tickCount = 5;
            const ticks: number[] = [];
            for (let i = 0; i < tickCount; i++) {
              const frac = tickCount - 1 === 0 ? 0 : i / (tickCount - 1);
              ticks.push(Math.round((model?.monthStart ?? 0) + frac * monthSpan));
            }
            const yBase = padY + ph + 12;
            return (
              <g>
                {ticks.map((m, i) => (
                  <text
                    key={`risk-tick-${m}-${i}`}
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
