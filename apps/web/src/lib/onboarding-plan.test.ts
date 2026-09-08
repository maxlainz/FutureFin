/**
 * Paso «Tu plan» del asistente de primera vez (5.0.0, decisión U8, issue #207; reescrito para el
 * modelo v2, C1–C8).
 *
 * Cuatro cosas que esta suite existe para impedir, todas silenciosas si se rompen:
 *
 *  1. **`onboardingPlanFields` divergiendo de `requiredPlanFields`.** Si alguien cambia la tabla
 *     de `lib/plan-fields.ts` (U2/U12) sin tocar este fichero, el asistente de alta y el
 *     formulario de Jubilación empezarían a pedir cosas distintas para la misma estrategia.
 *  2. **Un MODO que no llega al PATCH.** Con el modelo v2, `coast` y `partial` tienen dos modos
 *     cada una (M10/M11) y cada uno pide una edad distinta. Un asistente que recoja el modo y no
 *     lo mande deja al servidor resolviendo el plan por el modo por defecto, con la edad del otro
 *     — y el usuario ve un plan que no es el que pidió, sin ningún error de por medio.
 *  3. **Un PATCH que se cuela con `withdrawal_rule`, con el bloque de la estrategia contraria, o
 *     con una edad/decimal a medio teclear.** El servidor lo rechazaría con 400, pero el usuario
 *     vería «No se ha podido guardar» sin saber por qué — la guarda tiene que atraparlo antes.
 *  4. **Una cota que se mueve en `retirementProfile.ts` (`MIN_PROFILE_AGE`,
 *     `MAX_HORIZON_LIFESPAN_AGE`, `MIN_PENSION_AGE`) sin que este formulario se entere.** Las
 *     constantes se IMPORTAN, nunca se copian a mano, precisamente para que un cambio ahí mueva
 *     también el mensaje y el rango aceptado aquí.
 *
 * **`pension_bridge` ya no es una estrategia** (C7): el selector tiene CUATRO tarjetas y el puente
 * es un ajuste de la tarjeta Pensión, que este paso no ofrece. Lo que sí sobrevive de aquel caso
 * es la pensión misma: si el borrador la trae, sus dos campos son obligatorios y —C5— la fecha de
 * nacimiento pasa a serlo también.
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

/** Las CUATRO del modelo v2. Se listan contra la unión del wire para que el compilador caiga
 *  sobre este array si nace o muere una estrategia. */
const STRATEGIES: readonly RetirementStrategyApi[] = [
  "asap",
  "retire_at_age",
  "coast",
  "partial",
];

function fieldIds(
  strategy: RetirementStrategyApi,
  ctx?: Parameters<typeof onboardingPlanFields>[1],
): string[] {
  return onboardingPlanFields(strategy, ctx).map((f) => f.id);
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
      return { ...base, birthDate: "" }; // asap sin pensión no exige fecha de nacimiento
    case "retire_at_age":
    case "coast":
      return { ...base, targetRetirementAge: "55" };
    case "partial":
      return { ...base, partialStartAge: "55", partialIncome: "800" };
  }
}

/** Coast en su modo B: fijo la edad a la que dejo de aportar y la fecha sale donde salga. */
const coastModeB = (over: Partial<OnboardingPlanState> = {}): OnboardingPlanState => ({
  ...validState("coast"),
  coastMode: "fixed_stop_age",
  targetRetirementAge: "",
  coastStopAge: "45",
  ...over,
});

/** Media jornada en su modo B: empiezo en cuanto pueda, y la edad la resuelve el servidor. */
const partialModeB = (over: Partial<OnboardingPlanState> = {}): OnboardingPlanState => ({
  ...validState("partial"),
  partialMode: "asap",
  partialStartAge: "",
  ...over,
});

/** Un borrador con pensión declarada, en la estrategia que sea. */
const withPension = (
  strategy: RetirementStrategyApi = "asap",
  over: Partial<OnboardingPlanState> = {},
): OnboardingPlanState => ({
  ...validState(strategy),
  birthDate: "1990-05-20",
  pensionAmount: "1200",
  pensionStartAge: "67",
  ...over,
});

describe("onboardingPlanFields — envoltorio de requiredPlanFields (U2/U12)", () => {
  it("asap no pide ningún esencial", () => {
    expect(fieldIds("asap")).toEqual([]);
  });

  it("retire_at_age pide solo la edad objetivo", () => {
    expect(fieldIds("retire_at_age")).toEqual(["target_retirement_age"]);
  });

  it("coast modo A pide la edad de jubilación; modo B, la de parada — nunca las dos", () => {
    expect(fieldIds("coast")).toEqual(["target_retirement_age"]);
    expect(fieldIds("coast", { coastMode: "fixed_retirement_age" })).toEqual([
      "target_retirement_age",
    ]);
    expect(fieldIds("coast", { coastMode: "fixed_stop_age" })).toEqual(["coast_stop_age"]);
  });

  it("partial modo A pide la edad de inicio y el ingreso; modo B, solo el ingreso", () => {
    expect(fieldIds("partial")).toEqual(["partial_start_age", "partial_income"]);
    expect(fieldIds("partial", { partialMode: "asap" })).toEqual(["partial_income"]);
  });

  it("una pensión declarada añade sus dos campos en cualquier estrategia (C7)", () => {
    expect(fieldIds("asap", { hasPension: true })).toEqual([
      "pension_amount",
      "pension_start_age",
    ]);
    expect(fieldIds("retire_at_age", { hasPension: true })).toEqual([
      "target_retirement_age",
      "pension_amount",
      "pension_start_age",
    ]);
  });

  it("sin contexto, los defaults son los del servidor: modo A en las dos y sin pensión", () => {
    for (const s of STRATEGIES) {
      expect(fieldIds(s), s).toEqual(
        fieldIds(s, {
          coastMode: "fixed_retirement_age",
          partialMode: "at_age",
          hasPension: false,
        }),
      );
    }
  });

  it("cada descriptor trae su rótulo canónico, no un id pelado", () => {
    for (const f of onboardingPlanFields("partial")) {
      expect(f.label.length).toBeGreaterThan(0);
      // Los esenciales viven en las tarjetas que se pueden dejar a medias; un supuesto con
      // default del servidor nunca es obligatorio y por tanto nunca llega aquí (V3).
      expect(["ages", "pension", "spending"]).toContain(f.card);
      expect(f.required).toBe(true);
    }
  });

  it("las 4 estrategias tienen una entrada — ninguna hace que la tabla lance", () => {
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

  it("asap sin pensión no la necesita (se jubila por el sorteo, no por una edad)", () => {
    expect(strategyNeedsBirthDate("asap")).toBe(false);
    expect(strategyNeedsBirthDate("asap", false)).toBe(false);
  });

  it("con pensión declarada la necesitan TODAS, asap incluida (C5)", () => {
    // Sin fecha de nacimiento el servidor no sabe si la pensión ya se cobra en la fecha válida y
    // devuelve el bloque «plan» vacío (`plan_absent_reason: "birth_date_missing"`): ni fecha, ni
    // éxito, ni capital necesario.
    for (const s of STRATEGIES) {
      expect(strategyNeedsBirthDate(s, true), s).toBe(true);
    }
  });
});

describe("validateOnboardingPlan — un estado válido por estrategia y modo no tiene problemas", () => {
  for (const s of STRATEGIES) {
    it(`${s}`, () => {
      expect(validateOnboardingPlan(validState(s))).toEqual([]);
    });
  }

  it("coast modo B", () => {
    expect(validateOnboardingPlan(coastModeB())).toEqual([]);
  });

  it("partial modo «en cuanto pueda»", () => {
    expect(validateOnboardingPlan(partialModeB())).toEqual([]);
  });

  it("asap con pensión declarada y fecha de nacimiento", () => {
    expect(validateOnboardingPlan(withPension())).toEqual([]);
  });
});

describe("validateOnboardingPlan — fecha de nacimiento", () => {
  it("vacía + estrategia que la necesita ⇒ birth_date_required", () => {
    const s = validState("retire_at_age");
    expect(codes({ ...s, birthDate: "" })).toContain("birth_date_required");
  });

  it("vacía + asap SIN pensión ⇒ sin problema", () => {
    expect(validateOnboardingPlan(validState("asap"))).toEqual([]);
  });

  it("vacía + asap CON pensión ⇒ birth_date_required (C5)", () => {
    expect(codes(withPension("asap", { birthDate: "" }))).toContain("birth_date_required");
  });

  it("basta con medio bloque de pensión para que haga falta: media pensión es una pensión", () => {
    const soloElImporte: OnboardingPlanState = {
      ...validState("asap"),
      birthDate: "",
      pensionAmount: "1200",
    };
    expect(codes(soloElImporte)).toContain("birth_date_required");
    const soloLaEdad: OnboardingPlanState = {
      ...validState("asap"),
      birthDate: "",
      pensionStartAge: "67",
    };
    expect(codes(soloLaEdad)).toContain("birth_date_required");
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

  it("se ofrece igualmente en asap y, si se rellena mal, se valida igual", () => {
    const s = validState("asap");
    expect(codes({ ...s, birthDate: "no-es-una-fecha" })).toContain("birth_date_format");
  });
});

describe("validateOnboardingPlan — retire_at_age / coast modo A (edad objetivo)", () => {
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

describe("validateOnboardingPlan — coast modo B (edad de parada, M10)", () => {
  it("vacía ⇒ coast_stop_age_required, el mismo código que el servidor", () => {
    expect(codes(coastModeB({ coastStopAge: "" }))).toContain("coast_stop_age_required");
  });

  it("a medio teclear tampoco es una edad ⇒ coast_stop_age_required", () => {
    expect(codes(coastModeB({ coastStopAge: "cuarenta" }))).toContain(
      "coast_stop_age_required",
    );
    expect(codes(coastModeB({ coastStopAge: "45,5" }))).toContain("coast_stop_age_required");
  });

  it("fuera de rango ⇒ coast_stop_age_out_of_range (código distinto: el dato ESTÁ)", () => {
    expect(codes(coastModeB({ coastStopAge: String(MIN_PROFILE_AGE - 1) }))).toContain(
      "coast_stop_age_out_of_range",
    );
    expect(
      codes(coastModeB({ coastStopAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) })),
    ).toContain("coast_stop_age_out_of_range");
  });

  it("los dos extremos son válidos", () => {
    expect(
      validateOnboardingPlan(coastModeB({ coastStopAge: String(MIN_PROFILE_AGE) })),
    ).toEqual([]);
    expect(
      validateOnboardingPlan(
        coastModeB({ coastStopAge: String(MAX_HORIZON_LIFESPAN_AGE) }),
      ),
    ).toEqual([]);
  });

  it("en modo B la edad de JUBILACIÓN no se valida: no se pregunta", () => {
    expect(validateOnboardingPlan(coastModeB({ targetRetirementAge: "basura" }))).toEqual([]);
  });

  it("y en modo A la de PARADA tampoco: el simétrico exacto", () => {
    const s = validState("coast");
    expect(validateOnboardingPlan({ ...s, coastStopAge: "-3" })).toEqual([]);
  });
});

describe("validateOnboardingPlan — partial (media jornada)", () => {
  it("edad de inicio vacía en modo A ⇒ partial_start_age_required", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialStartAge: "" })).toContain("partial_start_age_required");
  });

  it("edad fuera de rango ⇒ partial_age_out_of_range (el dato está, pero no vale)", () => {
    const s = validState("partial");
    expect(codes({ ...s, partialStartAge: String(MIN_PROFILE_AGE - 1) })).toContain(
      "partial_age_out_of_range",
    );
    expect(
      codes({ ...s, partialStartAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) }),
    ).toContain("partial_age_out_of_range");
  });

  it("en modo «en cuanto pueda» la edad de inicio no se valida: la resuelve el servidor", () => {
    expect(validateOnboardingPlan(partialModeB({ partialStartAge: "-3" }))).toEqual([]);
    expect(validateOnboardingPlan(partialModeB({ partialStartAge: "" }))).toEqual([]);
  });

  it("el ingreso sigue siendo obligatorio en los DOS modos", () => {
    expect(codes({ ...validState("partial"), partialIncome: "" })).toContain(
      "partial_income_not_positive",
    );
    expect(codes(partialModeB({ partialIncome: "" }))).toContain(
      "partial_income_not_positive",
    );
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

describe("validateOnboardingPlan — la pensión, cuando el borrador la trae (C7)", () => {
  it("importe vacío con la edad escrita ⇒ pension_amount_not_positive", () => {
    expect(codes(withPension("asap", { pensionAmount: "" }))).toContain(
      "pension_amount_not_positive",
    );
  });

  it("importe 0 o negativo ⇒ pension_amount_not_positive", () => {
    expect(codes(withPension("asap", { pensionAmount: "0" }))).toContain(
      "pension_amount_not_positive",
    );
    expect(codes(withPension("asap", { pensionAmount: "-1" }))).toContain(
      "pension_amount_not_positive",
    );
  });

  it("importe no numérico ⇒ decimal_invalid", () => {
    expect(codes(withPension("asap", { pensionAmount: "mucho" }))).toContain(
      "decimal_invalid",
    );
  });

  it(`edad fuera de [${MIN_PENSION_AGE}, ${MAX_HORIZON_LIFESPAN_AGE}] ⇒ pension_age_out_of_range`, () => {
    expect(
      codes(withPension("asap", { pensionStartAge: String(MIN_PENSION_AGE - 1) })),
    ).toContain("pension_age_out_of_range");
    expect(
      codes(withPension("asap", { pensionStartAge: String(MAX_HORIZON_LIFESPAN_AGE + 1) })),
    ).toContain("pension_age_out_of_range");
  });

  it(`los dos extremos (${MIN_PENSION_AGE} y ${MAX_HORIZON_LIFESPAN_AGE}) son válidos`, () => {
    expect(
      validateOnboardingPlan(withPension("asap", { pensionStartAge: String(MIN_PENSION_AGE) })),
    ).toEqual([]);
    expect(
      validateOnboardingPlan(
        withPension("asap", { pensionStartAge: String(MAX_HORIZON_LIFESPAN_AGE) }),
      ),
    ).toEqual([]);
  });

  it("edad vacía con importe escrito ⇒ pension_age_out_of_range (no hay código 'required' aparte)", () => {
    expect(codes(withPension("asap", { pensionStartAge: "" }))).toContain(
      "pension_age_out_of_range",
    );
  });
});

describe("validateOnboardingPlan — asap no valida nada de lo que no pregunta", () => {
  it("basura en los campos que asap no pinta no genera ningún problema", () => {
    const s = validState("asap");
    expect(
      validateOnboardingPlan({
        ...s,
        targetRetirementAge: "no-es-una-edad",
        coastStopAge: "-3",
        partialStartAge: "-3",
        partialIncome: "no-es-dinero",
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

describe("buildOnboardingPlanPatch — cuerpo exacto por estrategia y modo", () => {
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

  it("retire_at_age NO manda `coast_mode`: ese eje no es suyo", () => {
    expect(buildOnboardingPlanPatch(validState("retire_at_age"))).not.toHaveProperty(
      "coast_mode",
    );
  });

  it("coast modo A: manda el modo EXPLÍCITO y la edad de jubilación, sin la de parada", () => {
    // El modo viaja aunque sea el default del servidor: quien vuelva al asistente con un perfil
    // ya en modo B y elija «fijo la edad de jubilación» necesita que el PATCH lo diga.
    const patch = buildOnboardingPlanPatch(validState("coast"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "coast",
      birth_date: "1990-05-20",
      coast_mode: "fixed_retirement_age",
      target_retirement_age: 55,
    });
    expect(patch).not.toHaveProperty("coast_stop_age");
  });

  it("coast modo B: manda el modo y la edad de parada, sin la de jubilación", () => {
    const patch = buildOnboardingPlanPatch(coastModeB());
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "coast",
      birth_date: "1990-05-20",
      coast_mode: "fixed_stop_age",
      coast_stop_age: 45,
    });
    expect(patch).not.toHaveProperty("target_retirement_age");
  });

  it("partial modo A: bloque completo con `mode: at_age` y su edad", () => {
    const patch = buildOnboardingPlanPatch(validState("partial"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "partial",
      birth_date: "1990-05-20",
      partial_retirement: {
        mode: "at_age",
        starts_at_age: 55,
        income_monthly_today: "800",
        expense_basis: "retirement",
      },
    });
  });

  it("partial modo B: `starts_at_age` viaja NULL, no una edad inventada", () => {
    // Mandar un número aquí fijaría la fase a una edad que nadie eligió; el `null` es lo que le
    // dice al servidor que la resuelva él (`earliest_partial_start`).
    const patch = buildOnboardingPlanPatch(partialModeB());
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "partial",
      birth_date: "1990-05-20",
      partial_retirement: {
        mode: "asap",
        starts_at_age: null,
        income_monthly_today: "800",
        expense_basis: "retirement",
      },
    });
  });

  it("partial modo B ignora una edad de inicio que hubiera quedado escrita en otro modo", () => {
    const patch = buildOnboardingPlanPatch(partialModeB({ partialStartAge: "55" }));
    expect(patch.partial_retirement?.starts_at_age).toBeNull();
  });

  it("con pensión: bloque completo, con el puente APAGADO y sus dos números en null", () => {
    // El puente es un ajuste fino sobre una pensión ya declarada (C7) y se activa en Jubilación;
    // este paso solo recoge lo mínimo para tener un plan.
    const patch = buildOnboardingPlanPatch(withPension("asap"));
    expect(patch).toEqual<RetirementProfilePatchApi>({
      strategy: "asap",
      birth_date: "1990-05-20",
      pension: {
        monthly_amount_today: "1200",
        starts_at_age: 67,
        indexed: true,
        fraction_while_partial: "0",
        bridge_enabled: false,
        bridge_max_pct: null,
        bridge_max_years: null,
      },
    });
  });

  it("sin pensión en el borrador, el bloque no viaja", () => {
    for (const s of STRATEGIES) {
      expect(buildOnboardingPlanPatch(validState(s)), s).not.toHaveProperty("pension");
    }
  });

  it("nunca incluye withdrawal_rule — en ninguna estrategia ni modo", () => {
    for (const state of [
      ...STRATEGIES.map(validState),
      coastModeB(),
      partialModeB(),
      withPension(),
    ]) {
      expect(buildOnboardingPlanPatch(state)).not.toHaveProperty("withdrawal_rule");
    }
  });

  it("nunca incluye swr_pct, horizon_lifespan_age, fire_number_mode ni el umbral", () => {
    for (const state of [
      ...STRATEGIES.map(validState),
      coastModeB(),
      partialModeB(),
      withPension(),
    ]) {
      const patch = buildOnboardingPlanPatch(state);
      expect(patch).not.toHaveProperty("swr_pct");
      expect(patch).not.toHaveProperty("horizon_lifespan_age");
      expect(patch).not.toHaveProperty("fire_number_mode");
      expect(patch).not.toHaveProperty("success_threshold_pct");
      // Y los dos ejes que murieron con el objetivo (C1/M4) tampoco resucitan por aquí.
      expect(patch).not.toHaveProperty("target_basis");
      expect(patch).not.toHaveProperty("bridge_discount_basis");
      expect(patch).not.toHaveProperty("cash_buffer_months");
    }
  });

  it("los dos bloques son mutuamente excluyentes salvo que el borrador traiga los dos datos", () => {
    const partialPatch = buildOnboardingPlanPatch(validState("partial"));
    expect(partialPatch).not.toHaveProperty("pension");
    const pensionPatch = buildOnboardingPlanPatch(withPension("asap"));
    expect(pensionPatch).not.toHaveProperty("partial_retirement");
  });

  it("el importe con coma decimal se normaliza a decimal-string de la API (punto)", () => {
    const patch = buildOnboardingPlanPatch(
      withPension("asap", { pensionAmount: "1.234,5" }),
    );
    expect(patch.pension?.monthly_amount_today).toBe("1234.5");
  });
});
