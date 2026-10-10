/**
 * Las piezas de lógica del formulario de Jubilación que no son JSX, y el COPY que las acompaña.
 *
 * Del copy, este fichero fija dos cosas de distinta naturaleza y por eso hay dos bloques: la
 * FORMA de `PLAN_CARD_COPY` (una entrada por tarjeta, título corto, frase con sustancia acabada en
 * punto), que caza la tarjeta añadida sin frase; y **una afirmación concreta**, la de «Horizonte»,
 * que el modelo v2 volvió falsa y que nadie habría vuelto a leer si no falla un test.
 */

import { describe, expect, it } from "vitest";
import {
  PLAN_CARD_COPY,
  PLAN_CARD_HELP,
  PLAN_FIELD_HELP,
  fractionFromPercent,
  missingRequiredPlanFields,
  percentFromFraction,
  saveIndicatorLabel,
  withdrawalPctNote,
} from "./retirement-form";
import { HELP_TEXTS } from "./helpTexts";
import { defaultWithdrawalRuleApi } from "./retirementProfile";
import type { RetirementProfileApi } from "../api/types";

/** Perfil v2 completo, escrito a mano: lo que este fichero prueba es el contrato contra
 *  `RetirementProfileApi` (el tipo del wire), no los defaults de otro módulo. */
const base = (over: Partial<RetirementProfileApi> = {}): RetirementProfileApi => ({
  strategy: "asap",
  target_retirement_age: null,
  fire_number_mode: "annual_expense",
  fire_number_manual_amount: null,
  swr_pct: "3.5",
  horizon_lifespan_age: 90,
  success_threshold_pct: 95,
  coast_mode: "fixed_retirement_age",
  coast_stop_age: null,
  withdrawal_rule: defaultWithdrawalRuleApi(),
  pension: null,
  partial_retirement: null,
  ...over,
});

const pension = (
  over: Partial<NonNullable<RetirementProfileApi["pension"]>> = {},
): NonNullable<RetirementProfileApi["pension"]> => ({
  monthly_amount_today: "1200",
  starts_at_age: 67,
  indexed: true,
  fraction_while_partial: "0",
  bridge_enabled: false,
  bridge_max_pct: null,
  bridge_max_years: null,
  ...over,
});

describe("S3 — la fracción de pensión se edita en porcentaje", () => {
  it("fracción → porcentaje", () => {
    expect(percentFromFraction("0.4")).toBe("40");
    expect(percentFromFraction("1")).toBe("100");
    expect(percentFromFraction("0")).toBe("0");
    expect(percentFromFraction("0,25")).toBe("25");
  });

  it("sin valor legible el campo queda VACÍO, no en cero", () => {
    expect(percentFromFraction(null)).toBe("");
    expect(percentFromFraction("")).toBe("");
    expect(percentFromFraction("abc")).toBe("");
  });

  it("porcentaje → fracción, y el vacío es cero en el wire", () => {
    expect(fractionFromPercent("40")).toBe("0.4");
    expect(fractionFromPercent("100")).toBe("1");
    expect(fractionFromPercent("")).toBe("0");
    expect(fractionFromPercent("12,5")).toBe("0.125");
  });

  it("ida y vuelta sin deriva de coma flotante", () => {
    for (const pct of ["0", "1", "7", "12,5", "33", "60", "100"]) {
      expect(percentFromFraction(fractionFromPercent(pct))).toBe(
        pct.replace(",", "."),
      );
    }
  });

  it("NO acota: un 140 % llega entero a la guarda, que es quien lo rechaza", () => {
    expect(fractionFromPercent("140")).toBe("1.4");
  });
});

describe("S6 — el indicador único de guardado", () => {
  const t0 = 1_700_000_000_000;

  it("el error manda sobre todo lo demás", () => {
    expect(
      saveIndicatorLabel({
        saving: true,
        savedAtMs: t0,
        nowMs: t0,
        error: true,
        blocked: true,
      }),
    ).toEqual({ text: "No se pudo guardar", tone: "danger" });
  });

  it("guardando gana al bloqueo y al último guardado", () => {
    expect(
      saveIndicatorLabel({ saving: true, savedAtMs: t0, nowMs: t0 + 9000, blocked: true })
        .text,
    ).toBe("Guardando…");
  });

  it("un obligatorio sin rellenar se dice, no se calla", () => {
    const r = saveIndicatorLabel({
      saving: false,
      savedAtMs: t0,
      nowMs: t0 + 9000,
      blocked: true,
    });
    expect(r).toEqual({ text: "Sin guardar · falta un dato", tone: "danger" });
  });

  it("sin ningún guardado todavía, el estado es la promesa del autosave", () => {
    expect(saveIndicatorLabel({ saving: false, savedAtMs: null, nowMs: t0 }).text).toBe(
      "Guardado automático",
    );
  });

  it("los plazos: segundos, minutos y horas — y nada por debajo de 5 s", () => {
    const at = (ms: number) =>
      saveIndicatorLabel({ saving: false, savedAtMs: t0, nowMs: t0 + ms }).text;
    expect(at(0)).toBe("Guardado");
    expect(at(4_900)).toBe("Guardado");
    expect(at(9_000)).toBe("Guardado · hace 9 s");
    expect(at(59_000)).toBe("Guardado · hace 59 s");
    expect(at(60_000)).toBe("Guardado · hace 1 min");
    expect(at(59 * 60_000)).toBe("Guardado · hace 59 min");
    expect(at(3 * 3600_000)).toBe("Guardado · hace 3 h");
  });

  it("un reloj que retrocede no imprime plazos negativos", () => {
    expect(
      saveIndicatorLabel({ saving: false, savedAtMs: t0, nowMs: t0 - 60_000 }).text,
    ).toBe("Guardado");
  });
});

describe("U2 — qué obligatorio falta", () => {
  it("sin fecha de nacimiento, la estrategia por edad no está completa", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ strategy: "retire_at_age", target_retirement_age: 60 }),
        required: ["birth_date", "target_retirement_age"],
        birthDate: null,
      }),
    ).toEqual(["birth_date"]);
  });

  it("con fecha y edad, no falta nada", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ strategy: "retire_at_age", target_retirement_age: 60 }),
        required: ["birth_date", "target_retirement_age"],
        birthDate: "1990-06-15",
      }),
    ).toEqual([]);
  });

  it("la edad objetivo ausente se reporta, y el orden es el del formulario", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ strategy: "coast" }),
        required: ["birth_date", "target_retirement_age"],
        birthDate: "   ",
      }),
    ).toEqual(["birth_date", "target_retirement_age"]);
  });

  it("coast modo B: la edad de parada sin escribir es un hueco de verdad", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ strategy: "coast", coast_mode: "fixed_stop_age", coast_stop_age: null }),
        required: ["coast_stop_age"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["coast_stop_age"]);
    expect(
      missingRequiredPlanFields({
        profile: base({ strategy: "coast", coast_mode: "fixed_stop_age", coast_stop_age: 45 }),
        required: ["coast_stop_age"],
        birthDate: "1990-06-15",
      }),
    ).toEqual([]);
  });

  it("la pensión declarada exige su importe, no solo la casilla", () => {
    const sinImporte = base({ pension: pension({ monthly_amount_today: "" }) });
    expect(
      missingRequiredPlanFields({
        profile: sinImporte,
        required: ["pension_amount", "pension_start_age"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["pension_amount"]);
  });

  it("el puente encendido exige sus DOS números: un `null` es un puente sin tope", () => {
    const conPuente = base({
      pension: pension({ bridge_enabled: true, bridge_max_pct: null, bridge_max_years: null }),
    });
    expect(
      missingRequiredPlanFields({
        profile: conPuente,
        required: ["bridge_max_pct", "bridge_max_years"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["bridge_max_pct", "bridge_max_years"]);
    const completo = base({
      pension: pension({ bridge_enabled: true, bridge_max_pct: "8", bridge_max_years: 7 }),
    });
    expect(
      missingRequiredPlanFields({
        profile: completo,
        required: ["bridge_max_pct", "bridge_max_years"],
        birthDate: "1990-06-15",
      }),
    ).toEqual([]);
  });

  it("un ingreso parcial VACÍO es un año sabático declarado, no un hueco", () => {
    const p = base({
      strategy: "partial",
      partial_retirement: {
        mode: "at_age",
        starts_at_age: 55,
        income_monthly_today: "",
        expense_basis: "retirement",
      },
    });
    expect(
      missingRequiredPlanFields({
        profile: p,
        required: ["partial_start_age", "partial_income"],
        birthDate: "1990-06-15",
      }),
    ).toEqual([]);
  });

  it("la edad de inicio de la fase SÍ es un hueco cuando el bloque la trae en `null`", () => {
    // Cambio del modelo v2: `starts_at_age` pasó a `number | null` (en modo «en cuanto pueda» la
    // resuelve el servidor). Antes bastaba con que el bloque existiera; hoy un bloque en modo A
    // recién creado, sin edad escrita, es exactamente el hueco que hay que enseñar.
    const p = base({
      strategy: "partial",
      partial_retirement: {
        mode: "at_age",
        starts_at_age: null,
        income_monthly_today: "800",
        expense_basis: "retirement",
      },
    });
    expect(
      missingRequiredPlanFields({
        profile: p,
        required: ["partial_start_age", "partial_income"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["partial_start_age"]);
  });

  it("el gasto manual sin importe está incompleto", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ fire_number_mode: "manual", fire_number_manual_amount: null }),
        required: ["fire_number_manual_amount"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["fire_number_manual_amount"]);
  });

  it("los supuestos nunca faltan: tienen default del servidor (el umbral incluido)", () => {
    expect(
      missingRequiredPlanFields({
        profile: base(),
        required: ["success_threshold_pct", "swr_pct", "horizon_lifespan_age", "spend_mode"],
        birthDate: null,
      }),
    ).toEqual([]);
  });
});

describe("cableado campo → ayuda", () => {
  /**
   * Sin excepciones desde W8: las cuatro claves que el modelo v2 estrenó
   * (`retirement.success_threshold`, `retirement.bridge_settings`, `retirement.coast_mode`,
   * `retirement.partial_mode`) ya están en el catálogo, así que la lista de pendientes y el `it`
   * que la hacía caducar se borraron con ella — junto al `as` temporal de `retirement-form.ts`.
   */
  it("todo id de ayuda cableado existe en el catálogo", () => {
    for (const [field, entry] of Object.entries(PLAN_FIELD_HELP)) {
      expect(HELP_TEXTS[entry!.helpId], `${field} apunta a un texto inexistente`).toBeDefined();
    }
  });

  /**
   * 2026-09-13 — **un `<?>` por texto distinto, en el sitio más alto que lo gobierna.** Antes
   * cada campo llevaba el suyo, y como varios compartían entrada del catálogo la pantalla
   * pintaba el MISMO icono hasta cuatro veces en el mismo apartado (los cuatro de «Pensión»,
   * los tres del puente, los cuatro de la regla de retirada). El texto no se perdió: subió al
   * título del apartado o al de su sub-sección. Este test fija las dos mitades de esa regla —
   * quién conserva icono propio y quién NO debe volver a tenerlo—, porque re-añadir uno es el
   * error natural al tocar un campo sin mirar su vecino.
   */
  it("los campos con ayuda PROPIA la conservan", () => {
    for (const id of [
      "success_threshold_pct",
      "coast_mode",
      "partial_mode",
      "target_retirement_age",
      "partial_income",
      "swr_pct",
      "withdrawal_rule_kind",
      "spend_mode",
      // El interruptor del puente recupera el suyo al retirarse el rótulo de su sub-columna:
      // pasó a ser lo más alto que gobierna esos tres campos (2026-09-13).
      "bridge_enabled",
    ] as const) {
      expect(PLAN_FIELD_HELP[id], `${id} sin ayuda`).toBeDefined();
    }
  });

  it("los campos que COMPARTÍAN texto ya no repiten el icono", () => {
    // Cada uno con el sitio al que subió su ayuda, para que el que lo revierta sepa adónde mirar.
    const subieron = {
      coast_stop_age: "comparte `retirement.coast_mode` con el campo del modo",
      partial_start_age: "comparte `retirement.partial` con el ingreso de la fase",
      partial_expense_basis: "comparte `retirement.partial` con el ingreso de la fase",
      pension_amount: "sube al título del apartado «Pensión»",
      pension_start_age: "sube al título del apartado «Pensión»",
      pension_indexed: "sube al título del apartado «Pensión»",
      pension_fraction_while_partial: "sube al título del apartado «Pensión»",
      bridge_max_pct: "comparte `retirement.bridge_settings` con el interruptor del puente",
      bridge_max_years: "comparte `retirement.bridge_settings` con el interruptor del puente",
      hybrid_end_pct: "comparte `retirement.withdrawal_rule` con el selector de regla",
      guardrails_band_pct: "comparte `retirement.withdrawal_rule` con el selector de regla",
      guardrails_adjust_pct: "comparte `retirement.withdrawal_rule` con el selector de regla",
      horizon_lifespan_age: "sube al título del apartado «Horizonte»",
    } as const;
    for (const [id, donde] of Object.entries(subieron)) {
      expect(
        PLAN_FIELD_HELP[id as keyof typeof PLAN_FIELD_HELP],
        `${id} vuelve a duplicar un icono: su ayuda ${donde}`,
      ).toBeUndefined();
    }
  });

  it("los seis apartados tienen `<?>`, y su id existe en el catálogo", () => {
    expect(Object.keys(PLAN_CARD_HELP).sort()).toEqual([
      "ages",
      "horizon",
      "pension",
      "spending",
      "strategy",
      "withdrawal",
    ]);
    for (const [card, entry] of Object.entries(PLAN_CARD_HELP)) {
      expect(HELP_TEXTS[entry.helpId], `${card} apunta a un texto inexistente`).toBeDefined();
    }
  });

  it("los dos campos que murieron con el objetivo ya no están cableados", () => {
    const cableados = Object.keys(PLAN_FIELD_HELP);
    expect(cableados).not.toContain("target_basis");
    expect(cableados).not.toContain("bridge_discount_basis");
  });
});

describe("PLAN_CARD_COPY — el título de cada apartado", () => {
  // La FRASE de cada apartado se fue al catálogo el 2026-09-13 (ver `PLAN_CARD_HELP`), y con ella
  // los guardas que fijaban su contenido, que hoy viven en `helpTexts.test.ts`. Aquí queda lo que
  // sigue siendo de esta tabla: que hay seis y que ninguno se queda sin rótulo.
  it("todos los apartados tienen un título corto", () => {
    for (const [card, copy] of Object.entries(PLAN_CARD_COPY)) {
      expect(copy.title.length, `${card}: título vacío`).toBeGreaterThan(2);
      expect(copy.title.length, `${card}: título demasiado largo`).toBeLessThanOrEqual(28);
    }
  });

  it("son SEIS apartados: el umbral volvió a «Retirada», no abrió uno «Riesgo»", () => {
    expect(Object.keys(PLAN_CARD_COPY).sort()).toEqual([
      "ages",
      "horizon",
      "pension",
      "spending",
      "strategy",
      "withdrawal",
    ]);
  });
});

describe("U4 — la nota del porcentaje de la regla", () => {
  const rule = (over: Partial<ReturnType<typeof defaultWithdrawalRuleApi>> = {}) => ({
    ...defaultWithdrawalRuleApi(),
    ...over,
  });

  it("«Gasto fijo» no retira un porcentaje: no hay nota", () => {
    expect(withdrawalPctNote({ rule: rule(), swrPct: "3.5", pctSource: null })).toBeNull();
  });

  it("sin porcentaje propio, la regla hereda el SWR y dice QUÉ es el SWR en v2", () => {
    // Ya no es «tu tasa de retirada» a secas: en el modelo v2 el SWR es el tope de la TASA
    // INICIAL, medida en la fecha de jubilación sobre el patrimonio líquido (C1).
    expect(
      withdrawalPctNote({
        rule: rule({ kind: "percent_of_balance" }),
        swrPct: "3.5",
        pctSource: "swr",
      }),
    ).toBe("Retira el 3,5 %: lo máximo que sacas el primer año sobre tu líquido.");
  });

  it("la híbrida mira `start_pct`, no `pct`", () => {
    expect(
      withdrawalPctNote({
        rule: rule({ kind: "hybrid", start_pct: "5", end_pct: "3" }),
        swrPct: "3.5",
        pctSource: "explicit",
      }),
    ).toBe("Regla al 5,0 %, fijado por API.");
  });

  it("un porcentaje ESCRITO se anuncia como fijado por API aunque el backend no publique la procedencia", () => {
    // El caso real: backend anterior a U4, `pct` 4 % y SWR 3 %. Decir «tu tasa de retirada»
    // aquí sería mentir sobre la cifra que el slider mueve.
    expect(
      withdrawalPctNote({
        rule: rule({ kind: "guardrails", pct: "4", band_pct: "20", adjust_pct: "10" }),
        swrPct: "3",
        pctSource: null,
      }),
    ).toBe("Regla al 4,0 %, fijado por API.");
  });

  it("un servidor que dice «lo heredé» manda sobre la heurística del valor", () => {
    expect(
      withdrawalPctNote({
        rule: rule({ kind: "percent_of_balance", pct: "3" }),
        swrPct: "3",
        pctSource: "swr",
      }),
    ).toBe("Retira el 3,0 %: lo máximo que sacas el primer año sobre tu líquido.");
  });
});
