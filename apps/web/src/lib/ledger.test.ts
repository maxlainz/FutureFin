/**
 * Orden del presupuesto (`sortBudgetEntriesMacStyle`) tras la fusión de la 3.7.0: las cuotas de
 * pasivo dejaron de vivir en un bloque aparte y llegan dentro de `entries`, así que el orden es lo
 * único que las coloca «como segunda línea» de su categoría en vez de sueltas por la tabla.
 */
import { describe, expect, it } from "vitest";
import type { BudgetEntryApiRow, CategoryRow } from "../api/types";
import { sortBudgetEntriesMacStyle } from "./ledger";

function cat(id: string, name: string): CategoryRow {
  return { id, scope: "expense", name, sort_index: 0, is_fallback: false };
}

function entry(
  id: string,
  categoryId: string | undefined,
  amount: string,
  source: "manual" | "liability" = "manual",
): BudgetEntryApiRow {
  return {
    id,
    category_id: categoryId,
    scope: "expense",
    source,
    amount,
    notes: null,
    sort_index: 0,
    persists_after_retirement: false,
    ends_at_retirement: false,
    expense_end_date: null,
    ...(source === "liability" ? { liability_id: id, label: `Pasivo ${id}` } : {}),
  };
}

const CATEGORIES = new Map<string, CategoryRow>([
  ["hogar", cat("hogar", "Hogar")],
  ["ocio", cat("ocio", "Ocio")],
]);

describe("sortBudgetEntriesMacStyle", () => {
  it("coloca la cuota de pasivo justo detrás de la partida manual de su categoría", () => {
    // Hogar suma 100 + 274 = 374 → bloque por delante de Ocio (200), aunque su fila manual (100)
    // sea menor que la de Ocio: el orden es por total de categoría, no por importe de fila.
    const sorted = sortBudgetEntriesMacStyle(
      [
        entry("ocio-1", "ocio", "200"),
        entry("hogar-quota", "hogar", "274", "liability"),
        entry("hogar-1", "hogar", "100"),
      ],
      CATEGORIES,
    );
    expect(sorted.map((e) => e.id)).toEqual(["hogar-1", "hogar-quota", "ocio-1"]);
  });

  it("mantiene la cuota detrás aunque su importe sea mayor que el de la partida manual", () => {
    const sorted = sortBudgetEntriesMacStyle(
      [
        entry("quota", "hogar", "900", "liability"),
        entry("manual", "hogar", "50"),
      ],
      CATEGORIES,
    );
    expect(sorted.map((e) => e.id)).toEqual(["manual", "quota"]);
  });

  it("manda al final las cuotas sin categoría asignada (pasivos anteriores a 3.4.0)", () => {
    // Su importe (5.000) dominaría el orden si se agrupasen como una categoría más.
    const sorted = sortBudgetEntriesMacStyle(
      [
        entry("huerfana", undefined, "5000", "liability"),
        entry("ocio-1", "ocio", "200"),
        entry("hogar-1", "hogar", "100"),
      ],
      CATEGORIES,
    );
    expect(sorted.map((e) => e.id)).toEqual(["ocio-1", "hogar-1", "huerfana"]);
  });

  it("conserva el orden histórico cuando no hay cuotas", () => {
    const sorted = sortBudgetEntriesMacStyle(
      [
        entry("hogar-1", "hogar", "100"),
        entry("ocio-1", "ocio", "200"),
        entry("hogar-2", "hogar", "500"),
      ],
      CATEGORIES,
    );
    // Hogar 600 > Ocio 200; dentro de Hogar, importe descendente.
    expect(sorted.map((e) => e.id)).toEqual(["hogar-2", "hogar-1", "ocio-1"]);
  });
});
