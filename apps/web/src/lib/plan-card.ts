/**
 * Modelo PURO de la tarjeta «Tu plan» del Resumen (5.0.0, D27 / §G del plan de #207) y de las
 * tarjetas por miembro de la vista Hogar (D32).
 *
 * La tarjeta contesta tres cosas y nada más: con qué ESTRATEGIA simulas, cuál es tu PRÓXIMO HITO
 * (fecha y edad de la jubilación efectiva) y en qué ESTADO estás. Aquí vive la traducción de esos
 * tres campos del servidor a copy; la vista solo pinta.
 *
 * Por qué es un módulo aparte y no lógica dentro de `SummaryView`: el estado sale de un array de
 * literales cerrados (`warnings[]`) con precedencia entre ellos, y esa precedencia es exactamente
 * el tipo de regla que se rompe en silencio al añadir el cuarto aviso. Con la regla aquí, un test
 * la fija (`plan-card.test.ts`).
 */

import type { RetirementStrategyApi } from "../api/types";
import { formatDateDmy } from "./dates";

/**
 * Avisos que el plan sabe leer. Literales cerrados del contrato (`warnings[]` de
 * `GET /v1/projection/series` y de `members[]`).
 *
 * `retire_at_age_underfunded` **todavía no lo emite el motor** — lo publicará la ola de los solves
 * (`solve.rs`, §B.7: `underfunded = c > sobrante`). El mapeo se escribe ya, en rojo, para que el
 * día que llegue no haya que tocar la vista: un aviso sin traducción se pinta como nada, que es
 * peor que pintarlo mal.
 */
export type PlanWarning =
  | "birth_date_missing"
  | "target_retirement_age_missing"
  | "retire_at_age_underfunded";

export type PlanStatusTone = "ok" | "warn" | "danger";

export type PlanStatus = {
  /** El aviso que ganó la precedencia, o `null` cuando no hay ninguno («En plan»). */
  warning: PlanWarning | null;
  tone: PlanStatusTone;
  label: string;
  /** Adónde lleva el enlace de arreglo, cuando hay algo que arreglar. */
  action: { label: string; target: "account" | "retirement" } | null;
};

/**
 * Precedencia deliberada: **primero lo que invalida el plan, después lo que le falta**.
 *
 * - `retire_at_age_underfunded` gana siempre: el plan está completo y NO llega (D17 — la edad
 *   manda y el aviso es rojo grande). Es un resultado, no un hueco de configuración.
 * - `birth_date_missing` y `target_retirement_age_missing` son datos que faltan; con ellos la
 *   estrategia por edad **degradó a «Cuanto antes»**, así que el hito que se ve al lado es el de
 *   otra simulación — decirlo es el objetivo de la línea.
 * - Sin avisos: «En plan».
 *
 * Los literales que no conoce se ignoran (un aviso nuevo del servidor nunca deja la tarjeta sin
 * estado; a lo sumo dice «En plan» hasta que alguien lo traduzca aquí).
 */
export function planStatusFromWarnings(
  warnings: readonly string[] | null | undefined,
): PlanStatus {
  const set = new Set(warnings ?? []);
  if (set.has("retire_at_age_underfunded")) {
    return {
      warning: "retire_at_age_underfunded",
      tone: "danger",
      label: "Con tu ahorro actual no llegas a tu edad objetivo",
      action: { label: "Revisar tu plan", target: "retirement" },
    };
  }
  if (set.has("birth_date_missing")) {
    return {
      warning: "birth_date_missing",
      tone: "warn",
      label: "Falta tu fecha de nacimiento",
      action: { label: "Tu cuenta", target: "account" },
    };
  }
  if (set.has("target_retirement_age_missing")) {
    return {
      warning: "target_retirement_age_missing",
      tone: "warn",
      label: "Falta tu edad de jubilación objetivo",
      action: { label: "Elegir edad", target: "retirement" },
    };
  }
  return { warning: null, tone: "ok", label: "En plan", action: null };
}

/** Copy de cada razón por la que el hito no existe POR CONSTRUCCIÓN (`jubilacion_absent_reason`). */
const ABSENT_REASON_ES: Record<string, string> = {
  household_aggregate: "El hogar suma varios planes: mira la tarjeta de cada persona",
  no_retirement_trigger: "Sin objetivo ni edad objetivo: este plan no se jubila",
};

export type PlanMilestone = {
  /** Cifra grande de la tarjeta: la fecha de la jubilación efectiva, o el motivo de su ausencia. */
  value: string;
  /** Línea de apoyo: la edad a la que ocurre. `null` cuando no hay edad resoluble. */
  detail: string | null;
  /** `false` ⇒ `value` es una explicación, no una fecha (la vista la pinta apagada). */
  reached: boolean;
};

/**
 * El próximo hito del plan: la **jubilación EFECTIVA** (`jubilacion_*`, que desde 5.0.0 es el mes
 * en que el motor te jubila de verdad — cruce o edad, R8), nunca el cruce del objetivo.
 *
 * `null` en el índice significa cosas distintas según venga o no acompañado de razón, y la
 * diferencia es justo lo que hay que decir en pantalla: **con** razón, la pregunta no aplica en
 * esta respuesta; **sin** razón, hay plan y no se jubila dentro del horizonte.
 */
export function planMilestone(input: {
  jubilacionMonthIndex?: number | null;
  jubilacionDateYmd?: string | null;
  jubilacionAge?: number | null;
  jubilacionAbsentReason?: string | null;
}): PlanMilestone {
  if (input.jubilacionMonthIndex == null) {
    const reason = input.jubilacionAbsentReason;
    return {
      value: reason
        ? (ABSENT_REASON_ES[reason] ?? "Sin hito que mostrar")
        : "Sin cruce en el horizonte",
      detail: null,
      reached: false,
    };
  }
  const ymd = input.jubilacionDateYmd?.trim();
  return {
    value: ymd ? formatDateDmy(ymd) : `Mes ${input.jubilacionMonthIndex}`,
    detail: input.jubilacionAge != null ? `a los ${input.jubilacionAge} años` : null,
    reached: true,
  };
}

/** Una tarjeta de plan ya resuelta — la del usuario en «Yo», o una por miembro en «Hogar». */
export type PlanCardModel = {
  key: string;
  /** Nombre de la persona; `null` en la tarjeta propia (el título ya es «Tu plan»). */
  name: string | null;
  strategy: RetirementStrategyApi | null;
  milestone: PlanMilestone;
  status: PlanStatus;
};
