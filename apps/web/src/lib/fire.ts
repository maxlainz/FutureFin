/**
 * Cálculos FIRE en cliente: defaults, normalización del JSONB de la API, gross-up por tramos
 * españoles (mismo modelo que el motor Rust en `apps/api/src/handlers/projection.rs`).
 *
 * **Nota**: estos cálculos están duplicados en cliente para el **preview en vivo** del
 * formulario de FIRE (Settings → FIRE). El servidor sigue siendo source of truth en
 * `GET /v1/projection/series`; aquí solo se reproduce la matemática lo bastante fiel para
 * que el usuario vea el target actualizarse mientras teclea `swr_pct` o cambia el modo.
 */

import type {
  FireNumberModeApi,
  FireSettingsApi,
  AvgWindowModeApi,
  ProjectionPointApi,
  SavingsAvgBasisApi,
  SavingsSourceApi,
  TaxBracketApi,
} from "../api/types";
import { formatPercentAmount, parseDisplayDecimal } from "./format";

export const DEFAULT_ES_TAX_BRACKETS_API: TaxBracketApi[] = [
  { up_to: "6000", pct: "19" },
  { up_to: "50000", pct: "21" },
  { up_to: "200000", pct: "23" },
  { up_to: "300000", pct: "27" },
  { up_to: null, pct: "30" },
];

export function defaultFireSettingsApi(): FireSettingsApi {
  return {
    fire_number_mode: "annual_expense",
    fire_number_manual_amount: null,
    fire_number_expense_adjustment_pct: null,
    swr_pct: "3.5",
    taxes_enabled: true,
    tax_brackets: DEFAULT_ES_TAX_BRACKETS_API.map((b) => ({
      up_to: b.up_to,
      pct: b.pct,
    })),
    savings_source: "budget",
    income_avg_window_months: 3,
    income_avg_window_mode: "calendar",
    expense_avg_window_months: 12,
    expense_avg_window_mode: "calendar",
  };
}

/**
 * Parser/allow-list de `savings_source` (los 3 modos: `budget` | `transactions_avg` |
 * `budget_income_real_expense`). Cualquier valor ausente o desconocido cae a `"budget"` (modo A,
 * el default seguro). Punto único usado por `normalizeInstallationFireSettings` (respuesta de la
 * API) y por el `<select>` de Ajustes (evento onChange).
 */
export function parseSavingsSource(v: string | null | undefined): SavingsSourceApi {
  return v === "transactions_avg" ||
    v === "budget" ||
    v === "budget_income_real_expense"
    ? v
    : "budget";
}

export function normalizeInstallationFireSettings(
  raw: FireSettingsApi | undefined | null,
): FireSettingsApi {
  if (!raw || typeof raw !== "object") return defaultFireSettingsApi();
  const base = defaultFireSettingsApi();
  return {
    fire_number_mode: (() => {
      const m = raw.fire_number_mode as
        | FireNumberModeApi
        | "annual_expense_adjusted";
      if (m === "manual" || m === "annual_expense" || m === "current_income") {
        return m;
      }
      if (m === "annual_expense_adjusted") return "annual_expense";
      return base.fire_number_mode;
    })(),
    fire_number_manual_amount:
      raw.fire_number_manual_amount != null
        ? String(raw.fire_number_manual_amount)
        : null,
    fire_number_expense_adjustment_pct:
      raw.fire_number_expense_adjustment_pct != null
        ? String(raw.fire_number_expense_adjustment_pct)
        : null,
    swr_pct:
      raw.swr_pct != null && String(raw.swr_pct).trim() !== ""
        ? String(raw.swr_pct)
        : base.swr_pct,
    taxes_enabled:
      typeof raw.taxes_enabled === "boolean"
        ? raw.taxes_enabled
        : base.taxes_enabled,
    tax_brackets:
      Array.isArray(raw.tax_brackets) && raw.tax_brackets.length > 0
        ? raw.tax_brackets.map((t) => ({
            up_to:
              t.up_to === undefined || t.up_to === null
                ? null
                : String(t.up_to),
            pct: String(t.pct ?? ""),
          }))
        : base.tax_brackets,
    savings_source: parseSavingsSource(raw.savings_source),
    // Round-trip OBLIGATORIO de las cuatro ventanas. El PATCH manda el objeto COMPLETO y el
    // `#[serde(default)]` del servidor rellena lo ausente con los defaults: si la SPA no las
    // devolviera, guardar cualquier otro ajuste las resetearía en silencio.
    income_avg_window_months: clampWindowMonths(raw?.income_avg_window_months, 3),
    income_avg_window_mode: parseAvgWindowMode(raw?.income_avg_window_mode),
    expense_avg_window_months: clampWindowMonths(raw?.expense_avg_window_months, 12),
    expense_avg_window_mode: parseAvgWindowMode(raw?.expense_avg_window_mode),
  };
}

/** Allow-list de la semántica de ventana; cualquier otra cosa → `calendar` (el default). */
export function parseAvgWindowMode(v: unknown): AvgWindowModeApi {
  return v === "data" ? "data" : "calendar";
}

/** Meses de una ventana, acotados a 1–60 igual que el servidor. */
export function clampWindowMonths(v: unknown, fallback: number): number {
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isInteger(n) || n < 1 || n > 60) return fallback;
  return n;
}

/**
 * ¿La fuente del ahorro usa el promedio de transacciones? True para el modo B
 * (`transactions_avg`) y el modo C (`budget_income_real_expense`), que comparten el mismo
 * promedio real de gasto de 12 meses. False para `budget` (modo A) y para valores ausentes.
 * Punto único de gate en cliente para: parenthetical de Resumen, preview del target y fetch
 * de `/v1/summary` en RetirementView.
 */
export function savingsSourceUsesTransactions(s?: SavingsSourceApi): boolean {
  return s === "transactions_avg" || s === "budget_income_real_expense";
}

const MONTH_ABBR = [
  "ene", "feb", "mar", "abr", "may", "jun",
  "jul", "ago", "sep", "oct", "nov", "dic",
];

/** `"2026-07"` → `"jul 2026"`. Devuelve la cadena tal cual si no tiene la forma esperada. */
export function formatYearMonth(ym: string | undefined): string | undefined {
  if (!ym || ym.length < 7) return undefined;
  const y = ym.slice(0, 4);
  const m = Number(ym.slice(5, 7));
  if (!Number.isInteger(m) || m < 1 || m > 12) return ym;
  return `${MONTH_ABBR[m - 1]} ${y}`;
}

/**
 * Prosa que explica de dónde sale un lado del ahorro, a partir de su `SavingsAvgBasisApi`.
 * Punto ÚNICO compartido por el Resumen, el chart de proyección y los textos de ayuda.
 *
 * `undefined` cuando el lado salió del presupuesto (no hay promedio que explicar).
 *
 * La distinción por `has_gaps` es la que impide una etiqueta mentirosa: doce meses con datos
 * dispersos en tres años NO son «media de ene–dic 2025», así que en ese caso se dice cuántos
 * meses son y hasta cuándo llegan, sin fingir un rango contiguo.
 */
export function savingsBasisParenthetical(
  basis: SavingsAvgBasisApi | undefined,
): string | undefined {
  if (!basis || basis.basis !== "average" || basis.months_with_data < 1) return undefined;
  const n = basis.months_with_data;
  const last = formatYearMonth(basis.last_month);
  const first = formatYearMonth(basis.first_month);
  if (basis.has_gaps) {
    return last
      ? `media de ${n} ${n === 1 ? "mes" : "meses"} con datos, hasta ${last}`
      : `media de ${n} ${n === 1 ? "mes" : "meses"} con datos`;
  }
  if (!first || !last) return `media de ${n} ${n === 1 ? "mes" : "meses"}`;
  return first === last ? `media de ${first}` : `media de ${first}–${last}`;
}

/**
 * Paréntesis de la tarjeta Runway cuando el servidor la marca infinita: explica el porqué (la
 * retirada anual cabe en el SWR configurado en Jubilación). Solo pinta la etiqueta — la decisión
 * es exclusivamente del servidor (`runway_is_indefinite`), aquí no se re-deriva el umbral. Sin
 * `fire_settings` (instalación aún sin cargar) se omite el número en vez de inventar el default:
 * el porcentaje mostrado debe ser siempre el realmente configurado.
 */
export function runwaySwrParenthetical(
  fire: FireSettingsApi | undefined | null,
): string {
  if (!fire) return "dentro del SWR";
  const swr = normalizeInstallationFireSettings(fire).swr_pct;
  return `dentro del SWR ${formatPercentAmount(swr)}`;
}

export function taxOnGrossCapitalAnnual(
  gross: number,
  brackets: TaxBracketApi[],
): number {
  if (!(gross > 0) || brackets.length === 0) return 0;
  let prevCeiling = 0;
  let tax = 0;
  for (let i = 0; i < brackets.length; i++) {
    const b = brackets[i];
    const rate = parseDisplayDecimal(String(b.pct));
    if (rate === null || !Number.isFinite(rate)) continue;
    const r = rate / 100;
    const rawUp = b.up_to;
    const isOpen =
      rawUp === null ||
      rawUp === undefined ||
      String(rawUp).trim() === "";
    if (isOpen) {
      const taxable = Math.max(0, gross - prevCeiling);
      tax += taxable * r;
      break;
    }
    const ceiling = parseDisplayDecimal(String(rawUp));
    if (ceiling === null || !Number.isFinite(ceiling)) continue;
    const sliceEnd = Math.min(gross, ceiling);
    const taxable = Math.max(0, sliceEnd - prevCeiling);
    tax += taxable * r;
    prevCeiling = ceiling;
    if (gross <= ceiling) break;
  }
  return tax;
}

export function grossUpNetAnnualFire(
  netAnnual: number,
  brackets: TaxBracketApi[],
  taxesEnabled: boolean,
): number {
  if (!taxesEnabled || !(netAnnual > 0)) return Math.max(0, netAnnual);
  let lo = netAnnual;
  let hi = Math.max(netAnnual * 4, netAnnual + 200_000);
  for (let i = 0; i < 90; i++) {
    const mid = (lo + hi) / 2;
    const after = mid - taxOnGrossCapitalAnnual(mid, brackets);
    if (after < netAnnual) lo = mid;
    else hi = mid;
  }
  return hi;
}

/** Devuelve el primer índice donde `nw[i] >= target_base × (1 + inflation/100)^(month_index/12)`. */
export function findFirstMonthNetWorthAtLeastInflated(
  points: ProjectionPointApi[],
  baseTarget: number,
  annualInflationPct: number,
): number | null {
  if (!(baseTarget > 0)) return null;
  const inflFactor = 1 + Math.max(0, annualInflationPct) / 100;
  for (const p of points) {
    const nw = parseDisplayDecimal(String(p.net_worth));
    if (nw === null) continue;
    const target = baseTarget * Math.pow(inflFactor, p.month_index / 12);
    if (nw >= target) return p.month_index;
  }
  return null;
}

export function computeFireAnnualNeedNetEur(
  fire: FireSettingsApi,
  expenseRegularMonthlyEquivalent: string | null | undefined,
  incomeMonthlyEquivalent: string | null | undefined,
  incomeRetirementMonthlyEquivalent: string | null | undefined,
): number | null {
  const expenseM = parseDisplayDecimal(
    String(expenseRegularMonthlyEquivalent ?? ""),
  );
  const incomeM = parseDisplayDecimal(String(incomeMonthlyEquivalent ?? ""));
  const incomeRetM =
    parseDisplayDecimal(String(incomeRetirementMonthlyEquivalent ?? "")) ?? 0;
  switch (fire.fire_number_mode) {
    case "manual": {
      const m = parseDisplayDecimal(String(fire.fire_number_manual_amount ?? ""));
      return m !== null && m > 0 ? m : null;
    }
    case "annual_expense": {
      if (expenseM === null) return null;
      const net = expenseM - incomeRetM;
      return net > 0 ? net * 12 : null;
    }
    case "current_income": {
      if (incomeM === null) return null;
      const net = incomeM - incomeRetM;
      return net > 0 ? net * 12 : null;
    }
    default:
      return expenseM !== null ? expenseM * 12 : null;
  }
}
