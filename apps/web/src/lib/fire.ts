/**
 * Cálculos FIRE en cliente: defaults, normalización del JSONB de la API, gross-up por tramos
 * españoles (mismo modelo que el motor Rust en `crates/engine/src/tax.rs` — mudado ahí en la Ola 6/#140).
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
  RetirementProfileApi,
  SavingsAvgBasisApi,
  SavingsSourceApi,
  TaxBracketApi,
} from "../api/types";
import { formatPercentAmount, parseDisplayDecimal } from "./format";
import { normalizeRetirementProfile } from "./retirementProfile";

/**
 * Lo que la vista previa del objetivo necesita saber del PERFIL de jubilación (5.0.0): el modo
 * y, si es manual, la cifra. Es un subconjunto declarado a propósito y no el perfil entero —
 * así el fixture de paridad `fire-parity.json`, que sigue describiendo la BASE del objetivo,
 * puede alimentar esta función sin arrastrar toda la forma del perfil.
 */
export type FireTargetModeInputs = {
  fire_number_mode: FireNumberModeApi;
  fire_number_manual_amount: string | null;
};

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
    // 5.0.0: `fire_number_mode`, `fire_number_manual_amount`, `swr_pct` y
    // `horizon_lifespan_age` YA NO están aquí — son personales y viven en
    // `RetirementProfileApi` (`lib/retirementProfile.ts`, decisión D13). Un JSONB guardado por
    // 4.15.x que aún los traiga simplemente se ignora, igual que hace el servidor.
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
 * Paréntesis de la tarjeta Runway cuando el servidor la marca infinita: explica el porqué (la
 * retirada anual cabe en el SWR configurado en Jubilación). Solo pinta la etiqueta — la decisión
 * es exclusivamente del servidor (`runway_is_indefinite`), aquí no se re-deriva el umbral. Sin
 * perfil cargado se omite el número en vez de inventar el default: el porcentaje mostrado debe
 * ser siempre el realmente configurado.
 *
 * 5.0.0: el SWR dejó de ser del hogar y pasó al perfil de jubilación del usuario (D13), así que
 * esta función lee el PERFIL, no `fire_settings`.
 */
export function runwaySwrParenthetical(
  profile: RetirementProfileApi | undefined | null,
): string {
  if (!profile) return "dentro del SWR";
  const swr = normalizeRetirementProfile(profile).swr_pct;
  return `dentro del SWR ${formatPercentAmount(swr)}`;
}

export function grossUpNetAnnualFire(
  netAnnual: number,
  brackets: TaxBracketApi[],
  taxesEnabled: boolean,
  taxableGainRatio: number = 1,
): number {
  // Forma cerrada por tramos, espejo EXACTO de gross_up_net_annual_fire del servidor
  // (crates/engine/src/tax.rs desde la Ola 6/#140) — Ola 2, #118. Hasta 4.6.0 aquí vivía una bisección
  // de 90 iteraciones con techo mágico max(net·4, net+200.000): con un tramo alto el techo
  // SATURABA en silencio y la vista previa publicaba un objetivo un 20 % más bajo que el del
  // servidor (Δ 3,43 M€ en el caso del fixture nuevo). La forma cerrada no tiene techo: en
  // cada tramo resuelve g = (net + K − r·prev)/(1 − r) y avanza si g supera el techo del tramo.
  if (!taxesEnabled || !(netAnnual > 0)) return Math.max(0, netAnnual);
  let prevCeiling = 0;
  let k = 0; // impuesto acumulado de los tramos completos ya recorridos
  for (const b of brackets) {
    const pct = parseDisplayDecimal(String(b.pct));
    if (pct === null || !Number.isFinite(pct)) continue; // borrador a medio teclear (solo cliente)
    const r = pct / 100;
    // #140 fase 2: la base imponible es g·G — el denominador cambia Y el test de validez
    // cambia de forma (g·G ≤ techo, no G ≤ techo): espejo exacto del servidor.
    const denom = 1 - r * taxableGainRatio;
    // Tipo efectivo ≥ 100 %: degeneración idéntica al servidor (devuelve el techo previo).
    if (denom <= 0) return prevCeiling;
    const gross = (netAnnual + k - r * prevCeiling) / denom;
    const rawUp = b.up_to;
    const isOpen = rawUp === null || rawUp === undefined || String(rawUp).trim() === "";
    if (isOpen) return gross;
    const ceiling = parseDisplayDecimal(String(rawUp));
    if (ceiling === null || !Number.isFinite(ceiling)) continue;
    if (taxableGainRatio * gross <= ceiling) return gross;
    k += r * (ceiling - prevCeiling);
    prevCeiling = ceiling;
  }
  // Sin tramo abierto: espejo VERBATIM del servidor (return netAnnual). La configuración es
  // inalcanzable persistida (validate_tax_brackets exige el último tramo abierto) y con
  // brackets vacíos k = 0, así que solo iguala a la bisección de antes; se copia tal cual
  // porque la paridad exige el MISMO algoritmo, no uno «mejor» en ramas muertas.
  return netAnnual;
}


/**
 * Necesidad anual NETA del objetivo, según el modo elegido. 5.0.0: el modo y el importe manual
 * llegan del PERFIL de jubilación (D13), no de `fire_settings` — de ahí que el primer parámetro
 * sea el subconjunto `FireTargetModeInputs` y no el objeto de ajustes del hogar.
 */
export function computeFireAnnualNeedNetEur(
  target: FireTargetModeInputs,
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
  switch (target.fire_number_mode) {
    case "manual": {
      const m = parseDisplayDecimal(String(target.fire_number_manual_amount ?? ""));
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
