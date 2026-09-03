/**
 * La tabla §C del plan de #207, fijada: qué tarjetas enseña CADA estrategia y qué avisos salen
 * de `warnings[]`. Sin este test la correspondencia vive solo en un `if` dentro de una vista de
 * 1.700 líneas, y el fallo que produce no rompe nada: enseña un KPI que la estrategia elegida no
 * responde, con un guion que se lee como «esto se calcula y hoy falta el dato».
 *
 * Se comprueban además las dos cosas que el contrato del servidor pide y que un render descuidado
 * colapsa: `null` ≠ 0 (un margen ausente no es un margen de cero euros) y las DOS bases de
 * `disposable_monthly`, que con la misma cifra significan cosas distintas.
 */

import { describe, expect, it } from "vitest";
import {
  buildRetirementTiles,
  type RetirementTileSeries,
} from "./retirement-tiles";

const EUR = "EUR";
/** Rotulador de meses inyectado: el módulo no sabe si el eje va en fechas o en edades. */
const monthLabel = (mi: number) => `M${mi}`;

function series(over: Partial<RetirementTileSeries>): RetirementTileSeries {
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
    ...over,
  };
}

function build(over: Partial<RetirementTileSeries>, targetAge: number | null = 55) {
  return buildRetirementTiles({
    series: series(over),
    currencyIso: EUR,
    monthLabel,
    targetRetirementAge: targetAge,
  });
}

const keys = (m: ReturnType<typeof build>) => m.tiles.map((t) => t.key);

describe("tarjetas por estrategia (§C)", () => {
  it("«Cuanto antes» no añade ninguna: no hay edad contra la que resolver nada", () => {
    expect(keys(build({ strategy: "asap" }))).toEqual([]);
  });

  it("«A una edad fija» trae ahorro necesario y margen", () => {
    const m = build({
      strategy: "retire_at_age",
      required_contribution_monthly: "1200.0000",
      required_contribution_search_ceiling: "1800.0000",
      underfunded: false,
      disposable_monthly: "600.0000",
      disposable_capital_at_retirement: "90000.0000",
      disposable_capital_today: "45000.0000",
    });
    expect(keys(m)).toEqual(["required_contribution", "disposable"]);
    expect(m.notices).toEqual([]);
  });

  it("«Coast FIRE» trae mes coast, número coast y margen — nunca ahorro necesario", () => {
    const m = build({
      strategy: "coast",
      coast_fire_month_index: 84,
      coast_number: "310000.0000",
      disposable_monthly: "0.0000",
    });
    expect(keys(m)).toEqual(["coast_month", "coast_number", "disposable"]);
  });

  it("«Media jornada» trae ahorro necesario, margen y el hueco de la fase", () => {
    const m = build({
      strategy: "partial",
      required_contribution_monthly: "900.0000",
      required_contribution_search_ceiling: "1500.0000",
      underfunded: false,
      disposable_monthly: "600.0000",
      partial_gap_target: "270000.0000",
      partial_phase_capital_growing: true,
    });
    expect(keys(m)).toEqual([
      "required_contribution",
      "disposable",
      "partial_gap",
    ]);
  });

  it("«Puente hasta la pensión» trae la tarjeta de puente y ningún solve", () => {
    const m = build({
      strategy: "pension_bridge",
      pension_start_month_index: 180,
      bridge_effective_withdrawal_pct: "6.5000",
      pension_coverage_ratio: "0.6000",
      bridge_discount_annual_pct: "5.0000",
    });
    expect(keys(m)).toEqual(["bridge"]);
  });

  it("la tarjeta de puente la decide la PENSIÓN, no la estrategia", () => {
    // Quien declara una pensión con fecha tiene un puente que cubrir aunque se jubile por cruce.
    const m = build({ strategy: "asap", pension_start_month_index: 240 });
    expect(keys(m)).toEqual(["bridge"]);
  });
});

describe("`null` no es cero", () => {
  it("sin ahorro necesario no se pinta la tarjeta (ni un 0 €)", () => {
    const m = build({ strategy: "retire_at_age", required_contribution_monthly: null });
    expect(keys(m)).not.toContain("required_contribution");
  });

  it("sin margen publicado no se pinta la tarjeta de margen", () => {
    const m = build({
      strategy: "retire_at_age",
      required_contribution_monthly: "1200.0000",
      disposable_monthly: null,
    });
    expect(keys(m)).toEqual(["required_contribution"]);
  });

  it("`partial_phase_capital_growing: null` no afirma nada sobre la fase", () => {
    const m = build({
      strategy: "partial",
      partial_gap_target: "270000.0000",
      partial_phase_capital_growing: null,
    });
    const gap = m.tiles.find((t) => t.key === "partial_gap")!;
    expect(gap.detail).toBeUndefined();
    expect(gap.tone).toBe("default");
  });
});

describe("las dos bases de «Margen disponible»", () => {
  it("con edad objetivo: el capital acumulado al jubilarse y su versión en euros de hoy", () => {
    const m = build({
      strategy: "retire_at_age",
      disposable_monthly: "600.0000",
      disposable_capital_at_retirement: "90000.0000",
      disposable_capital_today: "45000.0000",
    });
    const tile = m.tiles.find((t) => t.key === "disposable")!;
    expect(tile.parenthetical).toContain("acumulados al jubilarte");
    expect(tile.detail).toContain("en dinero de hoy");
  });

  it("con Coast antes del mes coast: 0 € de verdad, y la copia dice hasta cuándo", () => {
    const m = build({
      strategy: "coast",
      coast_fire_month_index: 84,
      disposable_monthly: "0.0000",
      disposable_capital_at_retirement: "12000.0000",
    });
    const tile = m.tiles.find((t) => t.key === "disposable")!;
    expect(tile.parenthetical).toContain("hasta el mes coast");
    // El capital acumulado NO se anuncia mientras el margen mensual sea cero: la cifra grande
    // dice «no tienes margen» y el paréntesis diría «tienes 12.000 € de margen».
    expect(tile.parenthetical).not.toContain("acumulados");
    expect(tile.detail).toBeUndefined();
  });

  it("con Coast ya alcanzado (mes coast en el pasado o hoy) vuelve la base de capital", () => {
    const m = build({
      strategy: "coast",
      coast_fire_month_index: 0,
      disposable_monthly: "1800.0000",
      disposable_capital_at_retirement: "12000.0000",
      disposable_capital_today: "9000.0000",
    });
    const tile = m.tiles.find((t) => t.key === "disposable")!;
    expect(tile.parenthetical).toContain("acumulados al jubilarte");
    expect(tile.detail).toContain("en dinero de hoy");
  });
});

describe("infra-financiado (D17)", () => {
  const underfunded = {
    strategy: "retire_at_age" as const,
    required_contribution_monthly: "1800.0000",
    required_contribution_search_ceiling: "1800.0000",
    underfunded: true,
    disposable_monthly: "0.0000",
    warnings: ["retire_at_age_underfunded"],
  };

  it("la tarjeta de ahorro se pone en rojo y lo dice", () => {
    const tile = build(underfunded).tiles.find(
      (t) => t.key === "required_contribution",
    )!;
    expect(tile.tone).toBe("danger");
    expect(tile.detail).toContain("TODO tu sobrante");
  });

  it("el aviso rojo nombra la edad objetivo cuando se conoce", () => {
    const n = build(underfunded, 55).notices;
    expect(n[0]!.tone).toBe("danger");
    expect(n[0]!.text).toContain("55 años");
  });

  it("sin edad conocida el rojo sigue saliendo, sin inventarse una cifra", () => {
    const n = build(underfunded, null).notices;
    expect(n[0]!.text).toContain("tu edad objetivo");
    expect(n[0]!.text).not.toMatch(/\d/);
  });

  it("el booleano basta aunque no llegue el aviso, y no se duplica si llegan los dos", () => {
    const soloBool = build({ ...underfunded, warnings: [] }).notices;
    expect(soloBool.map((n) => n.code)).toEqual(["retire_at_age_underfunded"]);
    expect(build(underfunded).notices.map((n) => n.code)).toEqual([
      "retire_at_age_underfunded",
    ]);
  });

  it("`underfunded: false` NO es un aviso: es «llegas»", () => {
    expect(
      build({ ...underfunded, underfunded: false, warnings: [] }).notices,
    ).toEqual([]);
  });
});

describe("avisos", () => {
  it("coast inalcanzable: la tarjeta lo dice y el aviso también", () => {
    const m = build({
      strategy: "coast",
      coast_fire_month_index: null,
      disposable_monthly: "0.0000",
      warnings: ["coast_not_reachable"],
    });
    const tile = m.tiles.find((t) => t.key === "coast_month")!;
    expect(tile.value).toBe("No alcanzable");
    expect(m.notices.map((n) => n.code)).toEqual(["coast_not_reachable"]);
    expect(m.notices[0]!.tone).toBe("warn");
  });

  it("capital que mengua en media jornada: tarjeta en rojo + aviso", () => {
    const m = build({
      strategy: "partial",
      partial_gap_target: "270000.0000",
      partial_phase_capital_growing: false,
      warnings: ["partial_phase_capital_shrinking"],
    });
    expect(m.tiles.find((t) => t.key === "partial_gap")!.tone).toBe("danger");
    expect(m.notices.map((n) => n.code)).toEqual([
      "partial_phase_capital_shrinking",
    ]);
  });

  it("puente sin activos líquidos de los que sacar la tasa", () => {
    const m = build({
      strategy: "pension_bridge",
      pension_start_month_index: 120,
      bridge_discount_annual_pct: "0.0000",
      warnings: ["bridge_discount_no_liquid_assets"],
    });
    expect(m.notices.map((n) => n.code)).toEqual([
      "bridge_discount_no_liquid_assets",
    ]);
  });

  it("`birth_date_missing` NO genera aviso aquí: lo cuenta el banner de alta (D33)", () => {
    const m = build({ strategy: "retire_at_age", warnings: ["birth_date_missing"] });
    expect(m.notices).toEqual([]);
  });

  it("el rojo va SIEMPRE primero, antes de los avisos que solo matizan", () => {
    const m = build({
      strategy: "coast",
      underfunded: true,
      disposable_monthly: "0.0000",
      warnings: ["coast_not_reachable", "retire_at_age_underfunded"],
    });
    expect(m.notices.map((n) => n.code)).toEqual([
      "retire_at_age_underfunded",
      "coast_not_reachable",
    ]);
  });

  it("un literal desconocido del servidor no rompe nada ni inventa un aviso", () => {
    expect(build({ strategy: "coast", warnings: ["algo_nuevo"] }).notices).toEqual([]);
  });
});

describe("unidades del contrato", () => {
  it("el puente formatea porcentaje como porcentaje y fracción como fracción", () => {
    const tile = build({
      strategy: "pension_bridge",
      pension_start_month_index: 144,
      bridge_effective_withdrawal_pct: "6.5000",
      pension_coverage_ratio: "0.6000",
      bridge_discount_annual_pct: "5.0000",
    }).tiles.find((t) => t.key === "bridge")!;
    // `_pct` = porcentaje (6,5 → «6,5 %»); `_ratio` = fracción (0,6 → «60,0 %»).
    expect(tile.parenthetical).toContain("6,5 %");
    expect(tile.detail).toContain("60,0 %");
    expect(tile.detail).toContain("5,0 %");
    // 144 meses = 12 años hasta la pensión.
    expect(tile.value).toContain("12");
  });

  it("una pensión que ya se cobra no se anuncia como «dentro de 0 años»", () => {
    const tile = build({
      strategy: "asap",
      pension_start_month_index: 0,
    }).tiles.find((t) => t.key === "bridge")!;
    expect(tile.value).toBe("Ya la cobras");
  });
});

describe("sin serie", () => {
  it("no hay tarjetas ni avisos mientras la proyección no ha llegado", () => {
    const m = buildRetirementTiles({
      series: null,
      currencyIso: EUR,
      monthLabel,
      targetRetirementAge: 55,
    });
    expect(m.tiles).toEqual([]);
    expect(m.notices).toEqual([]);
  });
});
