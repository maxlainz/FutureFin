/**
 * Las dos series discontinuas del plan (D29), fijadas donde se rompen en silencio: el DESFASE
 * histórico y la DEFLACTACIÓN.
 *
 * Las dos fallan igual de mal — la curva sale, es plausible, y está corrida unos meses o unos
 * miles de euros. Nada en pantalla lo denuncia, que es exactamente el modo de fallo de esta casa.
 */

import { describe, expect, it } from "vitest";
import { buildPlanAuxSeries, PLAN_AUX_SERIES_LABEL } from "./plan-series";

/** Puntos DIBUJADOS: dos meses de histórico (−2, −1), luego el 0 y el futuro. */
const MERGED = [
  { month_index: -2 },
  { month_index: -1 },
  { month_index: 0 },
  { month_index: 12 },
  { month_index: 24 },
];
/** `series.points` solo lleva el futuro: 3 puntos, y `futureOffset` vale 2. */
const RESPONSE_POINTS = 3;
const FUTURE_OFFSET = 2;
const NO_DEFLATION = () => 1;

function build(over: Partial<Parameters<typeof buildPlanAuxSeries>[0]> = {}) {
  return buildPlanAuxSeries({
    requiredCapitalPath: null,
    coastPath: null,
    responsePointCount: RESPONSE_POINTS,
    points: MERGED,
    futureOffset: FUTURE_OFFSET,
    deflator: NO_DEFLATION,
    ...over,
  });
}

describe("series auxiliares del plan", () => {
  it("sin solve no hay ninguna línea, y el chart queda como estaba", () => {
    expect(build()).toEqual([]);
  });

  it("«Capital necesario» y «Si dejas de aportar» salen con su clave y su rótulo", () => {
    const lines = build({
      requiredCapitalPath: [100, 200, 300],
      coastPath: [10, 20, 30],
    });
    expect(lines.map((l) => l.key)).toEqual(["required_capital", "coast"]);
    expect(lines[0]!.label).toBe(PLAN_AUX_SERIES_LABEL.required_capital);
    expect(lines[1]!.label).toBe(PLAN_AUX_SERIES_LABEL.coast);
  });

  it("cada línea lleva token de color y patrón de guion propios — nunca un hex", () => {
    const lines = build({ requiredCapitalPath: [1, 2, 3], coastPath: [4, 5, 6] });
    for (const l of lines) {
      expect(l.color).toMatch(/^var\(--proj-[a-z-]+\)$/);
      expect(l.dash).toMatch(/^\d+ \d+$/);
    }
    expect(lines[0]!.dash).not.toBe(lines[1]!.dash);
  });

  it("el histórico va a `null`: la curva ARRANCA en el mes 0, no en el suelo del eje", () => {
    const [line] = build({ requiredCapitalPath: [100, 200, 300] });
    // Paralela a los puntos DIBUJADOS, no a los de la respuesta.
    expect(line!.values).toHaveLength(MERGED.length);
    expect(line!.values.slice(0, FUTURE_OFFSET)).toEqual([null, null]);
  });

  it("el desfase se aplica con `futureOffset`: el primer valor cae en el mes 0", () => {
    const [line] = build({ requiredCapitalPath: [100, 200, 300] });
    expect(line!.values).toEqual([null, null, 100, 200, 300]);
  });

  it("sin histórico (`futureOffset` 0) las dos rejillas coinciden punto a punto", () => {
    const [line] = build({
      requiredCapitalPath: [7, 8, 9],
      points: [{ month_index: 0 }, { month_index: 12 }, { month_index: 24 }],
      futureOffset: 0,
    });
    expect(line!.values).toEqual([7, 8, 9]);
  });

  it("se deflacta por el `month_index` REAL del punto, nunca por su posición", () => {
    // Con densidad hybrid la posición 4 es el mes 24: un deflactor por posición dividiría por
    // 4 meses de inflación donde tocan 24, y la curva saldría alta y plausible.
    const [line] = build({
      requiredCapitalPath: [1000, 1000, 1000],
      deflator: (mi) => 1 / (1 + mi / 12),
    });
    expect(line!.values[2]).toBeCloseTo(1000, 6); // mes 0
    expect(line!.values[3]).toBeCloseTo(500, 6); // mes 12
    expect(line!.values[4]!).toBeCloseTo(1000 / 3, 6); // mes 24
  });

  it("una longitud que no casa con `points[]` DESCARTA la serie entera", () => {
    // Media curva alineada y media desplazada es peor que ninguna: nada en pantalla diría cuál
    // de las dos mitades es la buena.
    expect(build({ requiredCapitalPath: [1, 2] })).toEqual([]);
    expect(build({ requiredCapitalPath: [1, 2, 3, 4] })).toEqual([]);
  });

  it("un array vacío no es una serie: es la ausencia de solve", () => {
    expect(build({ requiredCapitalPath: [], responsePointCount: 0 })).toEqual([]);
  });

  it("un valor no finito se anula en su punto, sin arrastrar el resto de la curva", () => {
    const [line] = build({
      requiredCapitalPath: [100, Number.NaN, 300],
    });
    expect(line!.values).toEqual([null, null, 100, null, 300]);
  });

  it("solo `coast_path`: sale ella sola, sin hueco reservado para la otra", () => {
    const lines = build({ coastPath: [1, 2, 3] });
    expect(lines.map((l) => l.key)).toEqual(["coast"]);
  });
});
