import { describe, expect, it } from "vitest";
import type { RecurringRuleApi } from "../api/types";
import {
  buildRecurringPatch,
  draftFromRule,
  recurringPatchBody,
  type RecurringPatchFields,
} from "./recurring-patch";

const RULE: RecurringRuleApi = {
  id: "00000000-0000-4000-8000-000000000001",
  concept: "Gimnasio",
  amount: "-40.0000",
  kind: "expense",
  category_id: "cat-ocio",
  category_name: "Ocio",
  notes: "cuota",
  origin_month: "2026-06-01",
  created_at: "2026-06-15T10:00:00Z",
  updated_at: "2026-06-15T10:00:00Z",
};

function fields(out: ReturnType<typeof buildRecurringPatch>): RecurringPatchFields {
  if (out === null || "error" in out) throw new Error(`se esperaban campos: ${JSON.stringify(out)}`);
  return out;
}

describe("draftFromRule", () => {
  it("parte de la plantilla con el importe en magnitud y coma decimal", () => {
    expect(draftFromRule(RULE)).toEqual({
      concept: "Gimnasio",
      amount: "40",
      kind: "expense",
      categoryId: "cat-ocio",
      notes: "cuota",
    });
    expect(draftFromRule({ ...RULE, amount: "-12.5000" }).amount).toBe("12,5");
  });
});

describe("buildRecurringPatch", () => {
  it("sin cambios devuelve null (no se abre el diálogo de alcance)", () => {
    expect(buildRecurringPatch(RULE, draftFromRule(RULE))).toBeNull();
  });

  it("envía SOLO los campos cambiados: con «Todos» el resto no se pisa", () => {
    const d = { ...draftFromRule(RULE), amount: "45" };
    expect(fields(buildRecurringPatch(RULE, d))).toEqual({ amount: "-45" });
  });

  it("el signo lo pone el tipo y la coma decimal viaja como punto", () => {
    const d = { ...draftFromRule(RULE), amount: "1.234,5" };
    expect(fields(buildRecurringPatch(RULE, d))).toEqual({ amount: "-1234.5" });
  });

  it("pasar a ingreso manda kind + categoría + importe positivo", () => {
    const d = { ...draftFromRule(RULE), kind: "income" as const, categoryId: "cat-extra" };
    expect(fields(buildRecurringPatch(RULE, d))).toEqual({
      amount: "40",
      kind: "income",
      category_id: "cat-extra",
    });
  });

  it("pasar a inversión limpia la categoría (savings no admite ninguna)", () => {
    const d = { ...draftFromRule(RULE), kind: "savings" as const, categoryId: "" };
    expect(fields(buildRecurringPatch(RULE, d))).toEqual({
      kind: "savings",
      clear_category: true,
    });
  });

  it("vaciar las notas manda clear_notes, no una cadena vacía", () => {
    const d = { ...draftFromRule(RULE), notes: "  " };
    expect(fields(buildRecurringPatch(RULE, d))).toEqual({ clear_notes: true });
  });

  it("rechaza concepto vacío e importe no positivo antes de preguntar el alcance", () => {
    expect(buildRecurringPatch(RULE, { ...draftFromRule(RULE), concept: " " })).toEqual({
      error: "El concepto no puede quedar vacío.",
    });
    expect(buildRecurringPatch(RULE, { ...draftFromRule(RULE), amount: "0" })).toEqual({
      error: "El importe debe ser un número mayor que 0.",
    });
  });
});

describe("recurringPatchBody", () => {
  it("el alcance elegido en el diálogo viaja como apply_to junto a los campos", () => {
    expect(recurringPatchBody("all", { amount: "-45" })).toEqual({
      apply_to: "all",
      amount: "-45",
    });
    expect(recurringPatchBody("future", { notes: "x" })).toEqual({
      apply_to: "future",
      notes: "x",
    });
  });
});
