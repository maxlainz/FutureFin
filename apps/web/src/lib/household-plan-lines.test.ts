/**
 * Las líneas del hogar (U10 + bug B7): números agregados arriba y **una oración por persona**,
 * cada una con SU estado.
 *
 * Dos invariantes que hay que proteger:
 *
 *  * **El ORDEN.** El índice de `members[]` es el que fija el color de la línea fina de cada
 *    persona en el chart y el de su tick en la tira de fases (`householdMemberColor`). Reordenar
 *    aquí no rompería nada visible — solo haría que la frase de Max acompañara a la curva de
 *    Mariona.
 *  * **El TONO (B7).** Cada línea lleva el suyo. Antes las frases se leían todas iguales, y un
 *    miembro con un aviso propio (le falta un dato, o no llega con la estrategia que fijó) pasaba
 *    por uno que llega.
 */

import { describe, expect, it } from "vitest";
import { householdPlanLines, type HouseholdPlanLineMember } from "./household-plan-lines";

const monthLabel = (mi: number) => `M${mi}`;

function member(over: Partial<HouseholdPlanLineMember> = {}): HouseholdPlanLineMember {
  return {
    user_id: "u1",
    username: "Max",
    strategy: "asap",
    jubilacion_month_index: null,
    jubilacion_age: null,
    partial_retirement_month_index: null,
    warnings: [],
    plan_state: "household_not_solved",
    ...over,
  };
}

describe("householdPlanLines", () => {
  it("el ejemplo de U10 en el modelo v2: fecha fijada de quien la tiene, con el rótulo de su estrategia", () => {
    const lines = householdPlanLines(
      [
        member({
          user_id: "u1",
          username: "Max",
          strategy: "retire_at_age",
          jubilacion_month_index: 144,
          jubilacion_age: 55,
        }),
        member({
          user_id: "u2",
          username: "Mariona",
          strategy: "partial",
          jubilacion_month_index: 216,
          jubilacion_age: 60,
          partial_retirement_month_index: 120,
        }),
      ],
      monthLabel,
    );
    expect(lines).toEqual([
      {
        userId: "u1",
        username: "Max",
        text: "Max: A una edad fija — a los 55, M144.",
        tone: "ok",
      },
      {
        userId: "u2",
        username: "Mariona",
        text:
          "Mariona: Jornada reducida (Barista FIRE) — a los 60, M216 (hace jornada reducida desde M120).",
        tone: "ok",
      },
    ]);
  });

  it("una estrategia por umbral no tiene fecha en Hogar: se dice, y se manda a su vista Jubilación (B7)", () => {
    const [line] = householdPlanLines(
      [member({ username: "Ada", strategy: "asap" })],
      monthLabel,
    );
    expect(line.text).toBe(
      "Ada: Cuanto antes (FIRE clásico) — fecha válida: en su vista Jubilación.",
    );
    expect(line.tone).toBe("ok");
  });

  // El hogar no publica aportación mínima ni margen por miembro (D9): sin ese booleano, lo único
  // que puede pintar de rojo o de ámbar a un miembro es uno de sus `warnings`.
  it("el estado de cada persona viaja en su línea: un aviso de configuración en ámbar, la fecha de nacimiento en rojo (B7)", () => {
    const lines = householdPlanLines(
      [
        member({
          user_id: "u1",
          username: "Max",
          strategy: "retire_at_age",
          jubilacion_month_index: 144,
          jubilacion_age: 55,
          warnings: ["target_retirement_age_missing"],
        }),
        member({
          user_id: "u2",
          username: "Ada",
          warnings: ["birth_date_missing"],
        }),
      ],
      monthLabel,
    );
    expect(lines[0].text).toBe(
      "Max: A una edad fija — a los 55, M144 — falta su edad de jubilación.",
    );
    expect(lines[0].tone).toBe("warn");
    expect(lines[1].tone).toBe("danger");
    expect(lines[1].text).toContain("falta su fecha de nacimiento");
  });

  it("conserva el orden del servidor: es el que empareja cada frase con su curva", () => {
    const ids = ["c", "a", "b"];
    const lines = householdPlanLines(
      ids.map((id, i) =>
        member({
          user_id: id,
          username: id.toUpperCase(),
          strategy: "retire_at_age",
          jubilacion_month_index: 12 * (3 - i),
          jubilacion_age: 50 + i,
        }),
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

  it("un miembro sin fecha no desaparece de la lista", () => {
    const lines = householdPlanLines(
      [
        member({
          user_id: "u1",
          username: "Max",
          strategy: "retire_at_age",
          jubilacion_month_index: 144,
          jubilacion_age: 55,
        }),
        member({ user_id: "u2", username: "Ada" }),
      ],
      monthLabel,
    );
    expect(lines).toHaveLength(2);
    expect(lines[1].username).toBe("Ada");
  });

  it("cada línea lleva su `userId` (key de React y ancla del color del miembro)", () => {
    const lines = householdPlanLines(
      [member({ user_id: "abc", strategy: "retire_at_age", jubilacion_month_index: 24 })],
      monthLabel,
    );
    expect(lines[0].userId).toBe("abc");
    expect(lines[0].username).toBe("Max");
  });

  it("no publica cifras al mes: en Hogar no hay bases comparables entre personas", () => {
    const [line] = householdPlanLines(
      [member({ strategy: "retire_at_age", jubilacion_month_index: 144, jubilacion_age: 55 })],
      monthLabel,
    );
    expect(Object.keys(line).sort()).toEqual(["text", "tone", "userId", "username"]);
  });
});
