/**
 * La línea «Supuestos» (U12): **nada se fuerza en silencio**.
 *
 * El test que de verdad importa es el último bloque: para cada campo AVANZADO, su cláusula está
 * en la línea si y solo si el campo es visible según `plan-fields`. Sin esa equivalencia, U2
 * (esconder lo que no aplica) degenera en «forzar sin decirlo», que es exactamente lo que U12
 * existe para impedir.
 */

import { describe, expect, it } from "vitest";
import type { RetirementProfileApi, RetirementStrategyApi } from "../api/types";
import { assumptionParts, assumptionsLine } from "./assumptions-line";
import { isFieldVisible, planFieldsContextFromProfile } from "./plan-fields";
import {
  defaultRetirementProfileApi,
  newPartialRetirementDraft,
  newPensionPlanDraft,
  RETIREMENT_STRATEGIES,
} from "./retirementProfile";

function profile(over: Partial<RetirementProfileApi> = {}): RetirementProfileApi {
  return { ...defaultRetirementProfileApi(), ...over };
}

const line = (p: RetirementProfileApi, hasBirthDate = true) =>
  assumptionsLine(p, { hasBirthDate });

describe("la línea de serie", () => {
  it("el perfil por defecto produce la línea del ejemplo de U12", () => {
    expect(line(profile())).toBe(
      "Supuestos: retirada 3,5 % · gasto fijo en euros de hoy · horizonte 90 años · sin colchón · umbral 95,0 %",
    );
  });

  it("siempre empieza por «Supuestos: » y nunca sale vacía", () => {
    for (const strategy of RETIREMENT_STRATEGIES) {
      const t = line(profile({ strategy }));
      expect(t.startsWith("Supuestos: "), strategy).toBe(true);
      expect(t.length, strategy).toBeGreaterThan(20);
    }
  });

  it("los porcentajes llevan un decimal y « %» detrás (helpers canónicos, no concatenación)", () => {
    const t = line(profile({ swr_pct: "3.25", success_threshold_pct: 90 }));
    expect(t).toContain("retirada 3,3 %");
    expect(t).toContain("umbral 90,0 %");
  });
});

describe("la regla de retirada — U4: la cláusula NO repite el porcentaje", () => {
  it("gasto fijo", () => {
    expect(assumptionParts(profile(), { hasBirthDate: true })[1]).toBe(
      "gasto fijo en euros de hoy",
    );
  });

  it("un % del saldo: el porcentaje ya va delante como «retirada», no se dice dos veces", () => {
    const p = profile({
      withdrawal_rule: {
        ...defaultRetirementProfileApi().withdrawal_rule,
        kind: "percent_of_balance",
        pct: "4",
      },
    });
    const parts = assumptionParts(p, { hasBirthDate: true });
    expect(parts[1]).toBe("un % del saldo cada año");
    expect(parts.filter((x) => x.includes("%")).length).toBeGreaterThan(0);
    // Solo una cláusula anuncia un porcentaje de RETIRADA.
    expect(parts.filter((x) => x.startsWith("retirada "))).toHaveLength(1);
  });

  it("híbrida enseña únicamente el «baja al X %» (U4)", () => {
    const p = profile({
      withdrawal_rule: {
        ...defaultRetirementProfileApi().withdrawal_rule,
        kind: "hybrid",
        start_pct: "5",
        end_pct: "3",
      },
    });
    // El `start_pct` guardado (5 %) NO se enuncia: el porcentaje de partida ES `swr_pct`.
    expect(assumptionParts(p, { hasBirthDate: true })[1]).toBe("híbrida: baja al 3,0 %");
    expect(line(p)).toContain("retirada 3,5 %");
  });

  it("bandas enseñan su banda y su ajuste", () => {
    const p = profile({
      withdrawal_rule: {
        ...defaultRetirementProfileApi().withdrawal_rule,
        kind: "guardrails",
        pct: "4",
        band_pct: "20",
        adjust_pct: "10",
      },
    });
    expect(line(p)).toContain("con bandas: ±20,0 %, ajuste 10,0 %");
  });

  it("«cómo se aplica la regla» solo aparece cuando el campo existe (≠ gasto fijo)", () => {
    expect(line(profile())).not.toContain("techo");
    const p = profile({
      withdrawal_rule: {
        ...defaultRetirementProfileApi().withdrawal_rule,
        kind: "percent_of_balance",
        pct: "4",
        spend_mode: "rule_is_spend",
      },
    });
    expect(line(p)).toContain("la regla es tu gasto");
  });
});

describe("los extras que solo aparecen con su campo visible", () => {
  const withPension = () => profile({ pension: newPensionPlanDraft() });

  it("«base: puente (derivada)» cuando nadie la eligió", () => {
    expect(line(withPension())).toContain("base: puente (derivada)");
  });

  it("sin «(derivada)» cuando la elección está guardada", () => {
    const p = { ...withPension(), target_basis: "bridge_to_pension" as const };
    const t = assumptionsLine(p, { hasBirthDate: true });
    expect(t).toContain("base: puente");
    expect(t).not.toContain("(derivada)");
  });

  it("la fuente se puede forzar desde fuera (el perfil resuelto no la distingue)", () => {
    const p = { ...withPension(), target_basis: "bridge_to_pension" as const };
    expect(
      assumptionsLine(p, { hasBirthDate: true, targetBasisSource: "derived" }),
    ).toContain("base: puente (derivada)");
  });

  it("«Puente hasta la pensión» impone la base, así que NO la enuncia como elección", () => {
    const p = profile({ strategy: "pension_bridge", pension: newPensionPlanDraft() });
    const t = line(p);
    expect(t).not.toContain("base:");
    // …pero el descuento sí, porque la base efectiva ES el puente.
    expect(t).toContain("descuento: rentabilidad esperada");
  });

  it("el descuento del puente solo con base puente, y con su nombre corto", () => {
    expect(line(profile())).not.toContain("descuento:");
    expect(line(withPension())).toContain("descuento: rentabilidad esperada");
    expect(
      line(profile({ pension: newPensionPlanDraft(), bridge_discount_basis: "swr" })),
    ).toContain("descuento: tu tasa segura de retirada");
    expect(
      line(profile({ pension: newPensionPlanDraft(), bridge_discount_basis: "none" })),
    ).toContain("descuento: ninguno");
  });

  it("la indexación de la pensión solo con pensión declarada, y dice las dos caras", () => {
    expect(line(profile())).not.toContain("pensión indexada");
    expect(line(withPension())).toContain("pensión indexada");
    const flat = profile({ pension: { ...newPensionPlanDraft(), indexed: false } });
    expect(line(flat)).toContain("pensión sin indexar");
  });

  it("la fracción durante la media jornada exige fase parcial Y pensión", () => {
    const onlyPension = withPension();
    expect(line(onlyPension)).not.toContain("durante la media jornada");

    const both = profile({
      strategy: "partial",
      partial_retirement: newPartialRetirementDraft(),
      pension: { ...newPensionPlanDraft(), fraction_while_partial: "0.5" },
    });
    expect(line(both)).toContain("pensión durante la media jornada: 50,0 %");
  });

  it("una fracción de 0 se enuncia igual: es un supuesto en vigor, no un hueco", () => {
    const p = profile({
      strategy: "partial",
      partial_retirement: newPartialRetirementDraft(),
      pension: newPensionPlanDraft(),
    });
    expect(line(p)).toContain("sin pensión durante la media jornada");
  });

  it("la base de gasto de la fase parcial solo en «Media jornada», con sus dos lecturas", () => {
    expect(line(profile())).not.toContain("gasto en media jornada");
    const ret = profile({
      strategy: "partial",
      partial_retirement: newPartialRetirementDraft(),
    });
    expect(line(ret)).toContain("gasto en media jornada: el de jubilación");
    const reg = profile({
      strategy: "partial",
      partial_retirement: { ...newPartialRetirementDraft(), expense_basis: "regular" },
    });
    expect(line(reg)).toContain("gasto en media jornada: el de ahora");
  });

  it("el colchón se enuncia con su número, y su ausencia también", () => {
    expect(line(profile())).toContain("sin colchón");
    expect(line(profile({ cash_buffer_months: 0 }))).toContain("sin colchón");
    expect(line(profile({ cash_buffer_months: 1 }))).toContain("colchón 1 mes");
    expect(line(profile({ cash_buffer_months: 12 }))).toContain("colchón 12 meses");
  });

  it("el horizonte va siempre, con la edad elegida", () => {
    expect(line(profile({ horizon_lifespan_age: 100 }))).toContain("horizonte 100 años");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("EQUIVALENCIA con plan-fields — la garantía de U12", () => {
  /** Los avanzados condicionales y la marca que los delata en la línea. */
  const CONDITIONAL: Array<[Parameters<typeof isFieldVisible>[0], (t: string) => boolean]> = [
    ["spend_mode", (t) => t.includes("techo:") || t.includes("la regla es tu gasto")],
    ["target_basis", (t) => t.includes("base:")],
    ["bridge_discount_basis", (t) => t.includes("descuento:")],
    ["pension_indexed", (t) => t.includes("pensión indexada") || t.includes("pensión sin indexar")],
    ["pension_fraction_while_partial", (t) => t.includes("durante la media jornada")],
    ["partial_expense_basis", (t) => t.includes("gasto en media jornada:")],
  ];

  function* profiles(): Generator<[string, RetirementProfileApi]> {
    const rules = defaultRetirementProfileApi().withdrawal_rule;
    for (const strategy of RETIREMENT_STRATEGIES as readonly RetirementStrategyApi[]) {
      for (const hasPension of [false, true]) {
        for (const hasPartial of [false, true]) {
          for (const kind of ["fixed_real", "percent_of_balance", "hybrid", "guardrails"] as const) {
            yield [
              `${strategy}/pension=${hasPension}/partial=${hasPartial}/${kind}`,
              profile({
                strategy,
                pension: hasPension
                  ? { ...newPensionPlanDraft(), monthly_amount_today: "1200" }
                  : null,
                partial_retirement: hasPartial ? newPartialRetirementDraft() : null,
                withdrawal_rule: {
                  ...rules,
                  kind,
                  pct: "4",
                  start_pct: "5",
                  end_pct: "3",
                  band_pct: "20",
                  adjust_pct: "10",
                },
              }),
            ];
          }
        }
      }
    }
  }

  it("cada supuesto condicional está en la línea ⟺ su campo es visible", () => {
    for (const [name, p] of profiles()) {
      const ctx = planFieldsContextFromProfile(p, true);
      const text = assumptionsLine(p, { hasBirthDate: true });
      for (const [id, present] of CONDITIONAL) {
        expect(present(text), `${name} · ${id} — línea: ${text}`).toBe(
          isFieldVisible(id, ctx),
        );
      }
    }
  });

  it("los cuatro supuestos incondicionales están SIEMPRE", () => {
    for (const [name, p] of profiles()) {
      const t = assumptionsLine(p, { hasBirthDate: true });
      expect(t, name).toContain("retirada ");
      expect(t, name).toContain("horizonte ");
      expect(t, name).toContain("umbral ");
      expect(t.includes("colchón"), name).toBe(true);
    }
  });

  it("ninguna cláusula sale vacía ni con un separador colgando", () => {
    for (const [name, p] of profiles()) {
      for (const part of assumptionParts(p, { hasBirthDate: true })) {
        expect(part.trim(), name).toBe(part);
        expect(part.length, `${name}: cláusula vacía`).toBeGreaterThan(2);
        expect(part.includes("undefined"), `${name}: ${part}`).toBe(false);
        expect(part.includes("null"), `${name}: ${part}`).toBe(false);
      }
    }
  });

  it("la fecha de nacimiento no cambia ni un supuesto (es un campo del grupo «plan»)", () => {
    for (const [name, p] of profiles()) {
      expect(assumptionsLine(p, { hasBirthDate: false }), name).toBe(
        assumptionsLine(p, { hasBirthDate: true }),
      );
    }
  });
});
