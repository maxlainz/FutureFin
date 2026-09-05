/**
 * Paso «Tu plan» del asistente de primera vez (5.0.0, decisión U8, issue #207).
 *
 * Tres cosas que esta suite existe para impedir, todas silenciosas si se rompen:
 *
 *  1. **`onboardingPlanFields` divergiendo de `requiredPlanFields`.** Si alguien cambia la tabla
 *     de `lib/plan-fields.ts` (U2/U12) sin tocar este fichero, el asistente de alta y la línea de
 *     supuestos de Jubilación empezarían a pedir cosas distintas para la misma estrategia.
 *  2. **Un PATCH que se cuela con `withdrawal_rule`, con el bloque de la estrategia contraria, o
 *     con una edad/decimal a medio teclear.** El servidor lo rechazaría con 400, pero el usuario
 *     vería «No se ha podido guardar» sin saber por qué — la guarda tiene que atraparlo antes.
 *  3. **Una cota que se mueve en `retirementProfile.ts` (`MIN_PROFILE_AGE`,
 *     `MAX_HORIZON_LIFESPAN_AGE`, `MIN_PENSION_AGE`) sin que este formulario se entere.** Las
 *     constantes se IMPORTAN, nunca se copian a mano, precisamente para que un cambio ahí mueva
 *     también el mensaje y el rango aceptado aquí.
 */

import { describe, expect, it } from "vitest";
import type { RetirementProfilePatchApi, RetirementStrategyApi } from "../api/types";
import {
  buildOnboardingPlanPatch,
  emptyOnboardingPlanState,
  onboardingPlanFields,
  strategyNeedsBirthDate,
  validateOnboardingPlan,
  type OnboardingPlanState,
} from "./onboarding-plan";
import {
  MAX_HORIZON_LIFESPAN_AGE,
  MIN_PENSION_AGE,
  MIN_PROFILE_AGE,
} from "./retirementProfile";

const STRATEGIES: readonly RetirementStrategyApi[] = [
  "asap",
  "retire_at_age",
  "coast",
  "partial",
  "pension_bridge",
];

function fieldIds(strategy: RetirementStrategyApi): string[] {
  return onboardingPlanFields(strategy).map((f) => f.id);
}

function codes(state: OnboardingPlanState): string[] {
  return validateOnboardingPlan(state).map((i) => i.code);
}

function fieldsWithIssues(state: OnboardingPlanState): string[] {
  return validateOnboardingPlan(state).map((i) => i.field);
}

/** Un estado válido para cada estrategia — punto de partida de los tests de "invalid". */
function validState(strategy: RetirementStrategyApi): OnboardingPlanState {
  const base = { ...emptyOnboardingPlanState(), strategy, birthDate: "1990-05-20" };
  switch (strategy) {
    case "asap":
      return { ...base, birthDate: "" }; // asap no exige fecha de nacimiento
    case "retire_at_age":
    case "coast":
      return { ...base, targetRetirementAge: "55" };
    case "partial":
      return { ...base, partialStartAge: "55", partialIncome: "800" };
    case "pension_bridge":
      return { ...base, birthDate: "", pensionAmount: "1200", pensionStartAge: "67" };
  }
}

describe("onboardingPlanFields — envoltorio de requiredPlanFields (U2/U12)", () => {
  it("asap no pide ningún esencial", () => {
    expect(fieldIds("asap")).toEqual([]);
  });

  it("retire_at_age pide solo la edad objetivo", () => {
    expect(fieldIds("retire_at_age")).toEqual(["target_retirement_age"]);
  });

  it("coast pide solo la edad objetivo", () => {
    expect(fieldIds("coast")).toEqual(["target_retirement_age"]);
  });

  it("partial pide la edad de inicio y el ingreso, NO la edad total (es opcional ahí)", () => {
    expect(fieldIds("partial")).toEqual(["partial_start_age", "partial_income"]);
  });

  it("pension_bridge pide el importe y la edad de la pensión", () => {
    expect(fieldIds("pension_bridge")).toEqual(["pension_amount", "pension_start_age"]);
  });

  it("cada descriptor trae su rótulo canónico, no un id pelado", () => {
    for (const f of onboardingPlanFields("pension_bridge")) {
      expect(f.label.length).toBeGreaterThan(0);
      // Los esenciales viven en las tarjetas que se pueden dejar a medias; un supuesto con
      // default del servidor nunca es obligatorio y por tanto nunca llega aquí (V3).
      expect(["ages", "pension", "spending"]).toContain(f.card);
      expect(f.required).toBe(true);
    }
  });

  it("las 5 estrategias tienen una entrada — ninguna hace que la tabla lance", () => {
    for (const s of STRATEGIES) {
      expect(() => onboardingPlanFields(s)).not.toThrow();
    }
  });
});

describe("strategyNeedsBirthDate", () => {
  it("retire_at_age, coast y partial la necesitan", () => {
    expect(strategyNeedsBirthDate("retire_at_age")).toBe(true);
    expect(strategyNeedsBirthDate("coast")).toBe(true);
    expect(strategyNeedsBirthDate("partial")).toBe(true);
  });

  it("asap y pension_bridge no la necesitan (se disparan por cruce o por la pensión)", () => {
    expect(strategyNeedsBirthDate("asap")).toBe(false);
    expect(strategyNeedsBirthDate("pension_bridge")).toBe(false);
  });
});

describe("validateOnboardingPlan — un estado válido por estrategia no tiene problemas", () => {
  for (const s of STRATEGIES) {
    it(`${s}`, () => {
      expect(validateOnboardingPlan(validState(s))).toEqual([]);
    });
  }
});

describe("validateOnboardingPlan — fecha de nacimiento", () => {
  it("vacía + estrategia que la necesita ⇒ birth_date_required", () => {
    const s = validState("retire_at_age");
    expect(codes({ ...s, birthDate: "" })).toContain("birth_date_required");
  });

  it("vacía + estrategia que NO la necesita ⇒ sin problema", () => {
    expect(validateOnboardingPlan(validState("asap"))).toEqual([]);
    expect(validateOnboardingPlan(validState("pension_bridge"))).toEqual([]);
  });

  it("formato inválido ⇒ birth_date_format", () => {
    const s = validState("asap");
    expect(codes({ ...s, birthDate: "20-05-1990" })).toContain("birth_date_format");
  });

  it("fecha de calendario inexistente (31 de abril) ⇒ birth_date_format", () => {
    const s = validState("asap");
    expect(codes({ ...s, birthDate: "1990-04-31" })).toContain("birth_date_format");
  });

  it("año anterior a 1900 ⇒ birth_date_too_old", () => {
    const s = validState("asap");
    expect(codes({ ...s, birthDate: "1899-12-31" })).toContain("birth_date_too_old");
  });

  it("fecha futura ⇒ birth_date_future", () => {
    const s = validState("asap");
    expect(codes({ ...s, birthDate: "2999-01-01" })).toContain("birth_date_future");
  });

  it("hoy mismo es una fecha de nacimiento válida (frontera inclusiva, como el servidor)", () => {
    const today = new Date().toISOString().slice(0, 10);
    const s = validState("asap");
    expect(validateOnboardingPlan({ ...s, birthDate: today })).toEqual([]);
  });

  it("se ofrece igualmente en asap/pension_bridge y, si se rellena mal, se valida igual", () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, birthDate: "no-es-una-fecha" })).toContain("birth_date_format");
  });
});

describe("validateOnboardingPlan — retire_at_age / coast (edad objetivo)", () => {
  for (const strategy of ["retire_at_age", "coast"] as const) {
    it(`${strategy}: vacía ⇒ target_retirement_age_required`, () => {
      const s = validState(strategy);
      expect(codes({ ...s, targetRetirementAge: "" })).toContain(
        "target_retirement_age_required",
      );
    });

    it(`${strategy}: por debajo de ${MIN_PROFILE_AGE} ⇒ retirement_age_out_of_range`, () => {
      const s = validState(strategy);
      expect(
        codes({ ...s, targetRetirementAge: String(MIN_PROFILE_AGE - 1) }),
      ).toContain("retirement_age_out_of_range");
    });

    it(`${strategy}: por encima de ${MAX_HORIZON_LIFESPAN_AGE} ⇒ retirement_age_out_of_range`, () => {
      const s = validState(strategy);
      expect(
        codes({ ...s, targetRetirementAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) }),
      ).toContain("retirement_age_out_of_range");
    });

    it(`${strategy}: los dos extremos (${MIN_PROFILE_AGE} y ${MAX_HORIZON_LIFESPAN_AGE}) son válidos`, () => {
      const s = validState(strategy);
      expect(
        validateOnboardingPlan({ ...s, targetRetirementAge: String(MIN_PROFILE_AGE) }),
      ).toEqual([]);
      expect(
        validateOnboardingPlan({
          ...s,
          targetRetirementAge: String(MAX_HORIZON_LIFESPAN_AGE),
        }),
      ).toEqual([]);
    });

    it(`${strategy}: decimales no son una edad ⇒ target_retirement_age_required`, () => {
      const s = validState(strategy);
      expect(codes({ ...s, targetRetirementAge: "55,5" })).toContain(
        "target_retirement_age_required",
      );
    });
  }
});

describe("validateOnboardingPlan — partial (media jornada)", () => {
  it("edad de inicio vacía ⇒ partial_age_out_of_range", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialStartAge: "" })).toContain("partial_age_out_of_range");
  });

  it("edad fuera de rango ⇒ partial_age_out_of_range", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialStartAge: String(MIN_PROFILE_AGE - 1) })).toContain(
      "partial_age_out_of_range",
    );
    expect(
      codes({ ...s, partialStartAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) }),
    ).toContain("partial_age_out_of_range");
  });

  it("ingreso vacío ⇒ partial_income_not_positive (aquí NO se admite el año sabático a 0)", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialIncome: "" })).toContain("partial_income_not_positive");
  });

  it("ingreso 0 ⇒ partial_income_not_positive", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialIncome: "0" })).toContain("partial_income_not_positive");
  });

  it("ingreso negativo ⇒ partial_income_not_positive (parsea bien, pero no es positivo)", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialIncome: "-100" })).toContain("partial_income_not_positive");
  });

  it("ingreso no numérico ⇒ decimal_invalid", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialIncome: "mil euros" })).toContain("decimal_invalid");
  });

  it("ingreso con coma decimal es válido (es-ES)", () => {
    const s = validState("partial");
    expect(validateOnboardingPlan({ ...s, partialIncome: "800,50" })).toEqual([]);
  });

  it("edad de media jornada >= edad total (si la total está escrita) ⇒ partial_not_before_retirement", () => {
    const s = validState("partial");
    expect(
      codes({ ...s, partialStartAge: "60", targetRetirementAge: "60" }),
    ).toContain("partial_not_before_retirement");
    expect(
      codes({ ...s, partialStartAge: "61", targetRetirementAge: "60" }),
    ).toContain("partial_not_before_retirement");
  });

  it("edad de media jornada < edad total ⇒ sin problema de coherencia", () => {
    const s = validState("partial");
    expect(
      validateOnboardingPlan({ ...s, partialStartAge: "55", targetRetirementAge: "60" }),
    ).toEqual([]);
  });

  it("sin edad total escrita (el caso normal: no se pregunta en `partial`) no hay coherencia que comprobar", () => {
    const s = validState("partial");
    expect(s.targetRetirementAge).toBe("");
    expect(validateOnboardingPlan(s)).toEqual([]);
  });
});

describe("validateOnboardingPlan — pension_bridge (puente hasta la pensión)", () => {
  it("importe vacío ⇒ pension_amount_not_positive (vacío no es decimal_invalid)", () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, pensionAmount: "" })).toContain("pension_amount_not_positive");
  });

  it("importe 0 o negativo ⇒ pension_amount_not_positive", () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, pensionAmount: "0" })).toContain("pension_amount_not_positive");
  });

  it("importe no numérico ⇒ decimal_invalid", () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, pensionAmount: "mucho" })).toContain("decimal_invalid");
  });

  it(`edad por debajo de ${MIN_PENSION_AGE} ⇒ pension_age_out_of_range`, () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, pensionStartAge: String(MIN_PENSION_AGE - 1) })).toContain(
      "pension_age_out_of_range",
    );
  });

  it(`edad por encima de ${MAX_HORIZON_LIFESPAN_AGE} ⇒ pension_age_out_of_range`, () => {
    const s = validState("pension_bridge");
    expect(
      codes({ ...s, pensionStartAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) }),
    ).toContain("pension_age_out_of_range");
  });

  it(`los dos extremos (${MIN_PENSION_AGE} y ${MAX_HORIZON_LIFESPAN_AGE}) son válidos`, () => {
    const s = validState("pension_bridge");
    expect(
      validateOnboardingPlan({ ...s, pensionStartAge: String(MIN_PENSION_AGE) }),
    ).toEqual([]);
    expect(
      validateOnboardingPlan({
        ...s,
        pensionStartAge: String(MAX_HORIZON_LIFESPAN_AGE),
      }),
    ).toEqual([]);
  });

  it("edad vacía ⇒ pension_age_out_of_range (no hay código 'required' separado)", () => {
    const s = validState("pension_bridge");
    expect(codes({ ...s, pensionStartAge: "" })).toContain("pension_age_out_of_range");
  });
});

describe("validateOnboardingPlan — asap no valida nada de lo que no pregunta", () => {
  it("basura en los campos que asap no pinta no genera ningún problema", () => {
    const s = validState("asap");
    expect(
      validateOnboardingPlan({
        ...s,
        targetRetirementAge: "no-es-una-edad",
        partialStartAge: "-3",
        partialIncome: "no-es-dinero",
        pensionAmount: "-1",
        pensionStartAge: "3",
      }),
    ).toEqual([]);
  });
});

describe("validateOnboardingPlan — cambiar de estrategia no arrastra el campo del `field` equivocado", () => {
  it("el mismo estado inválido para `partial` no molesta si la estrategia activa es asap", () => {
    const messy: OnboardingPlanState = {
      ...emptyOnboardingPlanState(),
      strategy: "asap",
      partialStartAge: "5",
      partialIncome: "",
    };
    expect(fieldsWithIssues(messy)).toEqual([]);
  });
});

describe("buildOnboardingPlanPatch — cuerpo exacto por estrategia", () => {
  it("asap: sin fecha de nacimiento ⇒ solo strategy", () => {
    const patch = buildOnboardingPlanPatch(validState("asap"));
    expect(patch).toEqual<RetirementProfilePatchApi>({ strategy: "asap" });
  });

  it("asap: con fecha de nacimiento escrita, viaja igual", () => {
    const patch = buildOnboardingPlanPatch({
      ...validState("asap"),
      birthDate: "1985-01-01",
    });
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "asap",
      birth_date: "1985-01-01",
    });
  });

  it("retire_at_age: strategy + birth_date + target_retirement_age, nada más", () => {
    const patch = buildOnboardingPlanPatch(validState("retire_at_age"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "retire_at_age",
      birth_date: "1990-05-20",
      target_retirement_age: 55,
    });
  });

  it("coast: mismo cuerpo que retire_at_age salvo la estrategia", () => {
    const patch = buildOnboardingPlanPatch(validState("coast"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "coast",
      birth_date: "1990-05-20",
      target_retirement_age: 55,
    });
  });

  it("partial: strategy + birth_date + partial_retirement completo, sin pension ni target_retirement_age", () => {
    const patch = buildOnboardingPlanPatch(validState("partial"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "partial",
      birth_date: "1990-05-20",
      partial_retirement: {
        starts_at_age: 55,
        income_monthly_today: "800",
        expense_basis: "retirement",
      },
    });
  });

  it("pension_bridge: strategy + pension completo, sin birth_date (no se escribió) ni partial_retirement", () => {
    const patch = buildOnboardingPlanPatch(validState("pension_bridge"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "pension_bridge",
      pension: {
        monthly_amount_today: "1200",
        starts_at_age: 67,
        indexed: true,
        fraction_while_partial: "0",
      },
    });
  });

  it("nunca incluye withdrawal_rule — ni para partial ni para pension_bridge", () => {
    for (const s of STRATEGIES) {
      const patch = buildOnboardingPlanPatch(validState(s));
      expect(patch).not.toHaveProperty("withdrawal_rule");
    }
  });

  it("nunca incluye swr_pct, horizon_lifespan_age ni fire_number_mode — ejes que este paso no toca", () => {
    for (const s of STRATEGIES) {
      const patch = buildOnboardingPlanPatch(validState(s));
      expect(patch).not.toHaveProperty("swr_pct");
      expect(patch).not.toHaveProperty("horizon_lifespan_age");
      expect(patch).not.toHaveProperty("fire_number_mode");
      expect(patch).not.toHaveProperty("target_basis");
      expect(patch).not.toHaveProperty("bridge_discount_basis");
      expect(patch).not.toHaveProperty("cash_buffer_months");
      expect(patch).not.toHaveProperty("success_threshold_pct");
    }
  });

  it("partial y pension_bridge son mutuamente excluyentes en el cuerpo", () => {
    const partialPatch = buildOnboardingPlanPatch(validState("partial"));
    expect(partialPatch).not.toHaveProperty("pension");
    const bridgePatch = buildOnboardingPlanPatch(validState("pension_bridge"));
    expect(bridgePatch).not.toHaveProperty("partial_retirement");
  });

  it("el importe con coma decimal se normaliza a decimal-string de la API (punto)", () => {
    const patch = buildOnboardingPlanPatch({
      ...validState("pension_bridge"),
      pensionAmount: "1.234,5",
    });
    expect(patch.pension?.monthly_amount_today).toBe("1234.5");
  });
});
