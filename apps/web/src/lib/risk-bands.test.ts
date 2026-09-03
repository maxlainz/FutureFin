/**
 * Tests de la sección «Riesgo» (5.0.0, D28). Fijan las tres cosas que aquí se rompen en
 * silencio —la alineación por mes con dos densidades, la deflactación de las CUATRO series a la
 * vez y el redondeo de la probabilidad— más las traducciones de veredicto y de `null` a copy.
 */

import { describe, expect, it } from "vitest";
import type {
  DepletionProbabilityPointApi,
  ProjectionBandPointApi,
  ProjectionBandsApi,
} from "../api/types";
import {
  buildDepletionRows,
  buildRiskExtraRows,
  buildRiskFan,
  formatSuccessScenarios,
  formatSuccessThreshold,
  riskFootnote,
  showsNoVolatilityNotice,
  successVerdictTone,
  summarySuccessTile,
} from "./risk-bands";
import { deflationFactorAt } from "./projection-chart";

const NO_DEFLATION = () => 1;

function bandPoint(
  month: number,
  p10: number,
  p50: number,
  p90: number,
): ProjectionBandPointApi {
  return {
    month_index: month,
    net_worth_p10: p10,
    net_worth_p50: p50,
    net_worth_p90: p90,
  };
}

/** Rejilla `hybrid` de juguete: meses 0..3 y luego 12, 24 — no equidistante a propósito. */
const HYBRID_BAND: ProjectionBandPointApi[] = [
  bandPoint(0, 100, 100, 100),
  bandPoint(1, 95, 105, 115),
  bandPoint(2, 90, 110, 130),
  bandPoint(3, 85, 115, 145),
  bandPoint(12, 60, 160, 260),
  bandPoint(24, 40, 220, 400),
];

/** La misma ventana a densidad `monthly`: 25 puntos, uno por mes. */
const MONTHLY_SERIES = Array.from({ length: 25 }, (_, i) => ({
  month_index: i,
  net_worth: 100 + i * 5,
}));

function bandsFixture(over: Partial<ProjectionBandsApi> = {}): ProjectionBandsApi {
  return {
    view: "mine",
    months: 24,
    horizon_basis: "lifespan_age",
    anchor_date_ymd: "2026-09-01",
    paths: 500,
    seed: "12345678901234567890",
    percentiles: [10, 50, 90],
    points: HYBRID_BAND,
    success_probability: "0.870000",
    success_threshold_pct: 95,
    success_verdict: "amber",
    depletion_probability_by_age: [],
    retirement_month_index_percentiles: null,
    underfunded_probability: null,
    months_below_need_p50: 0,
    withdrawal_to_need_ratio_p50: null,
    any_volatility_declared: true,
    buffer_active: false,
    buffer_refills_p50: null,
    buffer_refill_net_total_p50: null,
    strategy: "asap",
    retirement_trigger: "liquid_crossing",
    computed_in_ms: 55,
    model_note: "…",
    ...over,
  };
}

describe("buildRiskFan — abanico dibujable", () => {
  it("empareja banda y serie POR MES, no por posición (hybrid × monthly)", () => {
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
    })!;
    expect(fan).not.toBeNull();
    // La banda conserva sus 6 puntos; la determinista, sus 25. Emparejarlas por índice habría
    // recortado la línea a 6 puntos y la habría terminado en el mes 5 en vez de en el 24.
    expect(fan.band.map((b) => b.month)).toEqual([0, 1, 2, 3, 12, 24]);
    expect(fan.deterministic).toHaveLength(25);
    expect(fan.deterministic[fan.deterministic.length - 1]!.month).toBe(24);
    expect(fan.monthStart).toBe(0);
    expect(fan.monthEnd).toBe(24);
  });

  it("recorta la determinista a la ventana de la banda por MES, no por longitud", () => {
    // Serie más larga que la banda (el horizonte del chart llega al mes 40).
    const longer = Array.from({ length: 41 }, (_, i) => ({
      month_index: i,
      net_worth: 100 + i,
    }));
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: longer,
      deflator: NO_DEFLATION,
    })!;
    expect(fan.deterministic[fan.deterministic.length - 1]!.month).toBe(24);
    expect(fan.deterministic.every((d) => d.month <= 24)).toBe(true);
  });

  it("con la serie entera fuera de la ventana no cuela el primer punto", () => {
    // `lastPointIndexAtOrBeforeMonth` devuelve 0 «siempre hay algo que pintar»: sin el segundo
    // guard, el mes 100 aparecería dentro de una ventana que acaba en el 24.
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: [{ month_index: 100, net_worth: 999 }],
      deflator: NO_DEFLATION,
    })!;
    expect(fan.deterministic).toEqual([]);
  });

  it("aplica el MISMO deflactor por mes a las tres bandas y a la determinista", () => {
    const pct = 3;
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: (mi) => deflationFactorAt(mi, pct),
    })!;
    const f24 = deflationFactorAt(24, pct);
    const last = fan.band[fan.band.length - 1]!;
    expect(last.p10).toBeCloseTo(40 * f24, 9);
    expect(last.p50).toBeCloseTo(220 * f24, 9);
    expect(last.p90).toBeCloseTo(400 * f24, 9);
    const lastLine = fan.deterministic[fan.deterministic.length - 1]!;
    expect(lastLine.value).toBeCloseTo((100 + 24 * 5) * f24, 9);
    // El mes 0 no se mueve: el deflactor de hoy es 1 exacto.
    expect(fan.band[0]!.p50).toBeCloseTo(100, 12);
  });

  it("el deflactor NO separa la línea del abanico (sigue contenida en el mes en que lo estaba)", () => {
    // La determinista del mes 12 vale 160, exactamente el p50: deflactar solo la banda la
    // sacaría fuera. Con el mismo factor, la relación se conserva.
    for (const pct of [0, 3, -1]) {
      const fan = buildRiskFan({
        bandPoints: HYBRID_BAND,
        seriesPoints: MONTHLY_SERIES,
        deflator: (mi) => deflationFactorAt(mi, pct),
      })!;
      const b12 = fan.band.find((b) => b.month === 12)!;
      const d12 = fan.deterministic.find((d) => d.month === 12)!;
      expect(d12.value).toBeGreaterThanOrEqual(b12.p10);
      expect(d12.value).toBeLessThanOrEqual(b12.p90);
      expect(d12.value).toBeCloseTo(b12.p50, 9);
    }
  });

  it("el rango del eje Y cubre banda Y línea", () => {
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
    })!;
    expect(fan.valueMin).toBe(40);
    expect(fan.valueMax).toBe(400);
  });

  it("marca la jubilación solo si cae dentro de la ventana", () => {
    const inside = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
      retirementMonthIndex: 12,
    })!;
    expect(inside.retirementMonth).toBe(12);
    const outside = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
      retirementMonthIndex: 400,
    })!;
    expect(outside.retirementMonth).toBeNull();
    const none = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
      retirementMonthIndex: null,
    })!;
    expect(none.retirementMonth).toBeNull();
  });

  it("sin banda dibujable devuelve null (media banda no se pinta)", () => {
    expect(
      buildRiskFan({ bandPoints: [], seriesPoints: MONTHLY_SERIES, deflator: NO_DEFLATION }),
    ).toBeNull();
    expect(
      buildRiskFan({
        bandPoints: [bandPoint(0, 1, 1, 1)],
        seriesPoints: MONTHLY_SERIES,
        deflator: NO_DEFLATION,
      }),
    ).toBeNull();
  });

  it("sin serie determinista sigue dibujando el abanico", () => {
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: [],
      deflator: NO_DEFLATION,
    })!;
    expect(fan.band).toHaveLength(6);
    expect(fan.deterministic).toEqual([]);
  });
});

describe("semáforo de éxito", () => {
  it("traduce el veredicto del servidor, sin recalcularlo", () => {
    expect(successVerdictTone("green")).toBe("ok");
    expect(successVerdictTone("amber")).toBe("warn");
    expect(successVerdictTone("red")).toBe("danger");
  });

  it("un veredicto desconocido o ausente NO pinta alarma", () => {
    expect(successVerdictTone(null)).toBe("ok");
    expect(successVerdictTone(undefined)).toBe("ok");
    expect(successVerdictTone("teal")).toBe("ok");
  });

  it("«87 de cada 100 escenarios» sale de la fracción", () => {
    expect(formatSuccessScenarios("0.870000")).toBe("87 de cada 100 escenarios");
    expect(formatSuccessScenarios("1")).toBe("100 de cada 100 escenarios");
    expect(formatSuccessScenarios("0")).toBe("0 de cada 100 escenarios");
  });

  it("no redondea a 100 un plan que falla, ni a 0 uno que a veces sale", () => {
    expect(formatSuccessScenarios("0.999000")).toBe("99 de cada 100 escenarios");
    expect(formatSuccessScenarios("0.001000")).toBe("1 de cada 100 escenarios");
  });

  it("sin probabilidad es un guion, nunca un cero", () => {
    expect(formatSuccessScenarios(null)).toBe("—");
    expect(formatSuccessScenarios(undefined)).toBe("—");
  });

  it("el umbral se ecoa porque sin él el color no se puede auditar", () => {
    expect(formatSuccessThreshold(95)).toBe("umbral 95 %");
    expect(formatSuccessThreshold(null)).toBeUndefined();
  });
});

describe("tabla de agotamiento por edad", () => {
  const rows: DepletionProbabilityPointApi[] = [
    { month_index: 240, age: 70, probability: "0.0000" },
    { month_index: 300, age: 75, probability: "0.1060" },
    { month_index: 360, age: 80, probability: "0.2230" },
  ];

  it("rotula por edad y formatea la fracción como porcentaje", () => {
    const out = buildDepletionRows(rows);
    expect(out.map((r) => r.label)).toEqual(["a los 70", "a los 75", "a los 80"]);
    expect(out.map((r) => r.value)).toEqual(["0,0 %", "10,6 %", "22,3 %"]);
    expect(out.map((r) => r.key)).toEqual(["dep-240", "dep-300", "dep-360"]);
  });

  it("sin fecha de nacimiento la fila NO se esconde: se rotula por mes", () => {
    const out = buildDepletionRows([
      { month_index: 240, age: null, probability: "0.0500" },
    ]);
    expect(out).toEqual([{ key: "dep-240", label: "mes 240", value: "5,0 %" }]);
  });

  it("una probabilidad ausente es un guion, no un cero", () => {
    const out = buildDepletionRows([
      { month_index: 240, age: 70, probability: null },
    ]);
    expect(out[0]!.value).toBe("—");
  });

  it("sin jubilación en el horizonte la tabla va vacía", () => {
    expect(buildDepletionRows([])).toEqual([]);
    expect(buildDepletionRows(null)).toEqual([]);
    expect(buildDepletionRows(undefined)).toEqual([]);
  });
});

describe("aviso «sin volatilidad declarada»", () => {
  it("se dispara solo con `any_volatility_declared === false`", () => {
    expect(showsNoVolatilityNotice(bandsFixture({ any_volatility_declared: false }))).toBe(
      true,
    );
    expect(showsNoVolatilityNotice(bandsFixture({ any_volatility_declared: true }))).toBe(
      false,
    );
  });

  it("sin bandas no hay aviso (no se avisa de lo que no se ha pedido)", () => {
    expect(showsNoVolatilityNotice(null)).toBe(false);
    expect(showsNoVolatilityNotice(undefined)).toBe(false);
  });
});

describe("filas extra por estrategia", () => {
  const common = {
    currencyIso: "EUR",
    monthLabel: (mi: number) => `mes ${mi}`,
  };

  it("con trigger por cruce publica los percentiles del mes de jubilación", () => {
    const rows = buildRiskExtraRows({
      ...common,
      withdrawalRuleKind: "fixed_real",
      bands: bandsFixture({
        retirement_month_index_percentiles: { p10: 180, p50: 200, p90: null },
      }),
    });
    const row = rows.find((r) => r.key === "retirement_percentiles")!;
    expect(row.value).toBe("mes 200");
    expect(row.detail).toContain("mes 180");
    // Un `null` DENTRO del objeto es «ese percentil no se jubila nunca», no «no calculado».
    expect(row.detail).toContain("no se jubila");
  });

  it("con trigger por edad publica la probabilidad de no llegar, y no los percentiles", () => {
    const rows = buildRiskExtraRows({
      ...common,
      withdrawalRuleKind: "fixed_real",
      bands: bandsFixture({
        retirement_trigger: "target_age",
        retirement_month_index_percentiles: null,
        underfunded_probability: "0.3200",
      }),
    });
    expect(rows.find((r) => r.key === "retirement_percentiles")).toBeUndefined();
    expect(rows.find((r) => r.key === "underfunded_probability")!.value).toBe("32,0 %");
  });

  it("con `fixed_real` NO publica recorte: sus dos cifras son 0 y 1 por construcción", () => {
    const rows = buildRiskExtraRows({
      ...common,
      withdrawalRuleKind: "fixed_real",
      bands: bandsFixture({ months_below_need_p50: 0, withdrawal_to_need_ratio_p50: "1" }),
    });
    expect(rows.find((r) => r.key === "months_below_need")).toBeUndefined();
    expect(rows.find((r) => r.key === "withdrawal_to_need")).toBeUndefined();
  });

  it("con una regla con techo publica las dos lecturas del recorte", () => {
    const rows = buildRiskExtraRows({
      ...common,
      withdrawalRuleKind: "guardrails",
      bands: bandsFixture({
        months_below_need_p50: 14,
        withdrawal_to_need_ratio_p50: "0.9100",
      }),
    });
    expect(rows.find((r) => r.key === "months_below_need")!.value).toBe("14");
    expect(rows.find((r) => r.key === "withdrawal_to_need")!.value).toBe("91,0 %");
  });

  it("el colchón solo aparece si se SIMULÓ, y su importe se rotula como mediana", () => {
    expect(
      buildRiskExtraRows({
        ...common,
        withdrawalRuleKind: "fixed_real",
        bands: bandsFixture({ buffer_active: false, buffer_refills_p50: null }),
      }).find((r) => r.key === "buffer"),
    ).toBeUndefined();

    const row = buildRiskExtraRows({
      ...common,
      withdrawalRuleKind: "fixed_real",
      bands: bandsFixture({
        buffer_active: true,
        buffer_refills_p50: 7,
        buffer_refill_net_total_p50: "12500.0000",
      }),
    }).find((r) => r.key === "buffer")!;
    expect(row.value).toBe("7 meses con relleno");
    expect(row.detail).toContain("mediana");
    expect(row.detail).not.toContain("saldo actual");
  });

  it("sin bandas no hay filas", () => {
    expect(
      buildRiskExtraRows({ ...common, withdrawalRuleKind: "fixed_real", bands: null }),
    ).toEqual([]);
  });
});

describe("pie del panel", () => {
  it("declara coste, caminos y semilla como STRING", () => {
    const note = riskFootnote(bandsFixture());
    expect(note).toContain("55 ms");
    expect(note).toContain("500 caminos");
    // La semilla es un u64: si alguien la pasara por `Number` perdería dígitos y el sorteo
    // dejaría de reproducirse. El pie tiene que enseñarla entera.
    expect(note).toContain("12345678901234567890");
  });

  it("un HIT de cache se dice, no se disfraza de «0 ms»", () => {
    expect(riskFootnote(bandsFixture({ computed_in_ms: 0 }))).toContain(
      "Resultado en cache",
    );
  });
});

describe("KPI «Éxito del plan» del Resumen", () => {
  it("copia probabilidad, umbral y veredicto del plan", () => {
    const tile = summarySuccessTile({
      success_probability: "0.960000",
      success_threshold_pct: 95,
      success_verdict: "green",
      success_absent_reason: null,
      absent_reason: null,
    })!;
    expect(tile.value).toBe("96 de cada 100 escenarios");
    expect(tile.parenthetical).toBe("umbral 95 %");
    expect(tile.tone).toBe("default");
  });

  it("colorea los tres veredictos con el vocabulario de la app", () => {
    const tone = (v: string) =>
      summarySuccessTile({
        success_probability: "0.500000",
        success_threshold_pct: 95,
        success_verdict: v,
      })!.tone;
    expect(tone("green")).toBe("default");
    expect(tone("amber")).toBe("warn");
    expect(tone("red")).toBe("danger");
  });

  it("en Hogar es un guion CON su razón, no un hueco mudo", () => {
    const tile = summarySuccessTile({
      success_probability: null,
      success_threshold_pct: null,
      success_verdict: null,
      success_absent_reason: null,
      absent_reason: "household_aggregate",
    })!;
    expect(tile.value).toBe("—");
    expect(tile.detail).toContain("Yo");
    expect(tile.tone).toBe("default");
  });

  it("distingue «no sabemos tu probabilidad» de «no sabemos tu plan»", () => {
    const bands = summarySuccessTile({
      success_probability: null,
      success_threshold_pct: 95,
      success_absent_reason: "bands_unavailable",
      absent_reason: null,
    })!;
    const plan = summarySuccessTile({
      success_probability: null,
      success_absent_reason: null,
      absent_reason: "projection_unavailable",
    })!;
    expect(bands.detail).not.toBe(plan.detail);
    expect(bands.parenthetical).toBe("umbral 95 %");
  });

  it("sin bloque de éxito (backend antiguo) no se pinta tarjeta", () => {
    expect(summarySuccessTile(null)).toBeNull();
    expect(summarySuccessTile(undefined)).toBeNull();
    expect(summarySuccessTile({ absent_reason: null })).toBeNull();
  });
});
