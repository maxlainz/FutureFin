/**
 * La tabla U2 fijada: qué campos ve cada estrategia y cuáles exige.
 *
 * Sin este test la matriz vive repartida en `if`s de una vista y su fallo no rompe nada visible:
 * enseña un campo que la simulación no va a mirar (y el usuario lo rellena creyendo que sirve) o
 * esconde uno que sí (y el plan se calcula con un default que nadie eligió). Los dos son
 * silenciosos, así que la única defensa es recorrer la tabla entera.
 *
 * Se recorren **las 5 estrategias × los 22 campos × los dos estados de pensión × las cuatro
 * familias de regla × los dos estados de fecha de nacimiento**, y además se fijan explícitamente
 * las listas por estrategia: un cambio de la tabla tiene que ser un test rojo, no un descubrimiento
 * en producción.
 */

import { describe, expect, it } from "vitest";
import type {
  FireNumberModeApi,
  RetirementProfileApi,
  RetirementStrategyApi,
  TargetBasisApi,
  WithdrawalRuleKindApi,
} from "../api/types";
import {
  advancedGroupFields,
  isFieldVisible,
  planFields,
  planFieldsContextFromProfile,
  planGroupFields,
  requiredPlanFields,
  type PlanFieldId,
  type PlanFieldsContext,
} from "./plan-fields";
import {
  defaultRetirementProfileApi,
  newPartialRetirementDraft,
  newPensionPlanDraft,
  RETIREMENT_STRATEGIES,
} from "./retirementProfile";

const STRATEGIES: readonly RetirementStrategyApi[] = RETIREMENT_STRATEGIES;
const RULE_KINDS: readonly WithdrawalRuleKindApi[] = [
  "fixed_real",
  "percent_of_balance",
  "hybrid",
  "guardrails",
];

/** Los 22 ids del catálogo, listados a mano: si alguien añade uno sin tocar este array, el test
 *  de exhaustividad de abajo lo caza. */
const ALL_FIELD_IDS: readonly PlanFieldId[] = [
  "birth_date",
  "target_retirement_age",
  "partial_start_age",
  "partial_income",
  "pension_amount",
  "pension_start_age",
  "fire_number_mode",
  "fire_number_manual_amount",
  "swr_pct",
  "withdrawal_rule_kind",
  "hybrid_end_pct",
  "guardrails_band_pct",
  "guardrails_adjust_pct",
  "spend_mode",
  "target_basis",
  "bridge_discount_basis",
  "pension_indexed",
  "pension_fraction_while_partial",
  "partial_expense_basis",
  "horizon_lifespan_age",
  "cash_buffer_months",
  "success_threshold_pct",
];

function ctx(over: Partial<PlanFieldsContext> = {}): PlanFieldsContext {
  const strategy = over.strategy ?? "asap";
  const hasPension = over.hasPension ?? false;
  return {
    strategy,
    hasPension,
    hasBirthDate: true,
    ruleKind: "fixed_real",
    // R6 por defecto: puente si hay pensión, perpetua si no; el puente la fuerza.
    effectiveBasis:
      strategy === "pension_bridge" || hasPension ? "bridge_to_pension" : "perpetuity",
    strategyForcesBasis: strategy === "pension_bridge",
    fireNumberMode: "annual_expense",
    ...over,
  };
}

const ids = (c: PlanFieldsContext) => planFields(c).map((f) => f.id);
const planIds = (c: PlanFieldsContext) => planGroupFields(c).map((f) => f.id);
const advIds = (c: PlanFieldsContext) => advancedGroupFields(c).map((f) => f.id);

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("grupo «plan» — lo que cada estrategia PREGUNTA (U2)", () => {
  it("«Cuanto antes» no pide edad ninguna: solo la pensión opcional y el modo del objetivo", () => {
    expect(planIds(ctx({ strategy: "asap" }))).toEqual([
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
    ]);
  });

  it("«A una edad fija» añade la edad objetivo", () => {
    expect(planIds(ctx({ strategy: "retire_at_age" }))).toEqual([
      "target_retirement_age",
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
    ]);
  });

  it("«Coast FIRE» pide lo mismo que «A una edad fija»", () => {
    expect(planIds(ctx({ strategy: "coast" }))).toEqual([
      "target_retirement_age",
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
    ]);
  });

  it("«Media jornada» añade la edad de inicio y el ingreso parcial", () => {
    expect(planIds(ctx({ strategy: "partial" }))).toEqual([
      "target_retirement_age",
      "partial_start_age",
      "partial_income",
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
    ]);
  });

  it("«Puente hasta la pensión» no pide edad objetivo: se jubila por cruce", () => {
    expect(planIds(ctx({ strategy: "pension_bridge" }))).toEqual([
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
    ]);
  });
});

describe("fecha de nacimiento — solo cuando falta", () => {
  it("con fecha de nacimiento el campo no aparece en ninguna estrategia", () => {
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("birth_date", ctx({ strategy, hasBirthDate: true }))).toBe(false);
    }
  });

  it("sin ella aparece siempre, y es OBLIGATORIA solo en las tres estrategias por edad", () => {
    const required: Record<RetirementStrategyApi, boolean> = {
      asap: false,
      retire_at_age: true,
      coast: true,
      partial: true,
      pension_bridge: false,
    };
    for (const strategy of STRATEGIES) {
      const c = ctx({ strategy, hasBirthDate: false });
      const f = planFields(c).find((x) => x.id === "birth_date");
      expect(f, `${strategy}: falta birth_date`).toBeDefined();
      expect(f?.required, strategy).toBe(required[strategy]);
      // Va la PRIMERA: es el dato que hace que todo lo demás signifique algo.
      expect(planIds(c)[0]).toBe("birth_date");
    }
  });
});

describe("edad objetivo — obligatoria salvo en media jornada, y allí se llama distinto", () => {
  it("solo existe en las tres estrategias que la usan", () => {
    const visible: Record<RetirementStrategyApi, boolean> = {
      asap: false,
      retire_at_age: true,
      coast: true,
      partial: true,
      pension_bridge: false,
    };
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("target_retirement_age", ctx({ strategy })), strategy).toBe(
        visible[strategy],
      );
    }
  });

  it("es obligatoria en «A una edad fija» y «Coast FIRE», opcional en «Media jornada»", () => {
    const req = (s: RetirementStrategyApi) =>
      planFields(ctx({ strategy: s })).find((f) => f.id === "target_retirement_age")?.required;
    expect(req("retire_at_age")).toBe(true);
    expect(req("coast")).toBe(true);
    expect(req("partial")).toBe(false);
  });

  it("en media jornada se rotula «Edad de jubilación total»: no es la misma pregunta", () => {
    const label = (s: RetirementStrategyApi) =>
      planFields(ctx({ strategy: s })).find((f) => f.id === "target_retirement_age")?.label;
    expect(label("partial")).toBe("Edad de jubilación total");
    expect(label("retire_at_age")).toBe("Edad de jubilación objetivo");
    expect(label("coast")).toBe("Edad de jubilación objetivo");
  });
});

describe("media jornada — sus dos campos son obligatorios y solo existen ahí", () => {
  it("solo en «Media jornada»", () => {
    for (const strategy of STRATEGIES) {
      const expected = strategy === "partial";
      expect(isFieldVisible("partial_start_age", ctx({ strategy })), strategy).toBe(expected);
      expect(isFieldVisible("partial_income", ctx({ strategy })), strategy).toBe(expected);
    }
  });

  it("los dos son obligatorios: sin ellos la fase no se puede simular", () => {
    const fs = planFields(ctx({ strategy: "partial" }));
    expect(fs.find((f) => f.id === "partial_start_age")?.required).toBe(true);
    expect(fs.find((f) => f.id === "partial_income")?.required).toBe(true);
  });
});

describe("pensión — se ofrece siempre, la exige solo el puente", () => {
  it("los dos campos aparecen en las cinco estrategias, con o sin pensión declarada", () => {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        const c = ctx({ strategy, hasPension });
        expect(isFieldVisible("pension_amount", c), `${strategy}/${hasPension}`).toBe(true);
        expect(isFieldVisible("pension_start_age", c), `${strategy}/${hasPension}`).toBe(true);
      }
    }
  });

  it("solo son obligatorios en «Puente hasta la pensión»", () => {
    for (const strategy of STRATEGIES) {
      const fs = planFields(ctx({ strategy }));
      const expected = strategy === "pension_bridge";
      expect(fs.find((f) => f.id === "pension_amount")?.required, strategy).toBe(expected);
      expect(fs.find((f) => f.id === "pension_start_age")?.required, strategy).toBe(expected);
    }
  });
});

describe("modo del objetivo — siempre, y su importe solo en manual", () => {
  it("`fire_number_mode` está en las cinco y nunca es obligatorio", () => {
    for (const strategy of STRATEGIES) {
      const f = planFields(ctx({ strategy })).find((x) => x.id === "fire_number_mode");
      expect(f, strategy).toBeDefined();
      expect(f?.required, strategy).toBe(false);
    }
  });

  it("`fire_number_manual_amount` solo con el modo manual, y ahí es obligatorio", () => {
    const modes: FireNumberModeApi[] = ["manual", "annual_expense", "current_income"];
    for (const mode of modes) {
      for (const strategy of STRATEGIES) {
        const c = ctx({ strategy, fireNumberMode: mode });
        expect(isFieldVisible("fire_number_manual_amount", c), `${strategy}/${mode}`).toBe(
          mode === "manual",
        );
      }
    }
    const f = planFields(ctx({ fireNumberMode: "manual" })).find(
      (x) => x.id === "fire_number_manual_amount",
    );
    expect(f?.required).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("grupo «avanzado» · retirada — U4: UN solo porcentaje de retirada", () => {
  it("no existe ningún `withdrawal_pct` ni `hybrid_start_pct`: el porcentaje ES `swr_pct`", () => {
    for (const ruleKind of RULE_KINDS) {
      const list = ids(ctx({ ruleKind })) as string[];
      expect(list).toContain("swr_pct");
      expect(list).not.toContain("withdrawal_pct");
      expect(list).not.toContain("hybrid_start_pct");
      expect(list).not.toContain("guardrails_pct");
    }
  });

  it("cada familia de regla añade SOLO sus parámetros propios", () => {
    const expected: Record<WithdrawalRuleKindApi, PlanFieldId[]> = {
      fixed_real: ["swr_pct", "withdrawal_rule_kind"],
      percent_of_balance: ["swr_pct", "withdrawal_rule_kind", "spend_mode"],
      hybrid: ["swr_pct", "withdrawal_rule_kind", "hybrid_end_pct", "spend_mode"],
      guardrails: [
        "swr_pct",
        "withdrawal_rule_kind",
        "guardrails_band_pct",
        "guardrails_adjust_pct",
        "spend_mode",
      ],
    };
    for (const ruleKind of RULE_KINDS) {
      const retirada = advancedGroupFields(ctx({ ruleKind }))
        .filter((f) => f.section === "retirada")
        .map((f) => f.id);
      expect(retirada, ruleKind).toEqual(expected[ruleKind]);
    }
  });

  it("«Cómo se aplica la regla» no existe con gasto fijo: no hay techo del que hablar", () => {
    expect(isFieldVisible("spend_mode", ctx({ ruleKind: "fixed_real" }))).toBe(false);
    for (const ruleKind of RULE_KINDS.filter((k) => k !== "fixed_real")) {
      expect(isFieldVisible("spend_mode", ctx({ ruleKind })), ruleKind).toBe(true);
    }
  });
});

describe("grupo «avanzado» · objetivo", () => {
  it("la base del objetivo solo se elige con pensión declarada y sin estrategia que la imponga", () => {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        const c = ctx({ strategy, hasPension });
        const expected = hasPension && strategy !== "pension_bridge";
        expect(isFieldVisible("target_basis", c), `${strategy}/${hasPension}`).toBe(expected);
      }
    }
  });

  it("«Puente hasta la pensión» NO ofrece elegir la base: la impone", () => {
    expect(
      isFieldVisible("target_basis", ctx({ strategy: "pension_bridge", hasPension: true })),
    ).toBe(false);
  });

  it("el descuento del puente aparece si y solo si la base EFECTIVA es el puente", () => {
    const bases: TargetBasisApi[] = ["perpetuity", "bridge_to_pension"];
    for (const strategy of STRATEGIES) {
      for (const effectiveBasis of bases) {
        const c = ctx({ strategy, effectiveBasis });
        expect(isFieldVisible("bridge_discount_basis", c), `${strategy}/${effectiveBasis}`).toBe(
          effectiveBasis === "bridge_to_pension",
        );
      }
    }
  });
});

describe("grupo «avanzado» · pensión y media jornada", () => {
  it("la indexación solo existe con pensión declarada", () => {
    for (const strategy of STRATEGIES) {
      expect(
        isFieldVisible("pension_indexed", ctx({ strategy, hasPension: false })),
        strategy,
      ).toBe(false);
      expect(
        isFieldVisible("pension_indexed", ctx({ strategy, hasPension: true })),
        strategy,
      ).toBe(true);
    }
  });

  it("la fracción durante la media jornada exige las DOS cosas: fase parcial Y pensión", () => {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        const expected = strategy === "partial" && hasPension;
        expect(
          isFieldVisible("pension_fraction_while_partial", ctx({ strategy, hasPension })),
          `${strategy}/${hasPension}`,
        ).toBe(expected);
      }
    }
  });

  it("la base de gasto de la fase parcial solo existe en «Media jornada»", () => {
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("partial_expense_basis", ctx({ strategy })), strategy).toBe(
        strategy === "partial",
      );
    }
  });
});

describe("grupo «avanzado» · horizonte y riesgo — los tres, siempre", () => {
  it("aparecen en las cinco estrategias y con cualquier regla", () => {
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        const c = ctx({ strategy, ruleKind });
        expect(isFieldVisible("horizon_lifespan_age", c)).toBe(true);
        expect(isFieldVisible("cash_buffer_months", c)).toBe(true);
        expect(isFieldVisible("success_threshold_pct", c)).toBe(true);
      }
    }
  });

  it("y son las tres últimas, en ese orden", () => {
    const list = ids(ctx());
    expect(list.slice(-3)).toEqual([
      "horizon_lifespan_age",
      "cash_buffer_months",
      "success_threshold_pct",
    ]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("invariantes sobre TODO el producto cartesiano", () => {
  /** 5 estrategias × 2 pensiones × 2 fechas de nacimiento × 4 reglas × 2 bases × 3 modos. */
  function* everyContext(): Generator<PlanFieldsContext> {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        for (const hasBirthDate of [false, true]) {
          for (const ruleKind of RULE_KINDS) {
            for (const effectiveBasis of ["perpetuity", "bridge_to_pension"] as const) {
              for (const fireNumberMode of [
                "annual_expense",
                "manual",
                "current_income",
              ] as const) {
                yield ctx({
                  strategy,
                  hasPension,
                  hasBirthDate,
                  ruleKind,
                  effectiveBasis,
                  fireNumberMode,
                  strategyForcesBasis: strategy === "pension_bridge",
                });
              }
            }
          }
        }
      }
    }
  }

  it("`isFieldVisible` coincide con `planFields` para los 22 ids en todos los contextos", () => {
    for (const c of everyContext()) {
      const list = new Set(planFields(c).map((f) => f.id));
      for (const id of ALL_FIELD_IDS) {
        expect(isFieldVisible(id, c), `${id} @ ${JSON.stringify(c)}`).toBe(list.has(id));
      }
    }
  });

  it("ningún id se repite, y todos salen del catálogo cerrado", () => {
    const known = new Set<string>(ALL_FIELD_IDS);
    for (const c of everyContext()) {
      const list = planFields(c).map((f) => f.id);
      expect(new Set(list).size, JSON.stringify(c)).toBe(list.length);
      for (const id of list) expect(known.has(id), `${id} no está en ALL_FIELD_IDS`).toBe(true);
    }
  });

  it("un campo `plan` no tiene sección y uno `advanced` la tiene y NUNCA es obligatorio", () => {
    for (const c of everyContext()) {
      for (const f of planFields(c)) {
        if (f.group === "plan") {
          expect(f.section, f.id).toBeNull();
        } else {
          expect(f.section, f.id).not.toBeNull();
          expect(f.required, `${f.id} avanzado NO puede ser obligatorio`).toBe(false);
        }
      }
      // Los avanzados van todos DESPUÉS de los del plan: el orden del array es el de lectura.
      const groups = planFields(c).map((f) => f.group);
      expect(groups.indexOf("advanced") === -1 || !groups.slice(groups.indexOf("advanced")).includes("plan")).toBe(true);
    }
  });

  it("las secciones del bloque avanzado salen CONTIGUAS (la vista solo agrupa, no reordena)", () => {
    for (const c of everyContext()) {
      const seen: string[] = [];
      for (const f of advancedGroupFields(c)) {
        if (seen[seen.length - 1] !== f.section) seen.push(f.section);
      }
      expect(new Set(seen).size, JSON.stringify(c)).toBe(seen.length);
    }
  });

  it("todo campo obligatorio es del grupo «plan»: un supuesto con default nunca bloquea", () => {
    for (const c of everyContext()) {
      for (const f of planFields(c).filter((x) => x.required)) {
        expect(f.group, f.id).toBe("plan");
      }
    }
  });

  it("cada campo tiene un rótulo no vacío", () => {
    for (const c of everyContext()) {
      for (const f of planFields(c)) expect(f.label.length, f.id).toBeGreaterThan(2);
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("requiredPlanFields — lo que el asistente de alta y el autosave preguntan", () => {
  const rest = {
    hasPension: false,
    hasBirthDate: true,
    ruleKind: "fixed_real" as const,
    effectiveBasis: "perpetuity" as const,
    strategyForcesBasis: false,
    fireNumberMode: "annual_expense" as const,
  };

  it("«Cuanto antes» no exige nada", () => {
    expect(requiredPlanFields("asap", rest)).toEqual([]);
  });

  it("«A una edad fija» y «Coast FIRE» exigen la edad objetivo", () => {
    expect(requiredPlanFields("retire_at_age", rest)).toEqual(["target_retirement_age"]);
    expect(requiredPlanFields("coast", rest)).toEqual(["target_retirement_age"]);
  });

  it("«Media jornada» exige la fase, no la edad total", () => {
    expect(requiredPlanFields("partial", rest)).toEqual([
      "partial_start_age",
      "partial_income",
    ]);
  });

  it("«Puente hasta la pensión» exige la pensión entera", () => {
    expect(requiredPlanFields("pension_bridge", { ...rest, strategyForcesBasis: true })).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("sin fecha de nacimiento, las tres estrategias por edad la añaden la PRIMERA", () => {
    const noBirth = { ...rest, hasBirthDate: false };
    expect(requiredPlanFields("retire_at_age", noBirth)).toEqual([
      "birth_date",
      "target_retirement_age",
    ]);
    expect(requiredPlanFields("coast", noBirth)).toEqual([
      "birth_date",
      "target_retirement_age",
    ]);
    expect(requiredPlanFields("partial", noBirth)).toEqual([
      "birth_date",
      "partial_start_age",
      "partial_income",
    ]);
    // …y las otras dos no: sin fecha se simulan igual.
    expect(requiredPlanFields("asap", noBirth)).toEqual([]);
    expect(requiredPlanFields("pension_bridge", noBirth)).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("el modo manual añade su importe", () => {
    expect(requiredPlanFields("asap", { ...rest, fireNumberMode: "manual" })).toEqual([
      "fire_number_manual_amount",
    ]);
  });

  it("nunca devuelve un campo avanzado", () => {
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        const req = requiredPlanFields(strategy, { ...rest, ruleKind, hasPension: true });
        for (const id of req) {
          expect(advIds(ctx({ strategy, ruleKind, hasPension: true }))).not.toContain(id);
        }
      }
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("planFieldsContextFromProfile — una sola derivación para todos los consumidores", () => {
  function profile(over: Partial<RetirementProfileApi> = {}): RetirementProfileApi {
    return { ...defaultRetirementProfileApi(), ...over };
  }

  it("deriva la base con la regla R6: perpetua sin pensión, puente con ella", () => {
    expect(planFieldsContextFromProfile(profile(), true).effectiveBasis).toBe("perpetuity");
    expect(
      planFieldsContextFromProfile(profile({ pension: newPensionPlanDraft() }), true)
        .effectiveBasis,
    ).toBe("bridge_to_pension");
  });

  it("«Puente hasta la pensión» fuerza la base aunque no haya bloque de pensión todavía", () => {
    const c = planFieldsContextFromProfile(profile({ strategy: "pension_bridge" }), true);
    expect(c.effectiveBasis).toBe("bridge_to_pension");
    expect(c.strategyForcesBasis).toBe(true);
  });

  it("`hasPension` es «hay bloque declarado», no «la estrategia lo permitiría»", () => {
    expect(planFieldsContextFromProfile(profile(), true).hasPension).toBe(false);
    expect(
      planFieldsContextFromProfile(profile({ pension: newPensionPlanDraft() }), true).hasPension,
    ).toBe(true);
  });

  it("copia la regla, el modo del objetivo y la fecha de nacimiento sin tocarlos", () => {
    const p = profile({
      strategy: "partial",
      partial_retirement: newPartialRetirementDraft(),
      fire_number_mode: "manual",
      withdrawal_rule: { ...defaultRetirementProfileApi().withdrawal_rule, kind: "guardrails" },
    });
    const c = planFieldsContextFromProfile(p, false);
    expect(c.strategy).toBe("partial");
    expect(c.ruleKind).toBe("guardrails");
    expect(c.fireNumberMode).toBe("manual");
    expect(c.hasBirthDate).toBe(false);
  });

  it("el perfil por defecto produce exactamente la lista mínima", () => {
    const c = planFieldsContextFromProfile(profile(), true);
    expect(ids(c)).toEqual([
      "pension_amount",
      "pension_start_age",
      "fire_number_mode",
      "swr_pct",
      "withdrawal_rule_kind",
      "horizon_lifespan_age",
      "cash_buffer_months",
      "success_threshold_pct",
    ]);
  });
});
