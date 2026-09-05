import { describe, expect, it } from "vitest";
import {
  buildRetirementChartMarkers,
  placeMarkerLabels,
  type RetirementMarkerSeries,
} from "./retirement-chart";

const series = (over: Partial<RetirementMarkerSeries> = {}): RetirementMarkerSeries => ({
  jubilacion_month_index: null,
  coast_fire_month_index: null,
  partial_retirement_month_index: null,
  pension_start_month_index: null,
  ...over,
});

describe("marcas del chart único (U5)", () => {
  it("sin serie no hay marcas", () => {
    expect(buildRetirementChartMarkers(null, { startMonth: 0, endMonth: 600 })).toEqual([]);
  });

  it("emite solo los hitos que EXISTEN, ordenados por mes", () => {
    const out = buildRetirementChartMarkers(
      series({
        jubilacion_month_index: 240,
        partial_retirement_month_index: 120,
        pension_start_month_index: 400,
      }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["partial", "retirement", "pension"]);
    expect(out.map((m) => m.month)).toEqual([120, 240, 400]);
  });

  it("`null` no es el mes 0: la estrategia sin coast no trae marca de coast", () => {
    const out = buildRetirementChartMarkers(
      series({ jubilacion_month_index: 60, coast_fire_month_index: null }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["retirement"]);
  });

  it("un hito FUERA de la ventana no se pega al borde: no se dibuja", () => {
    const out = buildRetirementChartMarkers(
      series({ jubilacion_month_index: 60, pension_start_month_index: 900 }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["retirement"]);
  });

  it("los extremos de la ventana SÍ entran (inclusivos)", () => {
    const out = buildRetirementChartMarkers(
      series({ jubilacion_month_index: 0, pension_start_month_index: 600 }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.month)).toEqual([0, 600]);
  });

  it("solo la jubilación es primaria", () => {
    const out = buildRetirementChartMarkers(
      series({
        jubilacion_month_index: 240,
        coast_fire_month_index: 12,
        pension_start_month_index: 400,
      }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.filter((m) => m.emphasis === "primary").map((m) => m.key)).toEqual([
      "retirement",
    ]);
  });

  it("una ventana no finita no produce nada", () => {
    expect(
      buildRetirementChartMarkers(series({ jubilacion_month_index: 10 }), {
        startMonth: Number.NaN,
        endMonth: 600,
      }),
    ).toEqual([]);
  });
});

describe("colocación de rótulos", () => {
  /** Escala de juguete: 600 meses repartidos en 600 px → 1 px por mes. */
  const xAtMonth = (m: number) => m;

  it("con sitio de sobra, todos los rótulos se pintan", () => {
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({
          jubilacion_month_index: 240,
          partial_retirement_month_index: 100,
          pension_start_month_index: 480,
        }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth,
      width: 600,
    });
    expect(placed.every((p) => p.showLabel)).toBe(true);
  });

  it("la jubilación NUNCA cede su rótulo, aunque llegue después en el eje", () => {
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({ jubilacion_month_index: 250, partial_retirement_month_index: 240 }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth,
      width: 600,
      minGapPx: 46,
    });
    const byKey = Object.fromEntries(placed.map((p) => [p.key, p]));
    expect(byKey.retirement!.showLabel).toBe(true);
    expect(byKey.partial!.showLabel).toBe(false);
  });

  it("ceder el rótulo NO borra la marca: la línea sigue teniendo su x", () => {
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({ jubilacion_month_index: 250, pension_start_month_index: 255 }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth,
      width: 600,
    });
    expect(placed).toHaveLength(2);
    expect(placed.map((p) => p.x)).toEqual([250, 255]);
  });

  it("de dos secundarias que colisionan, sobrevive la de la izquierda", () => {
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({ coast_fire_month_index: 100, partial_retirement_month_index: 110 }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth,
      width: 600,
    });
    const byKey = Object.fromEntries(placed.map((p) => [p.key, p]));
    expect(byKey.coast!.showLabel).toBe(true);
    expect(byKey.partial!.showLabel).toBe(false);
  });

  it("los rótulos de los extremos se anclan al borde para no salirse del plot", () => {
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({ jubilacion_month_index: 2, pension_start_month_index: 598 }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth,
      width: 600,
    });
    expect(placed.map((p) => p.anchor)).toEqual(["start", "end"]);
  });

  it("un rótulo que CABE pero centrado se saldría se ancla al principio", () => {
    // «Media jornada» a los 4 años de un horizonte de 54: la marca está a 32 px del origen y el
    // rótulo mide ~68 px, así que centrado empezaría en x negativa y perdía la M.
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({ partial_retirement_month_index: 48, jubilacion_month_index: 300 }),
        { startMonth: 0, endMonth: 648 },
      ),
      xAtMonth: (m) => (m / 648) * 382 + 4,
      width: 390,
    });
    const byKey = Object.fromEntries(placed.map((p) => [p.key, p]));
    expect(byKey.partial!.anchor).toBe("start");
    expect(byKey.retirement!.anchor).toBe("middle");
  });

  it("a 390 px las cuatro marcas no caben y se ceden rótulos, no líneas", () => {
    const narrow = (m: number) => (m / 600) * 380 + 4;
    const placed = placeMarkerLabels({
      markers: buildRetirementChartMarkers(
        series({
          jubilacion_month_index: 240,
          coast_fire_month_index: 200,
          partial_retirement_month_index: 220,
          pension_start_month_index: 260,
        }),
        { startMonth: 0, endMonth: 600 },
      ),
      xAtMonth: narrow,
      width: 390,
    });
    expect(placed).toHaveLength(4);
    expect(placed.filter((p) => p.showLabel).map((p) => p.key)).toEqual(["retirement"]);
  });
});
