/**
 * Helpers puros del histórico de gasto mensual (pestaña «Gastos»). Sin React, sin fetch.
 * Cubren: etiquetas de mes, selección del mes por defecto, categorías válidas por kind,
 * construcción del payload de confirmación del import (decisiones paralelas por índice) y el
 * tono/semántica de las cifras delta de la comparativa.
 */

import type {
  CategoryRow,
  ImportDecisionApi,
  ImportPreviewRowApi,
  TransactionKindApi,
  TransactionMonthApi,
} from "../api/types";
import { DISPLAY_NUMBER_LOCALE, formatCurrencyNumber } from "./format";

/** Los tres kinds en orden de UI. */
export const TRANSACTION_KINDS: TransactionKindApi[] = [
  "expense",
  "income",
  "savings",
];

export const KIND_LABEL_ES: Record<TransactionKindApi, string> = {
  expense: "Gasto",
  income: "Ingreso",
  savings: "Ahorro",
};

/** `YYYY-MM` → `{y, m}` (m 1-based); null si no parsea. */
export function parseMonth(month: string): { y: number; m: number } | null {
  const mm = /^(\d{4})-(\d{2})$/.exec(String(month).trim());
  if (!mm) return null;
  const y = Number(mm[1]);
  const m = Number(mm[2]);
  if (!Number.isFinite(y) || !Number.isFinite(m) || m < 1 || m > 12) return null;
  return { y, m };
}

/** `YYYY-MM` → `junio 2026` (mes en minúscula, es-ES). Si no parsea, devuelve el crudo. */
export function monthLabelEs(month: string): string {
  const c = parseMonth(month);
  if (!c) return month;
  try {
    const name = new Date(c.y, c.m - 1, 1).toLocaleString(DISPLAY_NUMBER_LOCALE, {
      month: "long",
    });
    return `${name} ${c.y}`;
  } catch {
    return month;
  }
}

/** `YYYY-MM` → `jun` (mes corto es-ES). Para ejes de gráficas compactas. */
export function monthShortLabelEs(month: string): string {
  const c = parseMonth(month);
  if (!c) return month;
  try {
    return new Date(c.y, c.m - 1, 1).toLocaleString(DISPLAY_NUMBER_LOCALE, {
      month: "short",
    });
  } catch {
    return month;
  }
}

/**
 * Mes por defecto de la comparativa: el último mes COMPLETO. Si no hay ninguno completo, el más
 * reciente. `null` si la lista está vacía. `months` viene DESC (más reciente primero).
 */
export function defaultSelectedMonth(months: TransactionMonthApi[]): string | null {
  if (months.length === 0) return null;
  const complete = months.find((m) => m.is_complete);
  return (complete ?? months[0]).month;
}

/**
 * Vecino de `current` en la lista `months` (strings) hacia el pasado (`older`) o el futuro
 * (`newer`). `months` viene DESC → `older` es índice+1, `newer` índice−1. `null` en los extremos
 * o si `current` no está en la lista.
 */
export function adjacentMonthInList(
  months: string[],
  current: string,
  direction: "older" | "newer",
): string | null {
  const i = months.indexOf(current);
  if (i < 0) return null;
  const j = direction === "older" ? i + 1 : i - 1;
  if (j < 0 || j >= months.length) return null;
  return months[j];
}

/**
 * Categorías válidas para un kind. `savings` NO admite categoría (category_id NULL obligatorio,
 * 400 savings_no_category) → devuelve `[]`. `income` → categorías de ingreso; `expense` →
 * categorías de gasto.
 */
export function categoriesForKind(
  kind: TransactionKindApi,
  incomeCategories: CategoryRow[],
  expenseCategories: CategoryRow[],
): CategoryRow[] {
  if (kind === "savings") return [];
  return kind === "income" ? incomeCategories : expenseCategories;
}

/**
 * Estado editable por fila del wizard de import (overlay sobre la fila del preview). Paralelo por
 * índice a `rows` del preview.
 */
export type ImportRowDraft = {
  /** Checkbox «incluir». `false` → la fila se descarta en confirm. */
  include: boolean;
  kind: TransactionKindApi;
  /** "" = sin categoría. Ignorado si kind=savings. */
  categoryId: string;
  /** "" = sin vínculo. */
  linkedAssetId: string;
  /** "" = sin vínculo. */
  linkedLiabilityId: string;
  /** Fuerza el alta de una fila `already_imported` (nueva ocurrencia). */
  force: boolean;
};

/**
 * Semilla del draft a partir de la fila del preview. Filas nuevas normales quedan incluidas;
 * duplicados (`already_imported`), transferencias sugeridas y avisos de divisa quedan
 * DESMARCADOS (atenuados en UI) para que el usuario los revise antes de incluirlos.
 */
export function initialDraftForRow(row: ImportPreviewRowApi): ImportRowDraft {
  const include =
    row.status === "new" && !row.suggested_transfer && !row.currency_warning;
  return {
    include,
    kind: row.suggested_kind,
    categoryId:
      row.suggested_kind === "savings" ? "" : row.suggested_category_id ?? "",
    linkedAssetId: "",
    linkedLiabilityId: "",
    force: false,
  };
}

/**
 * Construye el array de decisiones para `POST /import/confirm`, PARALELO POR ÍNDICE a `rows`
 * (una decisión por fila, siempre). savings ⇒ category_id se omite (null obligatorio). "" en los
 * selects de vínculo ⇒ campo omitido.
 */
export function buildConfirmDecisions(
  rows: ImportPreviewRowApi[],
  drafts: ImportRowDraft[],
): ImportDecisionApi[] {
  return rows.map((_row, i) => {
    const d = drafts[i];
    const decision: ImportDecisionApi = { kind: d.kind };
    if (!d.include) decision.discard = true;
    if (d.force) decision.force = true;
    if (d.kind !== "savings" && d.categoryId) decision.category_id = d.categoryId;
    if (d.linkedAssetId) decision.linked_asset_id = d.linkedAssetId;
    if (d.linkedLiabilityId) decision.linked_liability_id = d.linkedLiabilityId;
    return decision;
  });
}

/**
 * Resumen dinámico del footer del wizard: cuántas filas se importarán / omitirán / descartarán,
 * con la MISMA semántica que el backend: discard=true → descartada; already_imported sin force →
 * omitida; resto → importada.
 */
export function summarizeDecisions(
  rows: ImportPreviewRowApi[],
  drafts: ImportRowDraft[],
): { toImport: number; toSkip: number; toDiscard: number } {
  let toImport = 0;
  let toSkip = 0;
  let toDiscard = 0;
  rows.forEach((row, i) => {
    const d = drafts[i];
    if (!d.include) {
      toDiscard += 1;
    } else if (row.status === "already_imported" && !d.force) {
      toSkip += 1;
    } else {
      toImport += 1;
    }
  });
  return { toImport, toSkip, toDiscard };
}

/** Filtros de la barra de acciones masivas del wizard. */
export type ImportRowFilter =
  | "all"
  | "new"
  | "duplicates"
  | "transfers"
  | "uncategorized";

/** ¿La fila `i` pasa el filtro? (uncategorized = expense/income sin categoría, savings nunca). */
export function rowMatchesFilter(
  row: ImportPreviewRowApi,
  draft: ImportRowDraft,
  filter: ImportRowFilter,
): boolean {
  switch (filter) {
    case "all":
      return true;
    case "new":
      return row.status === "new";
    case "duplicates":
      return row.status === "already_imported";
    case "transfers":
      return row.suggested_transfer;
    case "uncategorized":
      return draft.kind !== "savings" && draft.categoryId === "";
    default:
      return true;
  }
}

/** Signo redondeado a euros: `pos` (>0), `neg` (<0), `zero`. */
export function signOf(n: number): "pos" | "neg" | "zero" {
  const r = Math.round(n);
  if (r > 0) return "pos";
  if (r < 0) return "neg";
  return "zero";
}

/**
 * Tono de una cifra delta (real − budget/promedio) según el kind, para colorearla. En GASTOS
 * gastar de más (delta > 0) es desfavorable → rojo (`neg`); gastar de menos → verde (`pos`). En
 * INGRESOS es al revés. `zero` = neutro (sin color). Devuelve la clase CSS de color.
 */
export function deltaToneClass(value: number, kind: "expense" | "income"): string {
  const s = signOf(value);
  if (s === "zero") return "";
  const favorable = kind === "income" ? s === "pos" : s === "neg";
  return favorable ? "num-pos" : "num-neg";
}

/** Formatea una cifra delta con signo explícito (`+`/`−`) y símbolo de moneda. */
export function formatDeltaCurrency(value: number, currencyIso: string): string {
  const r = Math.round(value);
  if (r === 0) return formatCurrencyNumber(0, currencyIso);
  const sign = r > 0 ? "+" : "−";
  return `${sign}${formatCurrencyNumber(Math.abs(r), currencyIso)}`;
}
