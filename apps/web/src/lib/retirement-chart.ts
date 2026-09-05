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

/** Los campos de la serie que deciden las marcas. Un `Pick` para que un test escriba el caso
 *  mínimo sin inventarse una proyección entera. */
export type RetirementMarkerSeries = Pick<
  ProjectionSeriesApi,
  | "jubilacion_month_index"
  | "coast_fire_month_index"
  | "partial_retirement_month_index"
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
  if (visible(series.jubilacion_month_index)) {
    out.push({
      key: "retirement",
      kind: "retirement",
      month: series.jubilacion_month_index,
      label: "Jubilación",
      emphasis: "primary",
    });
  }
  if (visible(series.coast_fire_month_index)) {
    out.push({
      key: "coast",
      kind: "coast",
      month: series.coast_fire_month_index,
      label: "Coast",
      emphasis: "secondary",
    });
  }
  if (visible(series.partial_retirement_month_index)) {
    out.push({
      key: "partial",
      kind: "partial",
      month: series.partial_retirement_month_index,
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
