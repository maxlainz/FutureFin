/**
 * La tabla U2 fijada: qué campos ve cada estrategia **y cada MODO**, cuáles exige y en qué tarjeta
 * caen (V3 + modelo v2).
 *
 * Sin este test la matriz vive repartida en `if`s de una vista y su fallo no rompe nada visible:
 * enseña un campo que la simulación no va a mirar (y el usuario lo rellena creyendo que sirve) o
 * esconde uno que sí (y el plan se calcula con un default que nadie eligió). Los dos son
 * silenciosos, así que la única defensa es recorrer la tabla entera.
 *
 * Se recorren **las 4 estrategias × los 25 campos × los dos estados de pensión × los dos modos de
 * coast × los dos de media jornada × los dos del puente × las cuatro familias de regla × los dos
 * estados de fecha de nacimiento**, y además se fijan explícitamente las listas por estrategia: un
 * cambio de la tabla tiene que ser un test rojo, no un descubrimiento en producción.
 *
 * **Qué cambió con el modelo v2** (C1–C8), y por qué cada cambio tiene su propio `it`:
 *
 *  - **La estrategia ya no basta**: `coast` y `partial` preguntan una edad DISTINTA según su modo,
 *    y en cada modo la otra edad **no se pinta**. Es la clase de regresión que un test por
 *    estrategia no puede ver.
 *  - **El puente dejó de ser una estrategia** (C7): son tres campos de la tarjeta Pensión,
 *    disponibles en cualquier estrategia y solo con pensión declarada.
 *  - **`success_threshold_pct` VUELVE** (C3) y vuelve el primero de «Retirada». El `describe` que
 *    afirmaba que no volvía está invertido a propósito: `cash_buffer_months` sigue sin volver
 *    (M6 retiró el mecanismo entero), el umbral sí.
 *  - **`target_basis` y `bridge_discount_basis` mueren**: no hay objetivo que dimensionar (C1/M4),
 *    así que no hay base que elegir ni descuento que aplicar. Sus dos `describe` se borraron.
 *
 * **Por qué el perfil de prueba se construye AQUÍ y no con `defaultRetirementProfileApi()`**: lo
 * que este fichero fija es el contrato de `plan-fields` contra `RetirementProfileApi` —el tipo del
 * wire—, no contra los defaults de otro módulo. Un literal completo obliga además a que cualquier
 * campo nuevo del perfil pase por aquí en vez de colarse con un default.
 */

import { describe, expect, it } from "vitest";
import type {
  CoastModeApi,
  FireNumberModeApi,
  PartialRetirementApi,
  RetirementProfileApi,
  RetirementStrategyApi,
  WithdrawalRuleKindApi,
} from "../api/types";
import {
  isFieldVisible,
  planCardGroups,
  planFields,
  planFieldsContextFromProfile,
  requiredPlanFields,
  PLAN_CARD_ORDER,
  type PartialModeApi,
  type PlanCardId,
  type PlanFieldId,
  type PlanFieldsContext,
} from "./plan-fields";

/** Las CUATRO del modelo v2 (C7 retiró `pension_bridge` del selector). Se listan aquí y no se
 *  importan de `retirementProfile.ts` para que la unión del wire —`RetirementStrategyApi`— sea la
 *  única autoridad: si mañana nace o muere una estrategia, el compilador cae sobre este array. */
const STRATEGIES: readonly RetirementStrategyApi[] = [
  "asap",
  "retire_at_age",
  "coast",
  "partial",
];
const RULE_KINDS: readonly WithdrawalRuleKindApi[] = [
  "fixed_real",
  "percent_of_balance",
  "hybrid",
  "guardrails",
];
const COAST_MODES: readonly CoastModeApi[] = ["fixed_retirement_age", "fixed_stop_age"];
const PARTIAL_MODES: readonly PartialModeApi[] = ["at_age", "asap"];

/** Los 25 ids del catálogo, listados a mano: si alguien añade uno sin tocar este array, el test
 *  de exhaustividad de abajo lo caza. Eran 20 antes del modelo v2 (−2 muertos: `target_basis`,
 *  `bridge_discount_basis`; +7 nuevos: los dos modos, las dos edades que abren, los tres del
 *  puente y el umbral). */
const ALL_FIELD_IDS: readonly PlanFieldId[] = [
  "birth_date",
  "coast_mode",
  "target_retirement_age",
  "coast_stop_age",
  "partial_mode",
  "partial_start_age",
  "partial_income",
  "partial_expense_basis",
  "pension_amount",
  "pension_start_age",
  "pension_indexed",
  "pension_fraction_while_partial",
  "bridge_enabled",
  "bridge_max_pct",
  "bridge_max_years",
  "fire_number_mode",
  "fire_number_manual_amount",
  "success_threshold_pct",
  "swr_pct",
  "withdrawal_rule_kind",
  "hybrid_end_pct",
  "guardrails_band_pct",
  "guardrails_adjust_pct",
  "spend_mode",
  "horizon_lifespan_age",
];

function ctx(over: Partial<PlanFieldsContext> = {}): PlanFieldsContext {
  return {
    strategy: "asap",
    hasPension: false,
    hasBirthDate: true,
    ruleKind: "fixed_real",
    fireNumberMode: "annual_expense",
    coastMode: "fixed_retirement_age",
    partialMode: "at_age",
    bridgeEnabled: false,
    ...over,
  };
}

const ids = (c: PlanFieldsContext) => planFields(c).map((f) => f.id);
const cardIds = (card: PlanCardId, c: PlanFieldsContext) =>
  planFields(c)
    .filter((f) => f.card === card)
    .map((f) => f.id);
const cardsPainted = (c: PlanFieldsContext) => planCardGroups(c).map((g) => g.card);
const req = (id: PlanFieldId, c: PlanFieldsContext) =>
  planFields(c).find((f) => f.id === id)?.required;

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

  it("«Coast FIRE» modo A: el modo primero y luego la edad de jubilación", () => {
    expect(
      cardIds("ages", ctx({ strategy: "coast", coastMode: "fixed_retirement_age" })),
    ).toEqual(["coast_mode", "target_retirement_age"]);
  });

  it("«Media jornada» modo A añade la fase entera, gasto de la fase incluido", () => {
    expect(cardIds("ages", ctx({ strategy: "partial", partialMode: "at_age" }))).toEqual([
      "target_retirement_age",
      "partial_mode",
      "partial_start_age",
      "partial_income",
      "partial_expense_basis",
    ]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("coast — el modo decide QUÉ edad se pregunta (M10)", () => {
  it("modo B esconde la edad de jubilación y exige la de parada", () => {
    const b = ctx({ strategy: "coast", coastMode: "fixed_stop_age" });
    // La edad de jubilación NO se pinta: en modo B la resuelve el sorteo, y un campo visible
    // anunciaría un control sobre algo que el usuario acaba de delegar.
    expect(isFieldVisible("target_retirement_age", b)).toBe(false);
    expect(isFieldVisible("coast_stop_age", b)).toBe(true);
    expect(req("coast_stop_age", b)).toBe(true);
    expect(cardIds("ages", b)).toEqual(["coast_mode", "coast_stop_age"]);
  });

  it("modo A es el simétrico exacto: edad de jubilación obligatoria, sin edad de parada", () => {
    const a = ctx({ strategy: "coast", coastMode: "fixed_retirement_age" });
    expect(isFieldVisible("coast_stop_age", a)).toBe(false);
    expect(req("target_retirement_age", a)).toBe(true);
  });

  it("el selector de modo solo existe en «Coast FIRE», y nunca es obligatorio (tiene default)", () => {
    for (const strategy of STRATEGIES) {
      for (const coastMode of COAST_MODES) {
        const c = ctx({ strategy, coastMode });
        expect(isFieldVisible("coast_mode", c), `${strategy}/${coastMode}`).toBe(
          strategy === "coast",
        );
        expect(isFieldVisible("coast_stop_age", c), `${strategy}/${coastMode}`).toBe(
          strategy === "coast" && coastMode === "fixed_stop_age",
        );
      }
    }
    expect(req("coast_mode", ctx({ strategy: "coast" }))).toBe(false);
  });

  it("el modo va ANTES que su edad: primero qué fijas, luego el número", () => {
    for (const coastMode of COAST_MODES) {
      const list = cardIds("ages", ctx({ strategy: "coast", coastMode }));
      expect(list[0], coastMode).toBe("coast_mode");
      expect(list.length, coastMode).toBe(2);
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("media jornada — el modo decide si se pregunta la edad de inicio (M11)", () => {
  it("modo «en cuanto pueda» esconde la edad de inicio y deja solo el ingreso", () => {
    const b = ctx({ strategy: "partial", partialMode: "asap" });
    expect(isFieldVisible("partial_start_age", b)).toBe(false);
    // El ingreso sigue siendo obligatorio: sin él la fase no se puede simular en ningún modo.
    expect(req("partial_income", b)).toBe(true);
    expect(cardIds("ages", b)).toEqual([
      "target_retirement_age",
      "partial_mode",
      "partial_income",
      "partial_expense_basis",
    ]);
  });

  it("modo «a una edad»: la edad de inicio existe y es obligatoria", () => {
    const a = ctx({ strategy: "partial", partialMode: "at_age" });
    expect(isFieldVisible("partial_start_age", a)).toBe(true);
    expect(req("partial_start_age", a)).toBe(true);
  });

  it("el selector de modo solo existe en «Media jornada», y nunca es obligatorio", () => {
    for (const strategy of STRATEGIES) {
      for (const partialMode of PARTIAL_MODES) {
        const c = ctx({ strategy, partialMode });
        expect(isFieldVisible("partial_mode", c), `${strategy}/${partialMode}`).toBe(
          strategy === "partial",
        );
        expect(isFieldVisible("partial_start_age", c), `${strategy}/${partialMode}`).toBe(
          strategy === "partial" && partialMode === "at_age",
        );
      }
    }
    expect(req("partial_mode", ctx({ strategy: "partial" }))).toBe(false);
  });

  it("el ingreso y la base de gasto de la fase existen en los DOS modos", () => {
    for (const partialMode of PARTIAL_MODES) {
      const c = ctx({ strategy: "partial", partialMode });
      expect(isFieldVisible("partial_income", c), partialMode).toBe(true);
      expect(isFieldVisible("partial_expense_basis", c), partialMode).toBe(true);
      expect(req("partial_expense_basis", c), partialMode).toBe(false);
    }
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("partial_expense_basis", ctx({ strategy })), strategy).toBe(
        strategy === "partial",
      );
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("fecha de nacimiento — solo cuando falta, obligatoria en más casos que antes (C5)", () => {
  it("con fecha de nacimiento el campo no aparece en ninguna estrategia", () => {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        expect(
          isFieldVisible("birth_date", ctx({ strategy, hasBirthDate: true, hasPension })),
          `${strategy}/${hasPension}`,
        ).toBe(false);
      }
    }
  });

  it("sin ella aparece siempre, y es OBLIGATORIA en las tres estrategias por edad", () => {
    const required: Record<RetirementStrategyApi, boolean> = {
      asap: false,
      retire_at_age: true,
      coast: true,
      partial: true,
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

  it("es obligatoria en cuanto se declara una pensión, incluso en «Cuanto antes» (C5)", () => {
    // El caso real que esto tapa: un plan «Cuanto antes» con pensión y sin fecha de nacimiento se
    // guardaba tan tranquilo, y el servidor devolvía el bloque «plan» vacío con
    // `plan_absent_reason: "birth_date_missing"` — sin fecha, sin éxito y sin capital necesario,
    // sin que nada en el formulario lo hubiera anunciado.
    for (const strategy of STRATEGIES) {
      const conPension = ctx({ strategy, hasBirthDate: false, hasPension: true });
      expect(req("birth_date", conPension), strategy).toBe(true);
    }
    // Y el contraste que lo hace informativo: la MISMA estrategia sin pensión no la exige.
    expect(req("birth_date", ctx({ strategy: "asap", hasBirthDate: false }))).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("edad objetivo — obligatoria salvo en media jornada, y allí se llama distinto", () => {
  it("existe en las estrategias que la usan, y en coast solo en su modo A", () => {
    const visible: Record<RetirementStrategyApi, boolean> = {
      asap: false,
      retire_at_age: true,
      coast: true,
      partial: true,
    };
    for (const strategy of STRATEGIES) {
      expect(isFieldVisible("target_retirement_age", ctx({ strategy })), strategy).toBe(
        visible[strategy],
      );
    }
    expect(
      isFieldVisible(
        "target_retirement_age",
        ctx({ strategy: "coast", coastMode: "fixed_stop_age" }),
      ),
    ).toBe(false);
  });

  it("es obligatoria en «A una edad fija» y en «Coast FIRE» modo A, opcional en «Media jornada»", () => {
    expect(req("target_retirement_age", ctx({ strategy: "retire_at_age" }))).toBe(true);
    expect(req("target_retirement_age", ctx({ strategy: "coast" }))).toBe(true);
    for (const partialMode of PARTIAL_MODES) {
      expect(
        req("target_retirement_age", ctx({ strategy: "partial", partialMode })),
        partialMode,
      ).toBe(false);
    }
  });

  it("en media jornada se rotula «Edad de jubilación total»: no es la misma pregunta", () => {
    const label = (c: PlanFieldsContext) =>
      planFields(c).find((f) => f.id === "target_retirement_age")?.label;
    expect(label(ctx({ strategy: "partial" }))).toBe("Edad de jubilación total");
    expect(label(ctx({ strategy: "retire_at_age" }))).toBe("Edad de jubilación objetivo");
    expect(label(ctx({ strategy: "coast" }))).toBe("Edad de jubilación objetivo");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Pensión» — la renta con fecha y todo lo que solo existe con ella", () => {
  it("los dos campos base aparecen en las cuatro estrategias, con o sin pensión declarada", () => {
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        const c = ctx({ strategy, hasPension });
        expect(isFieldVisible("pension_amount", c), `${strategy}/${hasPension}`).toBe(true);
        expect(isFieldVisible("pension_start_age", c), `${strategy}/${hasPension}`).toBe(true);
      }
    }
  });

  it("son obligatorios en cuanto la pensión se declara, y en ninguna estrategia sin ella", () => {
    // Ya no hay una estrategia «Puente hasta la pensión» que los exigiera (C7). Lo que los
    // convierte en obligatorios es haber abierto el bloque: un bloque a medias entra en el bucle
    // como una renta de 0 € sin que nadie lo diga.
    for (const strategy of STRATEGIES) {
      for (const hasPension of [false, true]) {
        const c = ctx({ strategy, hasPension });
        expect(req("pension_amount", c), `${strategy}/${hasPension}`).toBe(hasPension);
        expect(req("pension_start_age", c), `${strategy}/${hasPension}`).toBe(hasPension);
      }
    }
  });

  it("sin pensión declarada la tarjeta son solo sus dos campos base", () => {
    expect(cardIds("pension", ctx({ strategy: "asap", hasPension: false }))).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("con pensión, los mandos finos caen en la MISMA tarjeta (V3, F9)", () => {
    expect(
      cardIds("pension", ctx({ strategy: "retire_at_age", hasPension: true })),
    ).toEqual([
      "pension_amount",
      "pension_start_age",
      "pension_indexed",
      "bridge_enabled",
    ]);
  });

  it("en media jornada con pensión entra además la fracción cobrada durante la fase", () => {
    expect(cardIds("pension", ctx({ strategy: "partial", hasPension: true }))).toEqual([
      "pension_amount",
      "pension_start_age",
      "pension_indexed",
      "pension_fraction_while_partial",
      "bridge_enabled",
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
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("el puente ya no es una estrategia: son tres campos de «Pensión» (C7)", () => {
  it("solo existen con pensión declarada, y sus dos números solo con el interruptor encendido", () => {
    for (const strategy of STRATEGIES) {
      for (const bridgeEnabled of [false, true]) {
        // Sin pensión no hay nada a lo que hacer de puente: ni el interruptor se pinta.
        const sinPension = ctx({ strategy, hasPension: false, bridgeEnabled });
        expect(isFieldVisible("bridge_enabled", sinPension), strategy).toBe(false);
        expect(isFieldVisible("bridge_max_pct", sinPension), strategy).toBe(false);
        expect(isFieldVisible("bridge_max_years", sinPension), strategy).toBe(false);

        const conPension = ctx({ strategy, hasPension: true, bridgeEnabled });
        expect(isFieldVisible("bridge_enabled", conPension), strategy).toBe(true);
        expect(isFieldVisible("bridge_max_pct", conPension), `${strategy}/${bridgeEnabled}`).toBe(
          bridgeEnabled,
        );
        expect(
          isFieldVisible("bridge_max_years", conPension),
          `${strategy}/${bridgeEnabled}`,
        ).toBe(bridgeEnabled);
      }
    }
  });

  it("están disponibles en CUALQUIER estrategia, no en una sola", () => {
    // Esta es la afirmación de C7 en una línea: el puente pasó de ser la quinta tarjeta del
    // selector a ser un ajuste que cualquiera puede activar si su pensión está cerca.
    const conPuente = STRATEGIES.filter((strategy) =>
      isFieldVisible("bridge_enabled", ctx({ strategy, hasPension: true })),
    );
    expect(conPuente).toEqual([...STRATEGIES]);
  });

  it("el interruptor no es obligatorio; sus dos números sí lo son cuando abre", () => {
    const on = ctx({ strategy: "asap", hasPension: true, bridgeEnabled: true });
    expect(req("bridge_enabled", on)).toBe(false);
    // El servidor los rellena con sus defaults al activarlo, pero un `null` llegado por API o por
    // MCP es un puente sin tope: eso no se puede simular como se pidió.
    expect(req("bridge_max_pct", on)).toBe(true);
    expect(req("bridge_max_years", on)).toBe(true);
  });

  it("los tres viven en la tarjeta «Pensión», detrás de los mandos de la renta", () => {
    expect(
      cardIds("pension", ctx({ strategy: "asap", hasPension: true, bridgeEnabled: true })),
    ).toEqual([
      "pension_amount",
      "pension_start_age",
      "pension_indexed",
      "bridge_enabled",
      "bridge_max_pct",
      "bridge_max_years",
    ]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("tarjeta «Gasto en jubilación» — siempre, y su importe solo en manual", () => {
  it("`fire_number_mode` está en las cuatro y nunca es obligatorio", () => {
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
    expect(req("fire_number_manual_amount", ctx({ fireNumberMode: "manual" }))).toBe(true);
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
describe("tarjeta «Retirada» — el umbral primero, y U4: UN solo porcentaje de retirada", () => {
  it("el umbral es el PRIMER campo de la tarjeta, en cualquier contexto", () => {
    // Va arriba porque en v2 es la restricción que DECIDE la fecha válida, no un corte de
    // semáforo: es el campo que más mueve el resultado de esta pantalla.
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        const c = ctx({ strategy, ruleKind, hasPension: true, bridgeEnabled: true });
        expect(cardIds("withdrawal", c)[0], `${strategy}/${ruleKind}`).toBe(
          "success_threshold_pct",
        );
      }
    }
  });

  it("el umbral existe siempre y NUNCA es obligatorio: el servidor tiene su default (95)", () => {
    for (const strategy of STRATEGIES) {
      const c = ctx({ strategy });
      expect(isFieldVisible("success_threshold_pct", c), strategy).toBe(true);
      expect(req("success_threshold_pct", c), strategy).toBe(false);
      expect(
        planFields(c).find((f) => f.id === "success_threshold_pct")?.card,
        strategy,
      ).toBe("withdrawal");
    }
  });

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
      fixed_real: ["success_threshold_pct", "swr_pct", "withdrawal_rule_kind"],
      percent_of_balance: [
        "success_threshold_pct",
        "swr_pct",
        "withdrawal_rule_kind",
        "spend_mode",
      ],
      hybrid: [
        "success_threshold_pct",
        "swr_pct",
        "withdrawal_rule_kind",
        "hybrid_end_pct",
        "spend_mode",
      ],
      guardrails: [
        "success_threshold_pct",
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
  it("aparece en las cuatro estrategias y con cualquier regla, y va el ÚLTIMO", () => {
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
describe("lo que el modelo v2 devolvió y lo que sigue sin volver", () => {
  it("`success_threshold_pct` VUELVE: existe en todos los contextos (C3)", () => {
    // El `describe` anterior afirmaba lo contrario, y tenía razón mientras el umbral era un corte
    // de semáforo fijo al 100 % (V7). En v2 es la restricción que decide la fecha válida y es del
    // usuario otra vez: 80–100, default 95.
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        for (const hasPension of [false, true]) {
          const list = ids(ctx({ strategy, ruleKind, hasPension })) as string[];
          expect(list, `${strategy}/${ruleKind}`).toContain("success_threshold_pct");
        }
      }
    }
  });

  it("`cash_buffer_months` sigue sin salir en NINGÚN contexto (V6/M6)", () => {
    // Aquí no hubo indulto: M6 retiró el COLCHÓN como mecanismo del motor, no solo su campo.
    for (const strategy of STRATEGIES) {
      for (const ruleKind of RULE_KINDS) {
        for (const hasPension of [false, true]) {
          const list = ids(ctx({ strategy, ruleKind, hasPension })) as string[];
          expect(list, `${strategy}/${ruleKind}`).not.toContain("cash_buffer_months");
        }
      }
    }
  });

  it("`target_basis` y `bridge_discount_basis` murieron con el objetivo (C1/M4)", () => {
    for (const c of everyContext()) {
      const list = ids(c) as string[];
      expect(list, JSON.stringify(c)).not.toContain("target_basis");
      expect(list, JSON.stringify(c)).not.toContain("bridge_discount_basis");
    }
  });

  it("tampoco existe una tarjeta «Riesgo»: el umbral vive en «Retirada», junto a la tasa", () => {
    expect(PLAN_CARD_ORDER as readonly string[]).not.toContain("risk");
    expect(
      planFields(ctx()).find((f) => f.id === "success_threshold_pct")?.card,
    ).toBe("withdrawal");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
/** 4 estrategias × 2 pensiones × 2 fechas de nacimiento × 4 reglas × 2 modos de coast × 2 de
 *  media jornada × 2 del puente × 3 modos de gasto = 1.536 contextos. */
function* everyContext(): Generator<PlanFieldsContext> {
  for (const strategy of STRATEGIES) {
    for (const hasPension of [false, true]) {
      for (const hasBirthDate of [false, true]) {
        for (const ruleKind of RULE_KINDS) {
          for (const coastMode of COAST_MODES) {
            for (const partialMode of PARTIAL_MODES) {
              for (const bridgeEnabled of [false, true]) {
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
                    coastMode,
                    partialMode,
                    bridgeEnabled,
                    fireNumberMode,
                  });
                }
              }
            }
          }
        }
      }
    }
  }
}

describe("invariantes sobre TODO el producto cartesiano", () => {
  it("`isFieldVisible` coincide con `planFields` para los 25 ids en todos los contextos", () => {
    for (const c of everyContext()) {
      const list = new Set(planFields(c).map((f) => f.id));
      for (const id of ALL_FIELD_IDS) {
        expect(isFieldVisible(id, c), `${id} @ ${JSON.stringify(c)}`).toBe(list.has(id));
      }
    }
  });

  it("ningún id se repite, y todos salen del catálogo cerrado de 25", () => {
    expect(new Set(ALL_FIELD_IDS).size).toBe(25);
    const known = new Set<string>(ALL_FIELD_IDS);
    for (const c of everyContext()) {
      const list = planFields(c).map((f) => f.id);
      expect(new Set(list).size, JSON.stringify(c)).toBe(list.length);
      for (const id of list) expect(known.has(id), `${id} no está en ALL_FIELD_IDS`).toBe(true);
    }
  });

  it("los 25 ids del catálogo son alcanzables: ninguno es letra muerta", () => {
    // El reverso del test anterior. Un id que la unión declara pero la tabla nunca produce es un
    // token huérfano que «parece vivo» y acaba usándose para otra cosa (precedente: `--proj-jub`).
    const seen = new Set<string>();
    for (const c of everyContext()) for (const f of planFields(c)) seen.add(f.id);
    expect([...ALL_FIELD_IDS].filter((id) => !seen.has(id))).toEqual([]);
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
          // Su contenido es el radiogroup de las cuatro estrategias, no campos de la tabla.
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

  it("las dos edades de coast y los dos caminos de la fase son EXCLUYENTES, nunca ambos", () => {
    // La regresión que esto caza es la peor de todas: pintar las dos edades a la vez deja al
    // usuario eligiendo dos cosas de las que la simulación solo mira una.
    for (const c of everyContext()) {
      const list = new Set(ids(c));
      expect(
        list.has("target_retirement_age") && list.has("coast_stop_age"),
        JSON.stringify(c),
      ).toBe(false);
      if (c.strategy !== "partial") {
        expect(list.has("partial_start_age"), JSON.stringify(c)).toBe(false);
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
  const rest: Omit<PlanFieldsContext, "strategy"> = {
    hasPension: false,
    hasBirthDate: true,
    ruleKind: "fixed_real",
    fireNumberMode: "annual_expense",
    coastMode: "fixed_retirement_age",
    partialMode: "at_age",
    bridgeEnabled: false,
  };

  it("«Cuanto antes» no exige nada", () => {
    expect(requiredPlanFields("asap", rest)).toEqual([]);
  });

  it("«A una edad fija» exige la edad objetivo", () => {
    expect(requiredPlanFields("retire_at_age", rest)).toEqual(["target_retirement_age"]);
  });

  it("«Coast FIRE» exige una edad U OTRA según su modo, nunca las dos", () => {
    expect(requiredPlanFields("coast", rest)).toEqual(["target_retirement_age"]);
    expect(
      requiredPlanFields("coast", { ...rest, coastMode: "fixed_stop_age" }),
    ).toEqual(["coast_stop_age"]);
  });

  it("«Media jornada» exige la fase, y en modo «en cuanto pueda» solo el ingreso", () => {
    expect(requiredPlanFields("partial", rest)).toEqual([
      "partial_start_age",
      "partial_income",
    ]);
    expect(requiredPlanFields("partial", { ...rest, partialMode: "asap" })).toEqual([
      "partial_income",
    ]);
  });

  it("declarar una pensión exige su importe y su edad en cualquier estrategia", () => {
    expect(requiredPlanFields("asap", { ...rest, hasPension: true })).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("activar el puente añade sus dos números", () => {
    expect(
      requiredPlanFields("asap", { ...rest, hasPension: true, bridgeEnabled: true }),
    ).toEqual([
      "pension_amount",
      "pension_start_age",
      "bridge_max_pct",
      "bridge_max_years",
    ]);
  });

  it("sin fecha de nacimiento, las estrategias por edad la añaden la PRIMERA", () => {
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
    // …y «Cuanto antes» SIN pensión no: se simula igual.
    expect(requiredPlanFields("asap", noBirth)).toEqual([]);
    // …pero CON pensión sí (C5), y también la primera.
    expect(requiredPlanFields("asap", { ...noBirth, hasPension: true })).toEqual([
      "birth_date",
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
  /** Perfil v2 COMPLETO, escrito a mano: ver el docblock del fichero. */
  function profile(over: Partial<RetirementProfileApi> = {}): RetirementProfileApi {
    return {
      strategy: "asap",
      target_retirement_age: null,
      fire_number_mode: "annual_expense",
      fire_number_manual_amount: null,
      swr_pct: "3.5",
      horizon_lifespan_age: 90,
      success_threshold_pct: 95,
      coast_mode: "fixed_retirement_age",
      coast_stop_age: null,
      withdrawal_rule: {
        kind: "fixed_real",
        pct: null,
        start_pct: null,
        end_pct: null,
        band_pct: null,
        adjust_pct: null,
        spend_mode: "ceiling",
      },
      pension: null,
      partial_retirement: null,
      ...over,
    };
  }

  const pension = (over: Partial<NonNullable<RetirementProfileApi["pension"]>> = {}) => ({
    monthly_amount_today: "1200",
    starts_at_age: 67,
    indexed: true,
    fraction_while_partial: "0",
    bridge_enabled: false,
    bridge_max_pct: null,
    bridge_max_years: null,
    ...over,
  });

  const partial = (over: Partial<PartialRetirementApi> = {}): PartialRetirementApi => ({
    mode: "at_age",
    starts_at_age: 60,
    income_monthly_today: "800",
    expense_basis: "retirement",
    ...over,
  });

  it("`hasPension` es «hay bloque declarado», no «la estrategia lo permitiría»", () => {
    expect(planFieldsContextFromProfile(profile(), true).hasPension).toBe(false);
    expect(
      planFieldsContextFromProfile(profile({ pension: pension() }), true).hasPension,
    ).toBe(true);
  });

  it("el modo de coast se lee de la RAÍZ del perfil, donde el wire lo pone", () => {
    expect(planFieldsContextFromProfile(profile(), true).coastMode).toBe(
      "fixed_retirement_age",
    );
    expect(
      planFieldsContextFromProfile(
        profile({ strategy: "coast", coast_mode: "fixed_stop_age", coast_stop_age: 50 }),
        true,
      ).coastMode,
    ).toBe("fixed_stop_age");
  });

  it("el modo de la fase parcial vive DENTRO del bloque, y sin bloque cae en su modo A", () => {
    // Sin fase no hay modo que leer; el `at_age` de ese caso es inerte, porque la tabla solo mira
    // `partialMode` con la estrategia `partial`, que siempre trae su bloque.
    expect(planFieldsContextFromProfile(profile(), true).partialMode).toBe("at_age");
    expect(
      planFieldsContextFromProfile(
        profile({
          strategy: "partial",
          partial_retirement: partial({ mode: "asap", starts_at_age: null }),
        }),
        true,
      ).partialMode,
    ).toBe("asap");
  });

  it("el puente se lee del bloque de pensión, y sin pensión está apagado", () => {
    expect(planFieldsContextFromProfile(profile(), true).bridgeEnabled).toBe(false);
    expect(
      planFieldsContextFromProfile(profile({ pension: pension() }), true).bridgeEnabled,
    ).toBe(false);
    expect(
      planFieldsContextFromProfile(
        profile({
          pension: pension({
            bridge_enabled: true,
            bridge_max_pct: "8",
            bridge_max_years: 7,
          }),
        }),
        true,
      ).bridgeEnabled,
    ).toBe(true);
  });

  it("copia la regla, el modo del gasto y la fecha de nacimiento sin tocarlos", () => {
    const p = profile({
      strategy: "partial",
      partial_retirement: partial(),
      fire_number_mode: "manual",
      withdrawal_rule: { ...profile().withdrawal_rule, kind: "guardrails" },
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
      "success_threshold_pct",
      "swr_pct",
      "withdrawal_rule_kind",
      "horizon_lifespan_age",
    ]);
  });

  it("un perfil de coast en modo B produce la edad de parada, no la de jubilación", () => {
    const c = planFieldsContextFromProfile(
      profile({ strategy: "coast", coast_mode: "fixed_stop_age", coast_stop_age: 45 }),
      true,
    );
    expect(ids(c)).toContain("coast_stop_age");
    expect(ids(c)).not.toContain("target_retirement_age");
  });
});
