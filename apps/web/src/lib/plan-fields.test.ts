/**
 * La tabla U2 fijada: qué campos ve cada estrategia, cuáles exige y **en qué tarjeta caen** (V3).
 *
 * Sin este test la matriz vive repartida en `if`s de una vista y su fallo no rompe nada visible:
 * enseña un campo que la simulación no va a mirar (y el usuario lo rellena creyendo que sirve) o
 * esconde uno que sí (y el plan se calcula con un default que nadie eligió). Los dos son
 * silenciosos, así que la única defensa es recorrer la tabla entera.
 *
 * Se recorren **las 5 estrategias × los 20 campos × los dos estados de pensión × las cuatro
 * familias de regla × los dos estados de fecha de nacimiento**, y además se fijan explícitamente
 * las listas por estrategia: un cambio de la tabla tiene que ser un test rojo, no un descubrimiento
 * en producción.
 *
 * **Qué cambió con V3**: el eje. Antes se comprobaba el reparto en dos GRUPOS (`plan`/`advanced`)
 * y la contigüidad de las seis secciones del acordeón «Avanzado»; ahora, el reparto en las seis
 * TARJETAS por tema y su contigüidad. Ni una condición de visibilidad se movió, y por eso las
 * pruebas de visibilidad son las mismas: lo que se reescribió son las que afirmaban sobre el
 * agrupamiento.
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
  isFieldVisible,
  planCardGroups,
  planFields,
  planFieldsContextFromProfile,
  requiredPlanFields,
  PLAN_CARD_ORDER,
  type PlanCardId,
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

/** Los 20 ids del catálogo, listados a mano: si alguien añade uno sin tocar este array, el test
 *  de exhaustividad de abajo lo caza. */
const ALL_FIELD_IDS: readonly PlanFieldId[] = [
  "birth_date",
  "target_retirement_age",
  "partial_start_age",
  "partial_income",
  "partial_expense_basis",
  "pension_amount",
  "pension_start_age",
  "pension_indexed",
  "pension_fraction_while_partial",
  "target_basis",
  "bridge_discount_basis",
  "fire_number_mode",
  "fire_number_manual_amount",
  "swr_pct",
  "withdrawal_rule_kind",
  "hybrid_end_pct",
  "guardrails_band_pct",
  "guardrails_adjust_pct",
  "spend_mode",
  "horizon_lifespan_age",
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
const cardIds = (card: PlanCardId, c: PlanFieldsContext) =>
  planFields(c)
    .filter((f) => f.card === card)
    .map((f) => f.id);
const cardsPainted = (c: PlanFieldsContext) => planCardGroups(c).map((g) => g.card);

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Edades» — el calendario del plan", () => {
  it("«Cuanto antes» no pide edad ninguna: la tarjeta no existe", () => {
    expect(cardIds("ages", ctx({ strategy: "asap" }))).toEqual([]);
  });

  it("«A una edad fija» pide solo la edad objetivo", () => {
    expect(cardIds("ages", ctx({ strategy: "retire_at_age" }))).toEqual([
      "target_retirement_age",
    ]);
  });

  it("«Coast FIRE» pide lo mismo que «A una edad fija»", () => {
    expect(cardIds("ages", ctx({ strategy: "coast" }))).toEqual(["target_retirement_age"]);
  });

  it("«Media jornada» añade la fase entera, gasto de la fase incluido", () => {
    expect(cardIds("ages", ctx({ strategy: "partial" }))).toEqual([
      "target_retirement_age",
      "partial_start_age",
      "partial_income",
      "partial_expense_basis",
    ]);
  });

  it("«Puente hasta la pensión» se jubila por cruce: sin edades", () => {
    expect(cardIds("ages", ctx({ strategy: "pension_bridge" }))).toEqual([]);
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
      expect(ids(c)[0]).toBe("birth_date");
      expect(f?.card, strategy).toBe("ages");
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

  it("la base de gasto de la fase solo existe en «Media jornada», y en SU tarjeta (V3)", () => {
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("partial_expense_basis", ctx({ strategy })), strategy).toBe(
        strategy === "partial",
      );
    }
    const f = planFields(ctx({ strategy: "partial" })).find(
      (x) => x.id === "partial_expense_basis",
    );
    expect(f?.card).toBe("ages");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Pensión» — la renta con fecha y todo lo que solo existe con ella", () => {
  it("los dos campos base aparecen en las cinco estrategias, con o sin pensión declarada", () => {
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

  it("sin pensión declarada la tarjeta son solo sus dos campos base", () => {
    expect(cardIds("pension", ctx({ strategy: "asap", hasPension: false }))).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("con pensión, los tres mandos finos caen en la MISMA tarjeta (V3, F9)", () => {
    expect(
      cardIds("pension", ctx({ strategy: "retire_at_age", hasPension: true })),
    ).toEqual([
      "pension_amount",
      "pension_start_age",
      "pension_indexed",
      "target_basis",
      "bridge_discount_basis",
    ]);
  });

  it("en media jornada con pensión entra además la fracción cobrada durante la fase", () => {
    expect(cardIds("pension", ctx({ strategy: "partial", hasPension: true }))).toEqual([
      "pension_amount",
      "pension_start_age",
      "pension_indexed",
      "pension_fraction_while_partial",
      "target_basis",
      "bridge_discount_basis",
    ]);
  });

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

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Gasto en jubilación» — siempre, y su importe solo en manual", () => {
  it("`fire_number_mode` está en las cinco y nunca es obligatorio", () => {
    for (const strategy of STRATEGIES) {
      const f = planFields(ctx({ strategy })).find((x) => x.id === "fire_number_mode");
      expect(f, strategy).toBeDefined();
      expect(f?.required, strategy).toBe(false);
      expect(f?.card, strategy).toBe("spending");
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

  it("la tarjeta son exactamente esos dos campos, nunca más", () => {
    expect(cardIds("spending", ctx())).toEqual(["fire_number_mode"]);
    expect(cardIds("spending", ctx({ fireNumberMode: "manual" }))).toEqual([
      "fire_number_mode",
      "fire_number_manual_amount",
    ]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Retirada» — U4: UN solo porcentaje de retirada", () => {
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
      expect(cardIds("withdrawal", ctx({ ruleKind })), ruleKind).toEqual(expected[ruleKind]);
    }
  });

  it("«Cómo se aplica la regla» no existe con gasto fijo: no hay techo del que hablar", () => {
    expect(isFieldVisible("spend_mode", ctx({ ruleKind: "fixed_real" }))).toBe(false);
    for (const ruleKind of RULE_KINDS.filter((k) => k !== "fixed_real")) {
      expect(isFieldVisible("spend_mode", ctx({ ruleKind })), ruleKind).toBe(true);
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Horizonte» — un campo, siempre", () => {
  it("aparece en las cinco estrategias y con cualquier regla, y va el ÚLTIMO", () => {
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        const c = ctx({ strategy, ruleKind });
        expect(isFieldVisible("horizon_lifespan_age", c)).toBe(true);
        expect(ids(c)[ids(c).length - 1]).toBe("horizon_lifespan_age");
      }
    }
  });

  it("es lo único que lleva", () => {
    expect(cardIds("horizon", ctx())).toEqual(["horizon_lifespan_age"]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("los dos campos que V6/V7 retiraron no vuelven por la puerta de atrás", () => {
  it("`cash_buffer_months` y `success_threshold_pct` no salen en NINGÚN contexto", () => {
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        for (const hasPension of [false, true]) {
          const list = ids(ctx({ strategy, ruleKind, hasPension })) as string[];
          expect(list, `${strategy}/${ruleKind}`).not.toContain("cash_buffer_months");
          expect(list, `${strategy}/${ruleKind}`).not.toContain("success_threshold_pct");
        }
      }
    }
  });

  it("tampoco existe una tarjeta «Riesgo»: se quedó sin un solo campo editable", () => {
    expect(PLAN_CARD_ORDER as readonly string[]).not.toContain("risk");
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

  it("`isFieldVisible` coincide con `planFields` para los 20 ids en todos los contextos", () => {
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

  it("las tarjetas salen CONTIGUAS y en `PLAN_CARD_ORDER` (la vista solo agrupa, no reordena)", () => {
    for (const c of everyContext()) {
      const seen: PlanCardId[] = [];
      for (const f of planFields(c)) {
        if (seen[seen.length - 1] !== f.card) seen.push(f.card);
      }
      // Contigüidad: ninguna tarjeta reaparece tras haberla dejado.
      expect(new Set(seen).size, JSON.stringify(c)).toBe(seen.length);
      // Y en el orden canónico.
      const rank = (card: PlanCardId) => PLAN_CARD_ORDER.indexOf(card);
      expect(seen.map(rank), JSON.stringify(c)).toEqual(
        [...seen.map(rank)].sort((a, b) => a - b),
      );
    }
  });

  it("`planCardGroups` no devuelve NUNCA una tarjeta sin campos (salvo «Estrategia»)", () => {
    for (const c of everyContext()) {
      for (const g of planCardGroups(c)) {
        if (g.card === "strategy") {
          // Su contenido es el radiogroup de las cinco estrategias, no campos de la tabla.
          expect(g.fields, JSON.stringify(c)).toEqual([]);
          continue;
        }
        expect(g.fields.length, `${g.card} vacía @ ${JSON.stringify(c)}`).toBeGreaterThan(0);
      }
    }
  });

  it("«Estrategia» se pinta siempre y va la primera", () => {
    for (const c of everyContext()) {
      expect(cardsPainted(c)[0], JSON.stringify(c)).toBe("strategy");
    }
  });

  it("todo campo obligatorio vive en «Edades», «Pensión» o «Gasto»: un supuesto nunca bloquea", () => {
    const puedenBloquear: readonly PlanCardId[] = ["ages", "pension", "spending"];
    for (const c of everyContext()) {
      for (const f of planFields(c).filter((x) => x.required)) {
        expect(puedenBloquear, f.id).toContain(f.card);
      }
    }
  });

  it("cada campo tiene un rótulo no vacío y una tarjeta del catálogo", () => {
    for (const c of everyContext()) {
      for (const f of planFields(c)) {
        expect(f.label.length, f.id).toBeGreaterThan(2);
        expect(PLAN_CARD_ORDER, f.id).toContain(f.card);
      }
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("planCardGroups — qué tarjetas se pintan de verdad", () => {
  it("con «Cuanto antes» y fecha de nacimiento conocida, «Edades» desaparece entera", () => {
    expect(cardsPainted(ctx({ strategy: "asap", hasBirthDate: true }))).toEqual([
      "strategy",
      "pension",
      "spending",
      "withdrawal",
      "horizon",
    ]);
  });

  it("sin fecha de nacimiento, «Edades» reaparece aunque la estrategia no la exija", () => {
    expect(cardsPainted(ctx({ strategy: "asap", hasBirthDate: false }))).toContain("ages");
  });

  it("«Media jornada» con pensión pinta las seis", () => {
    expect(
      cardsPainted(ctx({ strategy: "partial", hasPension: true })),
    ).toEqual([...PLAN_CARD_ORDER]);
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

  it("nunca devuelve un supuesto con default (los de «Retirada» y «Horizonte»)", () => {
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        const c = ctx({ strategy, ruleKind, hasPension: true });
        const supuestos = new Set<string>([
          ...cardIds("withdrawal", c),
          ...cardIds("horizon", c),
        ]);
        for (const id of requiredPlanFields(strategy, { ...rest, ruleKind, hasPension: true })) {
          expect(supuestos.has(id), id).toBe(false);
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
    ]);
  });
});
