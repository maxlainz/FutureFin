/**
 * Tests de la sección «Riesgo» (5.0.0, modelo v2 §2.4/§2.5, issue #207). Fijan las cuatro cosas
 * que aquí se rompen en silencio —la alineación por mes con dos densidades, la deflactación de las
 * CUATRO series a la vez, el redondeo de la probabilidad y el SUJETO de cada cifra (éxito =
 * aguantar las tres formas de fallo; cobertura = regla Y descubierto)— más las traducciones de
 * veredicto, de umbral y de `null` a copy.
 *
 * Y una quinta que el modelo v2 estrena: **«pp» no es «%»**. El semiancho de Wilson viaja en
 * puntos porcentuales y tiene formateador propio; pasarlo por el de porcentajes daría un número
 * plausible y falso.
 */

import { describe, expect, it } from "vitest";
import type {
  FailureProbabilityPointApi,
  ProjectionBandPointApi,
  ProjectionBandsApi,
} from "../api/types";
import {
  buildRiskExtraRows,
  buildRiskFan,
  formatSamplingErrorPp,
  formatScenariosPerHundred,
  formatSuccessPercent,
  riskFootnote,
  scenariosPerHundred,
  successParenthetical,
  showsNoVolatilityNotice,
  showsRiskGradient,
  successVerdictTone,
  summarySuccessTile,
} from "./risk-bands";

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
    paths: 2500,
    seed: "12345678901234567890",
    percentiles: [10, 50, 90],
    points: HYBRID_BAND,
    success_of_plan: 0.87,
    success_threshold_pct: 95,
    success_wilson_low: 0.856,
    success_sampling_error_pp: "1.2000",
    failures_by_kind: [0, 0, 0],
    success_verdict: "amber",
    success_absent_reason: null,
    failure_probability_by_age: [],
    months_below_need_p50: 0,
    withdrawal_to_need_ratio_p50: null,
    any_volatility_declared: true,
    strategy: "asap",
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

  it("deflacta las CUATRO series con el mismo factor por mes", () => {
    // Deflactar solo la banda (o solo la línea) las separa y el abanico deja de contener a la
    // línea, que es la lectura que el chart promete.
    const half = () => 0.5;
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: half,
    })!;
    expect(fan.band[0]!.p10).toBe(50);
    expect(fan.band[0]!.p50).toBe(50);
    expect(fan.band[0]!.p90).toBe(50);
    expect(fan.deterministic[0]!.value).toBe(50);
  });

  it("el rango de valores cubre banda Y línea", () => {
    const fan = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
    })!;
    expect(fan.valueMin).toBe(40);
    expect(fan.valueMax).toBe(400);
  });

  it("el marcador de jubilación solo se pinta si cae dentro de la ventana", () => {
    const inside = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
      retirementMonthIndex: 12,
    })!;
    expect(inside.retirementMonth).toBe(12);

    const none = buildRiskFan({
      bandPoints: HYBRID_BAND,
      seriesPoints: MONTHLY_SERIES,
      deflator: NO_DEFLATION,
      retirementMonthIndex: 300,
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

  // V1 — el valor del tile es un PORCENTAJE con un decimal, no una oración.
  it("la cifra es «87,0 %»: un decimal, como todo porcentaje de la casa", () => {
    expect(formatSuccessPercent("0.870000")).toBe("87,0 %");
    expect(formatSuccessPercent("1")).toBe("100,0 %");
    expect(formatSuccessPercent("0")).toBe("0,0 %");
  });

  it("acepta la FRACCIÓN como número, que es como viaja en el bloque «plan»", () => {
    // `success_of_plan` es `number` en la serie y en las bandas (excepción chart-only), no un
    // Decimal-string: si el formateador solo entendiera strings, el KPI saldría con guion.
    expect(formatSuccessPercent(0.87)).toBe("87,0 %");
    expect(formatSuccessPercent(1)).toBe("100,0 %");
    expect(formatSuccessPercent(0)).toBe("0,0 %");
    expect(scenariosPerHundred(0.331)).toBe(33);
  });

  // El tope vive en `scenariosPerHundred`, y por eso el formateador pasa por ahí en vez de
  // llamar a `formatFractionAsPercent`: con el atajo, «0,9999» imprimiría «100,0 %» sobre un plan
  // que falla, y con umbral 100 el servidor lo da por rojo.
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

  // C3 — el umbral VUELVE al subtítulo (V7 lo había quitado): en el modelo v2 es del usuario y es
  // lo que DEFINE la fecha. Sin él, el mismo 95,0 % es «justo lo que pedí» para uno y «cinco
  // puntos de más» para otro, y la tarjeta no distingue los dos casos.
  it("el subtítulo dice de QUÉ es el porcentaje y contra qué listón", () => {
    expect(successParenthetical("0.870000", 95)).toBe(
      "de los escenarios aguantan · umbral 95,0 %",
    );
  });

  it("el verbo es «aguantar»: el éxito ya no es solo «no agotar el capital»", () => {
    // §2.4: un camino falla por cartera vacía (F1), por tasa inicial (F2) o por regla corta (F3).
    // Nombrar solo la primera describiría un tercio del contrato.
    const s = successParenthetical("0.870000", 95)!;
    expect(s).toContain("aguantan");
    expect(s).not.toContain("agotan");
  });

  it("sin umbral conocido el subtítulo no se lo inventa", () => {
    expect(successParenthetical("0.870000")).toBe("de los escenarios aguantan");
    expect(successParenthetical("0.870000", null)).toBe("de los escenarios aguantan");
  });

  // §2.5 / S5 — «100 %» es «0 fallos de N», y eso NO es certeza. La regla de tres publica la cota
  // del riesgo real y es la condición con la que el owner aceptó el umbral 100.
  it("en el 100 % publica el recuento exacto Y la cota de la regla de tres", () => {
    expect(successParenthetical("1", 100, 2500)).toBe(
      "0 de 2500 escenarios fallan · el riesgo real puede llegar al 0,12 % · umbral 100,0 %",
    );
    // Recuento con la tipografía española de la casa: `es-ES` no agrupa a cuatro dígitos («2500»)
    // y sí a cinco («20.000»). Es un recuento, no un importe: nada de símbolo.
    expect(successParenthetical("1", null, 20000)).toContain("0 de 20.000 escenarios fallan");
  });

  it("la cota de la regla de tres es 3/N, y no se redondea hasta desaparecer", () => {
    // Un solo decimal imprimiría «0,1 %» donde el número es 0,12 y «0,0 %» con N grande — un
    // riesgo cero que es justo lo que esta frase existe para negar.
    expect(successParenthetical("1", null, 2500)).toContain("0,12 %");
    expect(successParenthetical("1", null, 500)).toContain("0,6 %");
    expect(successParenthetical("1", null, 10000)).toContain("0,03 %");
  });

  it("sin tamaño de muestra (el Resumen no lo publica) cae a la frase genérica", () => {
    expect(successParenthetical("1", 95)).toBe("de los escenarios aguantan · umbral 95,0 %");
    expect(successParenthetical("1", 95, null)).toBe(
      "de los escenarios aguantan · umbral 95,0 %",
    );
    expect(successParenthetical("1", 95, 0)).toBe("de los escenarios aguantan · umbral 95,0 %");
  });

  it("un 99,99 % NO es el 100 % y por tanto no presume de cero fallos", () => {
    expect(successParenthetical("0.999900", 95, 2500)).toBe(
      "de los escenarios aguantan · umbral 95,0 %",
    );
  });

  it("sin probabilidad no hay subtítulo que inventar", () => {
    expect(successParenthetical(null, 95, 2500)).toBeUndefined();
    expect(successParenthetical(undefined)).toBeUndefined();
  });
});

describe("formatSamplingErrorPp — «pp» no es «%»", () => {
  it("un decimal y el sufijo «pp», que pone la función", () => {
    expect(formatSamplingErrorPp("0.2000")).toBe("±0,2 pp");
    expect(formatSamplingErrorPp("12.0000")).toBe("±12,0 pp");
    expect(formatSamplingErrorPp("1.2000")).toBe("±1,2 pp");
  });

  it("el sufijo NO es «%»: ±1,2 pp sobre un 95,0 % es 93,8–96,2, no ±1,14 puntos", () => {
    const s = formatSamplingErrorPp("1.2000");
    expect(s).toContain("pp");
    expect(s).not.toContain("%");
  });

  it("acepta el número además del Decimal-string", () => {
    expect(formatSamplingErrorPp(1.2)).toBe("±1,2 pp");
  });

  it("el signo es «±» y no depende del signo del dato", () => {
    // Es un SEMIANCHO: un negativo sería un error del servidor, pero pintar «±−1,2 pp» sería peor.
    expect(formatSamplingErrorPp("-1.2000")).toBe("±1,2 pp");
  });

  it("sin precisión publicada es un guion, nunca «±0,0 pp» (eso afirmaría muestra infinita)", () => {
    expect(formatSamplingErrorPp(null)).toBe("—");
    expect(formatSamplingErrorPp(undefined)).toBe("—");
    expect(formatSamplingErrorPp("no-es-un-numero")).toBe("—");
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

// A12 — los dos vetos al degradado. Los dos existen porque su caso tiñe la banda de VERDE ENTERO
// (`failure_probability_by_age` en ceros) sobre un sorteo que no midió riesgo, y el verde es
// justamente el color que nadie va a cuestionar.
describe("permiso para teñir la banda de riesgo", () => {
  it("con volatilidad declarada y éxito medido, la banda se colorea", () => {
    expect(showsRiskGradient(bandsFixture())).toBe(true);
  });

  it("sin volatilidad declarada no se colorea: un abanico de ancho cero no mide riesgo", () => {
    expect(showsRiskGradient(bandsFixture({ any_volatility_declared: false }))).toBe(false);
  });

  it("sin fecha de jubilación no se colorea: sus ceros no son «riesgo cero»", () => {
    expect(
      showsRiskGradient(
        bandsFixture({
          success_absent_reason: "not_reachable",
          success_of_plan: null,
          success_wilson_low: null,
          success_sampling_error_pp: null,
          success_verdict: null,
          failures_by_kind: [0, 0, 0],
          failure_probability_by_age: [
            { month_index: 599, age: 85, probability: 0, by_kind: [0, 0, 0] },
          ],
        }),
      ),
    ).toBe(false);
    // La otra razón alcanzable por esta ruta veta igual: sin plan tampoco hay riesgo que teñir.
    expect(
      showsRiskGradient(bandsFixture({ success_absent_reason: "birth_date_missing" })),
    ).toBe(false);
  });

  it("sin bandas no se colorea nada", () => {
    expect(showsRiskGradient(null)).toBe(false);
    expect(showsRiskGradient(undefined)).toBe(false);
  });
});

describe("filas que hacen auditable el éxito", () => {
  const rowsOf = (over: Partial<ProjectionBandsApi> = {}) =>
    buildRiskExtraRows({ bands: bandsFixture(over) });
  const byKey = (over: Partial<ProjectionBandsApi> = {}) =>
    Object.fromEntries(rowsOf(over).map((r) => [r.key, r]));

  // §2.4 — por qué falla el que falla. Sin esto, el éxito es un número sin causa.
  it("desglosa los fallos por tipo, con el orden fijo F1/F2/F3 y su denominador", () => {
    const rows = byKey({ failures_by_kind: [120, 30, 5], paths: 2500 });
    expect(rows["failure_kind_1"]!.label).toBe("Se quedan sin dinero");
    expect(rows["failure_kind_1"]!.value).toBe("120 de 2500");
    expect(rows["failure_kind_2"]!.label).toBe("Tasa inicial por encima del tope");
    expect(rows["failure_kind_2"]!.value).toBe("30 de 2500");
    expect(rows["failure_kind_3"]!.label).toBe("La regla no cubre el gasto");
    expect(rows["failure_kind_3"]!.value).toBe("5 de 2500");
  });

  it("con fallos, las TRES casillas salen aunque alguna sea cero: el desglose tiene que sumar", () => {
    const rows = byKey({ failures_by_kind: [120, 0, 0], paths: 2500 });
    expect(rows["failure_kind_2"]!.value).toBe("0 de 2500");
    expect(rows["failure_kind_3"]!.value).toBe("0 de 2500");
  });

  it("sin ningún fallo no hay desglose: tres «0 de 2500» son ruido, no información", () => {
    const rows = byKey({ failures_by_kind: [0, 0, 0] });
    expect(rows["failure_kind_1"]).toBeUndefined();
    expect(rows["failure_kind_2"]).toBeUndefined();
    expect(rows["failure_kind_3"]).toBeUndefined();
  });

  it("un fallo por tasa inicial (F2) es visible aunque nadie agote la cartera", () => {
    // El caso que el modelo v1 no sabía contar: el dinero sigue ahí y el plan ha fallado igual.
    const rows = byKey({ failures_by_kind: [0, 400, 0], paths: 2500 });
    expect(rows["failure_kind_2"]!.value).toBe("400 de 2500");
  });

  // §F: las dos filas de cobertura ya no miden solo el recorte de la REGLA — incluyen el gasto que
  // la cartera no pudo financiar. Con `fixed_real` eso es justo el caso interesante (la regla no
  // recorta nunca, así que todo lo que se vea ahí es cartera), y esconderlas era esconder el peor
  // escenario posible. Este test es el que impide volver a esconderlas.
  it("publica la cobertura con TODAS las reglas, `fixed_real` incluida", () => {
    const rows = byKey({
      months_below_need_p50: 31,
      withdrawal_to_need_ratio_p50: "0.086500",
    });
    expect(rows["months_below_need"]!.value).toBe("31");
    expect(rows["withdrawal_to_need"]!.value).toBe("8,6 %");
  });

  it("la cobertura declara sus DOS causas y cuelga de su propia ayuda", () => {
    const rows = byKey();
    expect(rows["months_below_need"]!.helpId).toBe("retirement.coverage");
    // El rótulo ya no habla de «recorte»: el recorte es solo una de las dos causas.
    expect(rows["months_below_need"]!.label).not.toContain("recorte");
    expect(rows["withdrawal_to_need"]!.label).toBe(
      "Parte del gasto que la regla cubrió (mediana)",
    );
    expect(rows["withdrawal_to_need"]!.detail).toContain("la cartera no dio");
  });

  // V5: la tabla «agotar a los 65/70/…» se fue con el degradado de la banda, que dice lo mismo con
  // más resolución. Lo que el color NO puede decir es el TOTAL, porque su última parada cae en el
  // borde del plot y ahí no hay etiqueta — por eso esta fila.
  it("publica el fallo TOTAL: el último punto de la rejilla acumulada", () => {
    const rows = byKey({
      failure_probability_by_age: [
        { month_index: 240, age: 70, probability: 0, by_kind: [0, 0, 0] },
        { month_index: 300, age: 75, probability: 0.106, by_kind: [200, 30, 35] },
        { month_index: 360, age: 80, probability: 0.223, by_kind: [400, 60, 97] },
      ],
    });
    expect(rows["failure_total"]!.value).toBe("22,3 %");
    expect(rows["failure_total"]!.label).toBe("Escenarios que fallan en algún momento");
    expect(rows["failure_total"]!.helpId).toBe("retirement.failure_by_age");
  });

  it("sin rejilla, o con el último punto sin probabilidad, no inventa un 0 %", () => {
    const total = (failure_probability_by_age: FailureProbabilityPointApi[]) =>
      byKey({ failure_probability_by_age })["failure_total"];

    expect(total([])).toBeUndefined();
    expect(
      total([{ month_index: 240, age: 70, probability: null, by_kind: null }]),
    ).toBeUndefined();
    // Con probabilidad sí sale, aunque sea 0: un cero medido no es un hueco.
    expect(
      total([{ month_index: 240, age: 70, probability: 0, by_kind: [0, 0, 0] }])!.value,
    ).toBe("0,0 %");
  });

  // C3 — la magnitud que de verdad decide el umbral. Enseñar solo el estimador puntual invita a
  // leer un 95,0 % como un hecho cuando con 2.500 caminos puede ser un 85,6 %.
  it("publica el límite inferior de Wilson, que es lo que se compara con el umbral", () => {
    const row = byKey()["success_wilson_low"]!;
    expect(row.label).toBe("Con 95 % de confianza, al menos");
    expect(row.value).toBe("85,6 %");
    expect(row.detail).toContain("se compara con tu umbral");
    expect(row.helpId).toBe("retirement.success_threshold");
  });

  it("sin Wilson publicado no hay fila (un guion diría que el intervalo salió vacío)", () => {
    expect(
      byKey({ success_wilson_low: null as unknown as number })["success_wilson_low"],
    ).toBeUndefined();
  });

  it("sin bandas no hay filas", () => {
    expect(buildRiskExtraRows({ bands: null })).toEqual([]);
    expect(buildRiskExtraRows({ bands: undefined })).toEqual([]);
  });

  // A12 — el caso que este bloque existe para no contar mal. El servidor sigue publicando
  // `failures_by_kind`, la rejilla acumulada y las dos de cobertura (describen la trayectoria SIN
  // jubilarse), pero valen 0: pintadas como siempre dirían «cero fallos, cobertura entera, 0 %
  // fallan», o sea un plan impecable que no existe.
  describe("sin fecha de jubilación no hay éxito que auditar", () => {
    const noDate = (over: Partial<ProjectionBandsApi> = {}) =>
      buildRiskExtraRows({
        bands: bandsFixture({
          success_absent_reason: "not_reachable",
          success_of_plan: null,
          success_wilson_low: null,
          success_sampling_error_pp: null,
          success_verdict: null,
          // Exactamente lo que llega por el cable en ese caso: ceros de un sorteo que no pudo
          // clasificar ni un fallo, más una rejilla de una sola fila valiendo 0.
          failures_by_kind: [0, 0, 0],
          failure_probability_by_age: [
            { month_index: 599, age: 85, probability: 0, by_kind: [0, 0, 0] },
          ],
          months_below_need_p50: 0,
          withdrawal_to_need_ratio_p50: "1",
          ...over,
        }),
      });

    it("devuelve UNA sola fila, la del motivo, y ninguna cifra de riesgo", () => {
      const rows = noDate();
      expect(rows).toHaveLength(1);
      expect(rows[0]!.key).toBe("success_absent");
      expect(rows[0]!.label).toBe("Sin éxito que auditar");
      expect(rows[0]!.detail).toContain("no hay ninguna fecha que llegue a tu umbral");
      // Ni un porcentaje, ni un recuento: el valor es un guion CON su motivo al lado.
      expect(rows[0]!.value).toBe("—");
      expect(rows.some((r) => /%/.test(r.value))).toBe(false);
    });

    it("ninguna de las filas de riesgo se cuela con sus ceros", () => {
      const keys = noDate().map((r) => r.key);
      for (const gone of [
        "failure_kind_1",
        "failure_kind_2",
        "failure_kind_3",
        "months_below_need",
        "withdrawal_to_need",
        "failure_total",
        "success_wilson_low",
      ]) {
        expect(keys, gone).not.toContain(gone);
      }
    });

    it("con fallos contados de un plan sin fecha tampoco los desglosa: son de otra pregunta", () => {
      // Defensivo: aunque el sorteo trajera contadores, sin fecha no clasifican el plan del
      // usuario y desglosarlos daría una causa a un fracaso que no es el suyo.
      expect(noDate({ failures_by_kind: [120, 30, 5] })).toHaveLength(1);
    });

    it("la otra razón alcanzable (sin fecha de nacimiento) tiene su propia frase", () => {
      const rows = noDate({ success_absent_reason: "birth_date_missing" });
      expect(rows[0]!.detail).toBe("falta tu fecha de nacimiento");
    });

    it("una razón futura no se traduce a una frase inventada", () => {
      const rows = noDate({ success_absent_reason: "algo_nuevo" as never });
      expect(rows[0]!.detail).toBe("no disponible");
    });
  });

  it("todas las claves son únicas (son keys de React)", () => {
    const rows = rowsOf({
      failures_by_kind: [120, 30, 5],
      failure_probability_by_age: [
        { month_index: 240, age: 70, probability: 0.1, by_kind: [100, 30, 20] },
      ],
    });
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
  });
});

describe("pie del panel", () => {
  it("declara coste, caminos y semilla como STRING", () => {
    const note = riskFootnote(bandsFixture());
    expect(note).toContain("55 ms");
    expect(note).toContain("2500 caminos");
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
  it("copia éxito, umbral y veredicto del plan, sin recalcular nada", () => {
    const tile = summarySuccessTile({
      plan_state: "ready",
      success_of_plan: 0.96,
      success_threshold_pct: 95,
      success_verdict: "green",
      success_absent_reason: null,
      absent_reason: null,
    })!;
    expect(tile.value).toBe("96,0 %");
    expect(tile.parenthetical).toBe("de los escenarios aguantan · umbral 95,0 %");
    expect(tile.tone).toBe("default");
    expect(tile.detail).toBeUndefined();
  });

  // El estado que un guion NO puede contar: no es que no haya éxito, es que se está calculando.
  it("`pending` dice «calculando…», que no es lo mismo que un hueco", () => {
    const tile = summarySuccessTile({
      plan_state: "pending",
      success_of_plan: null,
      success_threshold_pct: null,
      absent_reason: null,
    })!;
    expect(tile.value).toBe("—");
    expect(tile.detail).toBe("calculando…");
    expect(tile.tone).toBe("default");
  });

  it("`pending` gana aunque llegue con una cifra vieja pegada", () => {
    // Un backend que reenvíe la cifra anterior mientras resuelve no debe pintarla como actual.
    const tile = summarySuccessTile({
      plan_state: "pending",
      success_of_plan: 0.96,
      success_threshold_pct: 95,
    })!;
    expect(tile.value).toBe("—");
    expect(tile.detail).toBe("calculando…");
  });

  it("colorea los tres veredictos con el vocabulario de la app", () => {
    const tone = (v: string) =>
      summarySuccessTile({
        plan_state: "ready",
        success_of_plan: 0.5,
        success_threshold_pct: 95,
        success_verdict: v as never,
      })!.tone;
    expect(tone("green")).toBe("default");
    expect(tone("amber")).toBe("warn");
    expect(tone("red")).toBe("danger");
  });

  it("en Hogar es un guion CON su razón, no un hueco mudo", () => {
    const tile = summarySuccessTile({
      plan_state: "absent",
      success_of_plan: null,
      success_verdict: null,
      success_absent_reason: null,
      absent_reason: "household_aggregate",
    })!;
    expect(tile.value).toBe("—");
    expect(tile.detail).toContain("Yo");
    expect(tile.tone).toBe("default");
  });

  it("sin fecha de nacimiento lo dice: sin edad no hay contra qué resolver nada (C5)", () => {
    const tile = summarySuccessTile({
      plan_state: "absent",
      success_of_plan: null,
      absent_reason: "birth_date_missing",
    })!;
    expect(tile.detail).toBe("falta tu fecha de nacimiento");
  });

  it("distingue «no sabemos tu probabilidad» de «no sabemos tu plan»", () => {
    const bands = summarySuccessTile({
      plan_state: "ready",
      success_of_plan: null,
      success_absent_reason: "bands_unavailable",
      absent_reason: null,
    })!;
    const plan = summarySuccessTile({
      plan_state: "absent",
      success_of_plan: null,
      success_absent_reason: null,
      absent_reason: "projection_unavailable",
    })!;
    expect(bands.detail).not.toBe(plan.detail);
    // Sin cifra no hay sujeto que subtitular: el slot queda vacío en vez de repetir una frase
    // sobre unos escenarios que no se sortearon.
    expect(bands.parenthetical).toBeUndefined();
  });

  it("una razón desconocida no se traduce a una frase inventada", () => {
    const tile = summarySuccessTile({
      plan_state: "absent",
      success_of_plan: null,
      absent_reason: "algo_nuevo",
    })!;
    expect(tile.detail).toBe("no disponible");
  });

  // A12 — el plan SÍ está resuelto (`plan_state: "ready"`: hay estrategia, hay capital necesario);
  // lo que no hay es fecha, y por tanto no hay «éxito en su fecha». La serie publica ahí un
  // `success_of_plan` que mide otra cosa —la mejor observación del solve—, así que el Resumen lo
  // recibe a `null` y esta tarjeta tiene que quedarse sin cifra Y sin color.
  it("`not_reachable`: sin porcentaje, sin color y con el motivo escrito", () => {
    const tile = summarySuccessTile({
      plan_state: "ready",
      success_of_plan: null,
      success_threshold_pct: null,
      success_verdict: null,
      success_absent_reason: "not_reachable",
      absent_reason: null,
    })!;
    expect(tile.value).toBe("—");
    expect(tile.value).not.toMatch(/%/);
    expect(tile.detail).toBe(
      "no hay ninguna fecha que llegue a tu umbral, así que no hay éxito que medir",
    );
    // Ni alarma ni calma: nada se ha roto (no ha llegado a empezar) y nada aguanta.
    expect(tile.tone).toBe("default");
    // Sin cifra no hay sujeto que subtitular en el paréntesis.
    expect(tile.parenthetical).toBeUndefined();
  });

  // La conflación que originó el bug: `no_liquid_assets` nunca fue una razón de la ausencia del
  // ÉXITO —vive en `needed_capital_absent_reason`, que dice por qué falta una CIFRA—, así que
  // traducirlo aquí prometía una frase que ningún backend podía disparar.
  it("`no_liquid_assets` ya no se traduce: no es una razón de este campo", () => {
    const tile = summarySuccessTile({
      plan_state: "absent",
      success_of_plan: null,
      absent_reason: "no_liquid_assets",
    })!;
    expect(tile.detail).toBe("no disponible");
    expect(tile.detail).not.toContain("líquidos");
  });

  it("sin bloque de plan (backend antiguo) no se pinta tarjeta", () => {
    expect(summarySuccessTile(null)).toBeNull();
    expect(summarySuccessTile(undefined)).toBeNull();
    expect(summarySuccessTile({ absent_reason: null })).toBeNull();
  });
});
