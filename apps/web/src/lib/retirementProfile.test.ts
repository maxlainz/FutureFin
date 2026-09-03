/**
 * Perfil de jubilación en cliente (5.0.0, issue #207).
 *
 * Tres cosas que este suite existe para impedir, todas silenciosas si se rompen:
 *
 *  1. **Un PATCH que no es mínimo.** Mandar el perfil entero resetea en el servidor lo que el
 *     usuario no tocó — el bug exacto que el tri-estado de `RetirementProfilePatch` existe para
 *     esquivar. Aquí se fija que solo viajan las claves cambiadas, que `null` borra de verdad y
 *     que `target_basis` **no** viaja cuando el borrador no lo cambia (o congelaría la
 *     derivación del servidor, R6).
 *  2. **Una guarda de validez que diverge de las cotas del servidor.** La tabla recorre CADA
 *     regla de `validate_retirement_profile` con su código estable: si Rust cambia una cota y
 *     nadie la trae aquí, el usuario recibe un 400 con el formulario prometiéndole «Guardado
 *     automático».
 *  3. **Una vista previa del objetivo que deja de cuadrar con el fixture compartido.** El SWR y
 *     el modo se mudaron al perfil; la fórmula no. Se recorre el mismo `fire-parity.json` que
 *     comparten Rust y `fire.test.ts`, pero pasando por un `RetirementProfileApi` real, que es
 *     la fontanería que la SPA usa de verdad desde 5.0.0.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type {
  FireNumberModeApi,
  RetirementProfileApi,
  TaxBracketApi,
} from "../api/types";
import { ERROR_MESSAGES } from "./errorMessages";
import { computeFireAnnualNeedNetEur, grossUpNetAnnualFire } from "./fire";
import {
  MAX_CASH_BUFFER_MONTHS,
  MAX_GUARDRAIL_PCT,
  MAX_SUCCESS_THRESHOLD_PCT,
  MAX_SWR_PCT,
  MAX_WITHDRAWAL_PCT,
  MIN_PENSION_AGE,
  MIN_PROFILE_AGE,
  MIN_SUCCESS_THRESHOLD_PCT,
  buildRetirementProfilePatch,
  defaultRetirementProfileApi,
  effectiveTargetBasis,
  isEmptyRetirementProfilePatch,
  normalizeRetirementProfile,
  retirementProfileIssue,
  targetBasisSource,
  withStoredTargetBasis,
} from "./retirementProfile";

const base = (over: Partial<RetirementProfileApi> = {}): RetirementProfileApi => ({
  ...defaultRetirementProfileApi(),
  ...over,
});

// ---------------------------------------------------------------------------
// Defaults y normalización — espejo de `resolve_retirement_profile`
// ---------------------------------------------------------------------------

describe("defaults del perfil", () => {
  it("un perfil por defecto es la conducta de 4.15.x", () => {
    const p = defaultRetirementProfileApi();
    expect(p.strategy).toBe("asap");
    expect(p.swr_pct).toBe("3.5");
    expect(p.horizon_lifespan_age).toBe(90);
    expect(p.fire_number_mode).toBe("annual_expense");
    expect(p.withdrawal_rule.kind).toBe("fixed_real");
    expect(p.withdrawal_rule.spend_mode).toBe("ceiling");
    expect(p.success_threshold_pct).toBe(95);
    // `null` = «derívalo tú», que NO es lo mismo que perpetuidad elegida.
    expect(p.target_basis).toBeNull();
  });

  it("y se puede guardar tal cual (la guarda no bloquea el estado de partida)", () => {
    expect(retirementProfileIssue(defaultRetirementProfileApi())).toBeNull();
  });
});

describe("normalizeRetirementProfile", () => {
  it("null / basura → los defaults", () => {
    expect(normalizeRetirementProfile(null)).toEqual(defaultRetirementProfileApi());
    expect(normalizeRetirementProfile(undefined)).toEqual(defaultRetirementProfileApi());
  });

  it("clampa en LECTURA lo que llegue fuera de rango, nunca lo rechaza", () => {
    const p = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      swr_pct: "99",
      horizon_lifespan_age: 200,
      success_threshold_pct: 5,
      cash_buffer_months: 999,
      target_retirement_age: 3,
    });
    expect(p.swr_pct).toBe(String(MAX_SWR_PCT));
    expect(p.horizon_lifespan_age).toBe(105);
    expect(p.success_threshold_pct).toBe(MIN_SUCCESS_THRESHOLD_PCT);
    expect(p.cash_buffer_months).toBe(MAX_CASH_BUFFER_MONTHS);
    expect(p.target_retirement_age).toBe(MIN_PROFILE_AGE);
  });

  it("un enumerado desconocido cae a su default, no revienta la vista", () => {
    const p = normalizeRetirementProfile({
      ...defaultRetirementProfileApi(),
      strategy: "monte_carlo",
      bridge_discount_basis: "vibes",
      withdrawal_rule: { ...defaultRetirementProfileApi().withdrawal_rule, kind: "magia" },
    } as unknown as RetirementProfileApi);
    expect(p.strategy).toBe("asap");
    expect(p.bridge_discount_basis).toBe("expected_return");
    expect(p.withdrawal_rule.kind).toBe("fixed_real");
  });

  it("NO deriva target_basis: conserva el `null` que distingue «no elegido»", () => {
    const withPension = normalizeRetirementProfile(
      base({
        pension: {
          monthly_amount_today: "1200",
          starts_at_age: 67,
          indexed: true,
          fraction_while_partial: "0",
        },
      }),
    );
    expect(withPension.target_basis).toBeNull();
    // La derivación para PINTAR vive aparte, y es la del servidor (R6).
    expect(effectiveTargetBasis(withPension)).toBe("bridge_to_pension");
  });
});

describe("effectiveTargetBasis (R6)", () => {
  it("sin pensión y sin elección → perpetuidad", () => {
    expect(effectiveTargetBasis(base())).toBe("perpetuity");
  });

  it("con pensión y sin elección → puente", () => {
    expect(
      effectiveTargetBasis(
        base({
          pension: {
            monthly_amount_today: "900",
            starts_at_age: 65,
            indexed: true,
            fraction_while_partial: "0",
          },
        }),
      ),
    ).toBe("bridge_to_pension");
  });

  it("una elección explícita gana sobre la derivación", () => {
    expect(
      effectiveTargetBasis(
        base({
          target_basis: "perpetuity",
          pension: {
            monthly_amount_today: "900",
            starts_at_age: 65,
            indexed: true,
            fraction_while_partial: "0",
          },
        }),
      ),
    ).toBe("perpetuity");
  });

  it("la estrategia del puente la fuerza, elija lo que elija el usuario", () => {
    expect(
      effectiveTargetBasis(base({ strategy: "pension_bridge", target_basis: "perpetuity" })),
    ).toBe("bridge_to_pension");
  });
});

// ---------------------------------------------------------------------------
// PATCH mínimo y tri-estado
// ---------------------------------------------------------------------------

describe("buildRetirementProfilePatch — mínimo", () => {
  it("sin cambios, el patch está vacío (el servidor lo rechazaría con patch_empty)", () => {
    const p = base();
    const patch = buildRetirementProfilePatch(p, { ...p });
    expect(patch).toEqual({});
    expect(isEmptyRetirementProfilePatch(patch)).toBe(true);
  });

  it("solo viaja la clave que cambia: la pensión declarada NO se toca", () => {
    const before = base({
      swr_pct: "3.0",
      pension: {
        monthly_amount_today: "1100",
        starts_at_age: 67,
        indexed: true,
        fraction_while_partial: "0",
      },
    });
    const patch = buildRetirementProfilePatch(before, { ...before, swr_pct: "3.5" });
    expect(Object.keys(patch)).toEqual(["swr_pct"]);
    expect(patch.swr_pct).toBe("3.5");
    expect("pension" in patch).toBe(false);
  });

  it("un decimal reescrito con el mismo VALOR no genera patch", () => {
    // Sin esto, teclear «3,50» sobre un «3.5» guardado sería una escritura (y una invalidación
    // de la proyección) por cada pulsación de coma.
    const before = base({ swr_pct: "3.5" });
    expect(buildRetirementProfilePatch(before, { ...before, swr_pct: "3.50" })).toEqual({});
    expect(buildRetirementProfilePatch(before, { ...before, swr_pct: "3,5" })).toEqual({});
  });

  it("varios cambios a la vez viajan juntos y nada más", () => {
    const before = base();
    const patch = buildRetirementProfilePatch(
      before,
      base({ strategy: "retire_at_age", target_retirement_age: 58, horizon_lifespan_age: 95 }),
    );
    expect(Object.keys(patch).sort()).toEqual(
      ["horizon_lifespan_age", "strategy", "target_retirement_age"].sort(),
    );
  });
});

describe("buildRetirementProfilePatch — tri-estado", () => {
  const pension = {
    monthly_amount_today: "1200",
    starts_at_age: 67,
    indexed: true,
    fraction_while_partial: "0",
  };
  const partial = {
    starts_at_age: 55,
    income_monthly_today: "800",
    expense_basis: "retirement" as const,
  };

  it("borrar un bloque manda `null` explícito, no lo omite", () => {
    const before = base({ pension, partial_retirement: partial, cash_buffer_months: 24 });
    const patch = buildRetirementProfilePatch(
      before,
      base({ pension: null, partial_retirement: null, cash_buffer_months: null }),
    );
    expect(patch.pension).toBeNull();
    expect(patch.partial_retirement).toBeNull();
    expect(patch.cash_buffer_months).toBeNull();
  });

  it("borrar la edad objetivo manda `null`, que no es 0", () => {
    const before = base({ target_retirement_age: 60 });
    const patch = buildRetirementProfilePatch(before, base({ target_retirement_age: null }));
    expect(patch.target_retirement_age).toBeNull();
    expect(patch.target_retirement_age).not.toBe(0);
  });

  it("declarar un bloque nuevo viaja entero", () => {
    const patch = buildRetirementProfilePatch(base(), base({ pension }));
    expect(patch.pension).toEqual(pension);
  });

  it("la regla de retirada viaja ENTERA, nunca campo a campo", () => {
    // Sus `pct` obligatorios dependen de `kind`: un merge parcial permitiría estados como
    // «guardrails con el pct del percent_of_balance anterior» que nadie escribió.
    const before = base();
    const patch = buildRetirementProfilePatch(
      before,
      base({
        withdrawal_rule: {
          kind: "guardrails",
          pct: "4",
          start_pct: null,
          end_pct: null,
          band_pct: "20",
          adjust_pct: "10",
          spend_mode: "rule_is_spend",
        },
      }),
    );
    expect(patch.withdrawal_rule).toEqual({
      kind: "guardrails",
      pct: "4",
      start_pct: null,
      end_pct: null,
      band_pct: "20",
      adjust_pct: "10",
      spend_mode: "rule_is_spend",
    });
  });

  it("un importe vacío de bloque llega al wire como «0», nunca como cadena vacía", () => {
    const patch = buildRetirementProfilePatch(
      base(),
      base({
        partial_retirement: { ...partial, income_monthly_today: "" },
      }),
    );
    expect(patch.partial_retirement?.income_monthly_today).toBe("0");
  });

  it("`target_basis` NO viaja si el borrador no lo cambia (no congela la derivación R6)", () => {
    // El servidor publica la base RESUELTA. Si el borrador la reenviara en cada PATCH, al
    // declarar después una pensión el objetivo se quedaría en perpetuidad —la opción
    // conservadora que nadie pidió— sin ningún aviso.
    const before = base({ target_basis: "perpetuity" });
    const patch = buildRetirementProfilePatch(before, { ...before, swr_pct: "3.2" });
    expect("target_basis" in patch).toBe(false);
  });

  it("…y sí viaja cuando el usuario elige la otra opción", () => {
    const before = base({ target_basis: "perpetuity" });
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      target_basis: "bridge_to_pension",
    });
    expect(patch.target_basis).toBe("bridge_to_pension");
  });
});

// ---------------------------------------------------------------------------
// La elección ALMACENADA de `target_basis` (WP5-2): «derivada» ≠ «elegida»
// ---------------------------------------------------------------------------

describe("target_basis almacenado vs resuelto", () => {
  const pension = {
    monthly_amount_today: "1200",
    starts_at_age: 67,
    indexed: true,
    fraction_while_partial: "0",
  };

  it("withStoredTargetBasis pone la elección guardada donde el servidor puso la resuelta", () => {
    // Lo que llega del servidor: `profile.target_basis` RESUELTO a puente (hay pensión) y
    // `target_basis_stored: null` (nadie lo eligió).
    const fromServer = base({ target_basis: "bridge_to_pension", pension });
    const p = withStoredTargetBasis(fromServer, null);
    expect(p.target_basis).toBeNull();
    // Y lo que se PINTA no cambia: se deriva igual que en el servidor.
    expect(effectiveTargetBasis(p)).toBe("bridge_to_pension");
  });

  it("una elección guardada se conserva tal cual", () => {
    const p = withStoredTargetBasis(
      base({ target_basis: "perpetuity", pension }),
      "perpetuity",
    );
    expect(p.target_basis).toBe("perpetuity");
    expect(effectiveTargetBasis(p)).toBe("perpetuity");
  });

  it("un backend sin el campo (undefined) deja el perfil intacto", () => {
    // Inventar un `null` diría «derivada» sobre una elección que quizá sí existe.
    const fromServer = base({ target_basis: "bridge_to_pension", pension });
    expect(withStoredTargetBasis(fromServer, undefined)).toBe(fromServer);
  });

  it("targetBasisSource distingue los tres orígenes", () => {
    expect(targetBasisSource(base({ target_basis: null }))).toBe("derived");
    expect(targetBasisSource(base({ target_basis: null, pension }))).toBe("derived");
    expect(targetBasisSource(base({ target_basis: "perpetuity" }))).toBe("stored");
    expect(targetBasisSource(base({ strategy: "pension_bridge", pension }))).toBe(
      "forced_by_strategy",
    );
  });

  it("elegir el radio que YA estaba marcado por derivación sí fija la elección", () => {
    // Este es el caso que la sustitución existe para permitir: el perfil deriva `perpetuity` y
    // el usuario marca `perpetuity` para congelarla a propósito. Comparando el valor RESUELTO
    // los dos lados se veían iguales y la fijación no se mandaba nunca.
    const before = withStoredTargetBasis(base({ target_basis: "perpetuity" }), null);
    const patch = buildRetirementProfilePatch(before, {
      ...before,
      target_basis: "perpetuity",
    });
    expect(patch.target_basis).toBe("perpetuity");
  });

  it("«Volver a la derivada» manda `null` EXPLÍCITO (el tri-estado del servidor)", () => {
    const before = base({ target_basis: "perpetuity", pension });
    const patch = buildRetirementProfilePatch(before, { ...before, target_basis: null });
    expect("target_basis" in patch).toBe(true);
    expect(patch.target_basis).toBeNull();
  });

  it("guardar otro campo con la base derivada no nombra `target_basis`", () => {
    const before = withStoredTargetBasis(base({ target_basis: "perpetuity" }), null);
    const patch = buildRetirementProfilePatch(before, { ...before, swr_pct: "3.2" });
    expect(Object.keys(patch)).toEqual(["swr_pct"]);
  });
});

// ---------------------------------------------------------------------------
// Guarda de validez — la tabla de cotas del servidor, regla por regla
// ---------------------------------------------------------------------------

describe("retirementProfileIssue — espejo de validate_retirement_profile", () => {
  const pension = {
    monthly_amount_today: "1200",
    starts_at_age: 67,
    indexed: true,
    fraction_while_partial: "0",
  };
  const rule = defaultRetirementProfileApi().withdrawal_rule;

  const cases: Array<[string, RetirementProfileApi, string | null]> = [
    // --- Los cuatro ejes movidos conservan sus códigos de 4.15.x -------------------------
    ["SWR en la cota alta es válido", base({ swr_pct: String(MAX_SWR_PCT) }), null],
    ["SWR por encima de la cota", base({ swr_pct: "4.1" }), "swr_out_of_range"],
    ["SWR negativo", base({ swr_pct: "-0.1" }), "swr_out_of_range"],
    ["SWR ilegible", base({ swr_pct: "tres" }), "decimal_invalid"],
    [
      "modo manual sin importe",
      base({ fire_number_mode: "manual", fire_number_manual_amount: null }),
      "fire_manual_amount_required",
    ],
    [
      "modo manual con importe 0",
      base({ fire_number_mode: "manual", fire_number_manual_amount: "0" }),
      "fire_manual_amount_not_positive",
    ],
    [
      "modo manual con importe positivo",
      base({ fire_number_mode: "manual", fire_number_manual_amount: "500000" }),
      null,
    ],
    [
      "horizonte por debajo de 85",
      base({ horizon_lifespan_age: 84 }),
      "horizon_lifespan_age_out_of_range",
    ],
    [
      "horizonte por encima de 105",
      base({ horizon_lifespan_age: 106 }),
      "horizon_lifespan_age_out_of_range",
    ],

    // --- Estrategia ---------------------------------------------------------------------
    [
      "retire_at_age sin edad",
      base({ strategy: "retire_at_age" }),
      "target_retirement_age_required",
    ],
    ["coast sin edad", base({ strategy: "coast" }), "target_retirement_age_required"],
    [
      "retire_at_age con edad",
      base({ strategy: "retire_at_age", target_retirement_age: 60 }),
      null,
    ],
    [
      "pension_bridge sin pensión",
      base({ strategy: "pension_bridge" }),
      "pension_required_for_bridge",
    ],
    ["pension_bridge con pensión", base({ strategy: "pension_bridge", pension }), null],

    // --- Edades -------------------------------------------------------------------------
    [
      "edad objetivo por debajo del mínimo",
      base({ target_retirement_age: MIN_PROFILE_AGE - 1 }),
      "retirement_age_out_of_range",
    ],
    [
      "edad objetivo por encima del horizonte",
      base({ horizon_lifespan_age: 90, target_retirement_age: 91 }),
      "retirement_age_out_of_range",
    ],
    [
      "pensión antes de la edad mínima",
      base({ pension: { ...pension, starts_at_age: MIN_PENSION_AGE - 1 } }),
      "pension_age_out_of_range",
    ],
    [
      "pensión después del horizonte",
      base({ horizon_lifespan_age: 90, pension: { ...pension, starts_at_age: 91 } }),
      "pension_age_out_of_range",
    ],
    [
      "pensión sin importe (recién activada)",
      base({ pension: { ...pension, monthly_amount_today: "" } }),
      "pension_amount_not_positive",
    ],
    [
      "pensión con importe 0",
      base({ pension: { ...pension, monthly_amount_today: "0" } }),
      "pension_amount_not_positive",
    ],
    [
      "fracción en media jornada fuera de [0,1]",
      base({ pension: { ...pension, fraction_while_partial: "1.5" } }),
      "pension_fraction_out_of_range",
    ],
    [
      "media jornada por debajo del mínimo",
      base({
        partial_retirement: {
          starts_at_age: MIN_PROFILE_AGE - 1,
          income_monthly_today: "500",
          expense_basis: "retirement",
        },
      }),
      "partial_age_out_of_range",
    ],
    [
      "media jornada con ingreso negativo",
      base({
        partial_retirement: {
          starts_at_age: 55,
          income_monthly_today: "-1",
          expense_basis: "retirement",
        },
      }),
      "partial_income_negative",
    ],
    [
      "media jornada con ingreso vacío = año sabático, válido",
      base({
        partial_retirement: {
          starts_at_age: 55,
          income_monthly_today: "",
          expense_basis: "retirement",
        },
      }),
      null,
    ],
    [
      "media jornada que no empieza antes de la total",
      base({
        target_retirement_age: 60,
        partial_retirement: {
          starts_at_age: 60,
          income_monthly_today: "500",
          expense_basis: "retirement",
        },
      }),
      "partial_not_before_retirement",
    ],

    // --- Colchón y umbral ---------------------------------------------------------------
    [
      "colchón por encima de la cota",
      base({ cash_buffer_months: MAX_CASH_BUFFER_MONTHS + 1 }),
      "cash_buffer_out_of_range",
    ],
    ["colchón en la cota", base({ cash_buffer_months: MAX_CASH_BUFFER_MONTHS }), null],
    [
      "umbral por debajo del mínimo",
      base({ success_threshold_pct: MIN_SUCCESS_THRESHOLD_PCT - 1 }),
      "success_threshold_out_of_range",
    ],
    [
      "umbral por encima del máximo",
      base({ success_threshold_pct: MAX_SUCCESS_THRESHOLD_PCT + 1 }),
      "success_threshold_out_of_range",
    ],

    // --- Reglas de retirada: cada `kind` exige SUS campos --------------------------------
    ["fixed_real no pide nada", base({ withdrawal_rule: { ...rule } }), null],
    // U4 — un porcentaje ausente ya NO es un hueco: hereda `swr_pct` (3,5 % por defecto), que es
    // el punto entero de que la pantalla tenga un solo porcentaje editable.
    [
      "percent_of_balance sin pct HEREDA el SWR",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance" } }),
      null,
    ],
    // …y la consecuencia declarada, la misma que en Rust: con el SWR a 0 lo heredado no es un
    // plan, y se dice en vez de devolver una simulación que no vende nada.
    [
      "percent_of_balance sin pct y SWR 0",
      base({ swr_pct: "0", withdrawal_rule: { ...rule, kind: "percent_of_balance" } }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "percent_of_balance con pct",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance", pct: "4" } }),
      null,
    ],
    [
      "percent_of_balance con pct 0",
      base({ withdrawal_rule: { ...rule, kind: "percent_of_balance", pct: "0" } }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "percent_of_balance por encima del techo",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "percent_of_balance",
          pct: String(MAX_WITHDRAWAL_PCT + 1),
        },
      }),
      "withdrawal_pct_out_of_range",
    ],
    [
      "hybrid sin start_pct hereda el SWR (3,5) y el 3 % queda por debajo",
      base({ withdrawal_rule: { ...rule, kind: "hybrid", end_pct: "3" } }),
      null,
    ],
    // El `end_pct` NO hereda nada: es el suelo del latch, no un porcentaje de retirada. Y se
    // compara contra el heredado, que es el que va a retirar el motor.
    [
      "hybrid sin start_pct y end por encima del SWR heredado",
      base({ withdrawal_rule: { ...rule, kind: "hybrid", end_pct: "3.9" } }),
      "hybrid_end_pct_not_below_start",
    ],
    [
      "hybrid sin end_pct sigue siendo un hueco",
      base({ withdrawal_rule: { ...rule, kind: "hybrid" } }),
      "withdrawal_pct_required",
    ],
    [
      "hybrid con end >= start",
      base({
        withdrawal_rule: { ...rule, kind: "hybrid", start_pct: "3", end_pct: "5" },
      }),
      "hybrid_end_pct_not_below_start",
    ],
    [
      "hybrid coherente",
      base({
        withdrawal_rule: { ...rule, kind: "hybrid", start_pct: "5", end_pct: "3" },
      }),
      null,
    ],
    [
      "guardrails sin adjust_pct",
      base({
        withdrawal_rule: { ...rule, kind: "guardrails", pct: "4", band_pct: "20" },
      }),
      "withdrawal_pct_required",
    ],
    [
      "guardrails con banda fuera de cota",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "guardrails",
          pct: "4",
          band_pct: String(MAX_GUARDRAIL_PCT + 1),
          adjust_pct: "10",
        },
      }),
      "withdrawal_band_out_of_range",
    ],
    [
      "guardrails completo",
      base({
        withdrawal_rule: {
          ...rule,
          kind: "guardrails",
          pct: "4",
          band_pct: "20",
          adjust_pct: "10",
        },
      }),
      null,
    ],
  ];

  for (const [name, profile, expected] of cases) {
    it(name, () => {
      expect(retirementProfileIssue(profile)).toBe(expected);
    });
  }

  it("todo código que devuelve la guarda tiene frase en español", () => {
    // Si la guarda inventara un código propio, el usuario vería el mensaje genérico y nadie se
    // enteraría: el catálogo es la única superficie donde se traduce lo que se lee en pantalla.
    const codes = new Set(
      cases.map(([, , code]) => code).filter((c): c is string => c !== null),
    );
    const missing = [...codes].filter((c) => !ERROR_MESSAGES[c]);
    expect(missing, `códigos sin frase: ${missing.join(", ")}`).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Vista previa del objetivo a través de la fontanería NUEVA (perfil → preview)
// ---------------------------------------------------------------------------

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/fire-parity.json",
);

type ParityCase = {
  name: string;
  fire_settings: {
    fire_number_mode: FireNumberModeApi;
    fire_number_manual_amount?: string | null;
    swr_pct: string;
    taxes_enabled: boolean;
    tax_brackets: TaxBracketApi[];
    taxable_gain_ratio?: string;
  };
  monthly: { income: string; income_retirement: string; expense_retirement: string };
  expected_target_nw: number | null;
};

describe("vista previa del objetivo desde el PERFIL (fixture compartido)", () => {
  const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as {
    cases: ParityCase[];
  };

  for (const c of fixture.cases) {
    it(`case "${c.name}" cuadra ±1 € pasando por RetirementProfileApi`, () => {
      // Los tres ejes personales del caso se meten en un perfil de verdad —normalizado como lo
      // normaliza la SPA— y los fiscales se quedan donde siguen viviendo (el hogar). Si alguien
      // se llevara el SWR o el modo a otro sitio sin arrastrar la fórmula, esto se cae.
      const profile = normalizeRetirementProfile(
        base({
          fire_number_mode: c.fire_settings.fire_number_mode,
          fire_number_manual_amount: c.fire_settings.fire_number_manual_amount ?? null,
          swr_pct: c.fire_settings.swr_pct,
        }),
      );

      const need = computeFireAnnualNeedNetEur(
        {
          fire_number_mode: profile.fire_number_mode,
          fire_number_manual_amount: profile.fire_number_manual_amount,
        },
        c.monthly.expense_retirement,
        c.monthly.income,
        c.monthly.income_retirement,
      );
      const swr = Number(profile.swr_pct);
      const actual =
        need === null || need <= 0 || !Number.isFinite(swr) || swr <= 0
          ? null
          : grossUpNetAnnualFire(
              need,
              c.fire_settings.tax_brackets,
              c.fire_settings.taxes_enabled,
              Number(c.fire_settings.taxable_gain_ratio ?? "1"),
            ) /
            (swr / 100);

      if (c.expected_target_nw === null) {
        expect(actual).toBeNull();
        return;
      }
      expect(actual).not.toBeNull();
      expect(Math.abs((actual as number) - c.expected_target_nw)).toBeLessThanOrEqual(1);
    });
  }

  it("el SWR del perfil se clampa antes de dividir: nunca un objetivo con SWR > 4 %", () => {
    // El clamp de lectura es lo que impide que un backup o una edición directa de la BD metan
    // un SWR imposible en la vista previa y publiquen un objetivo que el servidor no calcula.
    const p = normalizeRetirementProfile(base({ swr_pct: "40" }));
    expect(Number(p.swr_pct)).toBe(MAX_SWR_PCT);
  });
});
