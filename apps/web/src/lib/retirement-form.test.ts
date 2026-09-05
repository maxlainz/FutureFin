import { describe, expect, it } from "vitest";
import {
  PLAN_CARD_COPY,
  PLAN_FIELD_HELP,
  fractionFromPercent,
  missingRequiredPlanFields,
  percentFromFraction,
  saveIndicatorLabel,
  withdrawalPctNote,
} from "./retirement-form";
import { HELP_TEXTS } from "./helpTexts";
import {
  defaultRetirementProfileApi,
  defaultWithdrawalRuleApi,
} from "./retirementProfile";
import type { RetirementProfileApi } from "../api/types";

const base = (over: Partial<RetirementProfileApi> = {}): RetirementProfileApi => ({
  ...defaultRetirementProfileApi(),
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

  it("el puente exige la pensión CON importe, no solo la casilla", () => {
    const sinImporte = base({
      strategy: "pension_bridge",
      pension: { monthly_amount_today: "", starts_at_age: 67, indexed: true, fraction_while_partial: "0" },
    });
    expect(
      missingRequiredPlanFields({
        profile: sinImporte,
        required: ["pension_amount", "pension_start_age"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["pension_amount"]);
  });

  it("un ingreso parcial VACÍO es un año sabático declarado, no un hueco", () => {
    const p = base({
      strategy: "partial",
      partial_retirement: {
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

  it("el objetivo manual sin importe está incompleto", () => {
    expect(
      missingRequiredPlanFields({
        profile: base({ fire_number_mode: "manual", fire_number_manual_amount: null }),
        required: ["fire_number_manual_amount"],
        birthDate: "1990-06-15",
      }),
    ).toEqual(["fire_number_manual_amount"]);
  });

  it("los supuestos nunca faltan: tienen default del servidor", () => {
    expect(
      missingRequiredPlanFields({
        profile: base(),
        required: ["swr_pct", "horizon_lifespan_age", "spend_mode"],
        birthDate: null,
      }),
    ).toEqual([]);
  });
});

describe("cableado campo → ayuda", () => {
  it("todo id de ayuda cableado existe en el catálogo", () => {
    for (const [field, entry] of Object.entries(PLAN_FIELD_HELP)) {
      expect(HELP_TEXTS[entry!.helpId], `${field} apunta a un texto inexistente`).toBeDefined();
    }
  });

  it("todas las tarjetas tienen título corto y una frase con sustancia acabada en punto", () => {
    // No juzga prosa: caza la tarjeta que alguien añade sin frase. Una tarjeta con título y sin
    // frase deja el formulario igual de mudo que antes de V3, con un separador más.
    for (const [card, copy] of Object.entries(PLAN_CARD_COPY)) {
      expect(copy.title.length, `${card}: título vacío`).toBeGreaterThan(2);
      expect(copy.title.length, `${card}: título demasiado largo`).toBeLessThanOrEqual(28);
      expect(copy.blurb.length, `${card}: frase demasiado corta`).toBeGreaterThan(40);
      expect(copy.blurb.endsWith("."), `${card}: la frase no acaba en punto`).toBe(true);
    }
  });

  it("son SEIS tarjetas: «Riesgo» se quedó sin campos con V6/V7 y no se pinta", () => {
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

  it("sin porcentaje propio, la regla retira el SWR y lo dice", () => {
    expect(
      withdrawalPctNote({
        rule: rule({ kind: "percent_of_balance" }),
        swrPct: "3.5",
        pctSource: "swr",
      }),
    ).toBe("Retira el 3,5 %: tu tasa de retirada.");
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
    ).toBe("Retira el 3,0 %: tu tasa de retirada.");
  });
});
