/**
 * El COLOR de la banda 10–90 % del gráfico de Jubilación (5.0.0, V2/V5/P1 de la tercera vuelta
 * de UX; feedback F6/F7/F8 del owner; reescrito por el modelo v2, C3).
 *
 * El owner dijo tres cosas del gráfico de riesgo: que no dejaba claro qué representaba cada cosa
 * (F6), que los tiles de al lado no enseñaban nada que la gráfica no enseñara (F7) y que le
 * pusiéramos rojo y verde (F8). La respuesta no es un chart más ni un tile más: es que **la banda
 * diga el riesgo por sí misma**. Su relleno pasa de un azul plano —que no significaba nada— a un
 * degradado por EDAD con la probabilidad acumulada de haber FALLADO: verde donde no falla ningún
 * escenario, ámbar donde empiezan a fallar, rojo donde ya falla una fracción intolerable.
 *
 * ## Qué se cuenta como fallo (modelo v2, §2.4)
 *
 * Ya no es «agotar el capital» a secas. Un camino falla —tiene que **volver a trabajar**— por tres
 * vías, y `failure_probability_by_age` las suma en su `probability` y las desglosa en su
 * `by_kind`:
 *
 * | | Fallo | `by_kind` |
 * |---|---|---|
 * | F1 | la cartera no cubre la necesidad de un mes jubilado | `[0]` |
 * | F2 | la tasa inicial del mes de jubilación supera el tope (SWR o puente) | `[1]` |
 * | F3 | el permitido de una regla por saldo no cubre la necesidad ordinaria | `[2]` |
 *
 * Colorear solo con F1 —lo que hacía `depletion_probability_by_age`— pintaba de verde un plan que
 * el motor da por fallido en el primer mes por tasa inicial. El desglose no tiñe nada: viaja al
 * hover (`failureKindsAtMonth`), que es donde cabe decir POR QUÉ.
 *
 * ## Los cortes ya NO son absolutos: salen del umbral del perfil (C3)
 *
 * Con V7 el umbral de éxito no existía y el semáforo era fijo, así que la escala de color también.
 * En el modelo v2 el umbral es del usuario (`success_threshold_pct`, 80–100, default 95) y **es una
 * restricción sobre la fecha**: quien pide un 80 % está aceptando de antemano que dos de cada diez
 * escenarios fallen, y pintarle de rojo el 10 % le contaría como ruina el plan que él mismo eligió.
 *
 * `riskCutoffsForThreshold(u)` deriva la escala:
 *
 * - **rojo** en `max(0,10, 1 − u/100)` — el complemento del umbral, con un SUELO del 10 %: por
 *   encima del 90 % de éxito el listón deja de moverse. Sin ese suelo, un umbral del 100 % pondría
 *   el rojo en «un solo escenario fallido» y la banda entera se leería como ruina.
 * - **ámbar** a la mitad del rojo.
 *
 * Con umbral 100 (y con 95, y con cualquiera ≥ 90) los cortes son `0,05 / 0,10` — **idénticos** a
 * los constantes que había antes, y el test lo fija: el rediseño no puede cambiar el color de un
 * plan que no ha cambiado.
 *
 * **Todo por MES, nunca por posición.** La rejilla de fallo del servidor arranca en el mes de la
 * jubilación y avanza de cinco en cinco años (`crates/engine-stochastic/src/mc.rs`), así que no es
 * la rejilla de la serie ni la de la banda. Emparejarlas por posición desplazaría el color décadas
 * y el chart seguiría pareciendo correcto — el fallo silencioso más caro de esta pantalla, el
 * mismo que ya documenta `lib/risk-bands.ts`.
 *
 * **Este módulo no decide nada del modelo**: la probabilidad la sortea el servidor y aquí solo se
 * interpola y se traduce a color. Y una limitación declarada: entre dos paradas el SVG interpola
 * COLORES linealmente, mientras `failureProbabilityAtMonth` interpola PROBABILIDADES. Dentro de un
 * tramo de la escala las dos cosas coinciden; al cruzar un corte, el degradado se adelanta o se
 * retrasa unos píxeles. Por eso el tooltip **no** lee el color: lee la misma función que genera
 * las paradas, y así el número y el tinte no pueden contradecirse en el sitio que importa.
 */

import type { FailureProbabilityPointApi } from "../api/types";
import { parseDisplayDecimal } from "./format";

/**
 * Los dos cortes de la escala, en FRACCIÓN de escenarios fallidos (`0,10` = uno de cada diez), no
 * en porcentaje. Se pasan enteros a `riskColorForProbability` y a `riskGradientStops` para que el
 * color de la leyenda, el de la banda y el del hover salgan de la MISMA escala: tres derivaciones
 * del umbral en tres sitios distintos es exactamente cómo se destiñe una leyenda sin que nada falle.
 */
export type RiskCutoffs = {
  /** A partir de aquí el color es ámbar puro. */
  amber: number;
  /** A partir de aquí el color es rojo puro. */
  red: number;
};

/** Suelo del corte rojo (C3): por encima del 90 % de éxito el listón deja de moverse. */
export const RISK_RED_FLOOR = 0.1;

/**
 * Umbral del perfil (entero 80–100) → los dos cortes de la escala de color.
 *
 * `red = max(0,10, 1 − u/100)`, `amber = red / 2`. Un umbral ausente o no finito se trata como
 * **100**, que es el caso más exigente y da el mismo par que el 95 del default: ante un umbral que
 * no llegó, la escala no se ablanda sola. El valor se acota a `[0, 100]` porque fuera de ahí la
 * fórmula deja de significar nada (un `-1000` pondría el rojo en el 1.100 % y nada sería rojo
 * jamás); el rango del contrato es 80–100.
 */
export function riskCutoffsForThreshold(
  thresholdPct: number | null | undefined,
): RiskCutoffs {
  const raw = typeof thresholdPct === "number" && Number.isFinite(thresholdPct)
    ? thresholdPct
    : 100;
  const u = Math.min(100, Math.max(0, raw));
  // `(100 − u) / 100` y NO `1 − u/100`: el segundo da 0,19999999999999996 para u = 80, y ese
  // sobrante hace que `p === amber` no se cumpla nunca — el peldaño ámbar puro de la leyenda se
  // convertiría en una mezcla al 0 % y el cuadradito de la escala dejaría de coincidir con la
  // banda. Aritmética exacta en enteros primero, división después.
  const red = Math.max(RISK_RED_FLOOR, (100 - u) / 100);
  return { amber: red / 2, red };
}

/** Una parada del degradado: `offset ∈ [0, 1]` sobre el ANCHO de la banda, y su color en tokens. */
export type RiskGradientStop = {
  offset: number;
  color: string;
};

export type RiskGradientInput = {
  /** `failure_probability_by_age` tal cual la publica el servidor. */
  points: readonly FailureProbabilityPointApi[] | null | undefined;
  /** Primer y último MES de la ventana pintada — los mismos extremos con los que el chart
   *  coloca la banda. Si el llamante usara otros, el mapeo mes→color se desplazaría en silencio. */
  monthStart: number;
  monthEnd: number;
  /** La escala, ya derivada del umbral del perfil con `riskCutoffsForThreshold`. */
  cutoffs: RiskCutoffs;
};

type Sample = {
  month: number;
  p: number;
  /** Desglose F1/F2/F3 de esa misma acumulada. `null` = el servidor no lo publicó. */
  byKind: readonly [number, number, number] | null;
};

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/**
 * Muestras utilizables, ordenadas por mes.
 *
 * **Una `probability: null` se SALTA, no vale 0.** Es la regla de la casa («null nunca es cero»)
 * y aquí tiene un precio concreto: pintar de verde un mes cuyo fallo el servidor no publicó sería
 * afirmar que no falla ningún escenario, que es la mentira más cara que este gráfico puede contar.
 *
 * `probability` viaja como `number` (excepción chart-only del contrato del dinero), pero el parseo
 * tolera una Decimal-string por si alguna vez cambia de lado: lo que NO se tolera es un valor que
 * no es número, y ese se descarta como el `null`.
 */
function samplesOf(
  points: readonly FailureProbabilityPointApi[] | null | undefined,
): Sample[] {
  if (!Array.isArray(points)) return [];
  const out: Sample[] = [];
  for (const pt of points) {
    if (!finite(pt.month_index)) continue;
    if (pt.probability == null) continue;
    const p = finite(pt.probability)
      ? pt.probability
      : parseDisplayDecimal(String(pt.probability));
    if (p == null || !Number.isFinite(p)) continue;
    const bk = pt.by_kind;
    out.push({
      month: pt.month_index,
      p,
      byKind:
        Array.isArray(bk) && bk.length === 3 && bk.every((n) => finite(n))
          ? [bk[0], bk[1], bk[2]]
          : null,
    });
  }
  out.sort((a, b) => a.month - b.month);
  return out;
}

/**
 * Probabilidad acumulada de haber FALLADO (F1, F2 o F3) en un MES cualquiera.
 *
 * Plana antes de la primera muestra (la rejilla del servidor arranca en la jubilación: el tramo
 * de acumulación no tiene muestra propia y su fallo es el de la primera, no una rampa inventada),
 * LINEAL entre muestras y plana después de la última.
 *
 * `null` ⟺ no hay ninguna muestra utilizable. No es cero: es que no se sabe.
 *
 * **Es la función que colorea Y la que rotula el hover.** Que sea una sola no es economía: si el
 * tooltip dijera un porcentaje y el color viniera de otro cálculo, la discrepancia solo se vería
 * comparando a ojo un tinte con un número, que es exactamente lo que nadie hace.
 *
 * Se llamó `depletionProbabilityAtMonth` hasta el modelo v2, cuando el sujeto dejó de ser «agotar
 * el capital» para ser «volver a trabajar» (§2.4). La aritmética es la misma; el nombre viejo
 * describía una de las tres causas y se leía como si fueran todas.
 */
export function failureProbabilityAtMonth(
  points: readonly FailureProbabilityPointApi[] | null | undefined,
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

/**
 * El desglose `[F1, F2, F3]` de la muestra MÁS CERCANA a un mes, para el hover.
 *
 * **No se interpola**, a diferencia de la probabilidad: interpolar tres acumuladas por separado
 * daría un trío que no suma la acumulada que el tooltip enseña justo al lado, y una descomposición
 * que no cuadra es peor que ninguna. Lo que se enseña es una muestra REAL, la de al lado.
 *
 * Empate de distancia → gana la muestra ANTERIOR: las tres series son acumuladas, así que la
 * anterior nunca atribuye a este mes fallos que todavía no han ocurrido.
 *
 * `null` ⟺ no hay ninguna muestra con desglose (backend que solo publica la acumulada, o rejilla
 * vacía).
 */
export function failureKindsAtMonth(
  points: readonly FailureProbabilityPointApi[] | null | undefined,
  month: number,
): readonly [number, number, number] | null {
  if (!finite(month)) return null;
  const s = samplesOf(points).filter((x) => x.byKind != null);
  if (s.length === 0) return null;
  let best = s[0]!;
  let bestDist = Math.abs(best.month - month);
  for (let i = 1; i < s.length; i++) {
    const cand = s[i]!;
    const d = Math.abs(cand.month - month);
    // `<` y no `<=`: en un empate se queda la anterior, que es la que ya estaba en `best`.
    if (d < bestDist) {
      best = cand;
      bestDist = d;
    }
  }
  return best.byKind;
}

/** Un porcentaje para `color-mix`, con un decimal: suficiente para que dos paradas contiguas no
 *  colapsen y sin la cola de coma flotante que ensuciaría el atributo. */
function mixPct(fraction: number): string {
  const clamped = Math.min(1, Math.max(0, fraction));
  return `${Math.round(clamped * 1000) / 10}%`;
}

/**
 * Probabilidad de fallo → color, con los cortes que el UMBRAL del perfil dicta (C3).
 *
 * Los tres extremos son tokens PUROS (`--ff-pos`, `--ff-warn`, `--ff-neg`), no mezclas al 0 % o al
 * 100 %: son los tres peldaños que la leyenda nombra, y tienen que resolver al mismo color exacto
 * que el cuadradito de la escala.
 *
 * `cutoffs` es OBLIGATORIO a propósito. Un default aquí sería la escala del umbral 100 aplicada en
 * silencio al plan de quien pidió un 80 %, y el error no se vería: la banda saldría roja donde el
 * plan cumple lo que su dueño pidió.
 *
 * Una probabilidad no finita cae a verde y **eso es inalcanzable a propósito**: las muestras sin
 * probabilidad se descartan antes (`samplesOf`) y la escala de la leyenda pasa literales. El
 * cuidado de «no lo sé no es cero» vive en el descarte, no en un cuarto color que la leyenda no
 * podría explicar.
 */
export function riskColorForProbability(p: number, cutoffs: RiskCutoffs): string {
  const { amber, red } = cutoffs;
  if (!Number.isFinite(p) || p <= 0) return "var(--ff-pos)";
  if (p >= red) return "var(--ff-neg)";
  if (p < amber) {
    return `color-mix(in oklch, var(--ff-warn) ${mixPct(p / amber)}, var(--ff-pos))`;
  }
  const t = (p - amber) / (red - amber);
  // El ámbar PURO se emite por la fracción de mezcla, no comparando `p === amber`: con cortes
  // derivados de un umbral la igualdad exacta es un accidente de coma flotante, y una mezcla «al
  // 0 %» no es el mismo string que el token — la leyenda tiene que poder repetir el peldaño.
  if (!Number.isFinite(t) || t <= 0) return "var(--ff-warn)";
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
 *    afirmando el mismo riesgo durante cuarenta años sobre una serie que por contrato solo puede
 *    crecer. Media banda no es banda, y un color plano inventado tampoco.
 *  - **Ventana degenerada** (`monthEnd <= monthStart`): no hay eje sobre el que repartir nada.
 *  - **Extremos no finitos.**
 */
export function riskGradientStops(input: RiskGradientInput): RiskGradientStop[] {
  const { monthStart, monthEnd, cutoffs } = input;
  if (!finite(monthStart) || !finite(monthEnd) || monthEnd <= monthStart) return [];
  const s = samplesOf(input.points);
  if (s.length < 2) return [];

  const span = monthEnd - monthStart;
  const at = (m: number) => failureProbabilityAtMonth(input.points, m) ?? 0;

  const stops: RiskGradientStop[] = [
    { offset: 0, color: riskColorForProbability(at(monthStart), cutoffs) },
  ];
  for (const sample of s) {
    if (sample.month <= monthStart || sample.month >= monthEnd) continue;
    stops.push({
      offset: (sample.month - monthStart) / span,
      color: riskColorForProbability(sample.p, cutoffs),
    });
  }
  stops.push({ offset: 1, color: riskColorForProbability(at(monthEnd), cutoffs) });

  return stops;
}
