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
  SummaryTotalsApi,
  TransactionApi,
  TransactionKindApi,
  TransactionMonthApi,
} from "../api/types";
import {
  DISPLAY_NUMBER_LOCALE,
  formatCurrencyNumber,
  parseDisplayDecimal,
} from "./format";

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

/**
 * Umbral de significancia (en unidades de moneda) por debajo del cual una desviación es «ruido» y
 * se muestra en gris / sin flecha: 1 % del ingreso REAL del mes; si no hay ingreso real (≤0), 1 %
 * del ingreso presupuestado; si ambos son 0 → 0 (todo cuenta como significativo).
 */
export function significanceThreshold(totals: SummaryTotalsApi): number {
  const incActual = parseDisplayDecimal(totals.income_actual) ?? 0;
  if (incActual > 0) return incActual * 0.01;
  const incBudget = parseDisplayDecimal(totals.income_budget) ?? 0;
  if (incBudget > 0) return incBudget * 0.01;
  return 0;
}

/**
 * Flecha de tendencia «real vs promedio» de una línea de comparativa. Slot vacío
 * (`direction:null`) SOLO si no hay promedio (`!hasAvg`: sin datos ≠ sin cambio). Con promedio pero
 * desviación insignificante (`|Δ| <= threshold`) → `flat` (glifo «=» atenuado, tono `""`). En otro
 * caso, `up` si Δ>0 (gasté/ingresé más que la media) y `down` si Δ<0. El TONO es semántico: subir
 * ingresos o bajar gastos es favorable (`num-pos`); lo contrario es desfavorable (`num-neg`).
 */
export function trendArrow(
  deltaVsAvg: number,
  kind: "expense" | "income",
  threshold: number,
  hasAvg: boolean,
): { direction: "up" | "down" | "flat" | null; tone: "num-pos" | "num-neg" | "" } {
  if (!hasAvg) {
    return { direction: null, tone: "" };
  }
  if (Math.abs(deltaVsAvg) <= threshold) {
    return { direction: "flat", tone: "" };
  }
  const direction = deltaVsAvg > 0 ? "up" : "down";
  const favorable =
    (kind === "income" && deltaVsAvg > 0) ||
    (kind === "expense" && deltaVsAvg < 0);
  return { direction, tone: favorable ? "num-pos" : "num-neg" };
}

/**
 * Tono de una cifra delta que respeta el umbral de significancia: `""` (gris/neutro) si la
 * desviación no supera `threshold`, si no el tono habitual de `deltaToneClass` según el kind.
 */
export function significantDeltaTone(
  value: number,
  kind: "expense" | "income",
  threshold: number,
): string {
  if (Math.abs(value) <= threshold) return "";
  return deltaToneClass(value, kind);
}

/**
 * Tendencia de un KPI de la banda de Movimientos: compara el PROMEDIO de la ventana con el
 * PRESUPUESTO del mes (`delta = avg − budget`) para pintar bajo la cifra principal una flecha con su
 * tono. Devuelve `null` (sin tendencia → slot reservado pero vacío) si no hay promedio (`!hasAvg`) o
 * no hay presupuesto contra el que comparar (`budget <= 0`: comparar contra 0 no informa). Reutiliza
 * `trendArrow`, así que la semántica del color es idéntica a la comparativa: gastar MENOS que el
 * presupuesto o ingresar MÁS es favorable (`num-pos`); con `|Δ| <= threshold` la flecha es `flat`
 * (glifo «=», tono neutro `""`). `avg`/`budget` en unidades de moneda.
 */
export function kpiBudgetTrend(
  avg: number,
  budget: number,
  kind: "expense" | "income",
  threshold: number,
  hasAvg: boolean,
): {
  delta: number;
  direction: "up" | "down" | "flat";
  tone: "num-pos" | "num-neg" | "";
} | null {
  if (!hasAvg || budget <= 0) return null;
  const delta = avg - budget;
  const arrow = trendArrow(delta, kind, threshold, hasAvg);
  // hasAvg es true aquí ⇒ trendArrow nunca devuelve direction null (defensivo: fallback a flat).
  return { delta, direction: arrow.direction ?? "flat", tone: arrow.tone };
}

/**
 * Ventanas del promedio de la comparativa. `id` = valor del query `avg_window`; `pill` = etiqueta
 * del botón de la toolbar; `label` = etiqueta usada en cabeceras/sublines («Promedio {label}»).
 */
export const AVG_WINDOWS: { id: string; pill: string; label: string }[] = [
  { id: "3", pill: "3m", label: "3m" },
  { id: "6", pill: "6m", label: "6m" },
  { id: "12", pill: "12m", label: "12m" },
  { id: "ytd", pill: "YTD", label: "YTD" },
  { id: "all", pill: "Todo", label: "total" },
];

/** Etiqueta de una ventana del promedio; el propio `id` si no está en `AVG_WINDOWS`. */
export function avgWindowLabel(id: string): string {
  return AVG_WINDOWS.find((w) => w.id === id)?.label ?? id;
}

/**
 * Nombre presentable de una fuente/preset de import: `myinvestor`→`MyInvestor`, `n26`→`N26`, resto
 * con la primera letra en mayúscula. Cadena vacía → cadena vacía.
 */
export function capitalizeSource(raw: string): string {
  const t = String(raw).trim();
  if (t === "") return "";
  const lower = t.toLowerCase();
  if (lower === "myinvestor") return "MyInvestor";
  if (lower === "n26") return "N26";
  return t.charAt(0).toUpperCase() + t.slice(1);
}

// ---------------------------------------------------------------------------
// Tabla de movimientos — búsqueda, ordenación y agrupación por categoría.
// ---------------------------------------------------------------------------

/**
 * Normaliza texto para búsqueda: minúsculas + NFD sin diacríticos (`Café` → `cafe`), de modo que
 * la comparación sea insensible a mayúsculas Y a acentos.
 */
export function normalizeSearchText(s: string): string {
  return String(s)
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");
}

/**
 * ¿El movimiento casa con la query de búsqueda? Busca la query (normalizada) como subcadena del
 * concepto Y del nombre de categoría concatenados. Query vacía (o solo espacios) → `true`.
 */
export function transactionMatchesQuery(
  concept: string,
  categoryName: string | null,
  query: string,
): boolean {
  const q = normalizeSearchText(query).trim();
  if (q === "") return true;
  const haystack = normalizeSearchText(`${concept} ${categoryName ?? ""}`);
  return haystack.includes(q);
}

/** Columna de ordenación de la tabla de movimientos. */
export type TxnSortKey = "date" | "concept" | "amount";
/** Dirección de ordenación. */
export type TxnSortDir = "asc" | "desc";

/** Dirección «natural» (primera pulsación) de cada columna: fecha/importe desc, concepto asc. */
export function naturalSortDir(key: TxnSortKey): TxnSortDir {
  return key === "concept" ? "asc" : "desc";
}

/**
 * Compara dos movimientos por `key`/`dir`. IMPORTANTE: `amount` ordena por MAGNITUD (`|amount|`)
 * para destacar los movimientos más grandes con independencia del signo (ingreso vs gasto). El
 * desempate es un orden total ESTABLE independiente de `dir`: `op_date` desc y luego `id` asc.
 */
export function compareTransactions(
  a: TransactionApi,
  b: TransactionApi,
  key: TxnSortKey,
  dir: TxnSortDir,
): number {
  const mul = dir === "asc" ? 1 : -1;
  let primary = 0;
  if (key === "date") {
    primary = a.op_date.localeCompare(b.op_date);
  } else if (key === "concept") {
    primary = normalizeSearchText(a.concept).localeCompare(
      normalizeSearchText(b.concept),
      DISPLAY_NUMBER_LOCALE,
    );
  } else {
    const am = Math.abs(parseDisplayDecimal(a.amount) ?? 0);
    const bm = Math.abs(parseDisplayDecimal(b.amount) ?? 0);
    primary = am - bm;
  }
  if (primary !== 0) return primary * mul;
  const byDate = b.op_date.localeCompare(a.op_date);
  if (byDate !== 0) return byDate;
  return a.id.localeCompare(b.id);
}

/** Ordena los movimientos (copia; orden total estable). Ver `compareTransactions`. */
export function sortTransactions(
  rows: TransactionApi[],
  key: TxnSortKey,
  dir: TxnSortDir,
): TransactionApi[] {
  return [...rows].sort((a, b) => compareTransactions(a, b, key, dir));
}

/** Un grupo de la tabla de movimientos agrupada por categoría. */
export type TransactionGroup = {
  /** `category_id` | `"savings"` | `"uncategorized-income"` | `"uncategorized-expense"`. */
  key: string;
  label: string;
  /** Kind del grupo (una categoría solo tiene un scope → lo hereda de sus filas). */
  kind: TransactionKindApi;
  rows: TransactionApi[];
  /** Suma FIRMADA de los `amount` del grupo (los deltas se compensan por signo). */
  subtotal: number;
};

/**
 * Agrupa los movimientos por categoría. Grupo = categoría; `kind=savings` → un único grupo
 * «Ahorro / Inversión»; `category_id` nulo (y no savings) → «Sin categoría» POR KIND (un ingreso sin
 * categoría va con los ingresos, un gasto con los gastos). El nombre visible sale de
 * `categoryNameById` (categoría viva), con fallback al `category_name` cacheado en la fila. NO
 * ordena: devuelve los grupos en orden de primera aparición (ordénalos con `sortTransactionGroups`).
 */
export function groupTransactionsByCategory(
  rows: TransactionApi[],
  categoryNameById: Map<string, string>,
): TransactionGroup[] {
  const groups = new Map<string, TransactionGroup>();
  for (const r of rows) {
    const kind = r.kind ?? "expense";
    let key: string;
    let label: string;
    if (kind === "savings") {
      key = "savings";
      label = "Ahorro / Inversión";
    } else if (r.category_id) {
      key = r.category_id;
      label = categoryNameById.get(r.category_id) ?? r.category_name ?? "Sin categoría";
    } else {
      key = `uncategorized-${kind}`;
      label = "Sin categoría";
    }
    let g = groups.get(key);
    if (!g) {
      g = { key, label, kind, rows: [], subtotal: 0 };
      groups.set(key, g);
    }
    g.rows.push(r);
    g.subtotal += parseDisplayDecimal(r.amount) ?? 0;
  }
  return [...groups.values()];
}

/** Prioridad de sección de un kind en la tabla agrupada: ingresos, luego ahorro, abajo gastos. */
const KIND_GROUP_PRIORITY: Record<TransactionKindApi, number> = {
  income: 0,
  savings: 1,
  expense: 2,
};

/**
 * Ordena los grupos en un orden FIJO, ajeno a la clave/dirección activa de la tabla (esa solo
 * ordena las filas DENTRO de cada grupo): (1) secciones por kind — ingresos → ahorro → gastos —,
 * (2) dentro de cada sección, de mayor a menor cantidad (`|subtotal|` descendente). Desempate
 * estable: alfabético por `label` y luego `key`.
 */
export function sortTransactionGroups(groups: TransactionGroup[]): TransactionGroup[] {
  return [...groups].sort((a, b) => {
    const byKind = KIND_GROUP_PRIORITY[a.kind] - KIND_GROUP_PRIORITY[b.kind];
    if (byKind !== 0) return byKind;
    const byMagnitude = Math.abs(b.subtotal) - Math.abs(a.subtotal);
    if (byMagnitude !== 0) return byMagnitude;
    const byLabel = normalizeSearchText(a.label).localeCompare(
      normalizeSearchText(b.label),
      DISPLAY_NUMBER_LOCALE,
    );
    if (byLabel !== 0) return byLabel;
    return a.key.localeCompare(b.key);
  });
}
