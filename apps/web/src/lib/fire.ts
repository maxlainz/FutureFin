/**
 * Ajustes FIRE compartidos del hogar en cliente: defaults, normalización del JSONB de la API y
 * rótulos de la procedencia del ahorro.
 *
 * 6.0.0 (D-7): la vista previa del objetivo FIRE (`grossUpNetAnnualFire`,
 * `computeFireAnnualNeedNetEur`) y su fixture de paridad `fire-parity.json` se retiraron con el
 * modelo de jubilación.
 */

import type {
  FireSettingsApi,
  AvgWindowModeApi,
  SavingsAvgBasisApi,
  SavingsSourceApi,
  TaxBracketApi,
} from "../api/types";

export const DEFAULT_ES_TAX_BRACKETS_API: TaxBracketApi[] = [
  { up_to: "6000", pct: "19" },
  { up_to: "50000", pct: "21" },
  { up_to: "200000", pct: "23" },
  { up_to: "300000", pct: "27" },
  { up_to: null, pct: "30" },
];

export function defaultFireSettingsApi(): FireSettingsApi {
  return {
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
    taxable_gain_ratio: "1",
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
    // `fire_number_mode`, `fire_number_manual_amount`, `swr_pct` y `horizon_lifespan_age` YA NO
    // están aquí (5.0.0 los mudó al perfil de jubilación, que 6.0.0 retiró). Un JSONB guardado
    // por 4.15.x que aún los traiga simplemente se ignora, igual que hace el servidor.
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
    // #140 fase 2: round-trip obligatorio; clamp [0,1] espejo de resolve_fire_settings.
    taxable_gain_ratio: clampTaxableGainRatio(raw?.taxable_gain_ratio, "1"),
  };
}

/** Cota de la fracción de plusvalía gravable ([0,1]), espejo de resolve_fire_settings. */
export function clampTaxableGainRatio(v: unknown, fallback: string): string {
  const n = typeof v === "string" ? Number(v) : typeof v === "number" ? v : NaN;
  if (!Number.isFinite(n)) return fallback;
  return String(Math.min(1, Math.max(0, n)));
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
 * Punto único de gate en cliente (p. ej. el parenthetical del Resumen).
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
  if (!basis || basis.basis !== "average" || basis.avg_months < 1) return undefined;
  const n = basis.avg_months;
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
 * Paréntesis del runway «indefinido». Desde 6.0.0 (D-7) el SWR ya no es del perfil de jubilación
 * (retirado): el servidor decide la marca con un 3,5 % fijo (`RUNWAY_LEGACY_SWR_PCT`) hasta que
 * D-22 la retire, y este texto lo dice con la misma cifra.
 */
export function runwaySwrParenthetical(): string {
  return "dentro del SWR 3,5 %";
}
