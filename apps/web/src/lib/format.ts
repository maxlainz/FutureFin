/**
 * Funciones puras de formato (importes, porcentajes, métricas). Sin estado, sin React.
 * Convención: monedas sin decimales, porcentajes con 1 decimal, locale es-ES.
 */

export const DISPLAY_NUMBER_LOCALE = "es-ES";

/** Placeholder visible cuando un KPI no tiene dato. */
export const METRIC_DASH = "—";

/** Parsea un string decimal "es" (`,` o `.`) a number; null si no parsea o vacío. */
export function parseDisplayDecimal(s: string): number | null {
  const t = String(s).trim();
  if (!t) return null;
  const n = Number(t.replace(",", "."));
  return Number.isFinite(n) ? n : null;
}

/**
 * Valores numéricos devueltos por la API (p. ej. `2.500000`) compactados para `<input>`
 * (`2.5`). Si no parsea a número finito, devuelve el texto recortado sin cambiar.
 */
export function formatEditableDecimalString(raw: string | null | undefined): string {
  if (raw == null) return "";
  const t = String(raw).trim();
  if (!t) return "";
  const n = parseDisplayDecimal(t);
  if (n === null || !Number.isFinite(n)) return t;
  return JSON.stringify(n);
}

/** Importes sin decimales. Miles con punto a partir de 10.000. */
export function formatMoneyAmount(s: string): string {
  const n = parseDisplayDecimal(s);
  if (n === null) return s;
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(n);
}

/** ISO 4217 (EUR, USD, GBP); inválido → null. */
export function normalizeCurrencyIso(code: string | undefined | null): string | null {
  const c = String(code ?? "").trim().toUpperCase();
  if (c.length !== 3 || !/^[A-Z]{3}$/.test(c)) return null;
  return c;
}

export function formatCurrencyAmount(s: string, currencyIso: string): string {
  const n = parseDisplayDecimal(s);
  if (n === null) return s;
  const iso = normalizeCurrencyIso(currencyIso);
  if (!iso) return formatMoneyAmount(s);
  try {
    return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
      style: "currency",
      currency: iso,
      currencyDisplay: "symbol",
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    }).format(n);
  } catch {
    return `${formatMoneyAmount(s)} ${iso}`;
  }
}

export function formatCurrencyOrDash(
  s: string | null | undefined,
  currencyIso: string,
): string {
  if (s == null || String(s).trim() === "") return METRIC_DASH;
  return formatCurrencyAmount(String(s), currencyIso);
}

/** Variante para los arrays de proyección que ya viajan como f64. */
export function formatCurrencyOrDashNumber(
  n: number | null | undefined,
  currencyIso: string,
): string {
  if (n == null || !Number.isFinite(n)) return METRIC_DASH;
  return formatCurrencyNumber(n, currencyIso);
}

export function formatCurrencyNumber(n: number, currencyIso: string): string {
  const iso = normalizeCurrencyIso(currencyIso);
  if (!iso || !Number.isFinite(n)) return formatMoneyAmount(String(n));
  try {
    return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
      style: "currency",
      currency: iso,
      currencyDisplay: "symbol",
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    }).format(n);
  } catch {
    return `${formatMoneyAmount(String(n))} ${iso}`;
  }
}

/** Porcentaje en pantalla: un decimal y « %» detrás (no usar como sufijo duplicado). */
export function formatPercentDisplay(n: number): string {
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(n)} %`;
}

/** Igual que formatPercentDisplay pero con «+» explícito en positivos (retornos). */
export function formatPercentDisplaySigned(n: number): string {
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
    signDisplay: "exceptZero",
  }).format(n)} %`;
}

/** Tasas en % ya como número API (ej. TAE 3.25 → «3,3 %»). */
export function formatPercentAmount(s: string): string {
  const n = parseDisplayDecimal(s);
  if (n === null) return s;
  return formatPercentDisplay(n);
}

/** Retorno acumulado (valor/compra − 1); no es TAE. `null` cuando no se puede calcular. */
export function assetImplicitTotalReturnLabel(
  currentValueStr: string,
  purchasePriceStr: string | null | undefined,
): string | null {
  if (purchasePriceStr == null || String(purchasePriceStr).trim() === "") {
    return null;
  }
  const cur = parseDisplayDecimal(currentValueStr);
  const pur = parseDisplayDecimal(String(purchasePriceStr));
  if (cur === null || pur === null || pur <= 0) return null;
  const pct = (cur / pur - 1) * 100;
  if (!Number.isFinite(pct)) return null;
  return formatPercentDisplaySigned(pct);
}

export function formatDebtToAssetsPct(ratio: string | null | undefined): string {
  if (ratio == null || ratio === "") return METRIC_DASH;
  const r = Number(String(ratio).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return formatPercentDisplay(r * 100);
}

/** Decimal fraction (e.g. 0.25) shown as percent. */
export function formatFractionAsPercent(ratio: string | null | undefined): string {
  if (ratio == null || ratio === "") return METRIC_DASH;
  const r = Number(String(ratio).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return formatPercentDisplay(r * 100);
}

/** Ocultar KPI de importe cuando falta dato o es exactamente 0. */
export function isZeroMoneyMetric(s: string | null | undefined): boolean {
  if (s == null || String(s).trim() === "") return true;
  const n = parseDisplayDecimal(String(s));
  return n === null || n === 0;
}

/** Ocultar KPI de fracción (tasa, ratio 0–1) cuando falta o es 0. */
export function isZeroFractionMetric(ratio: string | null | undefined): boolean {
  if (ratio == null || String(ratio).trim() === "") return true;
  const r = Number(String(ratio).replace(",", "."));
  return !Number.isFinite(r) || r === 0;
}

export function formatMonthsRough(s: string | null | undefined): string {
  if (s == null || s === "") return METRIC_DASH;
  const r = Number(String(s).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return `${r.toLocaleString(DISPLAY_NUMBER_LOCALE, {
    maximumFractionDigits: 1,
  })} meses`;
}

export function breakdownPercentOfTotal(part: string, whole: string): number | null {
  const p = parseDisplayDecimal(part);
  const w = parseDisplayDecimal(whole);
  if (p === null || w === null || w <= 0) return null;
  return Math.min(100, (p / w) * 100);
}

export function formatBreakdownPct(part: string, whole: string): string {
  const pct = breakdownPercentOfTotal(part, whole);
  if (pct === null) return METRIC_DASH;
  return formatPercentDisplay(pct);
}
