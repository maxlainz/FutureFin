/**
 * Las líneas por miembro de la vista Hogar (5.0.0, rediseño UX U1a; decisión U10 de #207).
 *
 * U10 en una frase: **en Hogar se enseñan números agregados y UNA ORACIÓN por persona**, no una
 * tarjeta de plan por miembro con sus cifras. El motivo es de contrato, no de layout: el hogar no
 * tiene plan propio (`strategy` viaja `null` y todo el bloque de solves va vacío en
 * `view=household`), así que lo único que se puede decir de cada persona sin mezclar bases es su
 * hito. Una rejilla de tarjetas invitaba justo a lo contrario — comparar el «ahorro necesario» de
 * dos personas con edades objetivo distintas, que no es una comparación.
 *
 * El orden es **el del servidor** (`members[]`), sin reordenar: ese mismo orden es el que fija el
 * color de cada línea fina del chart (`householdMemberColor(idx)`, `lib/chart-legend.ts`) y el de
 * su tick en la tira de fases. Ordenar aquí por nombre o por fecha rompería el emparejamiento
 * entre la frase y la curva sin que nada fallara.
 */

import type { HouseholdMemberProjectionApi } from "../api/types";
import { memberPlanSentence, type MemberPlanSentenceMember } from "./plan-sentence";

/** Una línea lista para pintar. `userId` es la key de React y el ancla del color del miembro. */
export type HouseholdPlanLine = {
  userId: string;
  username: string;
  text: string;
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
  return members.map((m) => ({
    userId: m.user_id,
    username: m.username,
    text: memberPlanSentence(m, monthLabel),
  }));
}
