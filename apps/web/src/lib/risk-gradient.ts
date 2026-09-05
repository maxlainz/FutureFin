/**
 * El COLOR de la banda 10–90 % del gráfico de Jubilación (5.0.0, V2/V5/P1 de la tercera vuelta
 * de UX; feedback F6/F7/F8 del owner).
 *
 * El owner dijo tres cosas del gráfico de riesgo: que no dejaba claro qué representaba cada cosa
 * (F6), que los tiles de al lado no enseñaban nada que la gráfica no enseñara (F7) y que le
 * pusiéramos rojo y verde (F8). La respuesta no es un chart más ni un tile más: es que **la banda
 * diga el riesgo por sí misma**. Su relleno pasa de un azul plano —que no significaba nada— a un
 * degradado por EDAD con la probabilidad acumulada de haber agotado el capital: verde donde no
 * falla ningún escenario, ámbar donde empiezan a fallar, rojo donde ya falla uno de cada diez. La
 * tabla «agotar a los 65/70/…» desaparece con esto: decía lo mismo con menos resolución y
 * ocupando una caja entera.
 *
 * **Los cortes son ABSOLUTOS** (P1), no relativos a un umbral: con V7 el umbral de éxito dejó de
 * existir —el semáforo es fijo, verde solo con cero fallos— así que no hay listón personal contra
 * el que medir. La escala es el mismo semáforo leído del lado de la ruina:
 *
 * | Probabilidad acumulada de agotar | Color |
 * |---|---|
 * | `0` exacto | `--ff-pos` |
 * | `(0, 0,05]` | mezcla progresiva `--ff-pos` → `--ff-warn` |
 * | `(0,05, 0,10)` | mezcla progresiva `--ff-warn` → `--ff-neg` |
 * | `≥ 0,10` | `--ff-neg` |
 *
 * El 10 % de escenarios agotados es exactamente el 90 % de éxito, el corte por debajo del cual el
 * servidor da el plan por ROJO. Los dos extremos de la escala son, por construcción, los dos
 * extremos del veredicto: si alguien mueve uno, tiene que mover el otro.
 *
 * **Todo por MES, nunca por posición.** La rejilla de agotamiento del servidor arranca en el mes
 * de la jubilación y avanza de cinco en cinco años (`crates/engine-stochastic/src/mc.rs`), así que
 * no es la rejilla de la serie ni la de la banda. Emparejarlas por posición desplazaría el color
 * décadas y el chart seguiría pareciendo correcto — el fallo silencioso más caro de esta pantalla,
 * el mismo que ya documenta `lib/risk-bands.ts`.
 *
 * **Este módulo no decide nada del modelo**: la probabilidad la sortea el servidor y aquí solo se
 * interpola y se traduce a color. Y una limitación declarada: entre dos paradas el SVG interpola
 * COLORES linealmente, mientras `depletionProbabilityAtMonth` interpola PROBABILIDADES. Dentro de
 * un tramo de la escala las dos cosas coinciden; al cruzar un corte, el degradado se adelanta o
 * se retrasa unos píxeles. Por eso el tooltip **no** lee el color: lee la misma función que
 * genera las paradas, y así el número y el tinte no pueden contradecirse en el sitio que importa.
 */

import type { DepletionProbabilityPointApi } from "../api/types";
import { parseDisplayDecimal } from "./format";

/** Ruina a partir de la cual el color ya es ámbar puro. */
export const RISK_AMBER_AT = 0.05;
/** Ruina a partir de la cual el color es rojo puro (= éxito por debajo del 90 %, veredicto rojo). */
export const RISK_RED_AT = 0.1;

/** Una parada del degradado: `offset ∈ [0, 1]` sobre el ANCHO de la banda, y su color en tokens. */
export type RiskGradientStop = {
  offset: number;
  color: string;
};

export type RiskGradientInput = {
  /** `depletion_probability_by_age` tal cual la publica el servidor. */
  points: readonly DepletionProbabilityPointApi[] | null | undefined;
  /** Primer y último MES de la ventana pintada — los mismos extremos con los que el chart
   *  coloca la banda. Si el llamante usara otros, el mapeo mes→color se desplazaría en silencio. */
  monthStart: number;
  monthEnd: number;
};

type Sample = { month: number; p: number };

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/**
 * Muestras utilizables, ordenadas por mes.
 *
 * **Una `probability: null` se SALTA, no vale 0.** Es la regla de la casa («null nunca es cero»)
 * y aquí tiene un precio concreto: pintar de verde un mes cuya ruina el servidor no publicó sería
 * afirmar que no falla ningún escenario, que es la mentira más cara que este gráfico puede contar.
 */
function samplesOf(
  points: readonly DepletionProbabilityPointApi[] | null | undefined,
): Sample[] {
  if (!Array.isArray(points)) return [];
  const out: Sample[] = [];
  for (const pt of points) {
    if (!finite(pt.month_index)) continue;
    if (pt.probability == null) continue;
    const p = parseDisplayDecimal(String(pt.probability));
    if (p == null || !Number.isFinite(p)) continue;
    out.push({ month: pt.month_index, p });
  }
  out.sort((a, b) => a.month - b.month);
  return out;
}

/**
 * Probabilidad acumulada de haber agotado el capital en un MES cualquiera.
 *
 * Plana antes de la primera muestra (la rejilla del servidor arranca en la jubilación: el tramo
 * de acumulación no tiene muestra propia y su ruina es la de la primera, no una rampa inventada),
 * LINEAL entre muestras y plana después de la última.
 *
 * `null` ⟺ no hay ninguna muestra utilizable. No es cero: es que no se sabe.
 *
 * **Es la función que colorea Y la que rotula el hover.** Que sea una sola no es economía: si el
 * tooltip dijera un porcentaje y el color viniera de otro cálculo, la discrepancia solo se vería
 * comparando a ojo un tinte con un número, que es exactamente lo que nadie hace.
 */
export function depletionProbabilityAtMonth(
  points: readonly DepletionProbabilityPointApi[] | null | undefined,
  month: number,
): number | null {
  const s = samplesOf(points);
  if (s.length === 0 || !finite(month)) return null;
  const first = s[0]!;
  const last = s[s.length - 1]!;
  if (month <= first.month) return first.p;
  if (month >= last.month) return last.p;
  for (let i = 1; i < s.length; i++) {
    const a = s[i - 1]!;
    const b = s[i]!;
    if (month <= b.month) {
      const span = b.month - a.month;
      if (span <= 0) return b.p;
      return a.p + ((month - a.month) / span) * (b.p - a.p);
    }
  }
  return last.p;
}

/** Un porcentaje para `color-mix`, con un decimal: suficiente para que dos paradas contiguas no
 *  colapsen y sin la cola de coma flotante que ensuciaría el atributo. */
function mixPct(fraction: number): string {
  const clamped = Math.min(1, Math.max(0, fraction));
  return `${Math.round(clamped * 1000) / 10}%`;
}

/**
 * Probabilidad de ruina → color, con los cortes ABSOLUTOS de P1.
 *
 * Los tres extremos son tokens PUROS (`--ff-pos`, `--ff-warn`, `--ff-neg`), no mezclas al 0 % o al
 * 100 %: son los tres peldaños que la leyenda nombra, y tienen que resolver al mismo color exacto
 * que el cuadradito de la escala.
 *
 * Una probabilidad no finita cae a verde y **eso es inalcanzable a propósito**: las muestras sin
 * probabilidad se descartan antes (`samplesOf`) y la escala de la leyenda pasa literales. El
 * cuidado de «no lo sé no es cero» vive en el descarte, no en un cuarto color que la leyenda no
 * podría explicar.
 */
export function riskColorForProbability(p: number): string {
  if (!Number.isFinite(p) || p <= 0) return "var(--ff-pos)";
  if (p >= RISK_RED_AT) return "var(--ff-neg)";
  if (p === RISK_AMBER_AT) return "var(--ff-warn)";
  if (p < RISK_AMBER_AT) {
    return `color-mix(in oklch, var(--ff-warn) ${mixPct(p / RISK_AMBER_AT)}, var(--ff-pos))`;
  }
  const t = (p - RISK_AMBER_AT) / (RISK_RED_AT - RISK_AMBER_AT);
  return `color-mix(in oklch, var(--ff-neg) ${mixPct(t)}, var(--ff-warn))`;
}

/**
 * Las paradas del `<linearGradient>` que tiñe la banda, en orden y con `offset ∈ [0, 1]`.
 *
 * `offset(m) = (m − monthStart) / (monthEnd − monthStart)` — **por MES**. El chart declara el
 * degradado con `gradientUnits="userSpaceOnUse"` y los mismos dos extremos, así que estos offsets
 * caen en la X exacta de su mes aunque la banda ocupe solo una parte del plot.
 *
 * Devuelve `[]` —y el chart vuelve al acento plano de siempre— en tres casos, los tres «no hay
 * degradado que pintar», nunca «píntalo de verde»:
 *
 *  - **Menos de DOS muestras utilizables.** Con una sola, el degradado sería un color plano
 *    afirmando la misma ruina durante cuarenta años sobre una serie que por contrato solo puede
 *    crecer. Media banda no es banda, y un color plano inventado tampoco.
 *  - **Ventana degenerada** (`monthEnd <= monthStart`): no hay eje sobre el que repartir nada.
 *  - **Extremos no finitos.**
 */
export function riskGradientStops(input: RiskGradientInput): RiskGradientStop[] {
  const { monthStart, monthEnd } = input;
  if (!finite(monthStart) || !finite(monthEnd) || monthEnd <= monthStart) return [];
  const s = samplesOf(input.points);
  if (s.length < 2) return [];

  const span = monthEnd - monthStart;
  const at = (m: number) => depletionProbabilityAtMonth(input.points, m) ?? 0;

  const stops: RiskGradientStop[] = [
    { offset: 0, color: riskColorForProbability(at(monthStart)) },
  ];
  for (const sample of s) {
    if (sample.month <= monthStart || sample.month >= monthEnd) continue;
    stops.push({
      offset: (sample.month - monthStart) / span,
      color: riskColorForProbability(sample.p),
    });
  }
  stops.push({ offset: 1, color: riskColorForProbability(at(monthEnd)) });

  return stops;
}
