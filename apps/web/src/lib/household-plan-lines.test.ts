/**
 * Las líneas del hogar (U10): números agregados arriba y **una oración por persona**.
 *
 * El invariante que hay que proteger es el ORDEN: el índice de `members[]` es el que fija el
 * color de la línea fina de cada persona en el chart y el de su tick en la tira de fases
 * (`householdMemberColor`). Reordenar aquí no rompería nada visible — solo haría que la frase de
 * Max acompañara a la curva de Mariona.
 */

import { describe, expect, it } from "vitest";
import { householdPlanLines, type HouseholdPlanLineMember } from "./household-plan-lines";

const monthLabel = (mi: number) => `M${mi}`;

function member(over: Partial<HouseholdPlanLineMember> = {}): HouseholdPlanLineMember {
  return {
    user_id: "u1",
    username: "Max",
    jubilacion_month_index: null,
    partial_retirement_month_index: null,
    ...over,
  };
}

describe("householdPlanLines", () => {
  it("el ejemplo de U10, tal cual", () => {
    const lines = householdPlanLines(
      [
        member({ user_id: "u1", username: "Max", jubilacion_month_index: 144 }),
        member({
          user_id: "u2",
          username: "Mariona",
          jubilacion_month_index: 216,
          partial_retirement_month_index: 120,
        }),
      ],
      monthLabel,
    );
    expect(lines).toEqual([
      { userId: "u1", username: "Max", text: "Max se quiere jubilar en 12 años." },
      {
        userId: "u2",
        username: "Mariona",
        text: "Mariona se quiere jubilar en 18 años y hacer media jornada a partir de M120.",
      },
    ]);
  });

  it("conserva el orden del servidor: es el que empareja cada frase con su curva", () => {
    const ids = ["c", "a", "b"];
    const lines = householdPlanLines(
      ids.map((id, i) =>
        member({ user_id: id, username: id.toUpperCase(), jubilacion_month_index: 12 * (3 - i) }),
      ),
      monthLabel,
    );
    expect(lines.map((l) => l.userId)).toEqual(ids);
  });

  it("una lista vacía o ausente devuelve el array vacío, no una frase inventada", () => {
    expect(householdPlanLines([], monthLabel)).toEqual([]);
    expect(householdPlanLines(null, monthLabel)).toEqual([]);
    expect(householdPlanLines(undefined, monthLabel)).toEqual([]);
  });

  it("un miembro que no cruza el objetivo lo dice, y no desaparece de la lista", () => {
    const lines = householdPlanLines(
      [
        member({ user_id: "u1", username: "Max", jubilacion_month_index: 144 }),
        member({ user_id: "u2", username: "Ada" }),
      ],
      monthLabel,
    );
    expect(lines).toHaveLength(2);
    expect(lines[1].text).toBe("Ada no cruza el objetivo en el horizonte.");
  });

  it("cada línea lleva su `userId` (key de React y ancla del color del miembro)", () => {
    const lines = householdPlanLines(
      [member({ user_id: "abc", jubilacion_month_index: 24 })],
      monthLabel,
    );
    expect(lines[0].userId).toBe("abc");
    expect(lines[0].username).toBe("Max");
  });

  it("no publica cifras al mes: en Hogar no hay bases comparables entre personas", () => {
    const [line] = householdPlanLines(
      [member({ jubilacion_month_index: 144 })],
      monthLabel,
    );
    expect(Object.keys(line).sort()).toEqual(["text", "userId", "username"]);
  });
});
