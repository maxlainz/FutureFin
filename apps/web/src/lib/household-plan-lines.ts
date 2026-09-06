/**
 * Las líneas por miembro de la vista Hogar (5.0.0, rediseño UX U1a; decisión U10 de #207; bug
 * B7 del modelo v2).
 *
 * U10 en una frase: **en Hogar se enseñan números agregados y UNA ORACIÓN por persona**, no una
 * tarjeta de plan por miembro con sus cifras. El motivo es de contrato, no de layout: el hogar
 * no tiene plan propio (`strategy` viaja `null`, el bloque «plan» entero va vacío con
 * `plan_absent_reason: "household_not_solved"` y cada fila llega con
 * `plan_state: "household_not_solved"`), así que lo único que se puede decir de cada persona sin
 * mezclar bases es su hito determinista. Una rejilla de tarjetas invitaba justo a lo contrario —
 * comparar el «ahorro necesario» de dos personas con edades objetivo distintas, que no es una
 * comparación.
 *
 * **B7**: la línea lleva además el TONO de esa persona. Hasta el modelo v2 la frase leía tres
 * campos y se callaba el estado que el servidor ya publicaba, así que un miembro
 * infra-financiado o al que le falta la fecha de nacimiento se leía exactamente igual que uno
 * que llega — mientras la tarjeta propia sí lo pintaba de rojo.
 *
 * El orden es **el del servidor** (`members[]`), sin reordenar: ese mismo orden es el que fija el
 * color de cada línea fina del chart (`householdMemberColor(idx)`, `lib/chart-legend.ts`) y el de
 * su tick en la tira de fases. Ordenar aquí por nombre o por fecha rompería el emparejamiento
 * entre la frase y la curva sin que nada fallara.
 */

import type { HouseholdMemberProjectionApi } from "../api/types";
import {
  memberPlanSentence,
  type MemberPlanSentenceMember,
  type PlanSentenceTone,
} from "./plan-sentence";

/** Una línea lista para pintar. `userId` es la key de React y el ancla del color del miembro;
 *  `tone` es el estado de ESA persona (B7), que el `<li>` traduce a su piel. */
export type HouseholdPlanLine = {
  userId: string;
  username: string;
  text: string;
  tone: PlanSentenceTone;
};

export type HouseholdPlanLineMember = MemberPlanSentenceMember &
  Pick<HouseholdMemberProjectionApi, "user_id">;

/**
 * `members[]` → una frase por miembro, **en el orden en que llegaron**.
 *
 * Sin miembros devuelve el array vacío: la vista decide si eso es «cargando» o «no hay hogar», y
 * este módulo no puede saberlo.
 */
export function householdPlanLines(
  members: readonly HouseholdPlanLineMember[] | null | undefined,
  monthLabel: (monthIndex: number) => string,
): HouseholdPlanLine[] {
  if (!members || members.length === 0) return [];
  return members.map((m) => {
    const sentence = memberPlanSentence(m, monthLabel);
    return {
      userId: m.user_id,
      username: m.username,
      text: sentence.text,
      tone: sentence.tone,
    };
  });
}
