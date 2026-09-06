import { describe, expect, it } from "vitest";
import {
  buildRetirementChartMarkers,
  chartValidDateMark,
  placeMarkerLabels,
  type RetirementMarkerSeries,
  type ValidDateMarkSeries,
} from "./retirement-chart";

const series = (over: Partial<RetirementMarkerSeries> = {}): RetirementMarkerSeries => ({
  safe_date_month_index: null,
  coast_stop_month_index: null,
  partial_start_month_index: null,
  pension_start_month_index: null,
  ...over,
});

/**
 * Modelo v2 (C4): las cuatro marcas salen del bloque «plan». La de jubilación es la FECHA VÁLIDA
 * (`safe_date_month_index`), no el mes en que el patrimonio cruzaba un objetivo — ese objetivo, y
 * los campos `coast_fire_month_index`/`partial_retirement_month_index` que la acompañaban, están
 * fuera del contrato.
 */
describe("marcas del chart único (U5, campos del plan v2)", () => {
  it("sin serie no hay marcas", () => {
    expect(buildRetirementChartMarkers(null, { startMonth: 0, endMonth: 600 })).toEqual([]);
  });

  it("emite solo los hitos que EXISTEN, ordenados por mes", () => {
    const out = buildRetirementChartMarkers(
      series({
        safe_date_month_index: 240,
        partial_start_month_index: 120,
        pension_start_month_index: 400,
      }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["partial", "retirement", "pension"]);
    expect(out.map((m) => m.month)).toEqual([120, 240, 400]);
  });

  it("`null` no es el mes 0: la estrategia sin coast no trae marca de coast", () => {
    const out = buildRetirementChartMarkers(
      series({ safe_date_month_index: 60, coast_stop_month_index: null }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["retirement"]);
  });

  it("un hito FUERA de la ventana no se pega al borde: no se dibuja", () => {
    const out = buildRetirementChartMarkers(
      series({ safe_date_month_index: 60, pension_start_month_index: 900 }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.key)).toEqual(["retirement"]);
  });

  it("los extremos de la ventana SÍ entran (inclusivos)", () => {
    const out = buildRetirementChartMarkers(
      series({ safe_date_month_index: 0, pension_start_month_index: 600 }),
      { startMonth: 0, endMonth: 600 },
    );
    expect(out.map((m) => m.month)).toEqual([0, 600]);
  });

  it("solo la jubilación es primaria", () => {
    const out = buildRetirementChartMarkers(
      series({
        safe_date_month_index: 240,
        coast_stop_month_index: 12,
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
      buildRetirementChartMarkers(series({ safe_date_month_index: 10 }), {
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
          safe_date_month_index: 240,
          partial_start_month_index: 100,
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
        series({ safe_date_month_index: 250, partial_start_month_index: 240 }),
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
        series({ safe_date_month_index: 250, pension_start_month_index: 255 }),
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
        series({ coast_stop_month_index: 100, partial_start_month_index: 110 }),
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
        series({ safe_date_month_index: 2, pension_start_month_index: 598 }),
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
        series({ partial_start_month_index: 48, safe_date_month_index: 300 }),
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
          safe_date_month_index: 240,
          coast_stop_month_index: 200,
          partial_start_month_index: 220,
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

/**
 * La MARCA VERTICAL de la fecha válida (modelo v2, C4).
 *
 * Lo que este bloque impide que vuelva: el chart marcaba «el mes en que cruzaste el objetivo
 * FIRE». En v2 no hay objetivo, hay un mes que cumple TU umbral, y el rótulo tiene que llevar su
 * éxito — una fecha sin el éxito con el que se resolvió es media respuesta, y la mitad que falta
 * es justo la que decide si el plan sirve.
 *
 * Los tres pares base→sitio son distintos a propósito y ninguno es intercambiable:
 * `success_threshold` marca la fecha válida, `target_age` marca la EDAD QUE PEDISTE (el mes en que
 * la simulación se jubila de verdad), y las dos bases sin fecha no marcan nada.
 */
describe("chartValidDateMark", () => {
  const plan = (
    over: Partial<ValidDateMarkSeries> = {},
  ): ValidDateMarkSeries => ({
    retirement_date_basis: "success_threshold",
    safe_date_month_index: 243,
    jubilacion_month_index: 243,
    jubilacion_age: 55,
    success_of_plan: 0.952,
    success_threshold_pct: 95,
    ...over,
  });

  it("success_threshold: marca en la fecha válida, rótulo con su éxito", () => {
    const got = chartValidDateMark(plan());
    expect(got.mark).toEqual({
      monthIndex: 243,
      label: "Fecha válida · 95 de cada 100",
    });
    expect(got.note).toBeNull();
  });

  it("target_age: marca en la edad pedida, NO en la fecha válida", () => {
    const got = chartValidDateMark(
      plan({
        retirement_date_basis: "target_age",
        // La fecha válida cae 8 años más tarde; la simulación se jubila igualmente a los 55.
        safe_date_month_index: 339,
        jubilacion_month_index: 243,
        jubilacion_age: 55,
        success_of_plan: 0.82,
      }),
    );
    expect(got.mark).toEqual({
      monthIndex: 243,
      label: "A los 55, como pediste · 82 de cada 100",
    });
    expect(got.note).toBeNull();
  });

  it("target_age sin edad resuelta: el rótulo pierde la edad, no se la inventa", () => {
    const got = chartValidDateMark(
      plan({
        retirement_date_basis: "target_age",
        jubilacion_age: null,
        success_of_plan: 0.82,
      }),
    );
    expect(got.mark?.label).toBe("Como pediste · 82 de cada 100");
  });

  it("not_reachable: sin marca y con la nota que dice el umbral", () => {
    const got = chartValidDateMark(
      plan({
        retirement_date_basis: "not_reachable",
        safe_date_month_index: null,
        success_of_plan: null,
        success_threshold_pct: 95,
      }),
    );
    expect(got.mark).toBeNull();
    expect(got.note).toBe("sin fecha válida al 95 %");
  });

  // El hogar (y un backend anterior al bloque «plan») no publican base: ahí la ausencia de marca
  // no es un hecho del plan y no se explica — una nota diría que este hogar no tiene fecha, y lo
  // que pasa es que no se resuelve UNA fecha de N personas.
  it("sin base publicada: ni marca ni nota", () => {
    expect(chartValidDateMark(plan({ retirement_date_basis: undefined }))).toEqual({
      mark: null,
      note: null,
    });
    expect(chartValidDateMark(null)).toEqual({ mark: null, note: null });
  });

  // Los dos topes anti-mentira los pone `scenariosPerHundred`, la MISMA función que la frase-hito
  // y el tile de éxito: un 0,999 no puede rotularse «100 de cada 100» en la marca y «99» al lado.
  it("un éxito de 0,999 se rotula 99, nunca 100", () => {
    expect(chartValidDateMark(plan({ success_of_plan: 0.999 }))?.mark?.label).toBe(
      "Fecha válida · 99 de cada 100",
    );
    expect(chartValidDateMark(plan({ success_of_plan: 1 }))?.mark?.label).toBe(
      "Fecha válida · 100 de cada 100",
    );
  });

  it("sin éxito publicado el rótulo se queda en su primera mitad", () => {
    expect(chartValidDateMark(plan({ success_of_plan: null }))?.mark?.label).toBe(
      "Fecha válida",
    );
  });

  // Un mes ausente con base `success_threshold` no puede degradar a la nota de `not_reachable`:
  // son cosas distintas y la nota afirmaría un resultado del sorteo que nadie ha publicado.
  it("base con fecha pero mes ausente: ni marca ni nota", () => {
    expect(
      chartValidDateMark(plan({ safe_date_month_index: null })),
    ).toEqual({ mark: null, note: null });
  });
});
