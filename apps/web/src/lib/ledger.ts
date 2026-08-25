/**
 * Helpers compartidos por las vistas del ledger (activos, pasivos, presupuesto, planificación).
 * Funciones puras: leen un row y devuelven un derivado para la UI.
 */

import type {
  AssetApiRow,
  BudgetEntryApiRow,
  CategoryRow,
  LiabilityApiRow,
  LiabilityRepaymentModelApi,
} from "../api/types";
import { paymentIntervalCountUtc, todayYmdInTimeZone } from "./dates";
import {
  DISPLAY_NUMBER_LOCALE,
  METRIC_DASH,
  formatCurrencyNumber,
  formatMoneyAmount,
  normalizeCurrencyIso,
  parseDisplayDecimal,
} from "./format";

export const PAYMENT_FREQ_LABEL: Record<"monthly" | "weekly", string> = {
  monthly: "Mensual",
  weekly: "Semanal",
};

export type LiabilityPaymentFreq = "" | "monthly" | "weekly";

/** Orden de los `<option>` del formulario: el histórico primero, luego por cercanía conceptual. */
export const REPAYMENT_MODEL_ORDER: LiabilityRepaymentModelApi[] = [
  "fixed_payments",
  "french",
  "interest_only",
  "revolving",
];

export const REPAYMENT_MODEL_LABEL: Record<LiabilityRepaymentModelApi, string> =
  {
    fixed_payments: "Cuota fija",
    french: "Francés",
    interest_only: "Solo intereses",
    revolving: "Revolving",
  };

/**
 * Valor actual de una renta de `months` cuotas de `monthlyPayment` al TIN nominal anual
 * `aprPercent` (`i = apr / 1200`):
 *
 * `P = M · (1 − (1 + i)^−n) / i`
 *
 * Espejo EXACTO de `futurefin_engine::present_value_of_payments` (`crates/engine/src/projection.rs`),
 * incluido su caso degenerado: `apr` ausente o `≤ 0` devuelve `M · n` sin tocar la potencia — el
 * límite de la fórmula cuando `i → 0`. Aquí es `number` (f64) y allí `Decimal`: es una **vista
 * previa** que se pinta redondeada al euro, nunca el valor que se guarda (ese lo calcula el
 * servidor). La paridad de las dos implementaciones la fija
 * `apps/api/tests/fixtures/liability-derived-principal-parity.json`.
 */
export function presentValueOfPayments(
  monthlyPayment: number,
  months: number,
  aprPercent: number | null,
): number {
  const plain = monthlyPayment * months;
  if (aprPercent === null || !Number.isFinite(aprPercent) || aprPercent <= 0) {
    return plain;
  }
  const i = aprPercent / 1200;
  const pv = (monthlyPayment * (1 - Math.pow(1 + i, -months))) / i;
  return Number.isFinite(pv) ? pv : plain;
}

/**
 * Principal derivado de `n` cuotas, por modelo — espejo de `derive_principal_from_payment_plan`
 * (`apps/api/src/handlers/liabilities.rs`):
 *
 * - `fixed_payments` → `Σ cuotas` = `cuota × n` (contrato histórico, sin cambio).
 * - `french` → valor actual de esas cuotas al TIN: el capital pendiente de verdad.
 * - `interest_only` / `revolving` → el servidor rechaza derivar (`derive_not_supported_for_model`),
 *   así que aquí no hay número que enseñar: `null`.
 */
export function liabilityDerivedPrincipalNum(
  paymentAmount: number,
  intervals: number,
  model: LiabilityRepaymentModelApi,
  aprPercent: number | null,
): number | null {
  if (!Number.isFinite(paymentAmount) || paymentAmount <= 0) return null;
  if (!Number.isFinite(intervals) || intervals <= 0) return null;
  if (model === "interest_only" || model === "revolving") return null;
  if (model === "french") {
    if (aprPercent === null || !(aprPercent > 0)) return null;
    return presentValueOfPayments(paymentAmount, intervals, aprPercent);
  }
  return paymentAmount * intervals;
}

/**
 * Vista previa del principal derivado, ya formateada. Devuelve `null` —y la UI no pinta nada—
 * siempre que el servidor fuese a rechazar esa combinación: es preferible callar a prometer un
 * número que el POST no va a producir (`weekly` solo existe en `fixed_payments`, `french` exige
 * TIN > 0, y `interest_only`/`revolving` no derivan).
 */
export function liabilityDerivedPrincipalPreview(
  amountStr: string,
  freq: LiabilityPaymentFreq,
  endYmd: string,
  installationCalendarTz: string,
  currencyIso: string,
  model: LiabilityRepaymentModelApi = "fixed_payments",
  aprStr = "",
): string | null {
  if (!freq || !endYmd.trim()) return null;
  if (model !== "fixed_payments" && freq === "weekly") return null;
  const startYmd = todayYmdInTimeZone(installationCalendarTz);
  const n = paymentIntervalCountUtc(freq, startYmd, endYmd.trim());
  if (n === null || n <= 0) return null;
  const amount = Number(amountStr.trim().replace(",", "."));
  if (!Number.isFinite(amount) || amount <= 0) return null;
  const apr = parseDisplayDecimal(String(aprStr).trim());
  const total = liabilityDerivedPrincipalNum(amount, n, model, apr);
  if (total === null) return null;
  return formatCurrencyNumber(total, currencyIso);
}

/** Hogar = todos los registros de la instalación; usuario actual = solo filas con tu `owner_user_id`. */
export type LedgerPersonScope = "household" | "mine";

export function ledgerViewQs(scope: LedgerPersonScope): string {
  return scope === "mine" ? "?view=mine" : "";
}

/** Aporte mensual estimado (primer mes motor) leído de `contribution_nominal_monthly`. */
export function assetContributionMonthlyEstimateNum(a: AssetApiRow): number {
  const raw = a.contribution_nominal_monthly;
  if (raw == null) return 0;
  const n = parseDisplayDecimal(String(raw).trim());
  return n != null && n > 0 ? n : 0;
}

export function formatAssetContributionNominalCell(
  a: AssetApiRow,
  currencyIso: string,
): string {
  const n = assetContributionMonthlyEstimateNum(a);
  return n > 0 ? formatCurrencyNumber(n, currencyIso) : METRIC_DASH;
}

/** Suma valor actual y coste solo en posiciones con compra válida (> 0). */
export function assetPortfolioCostTotals(
  assets: AssetApiRow[],
): { cost: number; currentOnCost: number } | null {
  let cost = 0;
  let currentOnCost = 0;
  for (const a of assets) {
    const pur = parseDisplayDecimal(String(a.purchase_price ?? "").trim());
    const cur = parseDisplayDecimal(a.current_value);
    if (pur === null || pur <= 0 || cur === null) continue;
    cost += pur;
    currentOnCost += cur;
  }
  if (cost <= 0) return null;
  return { cost, currentOnCost };
}

/** Cuota mensual equivalente (mensual o semanal ×52/12). */
export function liabilityPaymentMonthlyEquivalentNum(
  row: LiabilityApiRow,
): number {
  const amt = parseDisplayDecimal(String(row.payment_amount ?? "").trim());
  if (amt === null || amt <= 0) return 0;
  if (row.payment_frequency === "weekly") return (amt * 52) / 12;
  if (row.payment_frequency === "monthly") return amt;
  return 0;
}

/** TAE % media ponderada por principal (solo pasivos con TAE informada). */
export function liabilitiesWeightedAprPercent(
  liabilities: LiabilityApiRow[],
): number | null {
  let num = 0;
  let den = 0;
  for (const row of liabilities) {
    const p = parseDisplayDecimal(row.principal);
    const apr = parseDisplayDecimal(String(row.apr_percent ?? "").trim());
    if (p === null || p <= 0 || apr === null || !Number.isFinite(apr)) {
      continue;
    }
    num += p * apr;
    den += p;
  }
  if (den <= 0) return null;
  return num / den;
}

/**
 * Suma aproximada de interés mensual (saldo × TAE ÷ 12 por pasivo).
 * No modela amortización; sirve como orden de magnitud.
 */
export function liabilitiesApproxMonthlyInterestSum(
  liabilities: LiabilityApiRow[],
): number {
  let sum = 0;
  for (const row of liabilities) {
    const p = parseDisplayDecimal(row.principal);
    const apr = parseDisplayDecimal(String(row.apr_percent ?? "").trim());
    if (
      p === null ||
      p <= 0 ||
      apr === null ||
      !Number.isFinite(apr) ||
      apr <= 0
    ) {
      continue;
    }
    sum += (p * (apr / 100)) / 12;
  }
  return sum;
}

/** Etiquetas de eje Y: compactas si el importe es muy grande. */
export function formatAxisMoney(n: number, currencyIso: string): string {
  const iso = normalizeCurrencyIso(currencyIso);
  if (!iso || !Number.isFinite(n)) return formatMoneyAmount(String(n));
  try {
    const big = Math.abs(n) >= 100_000;
    return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
      style: "currency",
      currency: iso,
      currencyDisplay: "symbol",
      notation: big ? "compact" : "standard",
      compactDisplay: "short",
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    }).format(n);
  } catch {
    return formatCurrencyNumber(n, currencyIso);
  }
}

/** Redondeo hacia arriba al siguiente múltiplo de 100. */
export function roundUpToHundred(n: number): number {
  return Math.ceil(n / 100) * 100;
}

export function formatProjectionMilestoneCompactLabel(target: string): string {
  if (target === "jubilacion") return "Jubilación";
  const value = parseDisplayDecimal(target);
  if (value === null || !Number.isFinite(value)) return METRIC_DASH;
  const abs = Math.abs(value);
  const units: Array<{ scale: number; suffix: string }> = [
    { scale: 1_000_000_000_000, suffix: "T" },
    { scale: 1_000_000_000, suffix: "B" },
    { scale: 1_000_000, suffix: "M" },
    { scale: 1_000, suffix: "K" },
  ];
  for (const u of units) {
    if (abs >= u.scale) {
      const scaled = value / u.scale;
      const rounded = Math.round(scaled * 10) / 10;
      const txt =
        Math.abs(rounded - Math.trunc(rounded)) < 1e-9
          ? String(Math.trunc(rounded))
          : rounded.toFixed(1);
      return `${txt}${u.suffix}`;
    }
  }
  return formatMoneyAmount(String(value));
}

export function budgetCategoryMap(
  incomeCats: CategoryRow[],
  expenseCats: CategoryRow[],
): Map<string, CategoryRow> {
  const m = new Map<string, CategoryRow>();
  for (const c of incomeCats) {
    m.set(c.id, c);
  }
  for (const c of expenseCats) {
    m.set(c.id, c);
  }
  return m;
}

/**
 * Orden del presupuesto: bloques de categoría por total descendente, y dentro de cada bloque la
 * partida manual primero y la **cuota de pasivo justo detrás** (3.7.0) — que es lo que hace que la
 * cuota se lea como una segunda línea de su categoría en vez de como un bloque aparte.
 *
 * Las cuotas de pasivos sin categoría de gasto asignada (anteriores a 3.4.0) no tienen bloque: se
 * agrupan bajo la clave vacía y caen al final de la lista, nunca se descartan.
 */
export function sortBudgetEntriesMacStyle(
  entries: BudgetEntryApiRow[],
  categoryById: Map<string, CategoryRow>,
): BudgetEntryApiRow[] {
  const monthlyEq = (e: BudgetEntryApiRow) =>
    Number(String(e.amount).replace(",", "."));
  const catKey = (e: BudgetEntryApiRow) => e.category_id ?? "";
  const byCatTotal = new Map<string, number>();
  for (const e of entries) {
    const k = catKey(e);
    byCatTotal.set(k, (byCatTotal.get(k) ?? 0) + monthlyEq(e));
  }
  const catName = (id: string) => categoryById.get(id)?.name ?? id;
  const sourceRank = (e: BudgetEntryApiRow) =>
    e.source === "liability" ? 1 : 0;
  return [...entries].sort((a, b) => {
    const ka = catKey(a);
    const kb = catKey(b);
    if ((ka === "") !== (kb === "")) return ka === "" ? 1 : -1;
    const ta = byCatTotal.get(ka) ?? 0;
    const tb = byCatTotal.get(kb) ?? 0;
    if (tb !== ta) return tb - ta;
    const cmp = catName(ka).localeCompare(catName(kb), "es");
    if (cmp !== 0) return cmp;
    const sa = sourceRank(a);
    const sb = sourceRank(b);
    if (sa !== sb) return sa - sb;
    const ea = monthlyEq(a);
    const eb = monthlyEq(b);
    if (eb !== ea) return eb - ea;
    return a.id.localeCompare(b.id, "es");
  });
}

/**
 * Filas agrupadas por categoría (orden de ajustes + IDs huérfanos al reunirlos).
 * `sortRowsDescending`: dentro de cada categoría, filas de mayor a menor `value`; empates con `tieBreak`.
 * `categoryTotalDescending`: bloques de categoría de mayor a menor total; empates por nombre de categoría.
 */
export function groupRowsByCategoryOrdered<T extends { category_id: string }>(
  rows: T[],
  categories: CategoryRow[],
  opts?: {
    categoryTotalDescending?: (items: T[]) => number;
    sortRowsDescending?: {
      value: (row: T) => number;
      tieBreak: (a: T, b: T) => number;
    };
  },
): { categoryId: string; label: string; items: T[] }[] {
  const byCat = new Map<string, T[]>();
  for (const row of rows) {
    const prev = byCat.get(row.category_id);
    if (prev) prev.push(row);
    else byCat.set(row.category_id, [row]);
  }
  const out: { categoryId: string; label: string; items: T[] }[] = [];
  const seen = new Set<string>();
  for (const c of categories) {
    const items = byCat.get(c.id);
    if (items?.length) {
      out.push({ categoryId: c.id, label: c.name, items });
      seen.add(c.id);
    }
  }
  for (const [id, items] of byCat) {
    if (!seen.has(id) && items.length > 0) {
      out.push({
        categoryId: id,
        label: categories.find((x) => x.id === id)?.name ?? id.slice(0, 8),
        items,
      });
    }
  }
  const rs = opts?.sortRowsDescending;
  if (rs) {
    for (const g of out) {
      g.items.sort((a, b) => {
        const diff = rs.value(b) - rs.value(a);
        if (diff !== 0) return diff;
        return rs.tieBreak(a, b);
      });
    }
  }
  const rank = opts?.categoryTotalDescending;
  if (rank) {
    out.sort((a, b) => {
      const diff = rank(b.items) - rank(a.items);
      if (diff !== 0) return diff;
      return a.label.localeCompare(b.label, "es");
    });
  }
  return out;
}
