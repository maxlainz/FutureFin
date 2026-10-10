/**
 * Edición de una plantilla recurrente (5.0.0): del borrador del formulario al cuerpo de
 * `PATCH /v1/transactions/recurring/{id}`. Puro (sin I/O ni React), para poder testearlo.
 *
 * El cuerpo lleva SOLO los campos que el usuario ha cambiado. No es una optimización: con
 * `apply_to: "all"` el servidor reescribe exactamente los campos presentes en TODOS los
 * movimientos de la recurrencia, así que mandar el formulario entero pisaría los retoques
 * manuales de cada movimiento en campos que nadie ha tocado.
 */

import type {
  PatchRecurringRuleRequest,
  RecurringApplyTo,
  RecurringRuleApi,
  TransactionKindApi,
} from "../api/types";
import {
  DecimalInputError,
  formatEditableDecimalString,
  parseDisplayDecimal,
  toApiDecimalString,
} from "./format";

export type RuleDraft = {
  concept: string;
  /** Magnitud tecleada (sin signo): el signo lo pone el tipo, como en «Añadir efectivo». */
  amount: string;
  kind: TransactionKindApi;
  categoryId: string;
  notes: string;
};

/** Campos del PATCH sin el alcance, que se decide después en el diálogo. */
export type RecurringPatchFields = Omit<PatchRecurringRuleRequest, "apply_to">;

/** Borrador inicial: la plantilla tal cual, con el importe en magnitud. */
export function draftFromRule(r: RecurringRuleApi): RuleDraft {
  // Magnitud con coma decimal española («40», «12,5»): lo que el usuario espera ver y teclear.
  const raw = r.amount.trim().replace(/^[+-]/, "");
  return {
    concept: r.concept,
    amount: formatEditableDecimalString(raw),
    kind: r.kind,
    categoryId: r.kind === "savings" ? "" : r.category_id ?? "",
    notes: r.notes ?? "",
  };
}

/**
 * Diff del borrador contra la regla → campos del PATCH. `null` si no cambia nada; `{ error }` si
 * el borrador no es válido.
 */
export function buildRecurringPatch(
  rule: RecurringRuleApi,
  d: RuleDraft,
): RecurringPatchFields | null | { error: string } {
  const out: RecurringPatchFields = {};
  const concept = d.concept.trim();
  if (concept === "") return { error: "El concepto no puede quedar vacío." };
  if (concept !== rule.concept) out.concept = concept;

  // Lo que viaja pasa SIEMPRE por `toApiDecimalString` (punto decimal, sin millares).
  let magnitude: string;
  try {
    magnitude = toApiDecimalString(d.amount.trim().replace(/^[+-]/, ""));
  } catch (e) {
    if (e instanceof DecimalInputError) return { error: e.message };
    throw e;
  }
  const parsed = parseDisplayDecimal(magnitude);
  if (parsed === null || parsed <= 0) {
    return { error: "El importe debe ser un número mayor que 0." };
  }
  const signed = d.kind === "income" ? magnitude : `-${magnitude}`;
  // Comparación numérica solo para decidir si el campo viaja; lo que se envía es el string.
  const current = parseDisplayDecimal(rule.amount);
  const next = parseDisplayDecimal(signed);
  if (current === null || next === null || current !== next) out.amount = signed;

  if (d.kind !== rule.kind) {
    out.kind = d.kind;
    if (d.kind === "savings") {
      // La inversión va sin categoría: sin el clear, el servidor validaría la categoría vieja
      // contra el kind nuevo (`savings_no_category`).
      out.clear_category = true;
    } else if (d.categoryId) {
      out.category_id = d.categoryId;
    }
  } else if (
    d.kind !== "savings" &&
    d.categoryId &&
    d.categoryId !== (rule.category_id ?? "")
  ) {
    out.category_id = d.categoryId;
  }

  const notes = d.notes.trim();
  if (notes !== (rule.notes ?? "")) {
    if (notes === "") out.clear_notes = true;
    else out.notes = notes;
  }

  return Object.keys(out).length === 0 ? null : out;
}

/** Cuerpo final: el alcance elegido en el diálogo («Todos» / «Solo futuros») + los campos. */
export function recurringPatchBody(
  applyTo: RecurringApplyTo,
  fields: RecurringPatchFields,
): PatchRecurringRuleRequest {
  return { apply_to: applyTo, ...fields };
}
