/**
 * Preparación PURA de las **series auxiliares discontinuas** del chart de Proyección
 * (5.0.0, D29 / §B.7 del plan de #207): «Capital necesario» (`required_capital_path`) y «Si dejas
 * de aportar en el mes coast» (`coast_path`).
 *
 * Las dos son series LÍQUIDAS simuladas de verdad —una ejecución más del motor cada una—, no un
 * objetivo descontado a una tasa escalar: ese número sería plausible y ninguna simulación lo
 * produce (hallazgo M8 de la revisión adversarial). Por eso se dibujan como curvas y no como una
 * línea horizontal.
 *
 * Cuatro reglas que este módulo existe para no romper:
 *
 *  1. **Paralelas a `points[]` de la RESPUESTA, no a los puntos que dibuja el chart.** El chart
 *     mergea histórico (meses negativos) delante del futuro, así que la posición `i` de
 *     `required_capital_path` corresponde al punto `i + futureOffset` de lo dibujado — exactamente
 *     el mismo desfase que ya aplica `fire_target_series`. Alinearlas por posición sin el offset
 *     desplaza la curva tantos meses como snapshots tenga el usuario.
 *  2. **Longitud exacta o nada.** Si el array no mide lo mismo que `points[]`, la serie se
 *     descarta entera: media curva alineada y media desplazada es peor que ninguna, porque nada
 *     en pantalla dice cuál de las dos mitades es la buena.
 *  3. **La misma deflactación que el resto del chart**, mes a mes con el `month_index` REAL del
 *     punto (nunca su posición: con `density=hybrid` la posición 13 es el mes 24). El servidor no
 *     publica variante «en euros de hoy» de estas dos, así que el factor entra como parámetro y
 *     es el mismo que ya deflacta las áreas de activo y las líneas de miembro.
 *  4. **`disposable_capital` NO se dibuja** (D31): el margen es un tile con su importe acumulado,
 *     y una tercera curva discontinua sobre el mismo plot no añade una lectura, añade ruido.
 */

/** Clave estable de cada serie auxiliar: React, leyenda y tooltip usan la misma. */
export type PlanAuxSeriesKey = "required_capital" | "coast";

export type PlanAuxSeriesLine = {
  key: PlanAuxSeriesKey;
  /** Rótulo de la leyenda y del tooltip — el mismo, para que se emparejen de un vistazo. */
  label: string;
  /** Token `var(--proj-*)`, nunca hex. */
  color: string;
  /** Patrón de guion del SVG. Las dos series comparten familia de color y se separan por él. */
  dash: string;
  /**
   * Valores YA deflactados, **paralelos a los puntos dibujados** (histórico incluido): `null` en
   * los meses donde la serie no existe, que son todos los del pasado. Solo los vértices no nulos
   * se pintan, así que la curva arranca en el mes 0 igual que el objetivo FIRE.
   */
  values: (number | null)[];
};

export type PlanAuxSeriesInput = {
  requiredCapitalPath?: readonly number[] | null;
  coastPath?: readonly number[] | null;
  /** `series.points.length` — la longitud que ambas series deben tener para ser paralelas. */
  responsePointCount: number;
  /** Los puntos que el chart DIBUJA (histórico mergeado delante del futuro). */
  points: readonly { month_index: number }[];
  /** Posición del primer punto FUTURO dentro de `points` (`merged.futureOffset`). */
  futureOffset: number;
  /** Factor por el que se multiplica el importe nominal de ese mes; 1 en modo nominal. */
  deflator: (monthIndex: number) => number;
};

/** Rótulos publicados. Viven aquí y no en la vista porque el tooltip y la leyenda son dos sitios
 *  distintos que deben decir exactamente lo mismo. */
export const PLAN_AUX_SERIES_LABEL: Record<PlanAuxSeriesKey, string> = {
  required_capital: "Capital necesario",
  coast: "Si dejas de aportar en el mes coast",
};

const SPEC: Record<PlanAuxSeriesKey, { color: string; dash: string }> = {
  required_capital: { color: "var(--proj-required)", dash: "6 4" },
  coast: { color: "var(--proj-coast)", dash: "2 5" },
};

function remap(
  raw: readonly number[] | null | undefined,
  input: PlanAuxSeriesInput,
): (number | null)[] | null {
  if (!Array.isArray(raw)) return null;
  if (raw.length === 0 || raw.length !== input.responsePointCount) return null;
  return input.points.map((p, i) => {
    if (i < input.futureOffset) return null;
    const v = raw[i - input.futureOffset];
    if (v == null || !Number.isFinite(v)) return null;
    return v * input.deflator(p.month_index);
  });
}

/**
 * `required_capital_path` / `coast_path` → las líneas discontinuas listas para pintar.
 *
 * Devuelve solo las que EXISTEN: con `asap` y `pension_bridge` no hay solve y el array llega
 * vacío, así que el chart queda idéntico a 4.15.x — la lista vacía no cuesta nada y no reserva
 * hueco en la leyenda para una curva que no está.
 */
export function buildPlanAuxSeries(
  input: PlanAuxSeriesInput,
): PlanAuxSeriesLine[] {
  const out: PlanAuxSeriesLine[] = [];
  const required = remap(input.requiredCapitalPath, input);
  if (required) {
    out.push({
      key: "required_capital",
      label: PLAN_AUX_SERIES_LABEL.required_capital,
      ...SPEC.required_capital,
      values: required,
    });
  }
  const coast = remap(input.coastPath, input);
  if (coast) {
    out.push({
      key: "coast",
      label: PLAN_AUX_SERIES_LABEL.coast,
      ...SPEC.coast,
      values: coast,
    });
  }
  return out;
}
