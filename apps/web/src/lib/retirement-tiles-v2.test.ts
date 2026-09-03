/**
 * La cabecera de resultados de U7: **una frase + como mucho 3 tarjetas**, una cifra por tarjeta.
 *
 * Dos cosas se fijan aquí porque son decisiones, no accidentes de implementación:
 *
 *  1. **La regla de prioridad del tope de 3** — el orden de construcción ES la prioridad, el
 *     objetivo nunca se cae y el puente es siempre el primero en caerse. Sin test, el día que
 *     alguien reordene el `if` de la media jornada la cabecera cambiará de contenido sin que
 *     nada falle.
 *  2. **S8** — el puente mide `pension_start − jubilación`, no meses desde hoy. La V1 medía lo
 *     segundo y el número era plausible: 22 años donde el puente real son 12.
 */

import { describe, expect, it } from "vitest";
import {
  buildRetirementTilesV2,
  retirementDetailRows,
  RETIREMENT_TILES_V2_CAP,
  type RetirementTileV2Series,
  type RetirementTilesV2Input,
} from "./retirement-tiles";

const EUR = "EUR";
const monthLabel = (mi: number) => `M${mi}`;
/** Lo que emite `Intl` en es-ES: espacio DURO antes del símbolo y miles solo a partir de
 *  10.000. Se construye aquí para que el test no dependa de cómo se teclea un NBSP. */
const eur = (digits: string) => `${digits}\u00a0\u20ac`;

function series(over: Partial<RetirementTileV2Series> = {}): RetirementTileV2Series {
  return {
    strategy: "asap",
    required_contribution_monthly: null,
    required_contribution_search_ceiling: null,
    underfunded: null,
    disposable_monthly: null,
    disposable_capital_at_retirement: null,
    disposable_capital_today: null,
    coast_fire_month_index: null,
    coast_number: null,
    partial_gap_target: null,
    partial_phase_capital_growing: null,
    pension_start_month_index: null,
    bridge_effective_withdrawal_pct: null,
    pension_coverage_ratio: null,
    bridge_discount_annual_pct: null,
    warnings: [],
    jubilacion_month_index: null,
    jubilacion_age: null,
    jubilacion_target_net_worth: "600000.0000",
    jubilacion_target_net_worth_nominal: null,
    liquid_crossing_month_index: null,
    ...over,
  };
}

function input(
  over: Partial<RetirementTileV2Series> = {},
  rest: Partial<Omit<RetirementTilesV2Input, "series">> = {},
): RetirementTilesV2Input {
  return {
    series: series(over),
    currencyIso: EUR,
    monthLabel,
    targetRetirementAge: 55,
    targetBasis: "perpetuity",
    pensionStartAge: null,
    ...rest,
  };
}

const keys = (i: RetirementTilesV2Input) => buildRetirementTilesV2(i).map((t) => t.key);
const tile = (i: RetirementTilesV2Input, key: string) =>
  buildRetirementTilesV2(i).find((t) => t.key === key);

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("el tope de 3 y su prioridad", () => {
  it("el tope es 3 y NUNCA se pasa, con ninguna combinación", () => {
    expect(RETIREMENT_TILES_V2_CAP).toBe(3);
    const rich = input(
      {
        strategy: "partial",
        required_contribution_monthly: "1200.0000",
        required_contribution_search_ceiling: "1800.0000",
        disposable_monthly: "600.0000",
        disposable_capital_at_retirement: "90000.0000",
        partial_gap_target: "150000.0000",
        partial_phase_capital_growing: true,
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
      },
      { pensionStartAge: 72 },
    );
    expect(buildRetirementTilesV2(rich).length).toBeLessThanOrEqual(3);
  });

  it("«Objetivo» es SIEMPRE la primera y nunca se cae", () => {
    for (const strategy of ["asap", "retire_at_age", "coast", "partial", "pension_bridge"] as const) {
      expect(keys(input({ strategy }))[0], strategy).toBe("target");
    }
  });

  it("el puente es el último candidato: se cae antes que cualquier tarjeta de la estrategia", () => {
    const withBridge = input(
      {
        strategy: "retire_at_age",
        required_contribution_monthly: "1200.0000",
        disposable_monthly: "600.0000",
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
      },
      { pensionStartAge: 72 },
    );
    expect(keys(withBridge)).toEqual(["target", "required_contribution", "disposable"]);

    // Quitando el margen, el puente entra: no estaba «desactivado», estaba desplazado.
    const roomy = input(
      {
        strategy: "retire_at_age",
        required_contribution_monthly: "1200.0000",
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
      },
      { pensionStartAge: 72 },
    );
    expect(keys(roomy)).toEqual(["target", "required_contribution", "bridge"]);
  });

  it("«Media jornada» con solve de edad pierde el hueco Y el puente, en ese orden", () => {
    const solved = input(
      {
        strategy: "partial",
        required_contribution_monthly: "1200.0000",
        disposable_monthly: "600.0000",
        partial_gap_target: "150000.0000",
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
      },
      { pensionStartAge: 72 },
    );
    expect(keys(solved)).toEqual(["target", "required_contribution", "disposable"]);

    // Sin solve de edad, el hueco sube y el puente cabe.
    const unsolved = input(
      {
        strategy: "partial",
        partial_gap_target: "150000.0000",
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
      },
      { pensionStartAge: 72 },
    );
    expect(keys(unsolved)).toEqual(["target", "partial_gap", "bridge"]);
  });
});

describe("qué tarjetas trae cada estrategia", () => {
  it("«Cuanto antes» solo el objetivo (y el puente si hay pensión con fecha)", () => {
    expect(keys(input({ strategy: "asap" }))).toEqual(["target"]);
    expect(
      keys(
        input({
          strategy: "asap",
          jubilacion_month_index: 120,
          pension_start_month_index: 264,
        }),
      ),
    ).toEqual(["target", "bridge"]);
  });

  it("«A una edad fija»: ahorro necesario y margen", () => {
    expect(
      keys(
        input({
          strategy: "retire_at_age",
          required_contribution_monthly: "1200.0000",
          disposable_monthly: "600.0000",
        }),
      ),
    ).toEqual(["target", "required_contribution", "disposable"]);
  });

  it("«Coast FIRE»: mes coast y número coast — nunca ahorro necesario", () => {
    const m = keys(
      input({
        strategy: "coast",
        coast_fire_month_index: 84,
        coast_number: "310000.0000",
        disposable_monthly: "500.0000",
      }),
    );
    expect(m).toEqual(["target", "coast_month", "coast_number"]);
    expect(m).not.toContain("required_contribution");
    // El margen de coast existe en el servidor y NO cabe aquí: se lee en el Resumen.
    expect(m).not.toContain("disposable");
  });

  it("«Media jornada» sin solve: el hueco", () => {
    expect(keys(input({ strategy: "partial", partial_gap_target: "150000.0000" }))).toEqual([
      "target",
      "partial_gap",
    ]);
  });

  it("una cifra que el servidor NO publica no se pinta con guion: la tarjeta no existe", () => {
    // `required_contribution_monthly: null` ≠ 0 €: la estrategia degradó y no hay solve.
    expect(keys(input({ strategy: "retire_at_age" }))).toEqual(["target"]);
    expect(keys(input({ strategy: "partial" }))).toEqual(["target"]);
  });

  it("sin serie no hay tarjetas", () => {
    expect(buildRetirementTilesV2({ ...input(), series: null })).toEqual([]);
  });
});

describe("contenido de las tarjetas — una cifra y un subtítulo COMPLETO", () => {
  it("«Objetivo (euros de hoy)» lleva la base en el subtítulo", () => {
    const t = tile(input(), "target");
    expect(t?.label).toBe("Objetivo (euros de hoy)");
    expect(t?.value).toBe(eur("600.000"));
    expect(t?.subtitle).toBe("base: renta perpetua");
    expect(t?.helpId).toBe("retirement.target");
  });

  it("sin base declarada, la tarjeta va sin subtítulo (no se inventa una)", () => {
    expect(tile(input({}, { targetBasis: null }), "target")?.subtitle).toBeUndefined();
  });

  it("el objetivo NOMINAL «al cruce» ya NO comparte tarjeta: baja al Detalle", () => {
    const i = input({ jubilacion_target_net_worth_nominal: "1200000.0000" });
    expect(tile(i, "target")?.subtitle).toBe("base: renta perpetua");
    expect(retirementDetailRows(i).map((r) => r.key)).toContain("target_nominal");
  });

  it("«Ahorro necesario» en rojo cuando es TODO el sobrante y no basta", () => {
    const t = tile(
      input({
        strategy: "retire_at_age",
        required_contribution_monthly: "1800.0000",
        required_contribution_search_ceiling: "1800.0000",
        underfunded: true,
      }),
      "required_contribution",
    );
    expect(t?.value).toBe(eur("1800"));
    expect(t?.subtitle).toBe(`de ${eur("1800")}/mes de sobrante · es TODO tu sobrante y no basta`);
    expect(t?.tone).toBe("danger");
  });

  it("…y en tono normal cuando sí basta", () => {
    const t = tile(
      input({
        strategy: "retire_at_age",
        required_contribution_monthly: "1200.0000",
        required_contribution_search_ceiling: "1800.0000",
        underfunded: false,
      }),
      "required_contribution",
    );
    expect(t?.subtitle).toBe(`de ${eur("1800")}/mes de sobrante`);
    expect(t?.tone).toBe("default");
  });

  it("«Mes coast» no alcanzable dice por qué, sin guion mudo", () => {
    const t = tile(input({ strategy: "coast" }), "coast_month");
    expect(t?.value).toBe("No alcanzable");
    expect(t?.subtitle).toBe("ni aportando todos los meses llegas al objetivo en tu edad");
  });

  it("«Mes coast» alcanzable dice el plazo como TRAMO, y «ya» si es hoy", () => {
    expect(tile(input({ strategy: "coast", coast_fire_month_index: 84 }), "coast_month")
      ?.subtitle).toBe("dentro de 7 años");
    expect(tile(input({ strategy: "coast", coast_fire_month_index: 0 }), "coast_month")
      ?.subtitle).toBe("ya puedes dejar de aportar");
  });

  it("«Hueco de media jornada» se pone en rojo si el capital DECRECE", () => {
    const shrinking = tile(
      input({
        strategy: "partial",
        partial_gap_target: "150000.0000",
        partial_phase_capital_growing: false,
      }),
      "partial_gap",
    );
    expect(shrinking?.tone).toBe("danger");
    expect(shrinking?.subtitle).toContain("el capital DECRECE en media jornada");

    // `null` (no hubo fase que medir) no añade línea ni tiñe nada.
    const unknown = tile(
      input({ strategy: "partial", partial_gap_target: "150000.0000" }),
      "partial_gap",
    );
    expect(unknown?.tone).toBe("default");
    expect(unknown?.subtitle).toBe("capital que cubriría ese hueco a perpetuidad");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("la tarjeta de puente — S8", () => {
  const bridge = (
    over: Partial<RetirementTileV2Series> = {},
    rest: Partial<Omit<RetirementTilesV2Input, "series">> = {},
  ) =>
    tile(
      input(
        {
          strategy: "asap",
          jubilacion_month_index: 120,
          jubilacion_age: 60,
          pension_start_month_index: 264,
          bridge_effective_withdrawal_pct: "8.7000",
          pension_coverage_ratio: "0.9600",
          ...over,
        },
        { pensionStartAge: 72, ...rest },
      ),
      "bridge",
    );

  it("el rótulo y la cifra de U7: «Puente 60→72 · 12 años»", () => {
    const t = bridge();
    expect(t?.label).toBe("Puente 60→72");
    expect(t?.value).toBe("12 años");
  });

  it("mide el TRAMO jubilación→pensión, no los meses desde hoy", () => {
    // 264 − 120 = 144 meses = 12 años. Contando desde hoy serían 22, y sonaría igual de creíble.
    expect(bridge()?.value).toBe("12 años");
    expect(bridge()?.value).not.toBe("22 años");
  });

  it("el subtítulo lleva la tasa efectiva y la cobertura, con SUS unidades", () => {
    // `bridge_effective_withdrawal_pct` es un PORCENTAJE; `pension_coverage_ratio` una FRACCIÓN.
    expect(bridge()?.subtitle).toBe(
      "retiras el 8,7 % del capital al año · la pensión cubre el 96,0 % del gasto",
    );
  });

  it("la tasa de DESCUENTO no entra: es un supuesto, y vive en el Detalle", () => {
    const i = input(
      {
        jubilacion_month_index: 120,
        pension_start_month_index: 264,
        bridge_discount_annual_pct: "5.0000",
      },
      { pensionStartAge: 72 },
    );
    expect(tile(i, "bridge")?.subtitle ?? "").not.toContain("descontado");
    expect(retirementDetailRows(i).map((r) => r.key)).toContain("bridge_discount");
  });

  it("sin mes de jubilación NO hay puente que medir y la tarjeta no se emite", () => {
    expect(bridge({ jubilacion_month_index: null })).toBeUndefined();
  });

  it("sin pensión con fecha tampoco", () => {
    expect(bridge({ pension_start_month_index: null })).toBeUndefined();
  });

  it("una pensión anterior a la jubilación no es un puente negativo: es «Sin puente»", () => {
    const t = bridge({ jubilacion_month_index: 200, pension_start_month_index: 150 });
    expect(t?.value).toBe("Sin puente");
    expect(t?.subtitle).toContain("cobras la pensión desde el primer mes de jubilación");
  });

  it("sin edades resolubles, rótulo genérico en vez de un «Puente null→null»", () => {
    const t = bridge({ jubilacion_age: null }, { pensionStartAge: null, targetRetirementAge: null });
    expect(t?.label).toBe("Puente hasta la pensión");
  });

  it("la edad objetivo GUARDADA respalda a la calculada", () => {
    const t = bridge({ jubilacion_age: null }, { targetRetirementAge: 58 });
    expect(t?.label).toBe("Puente 58→72");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("retirementDetailRows — lo que la cabecera ya no lleva", () => {
  it("sin serie no hay filas", () => {
    expect(retirementDetailRows({ ...input(), series: null })).toEqual([]);
  });

  it("el objetivo nominal, con su rótulo propio", () => {
    const rows = retirementDetailRows(
      input({ jubilacion_target_net_worth_nominal: "1200000.0000" }),
    );
    const r = rows.find((x) => x.key === "target_nominal");
    expect(r?.label).toBe("Objetivo al cruce (euros de ese mes)");
    expect(r?.value).toBe(eur("1.200.000"));
  });

  it("el cruce puro SOLO cuando cae en un mes distinto de la jubilación efectiva", () => {
    const same = retirementDetailRows(
      input({ jubilacion_month_index: 120, liquid_crossing_month_index: 120 }),
    );
    expect(same.map((r) => r.key)).not.toContain("liquid_crossing");

    const diff = retirementDetailRows(
      input({ jubilacion_month_index: 240, liquid_crossing_month_index: 300 }),
    );
    const r = diff.find((x) => x.key === "liquid_crossing");
    expect(r?.label).toBe("Cruce del objetivo");
    expect(r?.value).toBe("M300");
  });

  it("el margen en dinero de hoy, el descuento y la cobertura, con sus unidades", () => {
    const rows = retirementDetailRows(
      input({
        disposable_capital_today: "45000.0000",
        bridge_discount_annual_pct: "5.0000",
        pension_coverage_ratio: "0.9600",
      }),
    );
    const by = Object.fromEntries(rows.map((r) => [r.key, r.value]));
    expect(by["disposable_today"]).toBe(eur("45.000"));
    expect(by["bridge_discount"]).toBe("5,0 %");
    expect(by["pension_coverage"]).toBe("96,0 %");
  });

  it("los avisos bajan aquí, con su tono y en su orden de precedencia", () => {
    const rows = retirementDetailRows(
      input({
        strategy: "retire_at_age",
        underfunded: true,
        warnings: ["coast_not_reachable", "retire_at_age_underfunded"],
      }),
    );
    const notices = rows.filter((r) => r.key.startsWith("notice:"));
    expect(notices.map((n) => n.key)).toEqual([
      "notice:retire_at_age_underfunded",
      "notice:coast_not_reachable",
    ]);
    expect(notices[0].tone).toBe("danger");
    expect(notices[1].tone).toBe("warn");
    expect(notices[0].value).toContain("55");
  });

  it("un campo ausente no produce una fila con guion", () => {
    expect(retirementDetailRows(input()).map((r) => r.key)).toEqual([]);
  });

  it("todas las claves son únicas (son keys de React)", () => {
    const rows = retirementDetailRows(
      input({
        jubilacion_month_index: 240,
        liquid_crossing_month_index: 300,
        jubilacion_target_net_worth_nominal: "1200000.0000",
        disposable_capital_today: "45000.0000",
        bridge_discount_annual_pct: "5.0000",
        pension_coverage_ratio: "0.9600",
        warnings: ["target_retirement_age_missing", "bridge_discount_clamped"],
      }),
    );
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
  });
});
