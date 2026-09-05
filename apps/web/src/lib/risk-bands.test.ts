/**
 * Tests de la sección «Riesgo» (5.0.0, D28 + pase de correcciones del motor, issue #207). Fijan
 * las cuatro cosas que aquí se rompen en silencio —la alineación por mes con dos densidades, la
 * deflactación de las CUATRO series a la vez, el redondeo de la probabilidad y el SUJETO de cada
 * cifra (éxito = jubilarse Y no agotar; cobertura = regla Y descubierto)— más las traducciones
 * de veredicto y de `null` a copy.
 */

import { describe, expect, it } from "vitest";
import type {
  DepletionProbabilityPointApi,
  ProjectionBandPointApi,
  ProjectionBandsApi,
} from "../api/types";
import {
  buildRiskExtraRows,
  cashBufferLine,
  buildRiskFan,
  formatScenariosPerHundred,
  formatSuccessPercent,
  riskFootnote,
  scenariosPerHundred,
  successParenthetical,
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

  // V1 — el valor del tile es un PORCENTAJE con un decimal, no una oración. La frase entera
  // («87 de cada 100 escenarios se jubilan y no agotan el capital») era correcta pero no cabía
  // en la tipografía del valor; la condición bajó al subtítulo, que sí envuelve.
  it("la cifra es «87,0 %»: un decimal, como todo porcentaje de la casa", () => {
    expect(formatSuccessPercent("0.870000")).toBe("87,0 %");
    expect(formatSuccessPercent("1")).toBe("100,0 %");
    expect(formatSuccessPercent("0")).toBe("0,0 %");
  });

  // El tope vive en `scenariosPerHundred`, y por eso el formateador pasa por ahí en vez de
  // llamar a `formatFractionAsPercent`: con el atajo, «0,9999» imprimiría «100,0 %» y —desde V7,
  // con el verde exclusivo del 100 %— pintaría de verde un plan que el servidor da por ámbar.
  it("no redondea a 100 un plan que falla, ni a 0 uno que a veces sale", () => {
    expect(formatSuccessPercent("0.999000")).toBe("99,0 %");
    expect(formatSuccessPercent("0.999900")).toBe("99,0 %");
    expect(formatSuccessPercent("0.001000")).toBe("1,0 %");
    expect(scenariosPerHundred("0.999000")).toBe(99);
    expect(scenariosPerHundred("0.001000")).toBe(1);
  });

  it("«de cada 100» sin sujeto conserva los mismos topes", () => {
    expect(formatScenariosPerHundred("0.040000")).toBe("4 de cada 100");
    expect(formatScenariosPerHundred("0.999000")).toBe("99 de cada 100");
    expect(formatScenariosPerHundred("0.001000")).toBe("1 de cada 100");
    expect(formatScenariosPerHundred(null)).toBe("—");
  });

  it("sin probabilidad es un guion, nunca un cero", () => {
    expect(formatSuccessPercent(null)).toBe("—");
    expect(formatSuccessPercent(undefined)).toBe("—");
    expect(scenariosPerHundred(null)).toBeNull();
    expect(scenariosPerHundred("no-es-un-numero")).toBeNull();
  });

  // V7 — el subtítulo ya no dice «umbral»: el corte dejó de ser del usuario. Lo que dice es el
  // SUJETO de la cifra, que sin él queda abierto.
  it("el subtítulo dice de QUÉ es ese porcentaje, sin hablar de umbrales", () => {
    expect(successParenthetical("0.870000")).toBe(
      "de los escenarios no agotan el capital",
    );
    expect(successParenthetical("0.870000")).not.toContain("umbral");
  });

  it("en verde el subtítulo es el recuento exacto, que es lo que hace auditable el 100 %", () => {
    expect(successParenthetical("1", 500)).toBe("0 de 500 escenarios agotan el capital");
    // Recuento con la tipografía española de la casa: `es-ES` no agrupa a cuatro dígitos
    // («2000») y sí a cinco («20.000»). Es un recuento, no un importe: nada de símbolo.
    expect(successParenthetical("1", 2000)).toBe("0 de 2000 escenarios agotan el capital");
    expect(successParenthetical("1", 20000)).toBe("0 de 20.000 escenarios agotan el capital");
  });

  it("sin tamaño de muestra (el Resumen no lo publica) cae a la frase genérica", () => {
    expect(successParenthetical("1")).toBe("de los escenarios no agotan el capital");
    expect(successParenthetical("1", null)).toBe("de los escenarios no agotan el capital");
    expect(successParenthetical("1", 0)).toBe("de los escenarios no agotan el capital");
  });

  it("un 99,99 % NO es verde y por tanto no presume de cero fallos", () => {
    expect(successParenthetical("0.999900", 500)).toBe(
      "de los escenarios no agotan el capital",
    );
  });

  it("sin probabilidad no hay subtítulo que inventar", () => {
    expect(successParenthetical(null, 500)).toBeUndefined();
    expect(successParenthetical(undefined)).toBeUndefined();
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
      bands: bandsFixture({
        retirement_trigger: "target_age",
        retirement_month_index_percentiles: null,
        underfunded_probability: "0.3200",
      }),
    });
    expect(rows.find((r) => r.key === "retirement_percentiles")).toBeUndefined();
    expect(rows.find((r) => r.key === "underfunded_probability")!.value).toBe("32,0 %");
  });

  // §F: las dos filas de cobertura ya no miden solo el recorte de la REGLA — incluyen el gasto
  // que la cartera no pudo financiar. Con `fixed_real` eso es justo el caso interesante (la regla
  // no recorta nunca, así que todo lo que se vea ahí es cartera), y esconderlas era esconder el
  // peor escenario posible. Este test es el que impide volver a esconderlas.
  it("publica la cobertura con TODAS las reglas, `fixed_real` incluida", () => {
    const rows = buildRiskExtraRows({
      ...common,
      bands: bandsFixture({
        months_below_need_p50: 31,
        withdrawal_to_need_ratio_p50: "0.086500",
      }),
    });
    expect(rows.find((r) => r.key === "months_below_need")!.value).toBe("31");
    expect(rows.find((r) => r.key === "withdrawal_to_need")!.value).toBe("8,6 %");
  });

  it("la cobertura declara sus DOS causas y cuelga de su propia ayuda", () => {
    const rows = buildRiskExtraRows({ ...common, bands: bandsFixture() });
    const months = rows.find((r) => r.key === "months_below_need")!;
    const ratio = rows.find((r) => r.key === "withdrawal_to_need")!;
    expect(months.helpId).toBe("retirement.coverage");
    // El rótulo ya no habla de «recorte»: el recorte es solo una de las dos causas.
    expect(months.label).not.toContain("recorte");
    expect(ratio.detail).toContain("la cartera no dio");
  });

  // §G: la mitad que `success_probability` ya no puede contar sola.
  it("con escenarios sin jubilar publica cuántos son y el éxito condicionado", () => {
    const rows = buildRiskExtraRows({
      ...common,
      bands: bandsFixture({
        success_probability: "0.629000",
        never_retired_probability: "0.331000",
        success_given_retired: "0.940000",
      }),
    });
    expect(rows.find((r) => r.key === "never_retired")!.value).toBe("33 de cada 100");
    expect(rows.find((r) => r.key === "success_given_retired")!.value).toBe("94,0 %");
    // Van las primeras: son la lectura del número grande que tienen justo encima.
    expect(rows[0]!.key).toBe("never_retired");
    expect(rows[1]!.key).toBe("success_given_retired");
  });

  it("sin escenarios sin jubilar no publica ninguna de las dos filas", () => {
    for (const p of ["0", null, undefined]) {
      const rows = buildRiskExtraRows({
        ...common,
        bands: bandsFixture({
          never_retired_probability: p,
          success_given_retired: "0.940000",
        }),
      });
      expect(rows.find((r) => r.key === "never_retired")).toBeUndefined();
      expect(rows.find((r) => r.key === "success_given_retired")).toBeUndefined();
    }
  });

  it("si NADIE se jubila no hay éxito condicionado: no hay denominador, no un guion", () => {
    const rows = buildRiskExtraRows({
      ...common,
      bands: bandsFixture({
        never_retired_probability: "1",
        success_given_retired: null,
      }),
    });
    expect(rows.find((r) => r.key === "never_retired")!.value).toBe("100 de cada 100");
    expect(rows.find((r) => r.key === "success_given_retired")).toBeUndefined();
  });

  it("el colchón simulado rotula su importe como mediana", () => {
    const row = buildRiskExtraRows({
      ...common,
      bands: bandsFixture({
        buffer_active: true,
        buffer_refills_p50: 7,
        buffer_refill_net_total_p50: "12500.0000",
      }),
    }).find((r) => r.key === "buffer")!;
    expect(row.value).toBe("7 meses con relleno");
    expect(row.detail).toContain("mediana");
    expect(row.detail).not.toContain("saldo actual");
    expect(row.helpId).toBe("retirement.cash_buffer");
  });

  // V6: el colchón que NO corrió ya no es una fila de detalle, es la línea informativa del
  // bloque «Riesgo» (`cashBufferLine`, más abajo). Aquí solo queda lo que el sorteo MIDIÓ.
  it("un colchón inactivo no deja fila de detalle: no hay nada que el sorteo midiera", () => {
    for (const reason of [
      "not_requested",
      "no_volatility",
      "no_safe_liquid_asset",
      "no_capped_rule",
      null,
      undefined,
    ]) {
      expect(
        buildRiskExtraRows({
          ...common,
          bands: bandsFixture({ buffer_active: false, buffer_inactive_reason: reason }),
        }).find((r) => r.key === "buffer"),
        String(reason),
      ).toBeUndefined();
    }
  });

  // V5: la tabla «agotar a los 65/70/…» se fue con el degradado de la banda, que dice lo mismo
  // con más resolución. Lo que el color NO puede decir es el TOTAL, porque su última parada cae
  // en el borde del plot y ahí no hay etiqueta — por eso esta fila.
  it("publica la ruina TOTAL: el último punto de la rejilla acumulada", () => {
    const rows = buildRiskExtraRows({
      ...common,
      bands: bandsFixture({
        depletion_probability_by_age: [
          { month_index: 240, age: 70, probability: "0.0000" },
          { month_index: 300, age: 75, probability: "0.1060" },
          { month_index: 360, age: 80, probability: "0.2230" },
        ],
      }),
    });
    const row = rows.find((r) => r.key === "depletion_total")!;
    expect(row.value).toBe("22,3 %");
    expect(row.helpId).toBe("retirement.depletion_by_age");
  });

  it("sin rejilla, o con el último punto sin probabilidad, no inventa un 0 %", () => {
    const total = (depletion_probability_by_age: DepletionProbabilityPointApi[]) =>
      buildRiskExtraRows({
        ...common,
        bands: bandsFixture({ depletion_probability_by_age }),
      }).find((r) => r.key === "depletion_total");

    expect(total([])).toBeUndefined();
    expect(total([{ month_index: 240, age: 70, probability: null }])).toBeUndefined();
    // Con probabilidad sí sale, aunque sea 0: un cero medido no es un hueco.
    expect(total([{ month_index: 240, age: 70, probability: "0.0000" }])!.value).toBe("0,0 %");
  });

  it("sin bandas no hay filas", () => {
    expect(buildRiskExtraRows({ ...common, bands: null })).toEqual([]);
  });
});

describe("línea informativa del colchón de caja (V6)", () => {
  const line = (over: Parameters<typeof bandsFixture>[0]) =>
    cashBufferLine(bandsFixture(over), "EUR");

  it("derivado del tope: dice el importe, de qué regla sale, el equivalente y el COSTE", () => {
    const l = line({
      buffer_active: true,
      buffer_source: "allocation_cap",
      buffer_target_amount: "6000.0000",
      buffer_months_effective: 4,
      buffer_source_asset_name: "Cuenta corriente",
    })!;
    // `es-ES` no agrupa a cuatro dígitos y separa el símbolo con un espacio DURO: «6000 €».
    expect(l.text).toMatch(/6000\s€/u);
    expect(l.text).toContain("«Cuenta corriente»");
    expect(l.text).toContain("≈ 4 meses");
    // El hallazgo P4 va DENTRO de la línea: un colchón que aparece solo y no dice su precio se
    // lee como seguridad gratis.
    expect(l.text).toContain("cuesta unos puntos de éxito");
    expect(l.linksToAllocationRules).toBe(true);
    expect(l.canResetToDerived).toBe(false);
  });

  it("sin equivalente en meses (gasto no positivo) la frase se aguanta sin inventarlo", () => {
    const l = line({
      buffer_active: true,
      buffer_source: "allocation_cap",
      buffer_target_amount: "6000.0000",
      buffer_months_effective: null,
      buffer_source_asset_name: "Cuenta corriente",
    })!;
    expect(l.text).not.toContain("meses de tu gasto");
    expect(l.text).toMatch(/6000\s€/u);
  });

  it("explícito: lo dice y ofrece SOLTARLO, que si no sería irreversible desde la pantalla", () => {
    const l = line({
      buffer_active: true,
      buffer_source: "explicit",
      buffer_months_effective: 6,
    })!;
    expect(l.text).toContain("6 meses");
    expect(l.text).toContain("fijados por API");
    expect(l.canResetToDerived).toBe(true);
    expect(l.linksToAllocationRules).toBe(false);
  });

  it("sin colchón, la razón dice qué habría que tocar para tenerlo", () => {
    const text = (buffer_inactive_reason: string) =>
      line({
        buffer_active: false,
        buffer_source: "none",
        buffer_inactive_reason,
      })?.text;
    expect(text("no_capped_rule")).toContain("ninguna regla de ahorro con tope");
    expect(text("cap_is_zero")).toContain("0 €");
    expect(text("no_safe_liquid_asset")).toContain("líquido sin volatilidad");
    expect(text("no_volatility")).toContain("no hay de qué protegerse");
  });

  it("una razón desconocida no se traduce a una frase inventada", () => {
    expect(
      line({ buffer_active: false, buffer_source: "none", buffer_inactive_reason: "nueva" }),
    ).toBeNull();
  });

  it("backend anterior a V6: solo habla cuando el colchón NO corrió", () => {
    expect(line({ buffer_active: true, buffer_source: undefined })).toBeNull();
    expect(
      line({
        buffer_active: false,
        buffer_source: undefined,
        buffer_inactive_reason: "no_volatility",
      })!.text,
    ).toContain("no hay de qué protegerse");
  });

  it("sin bandas no hay línea", () => {
    expect(cashBufferLine(null, "EUR")).toBeNull();
    expect(cashBufferLine(undefined, "EUR")).toBeNull();
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
  it("copia probabilidad y veredicto del plan, y el valor es el mismo «96,0 %» de Jubilación", () => {
    const tile = summarySuccessTile({
      success_probability: "0.960000",
      success_verdict: "green",
      success_absent_reason: null,
      absent_reason: null,
    })!;
    expect(tile.value).toBe("96,0 %");
    expect(tile.parenthetical).toBe("de los escenarios no agotan el capital");
    expect(tile.tone).toBe("default");
    // Sin escenarios sin jubilar no hay subtítulo: «0 de cada 100 no llegan» no añade nada.
    expect(tile.detail).toBeUndefined();
  });

  it("el subtítulo enseña los que no llegan a jubilarse, y solo cuando los hay", () => {
    const withNever = summarySuccessTile({
      success_probability: "0.629000",
      success_verdict: "red",
      never_retired_probability: "0.331000",
    })!;
    expect(withNever.detail).toBe("33 de cada 100 no llegan a jubilarse");

    for (const p of ["0", null, undefined]) {
      expect(
        summarySuccessTile({
          success_probability: "0.960000",
          success_verdict: "green",
          never_retired_probability: p,
        })!.detail,
      ).toBeUndefined();
    }
  });

  it("colorea los tres veredictos con el vocabulario de la app", () => {
    const tone = (v: string) =>
      summarySuccessTile({
        success_probability: "0.500000",
        success_verdict: v,
      })!.tone;
    expect(tone("green")).toBe("default");
    expect(tone("amber")).toBe("warn");
    expect(tone("red")).toBe("danger");
  });

  it("en Hogar es un guion CON su razón, no un hueco mudo", () => {
    const tile = summarySuccessTile({
      success_probability: null,
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
      success_absent_reason: "bands_unavailable",
      absent_reason: null,
    })!;
    const plan = summarySuccessTile({
      success_probability: null,
      success_absent_reason: null,
      absent_reason: "projection_unavailable",
    })!;
    expect(bands.detail).not.toBe(plan.detail);
    // Sin cifra no hay sujeto que subtitular: el slot queda vacío en vez de repetir una frase
    // sobre unos escenarios que no se sortearon.
    expect(bands.parenthetical).toBeUndefined();
  });

  it("sin bloque de éxito (backend antiguo) no se pinta tarjeta", () => {
    expect(summarySuccessTile(null)).toBeNull();
    expect(summarySuccessTile(undefined)).toBeNull();
    expect(summarySuccessTile({ absent_reason: null })).toBeNull();
  });
});
