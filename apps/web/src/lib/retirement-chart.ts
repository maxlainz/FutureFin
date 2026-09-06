/**
 * Los MARCADORES del chart único de Jubilación (5.0.0, rediseño UX U1b; decisión **U5** de
 * #207).
 *
 * U5 funde en un solo gráfico lo que antes eran dos —el determinista de «Patrimonio vs.
 * objetivo FIRE» y el abanico de la sección «Riesgo»—, y con ellos se funden sus marcas: la
 * jubilación efectiva, el mes coast, el inicio de la media jornada y la primera paga de la
 * pensión. Cuatro hitos del MISMO plan que estaban repartidos entre un chart, una tira de fases
 * y tres tarjetas.
 *
 * Dos reglas que este módulo existe para no romper:
 *
 *  1. **Todo en MESES** (`month_index`), jamás en posiciones de `points[]`. Con `density=hybrid`
 *     la posición 13 es el mes 24: una marca colocada por índice cae décadas de su sitio y el
 *     chart resultante sigue pareciendo correcto — el modo de fallo que ya costó una regresión
 *     en la tira de fases.
 *  2. **Un rótulo que no cabe no se dibuja, pero la marca sí.** A 390 px hay sitio para dos
 *     etiquetas, no para cuatro; superponerlas las hace ilegibles todas. La línea vertical
 *     siempre se pinta (es el hito), y el rótulo se cede por prioridad: la jubilación nunca lo
 *     pierde.
 *
 * Lo que NO hace: dibujar, resolver fechas (el rotulador de meses lo inyecta la vista, que es
 * quien sabe si el eje va en fechas o en edades) ni decidir la ventana visible.
 */

import type { ProjectionSeriesApi } from "../api/types";
import { scenariosPerHundred } from "./risk-bands";

/** Los cuatro hitos que el chart puede marcar. Cerrado: uno nuevo obliga a decidir su
 *  prioridad frente a los demás, que es justo lo que no puede quedar implícito. */
export type RetirementMarkerKind = "retirement" | "coast" | "partial" | "pension";

export type RetirementChartMarker = {
  /** Key de React y del test. Estable por hito, nunca por posición. */
  key: RetirementMarkerKind;
  kind: RetirementMarkerKind;
  /** MES de la rejilla (0 = hoy). */
  month: number;
  label: string;
  /**
   * `primary` = la jubilación efectiva, la única marca con el color de acento y la única que
   * nunca cede su rótulo. El resto son contexto.
   */
  emphasis: "primary" | "secondary";
};

/**
 * Los campos de la serie que deciden las marcas. Un `Pick` para que un test escriba el caso mínimo
 * sin inventarse una proyección entera.
 *
 * **Modelo v2 (C1/C4)**: los cuatro salen del bloque «plan», no de los cruces deterministas de
 * 4.15.x. La jubilación es la FECHA VÁLIDA (`safe_date_month_index`, el primer mes en el que
 * jubilarse cumple el umbral), no el mes en que el patrimonio cruzaba un objetivo — ese objetivo
 * ya no existe. El mes coast es `coast_stop_month_index` (el solve del motor) y el de la fase
 * parcial, `partial_start_month_index`; sus gemelos `coast_fire_month_index` y
 * `partial_retirement_month_index` se retiraron de la respuesta.
 */
export type RetirementMarkerSeries = Pick<
  ProjectionSeriesApi,
  | "safe_date_month_index"
  | "coast_stop_month_index"
  | "partial_start_month_index"
  | "pension_start_month_index"
>;

export type MonthWindow = { startMonth: number; endMonth: number };

function finite(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/**
 * Serie → marcas visibles, **ordenadas por mes**.
 *
 * Un hito fuera de la ventana no se emite: una marca pegada al borde derecho se lee como «pasa
 * justo aquí» cuando en realidad pasa fuera del gráfico. Y un `null` no es un cero — la
 * estrategia que no tiene mes coast simplemente no trae esa marca, no la trae en el mes 0.
 *
 * El orden es por mes y no por prioridad a propósito: quien pinta recorre el eje de izquierda a
 * derecha, y la prioridad solo decide los RÓTULOS (`placeMarkerLabels`).
 */
export function buildRetirementChartMarkers(
  series: RetirementMarkerSeries | null | undefined,
  window: MonthWindow,
): RetirementChartMarker[] {
  if (!series) return [];
  if (!finite(window.startMonth) || !finite(window.endMonth)) return [];
  const visible = (m: unknown): m is number =>
    finite(m) && m >= window.startMonth && m <= window.endMonth;

  const out: RetirementChartMarker[] = [];
  if (visible(series.safe_date_month_index)) {
    out.push({
      key: "retirement",
      kind: "retirement",
      month: series.safe_date_month_index,
      label: "Jubilación",
      emphasis: "primary",
    });
  }
  if (visible(series.coast_stop_month_index)) {
    out.push({
      key: "coast",
      kind: "coast",
      month: series.coast_stop_month_index,
      label: "Coast",
      emphasis: "secondary",
    });
  }
  if (visible(series.partial_start_month_index)) {
    out.push({
      key: "partial",
      kind: "partial",
      month: series.partial_start_month_index,
      label: "Media jornada",
      emphasis: "secondary",
    });
  }
  if (visible(series.pension_start_month_index)) {
    out.push({
      key: "pension",
      kind: "pension",
      month: series.pension_start_month_index,
      label: "Pensión",
      emphasis: "secondary",
    });
  }
  return out.sort((a, b) => a.month - b.month);
}

/** Anchura media de un carácter del rótulo a 9,5 px, medida a ojo sobre la tipografía del
 *  chart. Solo se usa para decidir el anclaje, nunca para posicionar nada. */
const APPROX_LABEL_CHAR_PX = 5.2;

/** Margen que se le respeta al lienzo antes de pegar un rótulo a su borde. */
const LABEL_EDGE_PAD_PX = 2;

export type PlacedMarker = RetirementChartMarker & {
  /** X en píxeles del lienzo. */
  x: number;
  /** `false` ⟺ su rótulo colisiona con uno ya colocado y se cede. La línea se pinta igual. */
  showLabel: boolean;
  /** Ancla del `<text>`: los extremos se pegan al borde para no salirse del plot. */
  anchor: "start" | "middle" | "end";
};

export type PlaceMarkerLabelsInput = {
  markers: readonly RetirementChartMarker[];
  /** MES → x en píxeles. La inyecta el chart, que es quien tiene la escala. */
  xAtMonth: (month: number) => number;
  /** Ancho del lienzo, para decidir el anclaje de los extremos. */
  width: number;
  /** Separación mínima entre dos rótulos, en píxeles. */
  minGapPx?: number;
};

/**
 * Coloca los rótulos resolviendo colisiones **por prioridad, no por orden de aparición**.
 *
 * La jubilación se coloca SIEMPRE (es el hito que la página entera está contestando); las demás
 * se colocan de izquierda a derecha y ceden su rótulo si caen a menos de `minGapPx` de uno ya
 * puesto. Ceder es perder el texto, nunca la línea: el usuario sigue viendo que ahí pasa algo y
 * lo puede leer en las tarjetas o en el «Detalle».
 *
 * El caso que esto arregla y que no se ve en escritorio: con «Media jornada» a los 40, la
 * jubilación total a los 60 y la pensión a los 72, a 390 px los tres rótulos se solapan en un
 * borrón. Elegir cuál sobrevive es una decisión, así que se toma aquí y se prueba.
 */
export function placeMarkerLabels(input: PlaceMarkerLabelsInput): PlacedMarker[] {
  const minGap = input.minGapPx ?? 46;
  const placed: number[] = [];
  const byPriority = input.markers
    .map((m, i) => ({ m, i, x: input.xAtMonth(m.month) }))
    .sort((a, b) => {
      if (a.m.emphasis !== b.m.emphasis) return a.m.emphasis === "primary" ? -1 : 1;
      return a.x - b.x;
    });

  const decided = new Map<string, boolean>();
  for (const c of byPriority) {
    const collides = placed.some((x) => Math.abs(x - c.x) < minGap);
    if (!collides) placed.push(c.x);
    decided.set(c.m.key, !collides);
  }

  return input.markers.map((m) => {
    const x = input.xAtMonth(m.month);
    // Ancho APROXIMADO del rótulo a la tipografía del chart (9,5 px): sin medir texto no hay
    // manera exacta, y la aproximación basta para lo único que decide — si el rótulo centrado se
    // saldría del lienzo. Sin esto, «Media jornada» a los 4 años del origen perdía la M por el
    // borde izquierdo: el rótulo cabía, pero centrado empezaba en x negativa.
    const halfLabel = (m.label.length * APPROX_LABEL_CHAR_PX) / 2;
    return {
      ...m,
      x,
      showLabel: decided.get(m.key) === true,
      anchor:
        x - halfLabel < LABEL_EDGE_PAD_PX
          ? "start"
          : x + halfLabel > input.width - LABEL_EDGE_PAD_PX
            ? "end"
            : "middle",
    };
  });
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// La MARCA VERTICAL de la fecha válida (modelo v2, C4)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/** Lo que la marca lee del bloque «plan». Un `Pick` para que el test escriba seis campos y no una
 *  proyección entera. */
export type ValidDateMarkSeries = Pick<
  ProjectionSeriesApi,
  | "retirement_date_basis"
  | "safe_date_month_index"
  | "jubilacion_month_index"
  | "jubilacion_age"
  | "success_of_plan"
  | "success_threshold_pct"
>;

export type ValidDateMark = {
  /** La marca vertical, o `null` si no hay ninguna que pintar. */
  mark: { monthIndex: number; label: string } | null;
  /** Nota bajo la leyenda cuando NO hay marca. `null` = no hay nada que explicar (el hogar, o un
   *  backend que no publica el bloque «plan»: ahí la ausencia no es un hecho del plan). */
  note: string | null;
};

/**
 * El bloque «plan» → la marca vertical del chart y, cuando no la hay, la nota que lo explica.
 *
 * En v2 el chart ya no marca «el mes en que cruzaste un objetivo» —no hay objetivo—, marca **el
 * mes en que jubilarse cumple tu umbral**, y el rótulo lleva su éxito porque la fecha sin el éxito
 * es media respuesta. Las cuatro bases se rotulan distinto a propósito:
 *
 * | `retirement_date_basis` | marca | rótulo |
 * |---|---|---|
 * | `success_threshold` | `safe_date_month_index` | «Fecha válida · 95 de cada 100» |
 * | `target_age` | `jubilacion_month_index` | «A los 55, como pediste · 82 de cada 100» |
 * | `not_reachable` | — | nota «sin fecha válida al 95 %» |
 * | `pending` | — | nota «Resolviendo tu fecha válida…» |
 *
 * Con `target_age` la marca va en la EDAD QUE PEDISTE, no en la fecha válida: es el mes en que el
 * plan simulado se jubila, y ponerla en la fecha válida marcaría un mes en el que esta simulación
 * no hace nada. La fecha válida de ese caso se lee al lado, en los tiles («para tu 95 %…»).
 *
 * El éxito se rotula con `scenariosPerHundred` —la MISMA función que la frase-hito y el tile de
 * éxito— para que la marca y la frase no puedan decir dos números distintos del mismo sorteo, ni
 * uno de ellos redondear un 0,999 a «100 de cada 100». Sin éxito publicado el
 * rótulo se queda en su primera mitad: nunca se inventa un «100 de cada 100».
 */
export function chartValidDateMark(
  series: ValidDateMarkSeries | null | undefined,
): ValidDateMark {
  const basis = series?.retirement_date_basis;
  if (!series || basis == null) return { mark: null, note: null };

  const n = scenariosPerHundred(series.success_of_plan);
  const success = n == null ? "" : ` · ${n} de cada 100`;

  if (basis === "not_reachable") {
    const u = series.success_threshold_pct;
    return {
      mark: null,
      note:
        finite(u) ? `sin fecha válida al ${u} %` : "sin fecha válida a tu umbral",
    };
  }
  if (basis === "pending") {
    return { mark: null, note: "Resolviendo tu fecha válida…" };
  }
  if (basis === "target_age") {
    const m = series.jubilacion_month_index;
    if (!finite(m)) return { mark: null, note: null };
    const age = series.jubilacion_age;
    const head = finite(age) ? `A los ${age}, como pediste` : "Como pediste";
    return { mark: { monthIndex: m, label: `${head}${success}` }, note: null };
  }
  // `success_threshold`
  const m = series.safe_date_month_index;
  if (!finite(m)) return { mark: null, note: null };
  return { mark: { monthIndex: m, label: `Fecha válida${success}` }, note: null };
}
