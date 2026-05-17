import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type FormEvent,
  type PointerEvent,
  type WheelEvent,
  type ReactNode,
  type SetStateAction,
} from "react";
import "./App.css";

type HealthResponse = {
  status: string;
  service: string;
  version: string;
};

type UserResponse = {
  id: string;
  username: string;
  /** YYYY-MM-DD desde la API; ausente en clientes antiguos. */
  birth_date?: string | null;
};

type FireNumberModeApi =
  | "manual"
  | "annual_expense"
  | "current_income";

type TaxBracketApi = {
  up_to: string | null;
  pct: string;
};

type FireSettingsApi = {
  fire_number_mode: FireNumberModeApi;
  fire_number_manual_amount: string | null;
  fire_number_expense_adjustment_pct: string | null;
  swr_pct: string;
  taxes_enabled: boolean;
  tax_brackets: TaxBracketApi[];
};

type InstallationSnapshot = {
  id: string;
  base_currency: string;
  /** IANA TZ for civil "today" (liability derive, etc.) */
  calendar_tz?: string;
  /** Tasa de inflación anual %. `0` = target FIRE plano; > 0 = target móvil que crece con la inflación. */
  annual_inflation_assumption_percent: string;
  show_age_mode: string;
  /** Ausente en clientes antiguos; usar `defaultFireSettingsApi`. */
  fire_settings?: FireSettingsApi;
};

type InstallationAccess = {
  installation: InstallationSnapshot;
  role: "owner" | "member" | "viewer";
};

type InstallationSessionContext = {
  installation_initialized: boolean;
  access: InstallationAccess | null;
};

type InstallationGate =
  | "loading"
  | "fetch_failed"
  | "member"
  | "pending"
  | "bootstrap";

type CategoryScope = "asset" | "liability" | "income" | "expense";

type CategoryRow = {
  id: string;
  scope: CategoryScope;
  name: string;
  sort_index: number;
};

type AssetApiRow = {
  id: string;
  category_id: string;
  name: string;
  current_value: string;
  purchase_price: string | null;
  is_liquid: boolean;
  expected_annual_return_percent?: string | null;
  /** Aporte estimado mes 1 derivado de las reglas de asignación del sobrante. */
  contribution_nominal_monthly?: string;
  /** Tope absoluto en € si una regla apunta a este activo con cap_kind='amount'. */
  contribution_target_amount?: string | null;
  notes: string | null;
  sort_index: number;
};

type AllocationRuleKind = "fixed" | "percent" | "remainder";
type AllocationRuleCapKind = "amount" | "months_expense" | "income_multiple";

type AllocationRuleApiRow = {
  id: string;
  target_asset_id: string;
  priority: number;
  kind: AllocationRuleKind;
  amount?: string | null;
  cap_kind?: AllocationRuleCapKind | null;
  cap_value?: string | null;
  enabled: boolean;
  notes?: string | null;
  owner_user_id?: string | null;
};

type FinancialHealthMetrics = {
  income_monthly_equivalent: string;
  expense_regular_monthly_equivalent: string;
  expense_derived_monthly_equivalent: string;
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
  savings_rate: string | null;
  monthly_net_excluding_derived_debt: string;
  savings_rate_excluding_derived_debt: string | null;
  liquid_assets_total: string;
  runway_months: string | null;
  upcoming_inflows_total: string;
  upcoming_outflows_total: string;
  upcoming_coverage_ratio: string | null;
};

type CategoryBreakdownLineApi = {
  category_id: string;
  category_name: string;
  total: string;
};

type SummaryResponse = {
  total_assets: string;
  total_liabilities: string;
  net_worth: string;
  debt_to_assets_ratio: string | null;
  financial_health: FinancialHealthMetrics;
  assets_by_category: CategoryBreakdownLineApi[];
  liabilities_by_category: CategoryBreakdownLineApi[];
};

type BudgetTotalsApi = {
  income_monthly_equivalent: string;
  income_retirement_monthly_equivalent: string;
  expense_regular_monthly_equivalent: string;
  expense_derived_monthly_equivalent: string;
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
};

type BudgetEntryApiRow = {
  id: string;
  category_id: string;
  scope: "income" | "expense";
  /** Importe mensual */
  amount: string;
  notes: string | null;
  sort_index: number;
  persists_after_retirement: boolean;
  ends_at_retirement: boolean;
  expense_end_date: string | null;
};

type DerivedBudgetLineApi = {
  liability_id: string;
  category_id: string;
  label: string;
  amount: string;
  frequency: "monthly" | "weekly";
  monthly_equivalent: string;
  notes: string;
};

type BudgetSnapshotApi = {
  entries: BudgetEntryApiRow[];
  derived_from_liabilities: DerivedBudgetLineApi[];
  totals: BudgetTotalsApi;
};

type BudgetScopeToggle = "income" | "expense";

type PlanningFlowDirectionApi = "inflow" | "outflow";

type PlanningFlowApiRow = {
  id: string;
  category_id: string;
  direction: PlanningFlowDirectionApi;
  title: string;
  expected_amount: string;
  due_date: string | null;
  notes: string | null;
  sort_index: number;
  show_in_chart: boolean;
};

type ProjectionPointApi = {
  month_index: number;
  net_worth: string;
  contributed_capital: string;
};

type ProjectionMilestoneApi = {
  target: string;
  reached_month_index: number;
  reached_date_ymd: string;
};

type AssetSeriesApi = {
  asset_id: string;
  asset_name: string;
  values: string[];
};

type ProjectionSeriesApi = {
  points: ProjectionPointApi[];
  milestones: ProjectionMilestoneApi[];
  compound_outpaces_true_savings_month_index?: number | null;
  months: number;
  horizon_years: number;
  horizon_basis: string;
  starting_net_worth: string;
  monthly_delta_assumption: string;
  model_note: string;
  /** Mes 0 de la serie = esta fecha civil (servidor). */
  anchor_date_ymd?: string;
  show_age_mode?: string;
  /** Decisión del servidor: eje en años cumplidos (no inferir solo desde instalación en cliente). */
  use_age_on_x_axis?: boolean;
  viewer_birth_date?: string | null;
  /** Primer mes en que el patrimonio neto ≥ objetivo FIRE móvil del mes en curso. `null` si no alcanzado. */
  jubilacion_month_index?: number | null;
  /** Objetivo FIRE base en euros de hoy. El target real de cada mes crece con la inflación (ver `fire_target_series`). */
  jubilacion_target_net_worth?: string | null;
  /** Serie del target FIRE ajustado por inflación, paralela a `points`. Vacío cuando no hay FIRE configurado. */
  fire_target_series?: string[];
  asset_series?: AssetSeriesApi[];
};

type LiabilityApiRow = {
  id: string;
  category_id: string;
  label: string;
  type_tag: string | null;
  principal_derived_from_plan?: boolean;
  principal: string;
  apr_percent: string | null;
  payment_amount: string | null;
  payment_frequency: "monthly" | "weekly" | null;
  payment_end_date: string | null;
  notes: string | null;
  sort_index: number;
};

type LiabilityPaymentFreq = "" | "monthly" | "weekly";

/** Fallback civil date when a TZ string is invalid. */
function utcTodayYmd(): string {
  const d = new Date();
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** Today's calendar date in an IANA zone (matches server derive-principal "today"). */
function todayYmdInTimeZone(tz: string): string {
  const trimmed = tz.trim() || "UTC";
  try {
    const fmt = new Intl.DateTimeFormat("en-CA", {
      timeZone: trimmed,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });
    const parts = fmt.formatToParts(new Date());
    const y = parts.find((p) => p.type === "year")?.value;
    const m = parts.find((p) => p.type === "month")?.value;
    const d = parts.find((p) => p.type === "day")?.value;
    if (y && m && d) return `${y}-${m}-${d}`;
  } catch {
    /* unknown time zone */
  }
  return utcTodayYmd();
}

function parseYmdUtc(ymd: string): Date {
  const [ys, ms, ds] = ymd.split("-").map((x) => Number(x));
  return new Date(Date.UTC(ys, ms - 1, ds));
}

function addOneMonthUtc(d: Date): Date {
  const y = d.getUTCFullYear();
  const m = d.getUTCMonth();
  const day = d.getUTCDate();
  const dim = new Date(Date.UTC(y, m + 2, 0)).getUTCDate();
  const next = new Date(Date.UTC(y, m + 1, 1));
  next.setUTCDate(Math.min(day, dim));
  return next;
}

function paymentIntervalCountUtc(
  freq: "monthly" | "weekly",
  startYmd: string,
  endYmd: string,
): number | null {
  const start = parseYmdUtc(startYmd);
  const end = parseYmdUtc(endYmd);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
    return null;
  }
  if (end.getTime() < start.getTime()) return null;
  if (freq === "monthly") {
    let n = 0;
    let cur = new Date(start.getTime());
    while (cur.getTime() <= end.getTime()) {
      n += 1;
      if (n > 1200) return null;
      cur = addOneMonthUtc(cur);
    }
    return n;
  }
  const days = Math.floor((end.getTime() - start.getTime()) / 86400000) + 1;
  const di = Math.max(1, days);
  return Math.ceil(di / 7);
}

const PAYMENT_FREQ_LABEL: Record<"monthly" | "weekly", string> = {
  monthly: "Mensual",
  weekly: "Semanal",
};

const CATEGORY_SCOPE_LABEL: Record<CategoryScope, string> = {
  asset: "Activos",
  liability: "Pasivos",
  income: "Ingresos",
  expense: "Gastos",
};

const CATEGORY_SCOPES: CategoryScope[] = [
  "asset",
  "liability",
  "income",
  "expense",
];

/**
 * Paletas por ámbito: cada entrada es un color bien diferenciado dentro de la misma
 * familia (fríos = activos, cálidos = pasivos). Se cicla si hay más categorías.
 */
/** Activos: saltos amplios en matiz + L/S alternados para que segmentos vecinos no parezcan gemelos. */
const SUMMARY_CHART_PALETTE_ASSET: readonly [number, number, number][] = [
  [205, 52, 37],
  [118, 62, 33],
  [178, 48, 44],
  [138, 68, 36],
  [218, 46, 42],
  [158, 70, 34],
  [192, 42, 47],
  [128, 56, 39],
];

const SUMMARY_CHART_PALETTE_LIABILITY: readonly [number, number, number][] = [
  [14, 64, 41],
  [32, 58, 44],
  [44, 56, 42],
  [22, 62, 39],
  [8, 66, 40],
  [38, 54, 46],
  [26, 60, 43],
  [354, 58, 41],
];

function summaryChartSliceParts(
  scope: "asset" | "liability",
  sliceIndex: number,
): { h: number; s: number; l: number } {
  const pal =
    scope === "asset"
      ? SUMMARY_CHART_PALETTE_ASSET
      : SUMMARY_CHART_PALETTE_LIABILITY;
  const [h, s, l] = pal[sliceIndex % pal.length]!;
  return { h, s, l };
}

function summaryChartSliceColor(
  scope: "asset" | "liability",
  sliceIndex: number,
): string {
  const { h, s, l } = summaryChartSliceParts(scope, sliceIndex);
  return `hsl(${h} ${s}% ${l}%)`;
}

function summaryBreakdownBarGradient(
  scope: "asset" | "liability",
  sliceIndex: number,
): string {
  const { h, s, l } = summaryChartSliceParts(scope, sliceIndex);
  const lo = `hsl(${h} ${s}% ${l}%)`;
  const hi = `hsl(${h} ${Math.max(s - 8, 36)}% ${Math.min(l + 12, 56)}%)`;
  return `linear-gradient(90deg, ${lo}, ${hi})`;
}

type TabId =
  | "summary"
  | "assets"
  | "liabilities"
  | "budget"
  | "upcoming"
  | "projection"
  | "retirement"
  | "settings";

type SettingsSubTabId =
  | "access"
  | "calendar"
  | "projection"
  | "retirement"
  | "categories"
  | "data";

const TABS: { id: TabId; label: string }[] = [
  { id: "summary", label: "Resumen" },
  { id: "assets", label: "Activos" },
  { id: "liabilities", label: "Pasivos" },
  { id: "budget", label: "Presupuesto" },
  { id: "upcoming", label: "Próximos" },
  { id: "retirement", label: "Jubilación" },
  { id: "projection", label: "Proyección" },
  { id: "settings", label: "Ajustes" },
];

/** Ruta canónica por pestaña (español, sin acentos en la URL salvo donde ya es habitual). */
const TAB_PATH: Record<TabId, string> = {
  summary: "/resumen",
  assets: "/activos",
  liabilities: "/pasivos",
  budget: "/presupuesto",
  upcoming: "/proximos",
  projection: "/proyeccion",
  retirement: "/jubilacion",
  settings: "/ajustes",
};

/** Slug por subsección de ajustes — la URL completa es `/ajustes/<slug>`. */
const SETTINGS_SUBTAB_SLUG: Record<SettingsSubTabId, string> = {
  access: "acceso",
  calendar: "calendario",
  projection: "proyeccion",
  retirement: "jubilacion",
  categories: "categorias",
  data: "datos",
};

const SETTINGS_SUBTAB_LABEL: Record<SettingsSubTabId, string> = {
  access: "Acceso",
  calendar: "Calendario",
  projection: "Proyección",
  retirement: "Jubilación",
  categories: "Categorías",
  data: "Datos y sistema",
};

function normalizeAppPath(pathname: string): string {
  const p = pathname.replace(/\/+$/, "") || "/";
  return p;
}

function tabFromPathname(pathname: string): TabId | null {
  const p = normalizeAppPath(pathname);
  if (p === TAB_PATH.settings || p.startsWith(`${TAB_PATH.settings}/`)) {
    return "settings";
  }
  const ids = Object.keys(TAB_PATH) as TabId[];
  for (const id of ids) {
    if (TAB_PATH[id] === p) return id;
  }
  return null;
}

function settingsSubTabFromPathname(pathname: string): SettingsSubTabId | null {
  const p = normalizeAppPath(pathname);
  const prefix = `${TAB_PATH.settings}/`;
  if (!p.startsWith(prefix)) return null;
  const slug = p.slice(prefix.length);
  const entries = Object.entries(SETTINGS_SUBTAB_SLUG) as [
    SettingsSubTabId,
    string,
  ][];
  for (const [id, s] of entries) {
    if (s === slug) return id;
  }
  return null;
}

function settingsSubTabPath(id: SettingsSubTabId): string {
  return `${TAB_PATH.settings}/${SETTINGS_SUBTAB_SLUG[id]}`;
}

function useAppPathNavigation(): [
  pathname: string,
  navigate: (path: string, replace?: boolean) => void,
] {
  const [pathname, setPathname] = useState(() =>
    typeof window !== "undefined"
      ? window.location.pathname
      : TAB_PATH.summary,
  );

  useEffect(() => {
    const onPop = () => setPathname(window.location.pathname);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const navigate = useCallback((path: string, replace = false) => {
    const url = path.startsWith("/") ? path : `/${path}`;
    if (replace) window.history.replaceState(null, "", url);
    else window.history.pushState(null, "", url);
    setPathname(window.location.pathname);
  }, []);

  return [pathname, navigate];
}

const defaultFetchInit: RequestInit = {
  credentials: "include",
};

const METRIC_DASH = "—";

/** Hogar = todos los registros de la instalación; usuario actual = solo filas con tu `owner_user_id`. */
type LedgerPersonScope = "household" | "mine";

const LEDGER_PERSON_SCOPE_STORAGE_KEY = "futurefin-ledger-person-scope";
const PROJECTION_FOCUS_STORAGE_KEY = "futurefin-projection-focus";
const PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY =
  "futurefin-projection-inflation-adjusted";

type FfbackupImportCounts = {
  assets: number;
  liabilities: number;
  budget_entries: number;
  planning_flows: number;
  categories_in_backup: number;
  categories_already_present: number;
  categories_to_create: number;
};

type FfbackupImportPreviewResponse = {
  schema_version: number;
  app_version: string;
  exported_at: string;
  username_original: string;
  counts: FfbackupImportCounts;
  birth_date_will_change: boolean;
  ui_preferences_present: boolean;
};

type FfbackupImportApplyResponse = {
  imported: FfbackupImportCounts;
  ui_preferences: {
    person_scope?: string | null;
    projection_focus?: string | null;
  };
};

function ledgerViewQs(scope: LedgerPersonScope): string {
  return scope === "mine" ? "?view=mine" : "";
}

/**
 * Locale para cantidades: coma decimal, y separador de miles «.» cuando el entero
 * tiene ≥ 5 cifras (≥ 10.000), vía `Intl` es-ES.
 */
const DISPLAY_NUMBER_LOCALE = "es-ES";

function parseDisplayDecimal(s: string): number | null {
  const t = String(s).trim();
  if (!t) return null;
  const n = Number(t.replace(",", "."));
  return Number.isFinite(n) ? n : null;
}

/**
 * Valores numéricos devueltos por la API (p. ej. `2.500000`) compactados para `<input>`
 * (`2.5`). Si no parsea a número finito, devuelve el texto recortado sin cambiar.
 */
function formatEditableDecimalString(raw: string | null | undefined): string {
  if (raw == null) return "";
  const t = String(raw).trim();
  if (!t) return "";
  const n = parseDisplayDecimal(t);
  if (n === null || !Number.isFinite(n)) return t;
  return JSON.stringify(n);
}

/**
 * Importes sin decimales. Miles con punto a partir de 10.000 (p. ej. `10.000`, no `10000`).
 */
function formatMoneyAmount(s: string): string {
  const n = parseDisplayDecimal(s);
  if (n === null) return s;
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(n);
}

/** ISO 4217 code from installation (EUR, USD, GBP); invalid → no symbol formatting. */
function normalizeCurrencyIso(code: string | undefined | null): string | null {
  const c = String(code ?? "").trim().toUpperCase();
  if (c.length !== 3 || !/^[A-Z]{3}$/.test(c)) return null;
  return c;
}

function formatCurrencyAmount(s: string, currencyIso: string): string {
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

function formatCurrencyOrDash(
  s: string | null | undefined,
  currencyIso: string,
): string {
  if (s == null || String(s).trim() === "") return METRIC_DASH;
  return formatCurrencyAmount(String(s), currencyIso);
}

function formatCurrencyNumber(n: number, currencyIso: string): string {
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

/** Aporte mensual estimado (primer mes motor) leído de `contribution_nominal_monthly`. */
function assetContributionMonthlyEstimateNum(a: AssetApiRow): number {
  const raw = a.contribution_nominal_monthly;
  if (raw == null) return 0;
  const n = parseDisplayDecimal(String(raw).trim());
  return n != null && n > 0 ? n : 0;
}

function formatAssetContributionNominalCell(
  a: AssetApiRow,
  currencyIso: string,
): string {
  const n = assetContributionMonthlyEstimateNum(a);
  return n > 0 ? formatCurrencyNumber(n, currencyIso) : METRIC_DASH;
}

/** Suma valor actual y coste solo en posiciones con compra válida (> 0). */
function assetPortfolioCostTotals(
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
function liabilityPaymentMonthlyEquivalentNum(
  row: LiabilityApiRow,
): number {
  const amt = parseDisplayDecimal(String(row.payment_amount ?? "").trim());
  if (amt === null || amt <= 0) return 0;
  if (row.payment_frequency === "weekly") return (amt * 52) / 12;
  if (row.payment_frequency === "monthly") return amt;
  return 0;
}

/** TAE % media ponderada por principal (solo pasivos con TAE informada). */
function liabilitiesWeightedAprPercent(
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
function liabilitiesApproxMonthlyInterestSum(
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
function formatAxisMoney(n: number, currencyIso: string): string {
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
function roundUpToHundred(n: number): number {
  return Math.ceil(n / 100) * 100;
}

function formatProjectionMilestoneCompactLabel(target: string): string {
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

function normalizeAgeUiMode(raw: string | null | undefined): "dates" | "ages" {
  const t = (raw ?? "").trim().toLowerCase();
  return t === "ages" ? "ages" : "dates";
}

/** Prioriza `use_age_on_x_axis` del GET /v1/projection/series (fuente de verdad). */
function resolveProjectionAxisAgeMode(
  series: ProjectionSeriesApi,
  installation: InstallationAccess | null,
): "dates" | "ages" {
  if (series.use_age_on_x_axis === true) {
    return "ages";
  }
  if (series.use_age_on_x_axis === false) {
    return "dates";
  }
  return normalizeAgeUiMode(
    series.show_age_mode ?? installation?.installation.show_age_mode,
  );
}

/**
 * Subtítulo bajo la gráfica de proyección: evita mostrar `horizon_years` como si fuera
 * una edad (es la duración en años de la vista). Preferimos edad objetivo + año de fin
 * de serie cuando procede.
 */
function formatProjectionChartHorizonLine(series: ProjectionSeriesApi): string {
  const basis = series.horizon_basis;
  const spanYears = series.horizon_years;
  const anchorStr = series.anchor_date_ymd?.trim();
  const anchor = anchorStr ? parseYmdComponents(anchorStr) : null;
  const mc = series.months;

  const endCivil =
    anchor != null && mc >= 0
      ? addMonthsCivil(anchor.y, anchor.m, anchor.d, mc)
      : null;
  const endYearStr = endCivil ? formatProjectionAxisYear(endCivil) : null;

  switch (basis) {
    case "lifespan_90":
      if (endYearStr != null) {
        return `Horizonte 90 años · fin ${endYearStr}`;
      }
      return `Horizonte 90 años`;
    case "fallback_no_demographics":
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista · sin fecha de nacimiento`;
    case "months_override":
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista (meses explícitos)`;
    default:
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista`;
  }
}

/** Civil Y-M-D desde `YYYY-MM-DD` o prefijo ISO (`YYYY-MM-DDTHH:mm:ss`). */
function parseYmdComponents(
  ymd: string | null | undefined,
): { y: number; m: number; d: number } | null {
  if (!ymd || typeof ymd !== "string") return null;
  const t = ymd.trim();
  const mm = /^(\d{4})-(\d{2})-(\d{2})/.exec(t);
  if (!mm) return null;
  const y = Number(mm[1]);
  const m = Number(mm[2]);
  const d = Number(mm[3]);
  if (
    !Number.isFinite(y) ||
    !Number.isFinite(m) ||
    !Number.isFinite(d) ||
    m < 1 ||
    m > 12 ||
    d < 1 ||
    d > 31
  ) {
    return null;
  }
  return { y, m, d };
}

function civilDaysInMonth(y: number, m: number): number {
  return new Date(y, m, 0).getDate();
}

/** Civil calendar: today + `deltaMonths` (alineado a edad completada tipo servidor). */
function addMonthsCivil(
  y: number,
  m: number,
  d: number,
  deltaMonths: number,
): { y: number; m: number; d: number } {
  const total = y * 12 + (m - 1) + deltaMonths;
  const ny = Math.floor(total / 12);
  const nm = (total % 12) + 1;
  const dim = civilDaysInMonth(ny, nm);
  const nd = Math.min(d, dim);
  return { y: ny, m: nm, d: nd };
}

/** Edad en años cumplidos (misma idea que `age_completed_years` en la API). */
function ageCompletedYearsCivil(
  today: { y: number; m: number; d: number },
  birth: { y: number; m: number; d: number },
): number {
  const tb = birth.y * 10000 + birth.m * 100 + birth.d;
  const tt = today.y * 10000 + today.m * 100 + today.d;
  if (birth.y > today.y) return 0;
  if (tb > tt) return 0;
  let years = today.y - birth.y;
  if (
    today.m < birth.m ||
    (today.m === birth.m && today.d < birth.d)
  ) {
    years -= 1;
  }
  return years;
}

/** Año civil en el eje (modo fechas); mismo calendario que la ancla + meses. */
function formatProjectionAxisYear(civil: { y: number; m: number; d: number }): string {
  const dt = new Date(civil.y, civil.m - 1, civil.d);
  return new Intl.DateTimeFormat(DISPLAY_NUMBER_LOCALE, {
    year: "numeric",
  }).format(dt);
}

/** Tooltip en modo fechas: mes/año civil (`MM/YYYY`). */
function formatProjectionHoverMonthYear(civil: { y: number; m: number; d: number }): string {
  const mm = String(civil.m).padStart(2, "0");
  return `${mm}/${civil.y}`;
}

/**
 * Índices de mes para marcas del eje X: respeta `maxTicks` según ancho en px y alinea a años si el horizonte es largo.
 */
function buildProjectionMonthTickIndices(
  mc: number,
  maxTicks: number,
): number[] {
  if (mc <= 0) {
    return [0];
  }
  const cap = Math.max(4, Math.min(maxTicks, 22));
  const roughStep = Math.ceil(mc / Math.max(1, cap - 1));
  let step = roughStep;
  if (mc > 36) {
    // Para horizontes largos, mantenemos detalle anual (nunca más grueso que 1 año).
    const yAligned = Math.max(12, Math.ceil(roughStep / 12) * 12);
    step = Math.min(12, yAligned);
  } else {
    const shortSteps = [1, 2, 3, 6, 12];
    step = shortSteps.find((s) => s >= roughStep) ?? roughStep;
  }
  const ticks: number[] = [0];
  for (let m = step; m < mc; m += step) {
    ticks.push(m);
  }
  if (ticks[ticks.length - 1] !== mc) {
    ticks.push(mc);
  }
  return ticks;
}

/**
 * Marcas del eje X (modo fechas): primer mes (enero) de cada año civil que
 * cae estrictamente dentro de [anchor, anchor+monthEnd]. Excluye el año del
 * ancla para evitar apilar un label en el borde izquierdo del plot.
 */
function buildProjectionTicksFirstMonthOfYear(
  anchor: { y: number; m: number; d: number },
  monthEnd: number,
): number[] {
  if (monthEnd < 1) return [];
  const end = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthEnd);
  const out: number[] = [];
  for (let y = anchor.y + 1; y <= end.y; y++) {
    const mi = (y - anchor.y) * 12 + (1 - anchor.m);
    if (mi < 1 || mi > monthEnd) continue;
    out.push(mi);
  }
  return out;
}

/**
 * Marcas del eje X (modo edades): mes en el que se cumplen los años (la edad
 * completada se incrementa). Itera mes a mes y detecta transiciones usando la
 * misma lógica que `projectionXTickLabel` (`ageCompletedYearsCivil`), de modo
 * que la posición coincida exactamente con el label. Excluye el ancla.
 */
function buildProjectionTicksFirstMonthOfAge(
  anchor: { y: number; m: number; d: number },
  birth: { y: number; m: number; d: number },
  monthEnd: number,
): number[] {
  if (monthEnd < 1) return [];
  const out: number[] = [];
  let prevAge = ageCompletedYearsCivil(anchor, birth);
  for (let i = 1; i <= monthEnd; i++) {
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, i);
    const age = ageCompletedYearsCivil(at, birth);
    if (age !== prevAge) {
      out.push(i);
      prevAge = age;
    }
  }
  return out;
}

function projectionXTickLabel(
  monthIndex: number,
  monthCount: number,
  opts?: {
    ageUiMode: "dates" | "ages";
    birthDateIso?: string | null;
    /** Fecha civil del mes 0 (YYYY-MM-DD), p. ej. desde GET /v1/projection/series. */
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
): string {
  const mc = Number(monthCount);
  const safeMc = Number.isFinite(mc) && mc >= 0 ? mc : 0;
  const relativeFallback =
    monthIndex === 0
      ? "Hoy"
      : safeMc <= 48
        ? `${monthIndex} m`
        : `${Math.round(monthIndex / 12)} a`;

  if (!opts) {
    return `Mes ${monthIndex}`;
  }

  const anchorStr =
    opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
      ? opts.anchorDateYmd.trim()
      : todayYmdInTimeZone(opts.calendarTz);
  const anchor = parseYmdComponents(anchorStr);

  if (opts.ageUiMode === "ages") {
    const birth = parseYmdComponents(opts.birthDateIso);
    if (!birth || !anchor) {
      return relativeFallback;
    }
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
    const age = ageCompletedYearsCivil(at, birth);
    return `${age} a`;
  }

  // Modo fechas: solo año civil en el eje.
  if (!anchor) {
    return `Mes ${monthIndex}`;
  }
  const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
  return formatProjectionAxisYear(at);
}

const DEFAULT_ES_TAX_BRACKETS_API: TaxBracketApi[] = [
  { up_to: "6000", pct: "19" },
  { up_to: "50000", pct: "21" },
  { up_to: "200000", pct: "23" },
  { up_to: "300000", pct: "27" },
  { up_to: null, pct: "30" },
];

function defaultFireSettingsApi(): FireSettingsApi {
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
  };
}

function normalizeInstallationFireSettings(
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
  };
}

function taxOnGrossCapitalAnnual(
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

function grossUpNetAnnualFire(
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
function findFirstMonthNetWorthAtLeastInflated(
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

function formatYearsEsFromMonths(months: number): string {
  const y = Math.round(months / 12);
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(y)} años`;
}

function computeFireAnnualNeedNetEur(
  fire: FireSettingsApi,
  expenseRegularMonthlyEquivalent: string | null | undefined,
  incomeMonthlyEquivalent: string | null | undefined,
  incomeRetirementMonthlyEquivalent: string | null | undefined,
): number | null {
  const expenseM = parseDisplayDecimal(
    String(expenseRegularMonthlyEquivalent ?? ""),
  );
  const incomeM = parseDisplayDecimal(String(incomeMonthlyEquivalent ?? ""));
  const incomeRetM = parseDisplayDecimal(String(incomeRetirementMonthlyEquivalent ?? "")) ?? 0;
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

function complementaryProjectionTickLabel(
  monthIndex: number,
  monthCount: number,
  primaryAgeMode: "dates" | "ages",
  opts: {
    birthDateIso?: string | null;
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
): string {
  const altMode = primaryAgeMode === "ages" ? "dates" : "ages";
  return projectionXTickLabel(monthIndex, monthCount, {
    ageUiMode: altMode,
    birthDateIso: opts.birthDateIso,
    anchorDateYmd: opts.anchorDateYmd,
    calendarTz: opts.calendarTz,
  });
}

function projectionHoverTitle(
  monthIndex: number,
  ageUiMode: "dates" | "ages",
  userBirthDate: string | null,
  calendarTz: string,
  anchorDateYmd?: string | null,
): string {
  const anchorStr =
    anchorDateYmd != null && anchorDateYmd.trim() !== ""
      ? anchorDateYmd.trim()
      : todayYmdInTimeZone(calendarTz);
  const anchor = parseYmdComponents(anchorStr);

  if (ageUiMode !== "ages") {
    if (!anchor) {
      return METRIC_DASH;
    }
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
    return formatProjectionHoverMonthYear(at);
  }

  const birth = parseYmdComponents(userBirthDate);
  if (!birth || !anchor) {
    return METRIC_DASH;
  }
  const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
  const age = ageCompletedYearsCivil(at, birth);
  return `${age} años`;
}

function projectionXTicks(
  monthCount: number,
  opts?: {
    ageUiMode: "dates" | "ages";
    birthDateIso?: string | null;
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
  density?: { plotWidthPx: number },
): { monthIndex: number; label: string }[] {
  const mcRaw = Number(monthCount);
  const mc = Number.isFinite(mcRaw) && mcRaw >= 0 ? mcRaw : 0;
  const pw = density?.plotWidthPx ?? 600;
  const minPx =
    opts?.ageUiMode === "dates"
      ? 34
      : 28;
  const maxTicks = Math.max(5, Math.min(18, Math.floor(Math.max(120, pw) / minPx)));

  let ticks: number[] | null = null;

  if (opts) {
    const anchorStr =
      opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
        ? opts.anchorDateYmd.trim()
        : todayYmdInTimeZone(opts.calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (opts.ageUiMode === "dates" && anchor) {
      ticks = buildProjectionTicksFirstMonthOfYear(anchor, mc);
    } else if (opts.ageUiMode === "ages" && anchor) {
      const birth = parseYmdComponents(opts.birthDateIso);
      if (birth) {
        ticks = buildProjectionTicksFirstMonthOfAge(anchor, birth, mc);
      }
    }
  }

  if (!ticks) {
    ticks = buildProjectionMonthTickIndices(mc, maxTicks);
  }

  return ticks.map((m) => ({
    monthIndex: m,
    label: projectionXTickLabel(m, mc, opts),
  }));
}

/** Geometría del SVG en unidades de usuario: depende del ancho CSS real del contenedor. */
function buildProjectionChartLayout(
  containerCssWidth: number,
  containerCssHeight: number | undefined,
  legendLabels: string[],
) {
  const W = Math.max(300, Math.round(containerCssWidth));
  const aspect = 460 / 1040;
  let H = Math.round(W * aspect);
  if (
    containerCssHeight != null &&
    Number.isFinite(containerCssHeight) &&
    containerCssHeight > 0
  ) {
    H = Math.round(containerCssHeight);
  }
  H = Math.max(260, Math.min(H, 980));

  const narrow = W < 560;
  const ml = narrow
    ? Math.round(42 + W * 0.035)
    : Math.round(68 + Math.min(36, (W - 560) * 0.045));
  const mr = narrow ? 12 : Math.round(26 + Math.min(30, (W - 560) * 0.022));
  const mb = narrow ? 32 : 38;

  const pw = W - ml - mr;

  // Empaquetado horizontal de la leyenda en filas, justificada a la derecha
  // del contenedor. Ancho aproximado por item: swatch (24px) + gap (6px)
  // + ancho de texto estimado (6.5px por carácter).
  const legendRowHeight = 22;
  const legendItemGap = 14;
  const legendCharPx = 6.5;
  const legendSwatchPx = 24 + 6;
  // Borde derecho de referencia: el borde derecho del plot (ml + pw),
  // nunca más a la derecha. Así la leyenda no sobresale del eje.
  const legendRightAnchor = ml + pw;
  // Espacio horizontal disponible para la leyenda: limitado para no chocar
  // con los headlines (a la izquierda) — ~60% del ancho del plot.
  const legendBudgetWidth = Math.max(160, Math.round(pw * 0.6));
  const itemWidths = legendLabels.map(
    (label) => legendSwatchPx + label.length * legendCharPx,
  );
  const rows: number[][] = [];
  {
    let currentRow: number[] = [];
    let currentRowWidth = 0;
    for (let i = 0; i < legendLabels.length; i++) {
      const itemW = itemWidths[i];
      const widthIfAdded =
        currentRow.length === 0
          ? itemW
          : currentRowWidth + legendItemGap + itemW;
      if (currentRow.length > 0 && widthIfAdded > legendBudgetWidth) {
        rows.push(currentRow);
        currentRow = [i];
        currentRowWidth = itemW;
      } else {
        currentRow.push(i);
        currentRowWidth = widthIfAdded;
      }
    }
    if (currentRow.length > 0) rows.push(currentRow);
  }
  const rowsNeeded = Math.max(1, rows.length);
  const placements: Array<{ x: number; y: number }> = new Array(
    legendLabels.length,
  );
  rows.forEach((row, rowIdx) => {
    const rowTotal = row.reduce(
      (sum, itemIdx, idx) =>
        sum + itemWidths[itemIdx] + (idx > 0 ? legendItemGap : 0),
      0,
    );
    let cursor = legendRightAnchor - rowTotal;
    for (const itemIdx of row) {
      placements[itemIdx] = { x: cursor, y: rowIdx * legendRowHeight };
      cursor += itemWidths[itemIdx] + legendItemGap;
    }
  });

  // Headlines fijos a la izquierda (y=34/56/74) y legend a la derecha.
  // mt acomoda lo más alto de los dos bloques con padding.
  const legendBlockHeight = rowsNeeded * legendRowHeight;
  const legendVerticalPad = 18;
  const headlineBlockTopY = 34;
  const headlineBlockBottom = headlineBlockTopY + 40 + 14;
  const mt = Math.max(
    legendBlockHeight + legendVerticalPad * 2,
    headlineBlockBottom,
  );
  // Centra verticalmente la leyenda en el margen superior.
  const legendY = Math.round((mt - legendBlockHeight) / 2);
  const ph = H - mt - mb;

  const legend = { x: 0, y: legendY };

  return {
    W,
    H,
    ml,
    mr,
    mt,
    mb,
    pw,
    ph,
    narrow,
    legend,
    legendPlacements: placements,
    legendRows: rowsNeeded,
    headlineBlockTopY,
  };
}

function niceYTicks(minV: number, maxV: number, tickCount: number): number[] {
  if (!Number.isFinite(minV) || !Number.isFinite(maxV)) return [0];
  if (minV === maxV) {
    const pad = Math.abs(minV) < 1 ? 1 : Math.abs(minV) * 0.05;
    return niceYTicks(minV - pad, maxV + pad, tickCount);
  }
  const span = maxV - minV;
  const rough = span / Math.max(2, tickCount - 1);
  const exp = Math.floor(Math.log10(rough));
  const frac = rough / 10 ** exp;
  const niceFrac = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 5 ? 5 : 10;
  const step = niceFrac * 10 ** exp;
  const lo = Math.floor(minV / step) * step;
  const hi = Math.ceil(maxV / step) * step;
  const out: number[] = [];
  for (let x = lo; x <= hi + step * 0.01; x += step) {
    out.push(Math.round(x / step) * step);
  }
  const dedup = [...new Set(out.map((v) => Number(v.toPrecision(12))))];
  return dedup.length > 8 ? dedup.filter((_, i) => i % 2 === 0) : dedup;
}

/** Porcentaje en pantalla: un decimal y « %» detrás (no usar como sufijo duplicado). */
function formatPercentDisplay(n: number): string {
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(n)} %`;
}

/** Igual que formatPercentDisplay pero con «+» explícito en positivos (retornos). */
function formatPercentDisplaySigned(n: number): string {
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
    signDisplay: "exceptZero",
  }).format(n)} %`;
}

/** Tasas en % ya como número API (ej. TAE 3.25 → «3,3 %»). */
function formatPercentAmount(s: string): string {
  const n = parseDisplayDecimal(s);
  if (n === null) return s;
  return formatPercentDisplay(n);
}

/** Retorno acumulado (valor/compra − 1); no es TAE. */
function assetImplicitTotalReturnLabel(
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

function liabilityDerivedPrincipalPreview(
  amountStr: string,
  freq: LiabilityPaymentFreq,
  endYmd: string,
  installationCalendarTz: string,
  currencyIso: string,
): string | null {
  if (!freq || !endYmd.trim()) return null;
  const startYmd = todayYmdInTimeZone(installationCalendarTz);
  const n = paymentIntervalCountUtc(freq, startYmd, endYmd.trim());
  if (n === null || n <= 0) return null;
  const amount = Number(amountStr.trim().replace(",", "."));
  if (!Number.isFinite(amount) || amount <= 0) return null;
  const total = amount * n;
  return formatCurrencyNumber(total, currencyIso);
}

function formatDebtToAssetsPct(ratio: string | null | undefined): string {
  if (ratio == null || ratio === "") return METRIC_DASH;
  const r = Number(String(ratio).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return formatPercentDisplay(r * 100);
}

/** Decimal fraction (e.g. 0.25) shown as percent */
function formatFractionAsPercent(ratio: string | null | undefined): string {
  if (ratio == null || ratio === "") return METRIC_DASH;
  const r = Number(String(ratio).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return formatPercentDisplay(r * 100);
}

/** Ocultar KPI de importe cuando falta dato o es exactamente 0. */
function isZeroMoneyMetric(s: string | null | undefined): boolean {
  if (s == null || String(s).trim() === "") return true;
  const n = parseDisplayDecimal(String(s));
  return n === null || n === 0;
}

/** Ocultar KPI de fracción (tasa, ratio 0–1) cuando falta o es 0. */
function isZeroFractionMetric(ratio: string | null | undefined): boolean {
  if (ratio == null || String(ratio).trim() === "") return true;
  const r = Number(String(ratio).replace(",", "."));
  return !Number.isFinite(r) || r === 0;
}

function formatMonthsRough(s: string | null | undefined): string {
  if (s == null || s === "") return METRIC_DASH;
  const r = Number(String(s).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return `${r.toLocaleString(DISPLAY_NUMBER_LOCALE, {
    maximumFractionDigits: 1,
  })} meses`;
}

function breakdownPercentOfTotal(part: string, whole: string): number | null {
  const p = parseDisplayDecimal(part);
  const w = parseDisplayDecimal(whole);
  if (p === null || w === null || w <= 0) return null;
  return Math.min(100, (p / w) * 100);
}

function formatBreakdownPct(part: string, whole: string): string {
  const pct = breakdownPercentOfTotal(part, whole);
  if (pct === null) return METRIC_DASH;
  return formatPercentDisplay(pct);
}

function budgetCategoryMap(
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

function sortBudgetEntriesMacStyle(
  entries: BudgetEntryApiRow[],
  categoryById: Map<string, CategoryRow>,
): BudgetEntryApiRow[] {
  const monthlyEq = (e: BudgetEntryApiRow) =>
    Number(String(e.amount).replace(",", "."));
  const byCatTotal = new Map<string, number>();
  for (const e of entries) {
    const k = e.category_id;
    byCatTotal.set(k, (byCatTotal.get(k) ?? 0) + monthlyEq(e));
  }
  const catName = (id: string) => categoryById.get(id)?.name ?? id;
  return [...entries].sort((a, b) => {
    const ta = byCatTotal.get(a.category_id) ?? 0;
    const tb = byCatTotal.get(b.category_id) ?? 0;
    if (tb !== ta) return tb - ta;
    const cmp = catName(a.category_id).localeCompare(
      catName(b.category_id),
      "es",
    );
    if (cmp !== 0) return cmp;
    const ea = monthlyEq(a);
    const eb = monthlyEq(b);
    if (eb !== ea) return eb - ea;
    return a.id.localeCompare(b.id, "es");
  });
}

function ModalFormError({
  message,
}: {
  message: string | null | undefined;
}) {
  if (!message) return null;
  return (
    <p className="error compact modal-form-error" role="alert">
      {message}
    </p>
  );
}

/** Tooltip nativo al pasar el cursor o foco. */
function InlineHint({ title }: { title: string }) {
  return (
    <span
      className="inline-hint-icon"
      title={title}
      role="img"
      aria-label={title}
    >
      i
    </span>
  );
}

function Modal({
  title,
  open,
  onClose,
  children,
  wide = false,
}: {
  title: string;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        className={
          wide ? "modal-dialog modal-dialog--wide card-elevated" : "modal-dialog card-elevated"
        }
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="modal-header">
          <h3 id={titleId} className="modal-title">
            {title}
          </h3>
          <button
            type="button"
            className="btn ghost modal-close"
            aria-label="Cerrar"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

async function errorMessageFromResponse(res: Response): Promise<string> {
  const ct = res.headers.get("content-type") ?? "";
  if (ct.includes("application/json")) {
    try {
      const body = (await res.json()) as { message?: string };
      if (typeof body.message === "string" && body.message.length > 0) {
        return body.message;
      }
    } catch {
      /* ignore */
    }
  }
  return `HTTP ${res.status}`;
}

export default function App() {
  const ledgerScopeSelectId = useId();
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);

  const [sessionBusy, setSessionBusy] = useState(true);
  const [user, setUser] = useState<UserResponse | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);

  const [ledgerPersonScope, setLedgerPersonScopeInner] =
    useState<LedgerPersonScope>(() => {
      if (typeof window === "undefined") return "household";
      try {
        return window.localStorage.getItem(LEDGER_PERSON_SCOPE_STORAGE_KEY) ===
          "mine"
          ? "mine"
          : "household";
      } catch {
        return "household";
      }
    });

  const setLedgerPersonScope = (next: LedgerPersonScope) => {
    setLedgerPersonScopeInner(next);
    try {
      window.localStorage.setItem(
        LEDGER_PERSON_SCOPE_STORAGE_KEY,
        next === "mine" ? "mine" : "household",
      );
    } catch {
      /* ignore */
    }
  };

  const [authMode, setAuthMode] = useState<"login" | "register">("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [registerBirthDate, setRegisterBirthDate] = useState("");
  const [authBusy, setAuthBusy] = useState(false);

  const [installation, setInstallation] = useState<InstallationAccess | null>(
    null,
  );
  const [installationError, setInstallationError] = useState<string | null>(
    null,
  );
  const [installationBusy, setInstallationBusy] = useState(false);
  const [installationGate, setInstallationGate] =
    useState<InstallationGate>("loading");
  const [setupCurrency, setSetupCurrency] = useState<"EUR" | "USD" | "GBP">(
    "EUR",
  );
  const [setupCalendarTz, setSetupCalendarTz] = useState("UTC");
  const [calendarTzDraft, setCalendarTzDraft] = useState("UTC");
  const [calendarTzSaving, setCalendarTzSaving] = useState(false);
  const [projectionInflationPctDraft, setProjectionInflationPctDraft] =
    useState("");
  const [showAgeModeDraft, setShowAgeModeDraft] = useState<"dates" | "ages">(
    "dates",
  );
  const [installationProjectionSaving, setInstallationProjectionSaving] =
    useState(false);

  const [pendingUsers, setPendingUsers] = useState<UserResponse[]>([]);
  const [pendingUsersBusy, setPendingUsersBusy] = useState(false);
  const [pendingUsersError, setPendingUsersError] = useState<string | null>(
    null,
  );
  const [approveRoles, setApproveRoles] = useState<
    Record<string, "member" | "viewer">
  >({});
  const [approveBusy, setApproveBusy] = useState(false);

  const [categories, setCategories] = useState<CategoryRow[]>([]);
  const [categoriesBusy, setCategoriesBusy] = useState(false);
  const [categoriesError, setCategoriesError] = useState<string | null>(null);
  const [categoryScopeFilter, setCategoryScopeFilter] = useState<
    CategoryScope | "all"
  >("all");
  const [newCatScope, setNewCatScope] = useState<CategoryScope>("asset");
  const [newCatName, setNewCatName] = useState("");
  const [categorySaving, setCategorySaving] = useState(false);
  const [categoryModalOpen, setCategoryModalOpen] = useState(false);
  const [categoryRenameModalOpen, setCategoryRenameModalOpen] = useState(false);
  const [editingCategoryId, setEditingCategoryId] = useState<string | null>(
    null,
  );
  const [editCategoryName, setEditCategoryName] = useState("");
  const [categoryDeleteModalOpen, setCategoryDeleteModalOpen] = useState(false);
  const [categoryDeletePending, setCategoryDeletePending] =
    useState<CategoryRow | null>(null);
  const [categoryRemapToId, setCategoryRemapToId] = useState("");

  const [assets, setAssets] = useState<AssetApiRow[]>([]);
  const [assetsBusy, setAssetsBusy] = useState(false);
  const [assetsError, setAssetsError] = useState<string | null>(null);
  const [assetCategories, setAssetCategories] = useState<CategoryRow[]>([]);
  const [allocationRules, setAllocationRules] = useState<AllocationRuleApiRow[]>([]);
  const [allocationRulesBusy, setAllocationRulesBusy] = useState(false);
  const [allocationRulesError, setAllocationRulesError] = useState<string | null>(null);
  const [allocationPanelOpen, setAllocationPanelOpen] = useState(false);
  const [ruleModalOpen, setRuleModalOpen] = useState(false);
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [ruleFormTargetAsset, setRuleFormTargetAsset] = useState("");
  const [ruleFormKind, setRuleFormKind] = useState<AllocationRuleKind>("remainder");
  const [ruleFormAmount, setRuleFormAmount] = useState("");
  const [ruleFormCapKind, setRuleFormCapKind] = useState<"none" | AllocationRuleCapKind>("none");
  const [ruleFormCapValue, setRuleFormCapValue] = useState("");
  const [ruleSaving, setRuleSaving] = useState(false);
  const [assetFormCategoryId, setAssetFormCategoryId] = useState("");
  const [assetFormName, setAssetFormName] = useState("");
  const [assetFormValue, setAssetFormValue] = useState("");
  const [assetFormPurchase, setAssetFormPurchase] = useState("");
  const [assetFormLiquid, setAssetFormLiquid] = useState(true);
  const [assetFormExpectedReturn, setAssetFormExpectedReturn] = useState("");
  const [assetFormNotes, setAssetFormNotes] = useState("");
  const [editingAssetId, setEditingAssetId] = useState<string | null>(null);
  const [assetSaving, setAssetSaving] = useState(false);
  const [assetModalOpen, setAssetModalOpen] = useState(false);

  const [liabilities, setLiabilities] = useState<LiabilityApiRow[]>([]);
  const [liabilitiesBusy, setLiabilitiesBusy] = useState(false);
  const [liabilitiesError, setLiabilitiesError] = useState<string | null>(
    null,
  );
  const [liabilityCategories, setLiabilityCategories] = useState<
    CategoryRow[]
  >([]);
  const [liabilityFormCategoryId, setLiabilityFormCategoryId] = useState("");
  const [liabilityFormLabel, setLiabilityFormLabel] = useState("");
  const [liabilityFormTypeTag, setLiabilityFormTypeTag] = useState("");
  const [liabilityFormPrincipal, setLiabilityFormPrincipal] = useState("");
  const [liabilityFormApr, setLiabilityFormApr] = useState("");
  const [liabilityFormPaymentAmount, setLiabilityFormPaymentAmount] =
    useState("");
  const [liabilityFormPaymentFrequency, setLiabilityFormPaymentFrequency] =
    useState<LiabilityPaymentFreq>("");
  const [liabilityFormPaymentEnd, setLiabilityFormPaymentEnd] = useState("");
  const [liabilityFormNotes, setLiabilityFormNotes] = useState("");
  const [liabilityFormDerivePrincipal, setLiabilityFormDerivePrincipal] =
    useState(false);
  const [editingLiabilityId, setEditingLiabilityId] = useState<string | null>(
    null,
  );
  const [liabilitySaving, setLiabilitySaving] = useState(false);
  const [liabilityModalOpen, setLiabilityModalOpen] = useState(false);

  const [budgetSnapshot, setBudgetSnapshot] = useState<BudgetSnapshotApi | null>(
    null,
  );
  const [budgetIncomeCategories, setBudgetIncomeCategories] = useState<
    CategoryRow[]
  >([]);
  const [budgetExpenseCategories, setBudgetExpenseCategories] = useState<
    CategoryRow[]
  >([]);
  const [budgetLiabilityCategories, setBudgetLiabilityCategories] = useState<
    CategoryRow[]
  >([]);
  const [budgetLoading, setBudgetLoading] = useState(false);
  const [budgetError, setBudgetError] = useState<string | null>(null);
  const [budgetSaving, setBudgetSaving] = useState(false);
  const [budgetModalOpen, setBudgetModalOpen] = useState(false);
  const [editingBudgetEntryId, setEditingBudgetEntryId] = useState<
    string | null
  >(null);
  const [budgetFormScope, setBudgetFormScope] =
    useState<BudgetScopeToggle>("expense");
  const [budgetFormCategoryId, setBudgetFormCategoryId] = useState("");
  const [budgetFormAmount, setBudgetFormAmount] = useState("");
  const [budgetFormNotes, setBudgetFormNotes] = useState("");
  const [budgetFormPersistsAfterRetirement, setBudgetFormPersistsAfterRetirement] = useState(false);
  const [budgetFormExpenseEndType, setBudgetFormExpenseEndType] = useState<"never" | "retirement" | "date">("never");
  const [budgetFormExpenseEndDate, setBudgetFormExpenseEndDate] = useState("");

  const [planningFlows, setPlanningFlows] = useState<PlanningFlowApiRow[]>([]);
  const [planningIncomeCategories, setPlanningIncomeCategories] = useState<
    CategoryRow[]
  >([]);
  const [planningExpenseCategories, setPlanningExpenseCategories] = useState<
    CategoryRow[]
  >([]);
  const [planningLoading, setPlanningLoading] = useState(false);
  const [planningError, setPlanningError] = useState<string | null>(null);
  const [planningSaving, setPlanningSaving] = useState(false);
  const [planningModalOpen, setPlanningModalOpen] = useState(false);
  const [editingPlanningFlowId, setEditingPlanningFlowId] = useState<
    string | null
  >(null);
  const [planningFormScope, setPlanningFormScope] =
    useState<BudgetScopeToggle>("expense");
  const [planningFormCategoryId, setPlanningFormCategoryId] = useState("");
  const [planningFormTitle, setPlanningFormTitle] = useState("");
  const [planningFormAmount, setPlanningFormAmount] = useState("");
  const [planningFormDue, setPlanningFormDue] = useState("");
  const [planningFormNotes, setPlanningFormNotes] = useState("");
  const [planningFormShowInChart, setPlanningFormShowInChart] = useState(false);

  const [summary, setSummary] = useState<SummaryResponse | null>(null);
  const [summaryBusy, setSummaryBusy] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);

  const [projectionSeries, setProjectionSeries] =
    useState<ProjectionSeriesApi | null>(null);
  const [projectionBusy, setProjectionBusy] = useState(false);
  const [projectionError, setProjectionError] = useState<string | null>(null);

  const [retirementBudgetSnapshot, setRetirementBudgetSnapshot] =
    useState<BudgetSnapshotApi | null>(null);
  const [retirementBusy, setRetirementBusy] = useState(false);
  const [retirementError, setRetirementError] = useState<string | null>(null);

  const [userProfileOpen, setUserProfileOpen] = useState(false);
  const [userBirthDraft, setUserBirthDraft] = useState("");
  const [userProfileSaving, setUserProfileSaving] = useState(false);
  const [userProfileError, setUserProfileError] = useState<string | null>(null);
  const [ffbackupExportModalOpen, setFfbackupExportModalOpen] = useState(false);
  const [ffbackupExportPassword, setFfbackupExportPassword] = useState("");
  const [ffbackupExportBusy, setFfbackupExportBusy] = useState(false);
  const [ffbackupExportError, setFfbackupExportError] = useState<string | null>(
    null,
  );

  const [ffbackupImportModalOpen, setFfbackupImportModalOpen] = useState(false);
  const [ffbackupImportFile, setFfbackupImportFile] = useState<File | null>(
    null,
  );
  const [ffbackupImportPassword, setFfbackupImportPassword] = useState("");
  const [ffbackupImportBusy, setFfbackupImportBusy] = useState(false);
  const [ffbackupImportError, setFfbackupImportError] = useState<string | null>(
    null,
  );
  const [ffbackupImportPreview, setFfbackupImportPreview] =
    useState<FfbackupImportPreviewResponse | null>(null);
  const [ffbackupImportDone, setFfbackupImportDone] = useState<string | null>(
    null,
  );

  const [pathname, navigate] = useAppPathNavigation();
  const activeTab = useMemo(
    () => tabFromPathname(pathname) ?? "summary",
    [pathname],
  );

  const hasMembership = installation !== null;
  const isInstallationOwner = installation?.role === "owner";

  const visibleSettingsSubTabs = useMemo<SettingsSubTabId[]>(() => {
    const out: SettingsSubTabId[] = [];
    if (isInstallationOwner) out.push("access");
    if (hasMembership) {
      out.push("calendar", "projection", "retirement", "categories");
    }
    out.push("data");
    return out;
  }, [isInstallationOwner, hasMembership]);

  const defaultSettingsSubTab: SettingsSubTabId =
    visibleSettingsSubTabs[0] ?? "data";

  const urlSettingsSubTab = useMemo(
    () => settingsSubTabFromPathname(pathname),
    [pathname],
  );
  const settingsSubTab: SettingsSubTabId =
    urlSettingsSubTab && visibleSettingsSubTabs.includes(urlSettingsSubTab)
      ? urlSettingsSubTab
      : defaultSettingsSubTab;
  const navigateSettingsSubTab = useCallback(
    (id: SettingsSubTabId) => {
      navigate(settingsSubTabPath(id));
    },
    [navigate],
  );

  useLayoutEffect(() => {
    if (!user) return;
    const p = normalizeAppPath(pathname);
    if (p === "/") {
      navigate("/resumen", true);
      return;
    }
    if (tabFromPathname(pathname) === null) {
      navigate("/resumen", true);
      return;
    }
    if (activeTab === "settings") {
      const sub = settingsSubTabFromPathname(pathname);
      if (!sub || !visibleSettingsSubTabs.includes(sub)) {
        navigate(settingsSubTabPath(defaultSettingsSubTab), true);
      }
    }
  }, [
    user,
    pathname,
    navigate,
    activeTab,
    visibleSettingsSubTabs,
    defaultSettingsSubTab,
  ]);

  const refreshSession = useCallback(async () => {
    setSessionBusy(true);
    setSessionError(null);
    try {
      const res = await fetch("/v1/auth/me", defaultFetchInit);
      if (res.status === 401) {
        setUser(null);
        return;
      }
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const body = (await res.json()) as UserResponse;
      setUser(body);
    } catch (e: unknown) {
      setUser(null);
      setSessionError(e instanceof Error ? e.message : String(e));
    } finally {
      setSessionBusy(false);
    }
  }, []);

  const loadInstallation = useCallback(async (opts?: { preserveGate?: boolean }) => {
    const preserveGate = opts?.preserveGate ?? false;
    setInstallationBusy(true);
    setInstallationError(null);
    if (!preserveGate) {
      setInstallationGate("loading");
    }
    try {
      const res = await fetch("/v1/installation/session-context", defaultFetchInit);
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const ctx = (await res.json()) as InstallationSessionContext;
      if (ctx.access) {
        setInstallation(ctx.access);
        setInstallationGate("member");
      } else if (!ctx.installation_initialized) {
        setInstallation(null);
        setInstallationGate("bootstrap");
      } else {
        setInstallation(null);
        setInstallationGate("pending");
      }
    } catch (e: unknown) {
      setInstallation(null);
      setInstallationError(e instanceof Error ? e.message : String(e));
      setInstallationGate("fetch_failed");
    } finally {
      setInstallationBusy(false);
    }
  }, []);

  const loadAssetsPage = useCallback(async () => {
    setAssetsBusy(true);
    setAssetsError(null);
    try {
      const [catRes, astRes] = await Promise.all([
        fetch("/v1/categories?scope=asset", defaultFetchInit),
        fetch(`/v1/assets${ledgerViewQs(ledgerPersonScope)}`, defaultFetchInit),
      ]);
      if (catRes.status === 403 || catRes.status === 404) {
        setAssetCategories([]);
      } else if (!catRes.ok) {
        throw new Error(await errorMessageFromResponse(catRes));
      } else {
        setAssetCategories((await catRes.json()) as CategoryRow[]);
      }
      if (astRes.status === 403 || astRes.status === 404) {
        setAssets([]);
      } else if (!astRes.ok) {
        throw new Error(await errorMessageFromResponse(astRes));
      } else {
        setAssets((await astRes.json()) as AssetApiRow[]);
      }
    } catch (e: unknown) {
      setAssets([]);
      setAssetCategories([]);
      setAssetsError(e instanceof Error ? e.message : String(e));
    } finally {
      setAssetsBusy(false);
    }
  }, [ledgerPersonScope]);

  const loadAllocationRules = useCallback(async () => {
    setAllocationRulesBusy(true);
    setAllocationRulesError(null);
    try {
      const res = await fetch(
        `/v1/allocation-rules${ledgerViewQs(ledgerPersonScope)}`,
        defaultFetchInit,
      );
      if (res.status === 403 || res.status === 404) {
        setAllocationRules([]);
      } else if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      } else {
        setAllocationRules((await res.json()) as AllocationRuleApiRow[]);
      }
    } catch (e: unknown) {
      setAllocationRules([]);
      setAllocationRulesError(e instanceof Error ? e.message : String(e));
    } finally {
      setAllocationRulesBusy(false);
    }
  }, [ledgerPersonScope]);

  function resetRuleForm() {
    setEditingRuleId(null);
    setRuleFormTargetAsset(assets[0]?.id ?? "");
    setRuleFormKind("remainder");
    setRuleFormAmount("");
    setRuleFormCapKind("none");
    setRuleFormCapValue("");
  }

  function beginEditRule(r: AllocationRuleApiRow) {
    setEditingRuleId(r.id);
    setRuleFormTargetAsset(r.target_asset_id);
    setRuleFormKind(r.kind);
    setRuleFormAmount(
      r.amount != null ? formatEditableDecimalString(String(r.amount)) : "",
    );
    if (r.cap_kind && r.cap_value != null) {
      setRuleFormCapKind(r.cap_kind);
      setRuleFormCapValue(formatEditableDecimalString(String(r.cap_value)));
    } else {
      setRuleFormCapKind("none");
      setRuleFormCapValue("");
    }
  }

  async function submitRuleForm(ev: FormEvent) {
    ev.preventDefault();
    if (!ruleFormTargetAsset) return;
    setRuleSaving(true);
    setAllocationRulesError(null);
    try {
      type RulePayload = {
        target_asset_id?: string;
        kind?: AllocationRuleKind;
        amount?: string | null;
        cap_kind?: AllocationRuleCapKind | null;
        cap_value?: string | null;
        cap?: { kind: AllocationRuleCapKind; value: string } | null;
      };
      const base: RulePayload = {};
      const capRaw = ruleFormCapValue.trim().replace(",", ".");
      const amountRaw = ruleFormAmount.trim().replace(",", ".");

      if (editingRuleId) {
        base.target_asset_id = ruleFormTargetAsset;
        base.kind = ruleFormKind;
        base.amount =
          ruleFormKind === "remainder" ? null : (amountRaw === "" ? "0" : amountRaw);
        base.cap =
          ruleFormCapKind === "none"
            ? null
            : { kind: ruleFormCapKind, value: capRaw === "" ? "0" : capRaw };
      } else {
        base.target_asset_id = ruleFormTargetAsset;
        base.kind = ruleFormKind;
        if (ruleFormKind !== "remainder") {
          base.amount = amountRaw === "" ? "0" : amountRaw;
        }
        if (ruleFormCapKind !== "none") {
          base.cap_kind = ruleFormCapKind;
          base.cap_value = capRaw === "" ? "0" : capRaw;
        }
      }

      const url = editingRuleId
        ? `/v1/allocation-rules/${encodeURIComponent(editingRuleId)}`
        : "/v1/allocation-rules";
      const method = editingRuleId ? "PATCH" : "POST";
      const res = await fetch(url, {
        ...defaultFetchInit,
        method,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(base),
      });
      if (!res.ok) throw new Error(await errorMessageFromResponse(res));
      setRuleModalOpen(false);
      resetRuleForm();
      await loadAllocationRules();
    } catch (e: unknown) {
      setAllocationRulesError(e instanceof Error ? e.message : String(e));
    } finally {
      setRuleSaving(false);
    }
  }

  async function deleteRule(id: string) {
    if (!confirm("¿Eliminar esta regla de asignación?")) return;
    setAllocationRulesError(null);
    try {
      const res = await fetch(
        `/v1/allocation-rules/${encodeURIComponent(id)}`,
        { ...defaultFetchInit, method: "DELETE" },
      );
      if (!res.ok) throw new Error(await errorMessageFromResponse(res));
      await loadAllocationRules();
    } catch (e: unknown) {
      setAllocationRulesError(e instanceof Error ? e.message : String(e));
    }
  }

  async function moveRule(id: string, direction: "up" | "down") {
    const idx = allocationRules.findIndex((r) => r.id === id);
    if (idx < 0) return;
    const swapWith = direction === "up" ? idx - 1 : idx + 1;
    if (swapWith < 0 || swapWith >= allocationRules.length) return;
    const reordered = [...allocationRules];
    const [moved] = reordered.splice(idx, 1);
    reordered.splice(swapWith, 0, moved);
    setAllocationRulesError(null);
    try {
      const res = await fetch(
        `/v1/allocation-rules/reorder${ledgerViewQs(ledgerPersonScope)}`,
        {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ ids: reordered.map((r) => r.id) }),
        },
      );
      if (!res.ok) throw new Error(await errorMessageFromResponse(res));
      await loadAllocationRules();
    } catch (e: unknown) {
      setAllocationRulesError(e instanceof Error ? e.message : String(e));
    }
  }

  const loadLiabilitiesPage = useCallback(async () => {
    setLiabilitiesBusy(true);
    setLiabilitiesError(null);
    try {
      const [catRes, libRes] = await Promise.all([
        fetch("/v1/categories?scope=liability", defaultFetchInit),
        fetch(
          `/v1/liabilities${ledgerViewQs(ledgerPersonScope)}`,
          defaultFetchInit,
        ),
      ]);
      if (catRes.status === 403 || catRes.status === 404) {
        setLiabilityCategories([]);
      } else if (!catRes.ok) {
        throw new Error(await errorMessageFromResponse(catRes));
      } else {
        setLiabilityCategories((await catRes.json()) as CategoryRow[]);
      }
      if (libRes.status === 403 || libRes.status === 404) {
        setLiabilities([]);
      } else if (!libRes.ok) {
        throw new Error(await errorMessageFromResponse(libRes));
      } else {
        setLiabilities((await libRes.json()) as LiabilityApiRow[]);
      }
    } catch (e: unknown) {
      setLiabilities([]);
      setLiabilityCategories([]);
      setLiabilitiesError(e instanceof Error ? e.message : String(e));
    } finally {
      setLiabilitiesBusy(false);
    }
  }, [ledgerPersonScope]);

  const loadBudgetPage = useCallback(async () => {
    setBudgetLoading(true);
    setBudgetError(null);
    try {
      const [budRes, incRes, expRes, libCatRes] = await Promise.all([
        fetch(`/v1/budget${ledgerViewQs(ledgerPersonScope)}`, defaultFetchInit),
        fetch("/v1/categories?scope=income", defaultFetchInit),
        fetch("/v1/categories?scope=expense", defaultFetchInit),
        fetch("/v1/categories?scope=liability", defaultFetchInit),
      ]);

      if (budRes.status === 403 || budRes.status === 404) {
        setBudgetSnapshot(null);
      } else if (!budRes.ok) {
        throw new Error(await errorMessageFromResponse(budRes));
      } else {
        const raw = (await budRes.json()) as BudgetSnapshotApi;
        setBudgetSnapshot({
          ...raw,
          entries: Array.isArray(raw.entries) ? raw.entries : [],
          derived_from_liabilities: Array.isArray(raw.derived_from_liabilities)
            ? raw.derived_from_liabilities
            : [],
        });
      }

      if (incRes.status === 403 || incRes.status === 404) {
        setBudgetIncomeCategories([]);
      } else if (!incRes.ok) {
        throw new Error(await errorMessageFromResponse(incRes));
      } else {
        setBudgetIncomeCategories((await incRes.json()) as CategoryRow[]);
      }

      if (expRes.status === 403 || expRes.status === 404) {
        setBudgetExpenseCategories([]);
      } else if (!expRes.ok) {
        throw new Error(await errorMessageFromResponse(expRes));
      } else {
        setBudgetExpenseCategories((await expRes.json()) as CategoryRow[]);
      }

      if (libCatRes.status === 403 || libCatRes.status === 404) {
        setBudgetLiabilityCategories([]);
      } else if (!libCatRes.ok) {
        throw new Error(await errorMessageFromResponse(libCatRes));
      } else {
        setBudgetLiabilityCategories((await libCatRes.json()) as CategoryRow[]);
      }
    } catch (e: unknown) {
      setBudgetSnapshot(null);
      setBudgetIncomeCategories([]);
      setBudgetExpenseCategories([]);
      setBudgetLiabilityCategories([]);
      setBudgetError(e instanceof Error ? e.message : String(e));
    } finally {
      setBudgetLoading(false);
    }
  }, [ledgerPersonScope]);

  const loadPlanningPage = useCallback(async () => {
    setPlanningLoading(true);
    setPlanningError(null);
    try {
      const [flowsRes, incRes, expRes] = await Promise.all([
        fetch(
          `/v1/planning/flows${ledgerViewQs(ledgerPersonScope)}`,
          defaultFetchInit,
        ),
        fetch("/v1/categories?scope=income", defaultFetchInit),
        fetch("/v1/categories?scope=expense", defaultFetchInit),
      ]);

      if (flowsRes.status === 403 || flowsRes.status === 404) {
        setPlanningFlows([]);
      } else if (!flowsRes.ok) {
        throw new Error(await errorMessageFromResponse(flowsRes));
      } else {
        setPlanningFlows((await flowsRes.json()) as PlanningFlowApiRow[]);
      }

      if (incRes.status === 403 || incRes.status === 404) {
        setPlanningIncomeCategories([]);
      } else if (!incRes.ok) {
        throw new Error(await errorMessageFromResponse(incRes));
      } else {
        setPlanningIncomeCategories((await incRes.json()) as CategoryRow[]);
      }

      if (expRes.status === 403 || expRes.status === 404) {
        setPlanningExpenseCategories([]);
      } else if (!expRes.ok) {
        throw new Error(await errorMessageFromResponse(expRes));
      } else {
        setPlanningExpenseCategories((await expRes.json()) as CategoryRow[]);
      }
    } catch (e: unknown) {
      setPlanningFlows([]);
      setPlanningIncomeCategories([]);
      setPlanningExpenseCategories([]);
      setPlanningError(e instanceof Error ? e.message : String(e));
    } finally {
      setPlanningLoading(false);
    }
  }, [ledgerPersonScope]);

  const loadSummaryPage = useCallback(async () => {
    setSummaryBusy(true);
    setSummaryError(null);
    try {
      const res = await fetch(
        `/v1/summary${ledgerViewQs(ledgerPersonScope)}`,
        defaultFetchInit,
      );
      if (res.status === 403 || res.status === 404) {
        setSummary(null);
        return;
      }
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setSummary((await res.json()) as SummaryResponse);
    } catch (e: unknown) {
      setSummary(null);
      setSummaryError(e instanceof Error ? e.message : String(e));
    } finally {
      setSummaryBusy(false);
    }
  }, [ledgerPersonScope]);

  const loadProjectionSeriesPage = useCallback(async () => {
    setProjectionBusy(true);
    setProjectionError(null);
    try {
      const qs =
        ledgerPersonScope === "mine" ? "?view=mine" : "";
      const res = await fetch(
        `/v1/projection/series${qs}`,
        defaultFetchInit,
      );
      if (res.status === 403 || res.status === 404) {
        setProjectionSeries(null);
        return;
      }
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setProjectionSeries((await res.json()) as ProjectionSeriesApi);
    } catch (e: unknown) {
      setProjectionSeries(null);
      setProjectionError(e instanceof Error ? e.message : String(e));
    } finally {
      setProjectionBusy(false);
    }
  }, [ledgerPersonScope]);

  const loadRetirementPage = useCallback(
    async (opts?: { silent?: boolean }) => {
      const silent = opts?.silent === true;
      if (!silent) {
        setRetirementBusy(true);
        setRetirementError(null);
        setProjectionError(null);
      }
      try {
        const qs =
          ledgerPersonScope === "mine" ? "?view=mine" : "";
        const [budRes, projRes] = await Promise.all([
          fetch(`/v1/budget${qs}`, defaultFetchInit),
          fetch(`/v1/projection/series${qs}`, defaultFetchInit),
        ]);
        if (budRes.status === 403 || budRes.status === 404) {
          setRetirementBudgetSnapshot(null);
        } else if (!budRes.ok) {
          throw new Error(await errorMessageFromResponse(budRes));
        } else {
          const raw = (await budRes.json()) as BudgetSnapshotApi;
          setRetirementBudgetSnapshot({
            ...raw,
            entries: Array.isArray(raw.entries) ? raw.entries : [],
            derived_from_liabilities: Array.isArray(raw.derived_from_liabilities)
              ? raw.derived_from_liabilities
              : [],
          });
        }
        if (projRes.status === 403 || projRes.status === 404) {
          setProjectionSeries(null);
        } else if (!projRes.ok) {
          throw new Error(await errorMessageFromResponse(projRes));
        } else {
          setProjectionSeries((await projRes.json()) as ProjectionSeriesApi);
        }
      } catch (e: unknown) {
        if (!silent) {
          setRetirementBudgetSnapshot(null);
          setProjectionSeries(null);
          setRetirementError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (!silent) {
          setRetirementBusy(false);
        }
      }
    },
    [ledgerPersonScope],
  );

  const loadCategories = useCallback(async () => {
    setCategoriesBusy(true);
    setCategoriesError(null);
    try {
      const res = await fetch("/v1/categories", defaultFetchInit);
      if (res.status === 403 || res.status === 404) {
        setCategories([]);
        return;
      }
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const list = (await res.json()) as CategoryRow[];
      setCategories(list);
    } catch (e: unknown) {
      setCategories([]);
      setCategoriesError(e instanceof Error ? e.message : String(e));
    } finally {
      setCategoriesBusy(false);
    }
  }, []);

  const loadPendingUsers = useCallback(async () => {
    setPendingUsersBusy(true);
    setPendingUsersError(null);
    try {
      const res = await fetch("/v1/installation/pending-users", defaultFetchInit);
      if (res.status === 403 || res.status === 404) {
        setPendingUsers([]);
        return;
      }
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const list = (await res.json()) as UserResponse[];
      setPendingUsers(list);
    } catch (e: unknown) {
      setPendingUsers([]);
      setPendingUsersError(e instanceof Error ? e.message : String(e));
    } finally {
      setPendingUsersBusy(false);
    }
  }, []);

  useEffect(() => {
    setApproveRoles((prev) => {
      const next = { ...prev };
      for (const u of pendingUsers) {
        if (next[u.id] === undefined) {
          next[u.id] = "member";
        }
      }
      return next;
    });
  }, [pendingUsers]);

  useEffect(() => {
    let cancelled = false;
    fetch("/v1/health")
      .then(async (res) => {
        if (!res.ok) {
          throw new Error(`HTTP ${res.status}`);
        }
        return res.json() as Promise<HealthResponse>;
      })
      .then((json) => {
        if (!cancelled) {
          setHealth(json);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setHealthError(e instanceof Error ? e.message : String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    void refreshSession();
  }, [refreshSession]);

  useEffect(() => {
    if (user) {
      void loadInstallation();
    } else {
      setInstallation(null);
      setInstallationGate("loading");
      setInstallationError(null);
      setPendingUsers([]);
      setPendingUsersError(null);
      setSummary(null);
      setSummaryError(null);
    }
  }, [user, loadInstallation]);

  useEffect(() => {
    const tz = installation?.installation.calendar_tz;
    if (typeof tz === "string" && tz.trim().length >= 3) {
      setCalendarTzDraft(tz.trim());
    }
  }, [installation?.installation.calendar_tz]);

  useEffect(() => {
    if (!installation) {
      setProjectionInflationPctDraft("");
      setShowAgeModeDraft("dates");
      return;
    }
    const inst = installation.installation;
    setProjectionInflationPctDraft(
      formatEditableDecimalString(inst.annual_inflation_assumption_percent),
    );
    setShowAgeModeDraft(inst.show_age_mode === "ages" ? "ages" : "dates");
  }, [installation]);

  useEffect(() => {
    if (!user || installation?.role !== "owner") {
      setPendingUsers([]);
      setPendingUsersError(null);
      return;
    }
    void loadPendingUsers();
  }, [user, installation?.role, loadPendingUsers]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "settings") {
      return;
    }
    void loadCategories();
  }, [user, hasMembership, activeTab, loadCategories]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "assets") {
      return;
    }
    void loadAssetsPage();
  }, [user, hasMembership, activeTab, loadAssetsPage]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "liabilities") {
      return;
    }
    void loadLiabilitiesPage();
  }, [user, hasMembership, activeTab, loadLiabilitiesPage]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "budget") {
      return;
    }
    void loadBudgetPage();
    // Allocation rules viven en la pestaña Presupuesto y necesitan la lista de
    // activos para el selector de destino — ambas cargas se disparan a la vez.
    void loadAssetsPage();
    void loadAllocationRules();
  }, [user, hasMembership, activeTab, loadBudgetPage, loadAssetsPage, loadAllocationRules]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "upcoming") {
      return;
    }
    void loadPlanningPage();
  }, [user, hasMembership, activeTab, loadPlanningPage]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "summary") {
      return;
    }
    void loadSummaryPage();
  }, [user, hasMembership, activeTab, loadSummaryPage]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "projection") {
      return;
    }
    void loadProjectionSeriesPage();
  }, [user, hasMembership, activeTab, loadProjectionSeriesPage]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "retirement") {
      return;
    }
    void loadRetirementPage();
    // Solo recargar bloqueante al cambiar sesión / pestaña / vista; no cuando `user` muta tras PATCH pensión.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user?.id, hasMembership, activeTab, loadRetirementPage]);

  useEffect(() => {
    if (activeTab !== "assets" || assetFormCategoryId || assetCategories.length === 0) {
      return;
    }
    setAssetFormCategoryId(assetCategories[0].id);
  }, [activeTab, assetFormCategoryId, assetCategories]);

  useEffect(() => {
    if (
      activeTab !== "liabilities" ||
      liabilityFormCategoryId ||
      liabilityCategories.length === 0
    ) {
      return;
    }
    setLiabilityFormCategoryId(liabilityCategories[0].id);
  }, [activeTab, liabilityFormCategoryId, liabilityCategories]);

  useEffect(() => {
    if (!user) {
      setCategories([]);
      setCategoriesError(null);
      setEditingCategoryId(null);
      setEditCategoryName("");
      setNewCatName("");
      setAssets([]);
      setAssetCategories([]);
      setAssetsError(null);
      setEditingAssetId(null);
      setAssetFormCategoryId("");
      setAssetFormName("");
      setAssetFormValue("");
      setAssetFormPurchase("");
      setAssetFormLiquid(true);
      setAssetFormNotes("");
      setLiabilities([]);
      setLiabilityCategories([]);
      setLiabilitiesError(null);
      setEditingLiabilityId(null);
      setLiabilityFormCategoryId("");
      setLiabilityFormLabel("");
      setLiabilityFormTypeTag("");
      setLiabilityFormPrincipal("");
      setLiabilityFormApr("");
      setLiabilityFormPaymentAmount("");
      setLiabilityFormPaymentFrequency("");
      setLiabilityFormPaymentEnd("");
      setLiabilityFormNotes("");
      setBudgetSnapshot(null);
      setBudgetIncomeCategories([]);
      setBudgetExpenseCategories([]);
      setBudgetLiabilityCategories([]);
      setBudgetError(null);
      setEditingBudgetEntryId(null);
      setBudgetFormScope("expense");
      setBudgetFormCategoryId("");
      setBudgetFormAmount("");
      setBudgetFormNotes("");
      setPlanningFlows([]);
      setPlanningIncomeCategories([]);
      setPlanningExpenseCategories([]);
      setPlanningError(null);
      setEditingPlanningFlowId(null);
      setPlanningFormScope("expense");
      setPlanningFormCategoryId("");
      setPlanningFormTitle("");
      setPlanningFormAmount("");
      setPlanningFormDue("");
      setPlanningFormNotes("");
      setPlanningFormShowInChart(false);
      setAssetModalOpen(false);
      setLiabilityModalOpen(false);
      setBudgetModalOpen(false);
      setPlanningModalOpen(false);
      setCategoryModalOpen(false);
      setCategoryRenameModalOpen(false);
      setProjectionSeries(null);
      setProjectionError(null);
    }
  }, [user]);

  useEffect(() => {
    setAssetModalOpen(false);
    setLiabilityModalOpen(false);
    setBudgetModalOpen(false);
    setPlanningModalOpen(false);
    setCategoryModalOpen(false);
    setCategoryRenameModalOpen(false);
  }, [activeTab]);

  useEffect(() => {
    if (activeTab !== "budget") {
      return;
    }
    const cats =
      budgetFormScope === "income"
        ? budgetIncomeCategories
        : budgetExpenseCategories;
    if (cats.length === 0) {
      return;
    }
    if (!budgetFormCategoryId || !cats.some((c) => c.id === budgetFormCategoryId)) {
      setBudgetFormCategoryId(cats[0].id);
    }
  }, [
    activeTab,
    budgetFormScope,
    budgetIncomeCategories,
    budgetExpenseCategories,
    budgetFormCategoryId,
  ]);

  useEffect(() => {
    if (activeTab !== "upcoming") {
      return;
    }
    const cats =
      planningFormScope === "income"
        ? planningIncomeCategories
        : planningExpenseCategories;
    if (cats.length === 0) {
      return;
    }
    if (
      !planningFormCategoryId ||
      !cats.some((c) => c.id === planningFormCategoryId)
    ) {
      setPlanningFormCategoryId(cats[0].id);
    }
  }, [
    activeTab,
    planningFormScope,
    planningIncomeCategories,
    planningExpenseCategories,
    planningFormCategoryId,
  ]);

  async function submitAuth(ev: FormEvent) {
    ev.preventDefault();
    setAuthBusy(true);
    setSessionError(null);
    try {
      if (authMode === "register") {
        const reg = await fetch("/v1/auth/register", {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            username,
            password,
            birth_date: registerBirthDate.trim(),
          }),
        });
        if (!reg.ok) {
          throw new Error(await errorMessageFromResponse(reg));
        }
      }
      const loginRes = await fetch("/v1/auth/login", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password }),
      });
      if (!loginRes.ok) {
        throw new Error(await errorMessageFromResponse(loginRes));
      }
      const me = (await loginRes.json()) as UserResponse;
      setUser(me);
      setPassword("");
    } catch (e: unknown) {
      setSessionError(e instanceof Error ? e.message : String(e));
    } finally {
      setAuthBusy(false);
    }
  }

  async function logout() {
    setAuthBusy(true);
    setSessionError(null);
    try {
      await fetch("/v1/auth/logout", {
        ...defaultFetchInit,
        method: "POST",
      });
      setUser(null);
      setInstallation(null);
      setInstallationGate("loading");
      setPendingUsers([]);
      setPendingUsersError(null);
      navigate("/resumen", true);
    } catch (e: unknown) {
      setSessionError(e instanceof Error ? e.message : String(e));
    } finally {
      setAuthBusy(false);
    }
  }

  async function setupInstallation(ev: FormEvent) {
    ev.preventDefault();
    setInstallationBusy(true);
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation/setup", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          base_currency: setupCurrency,
          calendar_tz: setupCalendarTz.trim(),
          show_age_mode: "dates",
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadInstallation({ preserveGate: true });
    } catch (e: unknown) {
      setInstallationError(e instanceof Error ? e.message : String(e));
    } finally {
      setInstallationBusy(false);
    }
  }

  async function saveInstallationCalendarTz(ev: FormEvent) {
    ev.preventDefault();
    setCalendarTzSaving(true);
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation", {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          calendar_tz: calendarTzDraft.trim(),
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadInstallation({ preserveGate: true });
    } catch (e: unknown) {
      setInstallationError(e instanceof Error ? e.message : String(e));
    } finally {
      setCalendarTzSaving(false);
    }
  }

  async function saveInstallationProjection(ev: FormEvent) {
    ev.preventDefault();
    const pctTrim = projectionInflationPctDraft.trim().replace(",", ".");
    const pctToSend = pctTrim === "" ? "0" : pctTrim;
    const n = Number(pctToSend);
    if (!Number.isFinite(n) || n < 0 || n > 50) {
      setInstallationError(
        "Supuesto de inflación anual: número entre 0 y 50 (0 = sin inflación).",
      );
      return;
    }
    setInstallationProjectionSaving(true);
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation", {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          show_age_mode: showAgeModeDraft,
          annual_inflation_assumption_percent: pctToSend,
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadInstallation({ preserveGate: true });
    } catch (e: unknown) {
      setInstallationError(e instanceof Error ? e.message : String(e));
    } finally {
      setInstallationProjectionSaving(false);
    }
  }

  async function saveFireSettingsPatch(fs: FireSettingsApi) {
    if (installation?.role !== "owner") return;
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation", {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ fire_settings: fs }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const updated = (await res.json()) as InstallationAccess;
      setInstallation(updated);
    } catch (e: unknown) {
      setInstallationError(e instanceof Error ? e.message : String(e));
      throw e;
    }
  }

  async function approvePendingUser(userId: string) {
    const role = approveRoles[userId] ?? "member";
    setApproveBusy(true);
    setPendingUsersError(null);
    try {
      const res = await fetch(
        `/v1/installation/pending-users/${encodeURIComponent(userId)}/approve`,
        {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ role }),
        },
      );
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadPendingUsers();
    } catch (e: unknown) {
      setPendingUsersError(e instanceof Error ? e.message : String(e));
    } finally {
      setApproveBusy(false);
    }
  }

  async function createCategory(ev: FormEvent) {
    ev.preventDefault();
    const trimmed = newCatName.trim();
    if (!trimmed) {
      return;
    }
    setCategorySaving(true);
    setCategoriesError(null);
    try {
      const res = await fetch("/v1/categories", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          scope: newCatScope,
          name: trimmed,
          sort_index: 0,
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setNewCatName("");
      setCategoryModalOpen(false);
      await loadCategories();
    } catch (e: unknown) {
      setCategoriesError(e instanceof Error ? e.message : String(e));
    } finally {
      setCategorySaving(false);
    }
  }

  function openCategoryDeleteModal(row: CategoryRow) {
    setCategoryDeletePending(row);
    const siblings = categories.filter(
      (x) => x.scope === row.scope && x.id !== row.id,
    );
    setCategoryRemapToId(siblings[0]?.id ?? "");
    setCategoriesError(null);
    setCategoryDeleteModalOpen(true);
  }

  function closeCategoryDeleteModal() {
    setCategoryDeleteModalOpen(false);
    setCategoryDeletePending(null);
    setCategoryRemapToId("");
  }

  async function confirmDeleteCategory() {
    const row = categoryDeletePending;
    if (!row) return;
    const siblings = categories.filter(
      (x) => x.scope === row.scope && x.id !== row.id,
    );
    const qs =
      siblings.length > 0 && categoryRemapToId.trim().length > 0
        ? `?remap_to=${encodeURIComponent(categoryRemapToId.trim())}`
        : "";
    setCategorySaving(true);
    setCategoriesError(null);
    try {
      const res = await fetch(
        `/v1/categories/${encodeURIComponent(row.id)}${qs}`,
        {
          ...defaultFetchInit,
          method: "DELETE",
        },
      );
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingCategoryId === row.id) {
        setEditingCategoryId(null);
        setEditCategoryName("");
        setCategoryRenameModalOpen(false);
      }
      closeCategoryDeleteModal();
      await loadCategories();
    } catch (e: unknown) {
      setCategoriesError(e instanceof Error ? e.message : String(e));
    } finally {
      setCategorySaving(false);
    }
  }

  async function saveCategoryEdit(id: string) {
    const trimmed = editCategoryName.trim();
    if (!trimmed) {
      return;
    }
    setCategorySaving(true);
    setCategoriesError(null);
    try {
      const res = await fetch(`/v1/categories/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: trimmed }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setEditingCategoryId(null);
      setEditCategoryName("");
      setCategoryRenameModalOpen(false);
      await loadCategories();
    } catch (e: unknown) {
      setCategoriesError(e instanceof Error ? e.message : String(e));
    } finally {
      setCategorySaving(false);
    }
  }

  function resetAssetForm() {
    setEditingAssetId(null);
    setAssetFormCategoryId(
      assetCategories[0]?.id ?? "",
    );
    setAssetFormName("");
    setAssetFormValue("");
    setAssetFormPurchase("");
    setAssetFormLiquid(true);
    setAssetFormExpectedReturn("");
    setAssetFormNotes("");
  }

  async function submitAssetForm(ev: FormEvent) {
    ev.preventDefault();
    if (
      !assetFormCategoryId ||
      !assetFormName.trim() ||
      !assetFormValue.trim()
    ) {
      return;
    }
    setAssetSaving(true);
    setAssetsError(null);
    try {
      const base: Record<string, unknown> = {
        category_id: assetFormCategoryId,
        name: assetFormName.trim(),
        current_value: assetFormValue.trim(),
        is_liquid: assetFormLiquid,
      };
      const er = assetFormExpectedReturn.trim().replace(",", ".");
      if (er) {
        base.expected_annual_return_percent = er;
      }

      const ppTrim = assetFormPurchase.trim().replace(",", ".");
      if (editingAssetId) {
        // PATCH: siempre enviar precio de compra — omisión antes podía dejar ambigüedad con el servidor.
        base.purchase_price = ppTrim === "" ? null : ppTrim;
      } else if (ppTrim !== "") {
        base.purchase_price = ppTrim;
      }
      const nt = assetFormNotes.trim();
      if (nt) {
        base.notes = nt;
      }

      if (editingAssetId) {
        const res = await fetch(
          `/v1/assets/${encodeURIComponent(editingAssetId)}`,
          {
            ...defaultFetchInit,
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(base),
          },
        );
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      } else {
        const res = await fetch("/v1/assets", {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(base),
        });
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      }
      resetAssetForm();
      setAssetModalOpen(false);
      await loadAssetsPage();
    } catch (e: unknown) {
      setAssetsError(e instanceof Error ? e.message : String(e));
    } finally {
      setAssetSaving(false);
    }
  }

  async function deleteAssetRow(id: string) {
    setAssetSaving(true);
    setAssetsError(null);
    try {
      const res = await fetch(`/v1/assets/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "DELETE",
      });
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingAssetId === id) {
        resetAssetForm();
        setAssetModalOpen(false);
      }
      await loadAssetsPage();
    } catch (e: unknown) {
      setAssetsError(e instanceof Error ? e.message : String(e));
    } finally {
      setAssetSaving(false);
    }
  }

  function beginEditAsset(a: AssetApiRow) {
    setEditingAssetId(a.id);
    setAssetFormCategoryId(a.category_id);
    setAssetFormName(a.name);
    setAssetFormValue(formatEditableDecimalString(a.current_value));
    setAssetFormPurchase(formatEditableDecimalString(a.purchase_price ?? ""));
    setAssetFormLiquid(a.is_liquid);
    setAssetFormExpectedReturn(
      formatEditableDecimalString(a.expected_annual_return_percent ?? ""),
    );
    setAssetFormNotes(a.notes ?? "");
  }

  function resetLiabilityForm() {
    setEditingLiabilityId(null);
    setLiabilityFormCategoryId(liabilityCategories[0]?.id ?? "");
    setLiabilityFormLabel("");
    setLiabilityFormTypeTag("");
    setLiabilityFormPrincipal("");
    setLiabilityFormApr("");
    setLiabilityFormPaymentAmount("");
    setLiabilityFormPaymentFrequency("");
    setLiabilityFormPaymentEnd("");
    setLiabilityFormNotes("");
    setLiabilityFormDerivePrincipal(false);
  }

  async function submitLiabilityForm(ev: FormEvent) {
    ev.preventDefault();
    if (!liabilityFormCategoryId || !liabilityFormLabel.trim()) {
      return;
    }
    const payAmt = liabilityFormPaymentAmount.trim();
    const payFreq = liabilityFormPaymentFrequency;
    const pend = liabilityFormPaymentEnd.trim();

    if (liabilityFormDerivePrincipal) {
      if (!payAmt || !payFreq || !pend) {
        setLiabilitiesError(
          "Derivar principal: indica cuota, frecuencia (mensual/semanal) y fecha fin del plan.",
        );
        return;
      }
    } else if (!liabilityFormPrincipal.trim()) {
      return;
    }

    if ((payAmt && !payFreq) || (!payAmt && payFreq)) {
      setLiabilitiesError(
        "Plan de pago: indica importe y frecuencia (mensual/semanal), u omite ambos.",
      );
      return;
    }
    setLiabilitySaving(true);
    setLiabilitiesError(null);
    try {
      const base: Record<string, unknown> = {
        category_id: liabilityFormCategoryId,
        label: liabilityFormLabel.trim(),
      };
      base.derive_principal_from_plan = liabilityFormDerivePrincipal;
      if (!liabilityFormDerivePrincipal) {
        base.principal = liabilityFormPrincipal.trim();
      }
      const tt = liabilityFormTypeTag.trim();
      if (tt) {
        base.type_tag = tt;
      }
      const apr = liabilityFormApr.trim();
      if (apr) {
        base.apr_percent = apr;
      }
      if (payAmt && payFreq) {
        base.payment_amount = payAmt;
        base.payment_frequency = payFreq;
      }
      if (pend) {
        base.payment_end_date = pend;
      }
      const nt = liabilityFormNotes.trim();
      if (nt) {
        base.notes = nt;
      }

      if (editingLiabilityId) {
        const res = await fetch(
          `/v1/liabilities/${encodeURIComponent(editingLiabilityId)}`,
          {
            ...defaultFetchInit,
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(base),
          },
        );
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      } else {
        const res = await fetch("/v1/liabilities", {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(base),
        });
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      }
      resetLiabilityForm();
      setLiabilityModalOpen(false);
      await loadLiabilitiesPage();
    } catch (e: unknown) {
      setLiabilitiesError(e instanceof Error ? e.message : String(e));
    } finally {
      setLiabilitySaving(false);
    }
  }

  async function deleteLiabilityRow(id: string) {
    setLiabilitySaving(true);
    setLiabilitiesError(null);
    try {
      const res = await fetch(`/v1/liabilities/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "DELETE",
      });
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingLiabilityId === id) {
        resetLiabilityForm();
        setLiabilityModalOpen(false);
      }
      await loadLiabilitiesPage();
    } catch (e: unknown) {
      setLiabilitiesError(e instanceof Error ? e.message : String(e));
    } finally {
      setLiabilitySaving(false);
    }
  }

  function beginEditLiability(row: LiabilityApiRow) {
    setEditingLiabilityId(row.id);
    setLiabilityFormCategoryId(row.category_id);
    setLiabilityFormLabel(row.label);
    setLiabilityFormTypeTag(row.type_tag ?? "");
    setLiabilityFormPrincipal(formatEditableDecimalString(row.principal));
    setLiabilityFormApr(formatEditableDecimalString(row.apr_percent ?? ""));
    setLiabilityFormPaymentAmount(
      formatEditableDecimalString(row.payment_amount ?? ""),
    );
    setLiabilityFormPaymentFrequency(row.payment_frequency ?? "");
    setLiabilityFormPaymentEnd(row.payment_end_date ?? "");
    setLiabilityFormNotes(row.notes ?? "");
    setLiabilityFormDerivePrincipal(row.principal_derived_from_plan ?? false);
  }

  function resetBudgetForm(overrideScope?: BudgetScopeToggle) {
    setEditingBudgetEntryId(null);
    const scope =
      overrideScope !== undefined ? overrideScope : budgetFormScope;
    if (overrideScope !== undefined) {
      setBudgetFormScope(overrideScope);
    }
    const cats =
      scope === "income"
        ? budgetIncomeCategories
        : budgetExpenseCategories;
    setBudgetFormCategoryId(cats[0]?.id ?? "");
    setBudgetFormAmount("");
    setBudgetFormNotes("");
    setBudgetFormPersistsAfterRetirement(false);
  }

  async function submitBudgetForm(ev: FormEvent) {
    ev.preventDefault();
    const amt = budgetFormAmount.trim();
    if (!budgetFormCategoryId || !amt) {
      return;
    }
    setBudgetSaving(true);
    setBudgetError(null);
    try {
      const base: Record<string, unknown> = {
        category_id: budgetFormCategoryId,
        amount: amt,
      };
      const nt = budgetFormNotes.trim();
      if (nt) {
        base.notes = nt;
      }

      if (editingBudgetEntryId) {
        const patchBody: Record<string, unknown> = {
          category_id: budgetFormCategoryId,
          amount: amt,
          notes: budgetFormNotes.trim(),
          persists_after_retirement: budgetFormScope === "income" ? budgetFormPersistsAfterRetirement : false,
        };
        if (budgetFormScope === "expense") {
          patchBody.ends_at_retirement = budgetFormExpenseEndType === "retirement";
          if (budgetFormExpenseEndType === "date") {
            patchBody.expense_end_date = budgetFormExpenseEndDate;
          } else {
            patchBody.clear_expense_end_date = true;
          }
        }
        const res = await fetch(
          `/v1/budget/entries/${encodeURIComponent(editingBudgetEntryId)}`,
          {
            ...defaultFetchInit,
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(patchBody),
          },
        );
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      } else {
        if (budgetFormScope === "income") {
          base.persists_after_retirement = budgetFormPersistsAfterRetirement;
        }
        if (budgetFormScope === "expense") {
          base.ends_at_retirement = budgetFormExpenseEndType === "retirement";
          if (budgetFormExpenseEndType === "date") {
            base.expense_end_date = budgetFormExpenseEndDate;
          }
        }
        const res = await fetch("/v1/budget/entries", {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(base),
        });
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      }
      resetBudgetForm();
      setBudgetModalOpen(false);
      await loadBudgetPage();
    } catch (e: unknown) {
      setBudgetError(e instanceof Error ? e.message : String(e));
    } finally {
      setBudgetSaving(false);
    }
  }

  async function deleteBudgetEntryRow(id: string) {
    setBudgetSaving(true);
    setBudgetError(null);
    try {
      const res = await fetch(`/v1/budget/entries/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "DELETE",
      });
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingBudgetEntryId === id) {
        resetBudgetForm();
        setBudgetModalOpen(false);
      }
      await loadBudgetPage();
    } catch (e: unknown) {
      setBudgetError(e instanceof Error ? e.message : String(e));
    } finally {
      setBudgetSaving(false);
    }
  }

  function beginEditBudgetEntry(row: BudgetEntryApiRow) {
    setEditingBudgetEntryId(row.id);
    setBudgetFormScope(row.scope);
    setBudgetFormCategoryId(row.category_id);
    setBudgetFormAmount(formatEditableDecimalString(row.amount));
    setBudgetFormNotes(row.notes ?? "");
    setBudgetFormPersistsAfterRetirement(row.persists_after_retirement);
    const endType = row.ends_at_retirement ? "retirement" : row.expense_end_date ? "date" : "never";
    setBudgetFormExpenseEndType(endType);
    setBudgetFormExpenseEndDate(row.expense_end_date ?? "");
  }

  function resetPlanningFlowForm() {
    setEditingPlanningFlowId(null);
    const cats =
      planningFormScope === "income"
        ? planningIncomeCategories
        : planningExpenseCategories;
    setPlanningFormCategoryId(cats[0]?.id ?? "");
    setPlanningFormTitle("");
    setPlanningFormAmount("");
    setPlanningFormDue("");
    setPlanningFormNotes("");
    setPlanningFormShowInChart(false);
  }

  async function submitPlanningFlowForm(ev: FormEvent) {
    ev.preventDefault();
    const amt = planningFormAmount.trim();
    const tit = planningFormTitle.trim();
    if (!planningFormCategoryId || !amt || !tit) {
      return;
    }
    setPlanningSaving(true);
    setPlanningError(null);
    try {
      const dueTrim = planningFormDue.trim();
      const showInChart = dueTrim !== "" && planningFormShowInChart;
      if (editingPlanningFlowId) {
        const patchBody: Record<string, unknown> = {
          category_id: planningFormCategoryId,
          title: tit,
          expected_amount: amt,
          due_date: dueTrim === "" ? null : dueTrim,
          notes: planningFormNotes.trim(),
          show_in_chart: showInChart,
        };
        const res = await fetch(
          `/v1/planning/flows/${encodeURIComponent(editingPlanningFlowId)}`,
          {
            ...defaultFetchInit,
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(patchBody),
          },
        );
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      } else {
        const base: Record<string, unknown> = {
          category_id: planningFormCategoryId,
          title: tit,
          expected_amount: amt,
        };
        if (dueTrim) {
          base.due_date = dueTrim;
        }
        const nt = planningFormNotes.trim();
        if (nt) {
          base.notes = nt;
        }
        if (showInChart) {
          base.show_in_chart = true;
        }
        const res = await fetch("/v1/planning/flows", {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(base),
        });
        if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        }
      }
      resetPlanningFlowForm();
      setPlanningModalOpen(false);
      await loadPlanningPage();
    } catch (e: unknown) {
      setPlanningError(e instanceof Error ? e.message : String(e));
    } finally {
      setPlanningSaving(false);
    }
  }

  async function deletePlanningFlowRow(id: string) {
    setPlanningSaving(true);
    setPlanningError(null);
    try {
      const res = await fetch(`/v1/planning/flows/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "DELETE",
      });
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingPlanningFlowId === id) {
        resetPlanningFlowForm();
        setPlanningModalOpen(false);
      }
      await loadPlanningPage();
    } catch (e: unknown) {
      setPlanningError(e instanceof Error ? e.message : String(e));
    } finally {
      setPlanningSaving(false);
    }
  }

  async function saveUserBirthProfile(ev: FormEvent) {
    ev.preventDefault();
    setUserProfileSaving(true);
    setUserProfileError(null);
    const trimmed = userBirthDraft.trim();
    try {
      const res = await fetch("/v1/auth/me", {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          birth_date: trimmed,
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const body = (await res.json()) as UserResponse;
      setUser(body);
      setUserProfileOpen(false);
    } catch (e: unknown) {
      setUserProfileError(e instanceof Error ? e.message : String(e));
    } finally {
      setUserProfileSaving(false);
    }
  }

  function readUiPreferencesFromStorage() {
    if (typeof window === "undefined") return {};
    let person_scope: string | undefined;
    let projection_focus: string | undefined;
    try {
      const ps = window.localStorage.getItem(LEDGER_PERSON_SCOPE_STORAGE_KEY);
      if (ps === "mine" || ps === "household") person_scope = ps;
      const pf = window.localStorage.getItem(PROJECTION_FOCUS_STORAGE_KEY);
      if (pf === "0" || pf === "1") projection_focus = pf;
    } catch {
      /* ignore */
    }
    return { person_scope, projection_focus };
  }

  function openFfbackupExportModal() {
    setFfbackupExportError(null);
    setFfbackupExportPassword("");
    setFfbackupExportModalOpen(true);
  }

  function closeFfbackupExportModal() {
    if (ffbackupExportBusy) return;
    setFfbackupExportModalOpen(false);
    setFfbackupExportPassword("");
    setFfbackupExportError(null);
  }

  async function runFfbackupExport(e: FormEvent) {
    e.preventDefault();
    if (!ffbackupExportPassword) {
      setFfbackupExportError("Introduce tu contraseña de cuenta.");
      return;
    }
    setFfbackupExportBusy(true);
    setFfbackupExportError(null);
    try {
      const res = await fetch("/v1/backup/user-export", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          password: ffbackupExportPassword,
          ui_preferences: readUiPreferencesFromStorage(),
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      const filenameHeader = res.headers.get("content-disposition") ?? "";
      const match = filenameHeader.match(/filename="?([^";]+)"?/);
      a.download = match ? match[1] : "futurefin-backup.ffbackup";
      a.rel = "noopener";
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      setFfbackupExportModalOpen(false);
      setFfbackupExportPassword("");
    } catch (err: unknown) {
      setFfbackupExportError(
        err instanceof Error ? err.message : String(err),
      );
    } finally {
      setFfbackupExportBusy(false);
    }
  }

  function openFfbackupImportModal() {
    setFfbackupImportError(null);
    setFfbackupImportPreview(null);
    setFfbackupImportFile(null);
    setFfbackupImportPassword("");
    setFfbackupImportDone(null);
    setFfbackupImportModalOpen(true);
  }

  function closeFfbackupImportModal() {
    if (ffbackupImportBusy) return;
    setFfbackupImportModalOpen(false);
    setFfbackupImportError(null);
    setFfbackupImportPreview(null);
    setFfbackupImportFile(null);
    setFfbackupImportPassword("");
  }

  async function readFileAsBase64(file: File): Promise<string> {
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    let binary = "";
    const chunk = 0x8000;
    for (let i = 0; i < bytes.length; i += chunk) {
      binary += String.fromCharCode(
        ...bytes.subarray(i, Math.min(i + chunk, bytes.length)),
      );
    }
    return window.btoa(binary);
  }

  async function runFfbackupImportPreview(e: FormEvent) {
    e.preventDefault();
    if (!ffbackupImportFile) {
      setFfbackupImportError("Selecciona un archivo .ffbackup.");
      return;
    }
    if (!ffbackupImportPassword) {
      setFfbackupImportError(
        "Introduce la contraseña con la que se generó el backup.",
      );
      return;
    }
    setFfbackupImportBusy(true);
    setFfbackupImportError(null);
    try {
      const fileB64 = await readFileAsBase64(ffbackupImportFile);
      const res = await fetch("/v1/backup/user-import/preview", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          file_b64: fileB64,
          password: ffbackupImportPassword,
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const preview = (await res.json()) as FfbackupImportPreviewResponse;
      setFfbackupImportPreview(preview);
    } catch (err: unknown) {
      setFfbackupImportError(
        err instanceof Error ? err.message : String(err),
      );
    } finally {
      setFfbackupImportBusy(false);
    }
  }

  async function runFfbackupImportApply() {
    if (!ffbackupImportFile || !ffbackupImportPassword) return;
    setFfbackupImportBusy(true);
    setFfbackupImportError(null);
    try {
      const fileB64 = await readFileAsBase64(ffbackupImportFile);
      const res = await fetch("/v1/backup/user-import", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          file_b64: fileB64,
          password: ffbackupImportPassword,
          confirm_replace: true,
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const body = (await res.json()) as FfbackupImportApplyResponse;
      const ui = body.ui_preferences ?? {};
      try {
        if (ui.person_scope === "mine" || ui.person_scope === "household") {
          window.localStorage.setItem(
            LEDGER_PERSON_SCOPE_STORAGE_KEY,
            ui.person_scope,
          );
          setLedgerPersonScopeInner(ui.person_scope);
        }
        if (ui.projection_focus === "0" || ui.projection_focus === "1") {
          window.localStorage.setItem(
            PROJECTION_FOCUS_STORAGE_KEY,
            ui.projection_focus,
          );
        }
      } catch {
        /* ignore */
      }
      const c = body.imported;
      setFfbackupImportDone(
        `Importado: ${c.assets} activos, ${c.liabilities} pasivos, ${c.budget_entries} entradas de presupuesto, ${c.planning_flows} flujos.`,
      );
      setFfbackupImportPreview(null);
      setFfbackupImportFile(null);
      setFfbackupImportPassword("");
      await Promise.all([
        loadAssetsPage(),
        loadLiabilitiesPage(),
        loadBudgetPage(),
        loadPlanningPage(),
        loadSummaryPage(),
        loadProjectionSeriesPage(),
        refreshSession(),
      ]);
    } catch (err: unknown) {
      setFfbackupImportError(
        err instanceof Error ? err.message : String(err),
      );
    } finally {
      setFfbackupImportBusy(false);
    }
  }

  function beginEditPlanningFlow(row: PlanningFlowApiRow) {
    setEditingPlanningFlowId(row.id);
    const scope: BudgetScopeToggle =
      row.direction === "inflow" ? "income" : "expense";
    setPlanningFormScope(scope);
    setPlanningFormCategoryId(row.category_id);
    setPlanningFormTitle(row.title);
    setPlanningFormAmount(formatEditableDecimalString(row.expected_amount));
    setPlanningFormDue(row.due_date ?? "");
    setPlanningFormNotes(row.notes ?? "");
    setPlanningFormShowInChart(row.show_in_chart);
  }

  if (sessionBusy) {
    return (
      <div className="app-loading">
        <div className="app-loading-inner">
          <span className="spinner" aria-hidden />
          <p>Cargando FutureFin…</p>
        </div>
      </div>
    );
  }

  if (!user) {
    return (
      <div className="auth-screen">
        <div className="auth-brand">
          <div className="auth-brand-inner">
            <span className="logo-mark">FF</span>
            <h1>FutureFin</h1>
          </div>
        </div>
        <div className="auth-panel-wrap">
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">Acceder</h2>
            <div className="segmented" role="tablist" aria-label="Modo">
              <button
                type="button"
                role="tab"
                aria-selected={authMode === "login"}
                className={authMode === "login" ? "active" : ""}
                onClick={() => setAuthMode("login")}
              >
                Iniciar sesión
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={authMode === "register"}
                className={authMode === "register" ? "active" : ""}
                onClick={() => setAuthMode("register")}
              >
                Crear cuenta
              </button>
            </div>
            <form className="stack" onSubmit={(e) => void submitAuth(e)}>
              <label className="field">
                <span>Usuario</span>
                <input
                  autoComplete="username"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  required
                  minLength={3}
                  maxLength={64}
                />
              </label>
              <label className="field">
                <span>Contraseña</span>
                <input
                  type="password"
                  autoComplete={
                    authMode === "register"
                      ? "new-password"
                      : "current-password"
                  }
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  required
                  minLength={12}
                  maxLength={256}
                />
              </label>
              {authMode === "register" && (
                <label className="field">
                  <span>Fecha de nacimiento</span>
                  <input
                    type="date"
                    autoComplete="bday"
                    value={registerBirthDate}
                    onChange={(e) => setRegisterBirthDate(e.target.value)}
                    max={new Date().toISOString().slice(0, 10)}
                    required
                  />
                </label>
              )}
              <button type="submit" className="btn primary wide" disabled={authBusy}>
                {authMode === "register" ? "Registrarse y entrar" : "Entrar"}
              </button>
            </form>
            {sessionError ? (
              <p className="error compact">{sessionError}</p>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className={
        activeTab === "projection"
          ? "app-root app-root--projection-viewport"
          : "app-root"
      }
    >
      <header className="app-header">
        <div className="app-header-left">
          <span className="logo-mark small">FF</span>
          <span className="app-title">FutureFin</span>
        </div>
        <div className="app-header-center" aria-hidden />
        <div className="app-header-right">
          <span
            className={`health-dot ${health && !healthError ? "ok" : "bad"}`}
            title={
              healthError
                ? `API: ${healthError}`
                : health
                  ? `API ${health.service} ${health.version}`
                  : "Comprobando…"
            }
          />
          <button
            type="button"
            className="user-chip user-chip-btn"
            title="Configuración de cuenta"
            disabled={authBusy}
            onClick={() => {
              setUserProfileError(null);
              setUserBirthDraft(user.birth_date?.trim() ?? "");
              setUserProfileOpen(true);
            }}
          >
            {user.username}
          </button>
          <button
            type="button"
            className="btn ghost text"
            disabled={authBusy}
            onClick={() => void logout()}
          >
            Salir
          </button>
        </div>
      </header>

      <Modal
        title="Tu cuenta"
        open={userProfileOpen}
        onClose={() => {
          setUserProfileOpen(false);
          setUserProfileError(null);
        }}
      >
        <form className="asset-form stack" onSubmit={(e) => void saveUserBirthProfile(e)}>
          {userProfileError ? (
            <ModalFormError message={userProfileError} />
          ) : null}
          <label className="field">
            <span>Fecha de nacimiento</span>
            <input
              type="date"
              value={userBirthDraft}
              onChange={(e) => setUserBirthDraft(e.target.value)}
              max={new Date().toISOString().slice(0, 10)}
              autoComplete="bday"
              required
            />
          </label>
          <div className="asset-form-actions">
            <button
              type="submit"
              className="btn primary"
              disabled={userProfileSaving}
            >
              Guardar
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={userProfileSaving}
              onClick={() => {
                setUserProfileOpen(false);
                setUserProfileError(null);
              }}
            >
              Cerrar
            </button>
          </div>
        </form>
      </Modal>

      {installationGate === "loading" ? (
        <div className="app-loading">
          <div className="app-loading-inner">
            <span className="spinner" aria-hidden />
            <p>Cargando acceso al hogar…</p>
          </div>
        </div>
      ) : installationGate === "fetch_failed" ? (
        <main className="app-main">
          <div className="workspace">
            <div className="workspace-header">
              <h2 className="workspace-title">No se pudo cargar el acceso</h2>
              <p className="workspace-sub muted">Revisa la conexión.</p>
            </div>
            {installationError ? (
              <div className="banner error-banner">{installationError}</div>
            ) : null}
            <button
              type="button"
              className="btn primary"
              disabled={installationBusy}
              onClick={() => void loadInstallation()}
            >
              Reintentar
            </button>
          </div>
        </main>
      ) : installationGate === "pending" ? (
        <main className="app-main">
          <div className="workspace">
            <div className="workspace-header">
              <h2 className="workspace-title">Acceso pendiente</h2>
              <p className="workspace-sub">
                <strong>Ajustes → Acceso</strong>
              </p>
            </div>
          </div>
        </main>
      ) : installationGate === "bootstrap" ? (
        <main className="app-main">
          <div className="workspace">
            <div className="workspace-header">
              <h2 className="workspace-title">Crear el hogar</h2>
            </div>
            {installationError ? (
              <div className="banner error-banner">{installationError}</div>
            ) : null}
            <BootstrapInstallationPanel
              installationBusy={installationBusy}
              setupCurrency={setupCurrency}
              setSetupCurrency={setSetupCurrency}
              setupCalendarTz={setupCalendarTz}
              setSetupCalendarTz={setSetupCalendarTz}
              setupInstallation={(e) => void setupInstallation(e)}
            />
          </div>
        </main>
      ) : (
        <>
          <nav className="tab-bar" aria-label="Secciones">
            {TABS.map((t) => (
              <a
                key={t.id}
                href={TAB_PATH[t.id]}
                className={`tab-btn ${activeTab === t.id ? "active" : ""}`}
                aria-current={activeTab === t.id ? "page" : undefined}
                onClick={(e) => {
                  if (
                    e.button !== 0 ||
                    e.metaKey ||
                    e.altKey ||
                    e.ctrlKey ||
                    e.shiftKey
                  ) {
                    return;
                  }
                  e.preventDefault();
                  navigate(TAB_PATH[t.id]);
                }}
              >
                {t.label}
              </a>
            ))}
          </nav>

          <div className="person-filter-bar person-filter-bar--discrete">
            <label
              htmlFor={ledgerScopeSelectId}
              className="person-filter-bar-label"
            >
              Vista
            </label>
            <select
              id={ledgerScopeSelectId}
              className="ledger-view-select"
              value={ledgerPersonScope}
              onChange={(e) =>
                setLedgerPersonScope(
                  e.target.value === "mine" ? "mine" : "household",
                )
              }
              aria-label="Ámbito de datos: hogar o solo tus registros"
              title="Hogar: todos los datos. Tu usuario: solo filas donde eres titular."
            >
              <option value="household">Todo el hogar</option>
              <option value="mine">{user.username}</option>
            </select>
          </div>

          <main
            className={
              activeTab === "projection"
                ? "app-main app-main--projection-fullbleed"
                : "app-main"
            }
          >
        <div
          className="app-global-errors"
          role="region"
          aria-label="Errores y avisos"
          aria-live="polite"
        >
          {sessionError ? (
            <div className="banner error-banner">{sessionError}</div>
          ) : null}

          {installationError ? (
            <div className="banner error-banner">{installationError}</div>
          ) : null}

          {pendingUsersError ? (
            <div className="banner error-banner">{pendingUsersError}</div>
          ) : null}

          {categoriesError ? (
            <div className="banner error-banner">{categoriesError}</div>
          ) : null}

          {assetsError ? (
            <div className="banner error-banner">{assetsError}</div>
          ) : null}

          {liabilitiesError ? (
            <div className="banner error-banner">{liabilitiesError}</div>
          ) : null}

          {budgetError ? (
            <div className="banner error-banner">{budgetError}</div>
          ) : null}

          {planningError ? (
            <div className="banner error-banner">{planningError}</div>
          ) : null}

          {summaryError ? (
            <div className="banner error-banner">{summaryError}</div>
          ) : null}
        </div>

        {activeTab === "summary" ? (
          <SummaryView
            installation={installation}
            loading={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            summary={summary}
            summaryBusy={summaryBusy}
          />
        ) : activeTab === "assets" ? (
          <AssetsView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            canEdit={installation?.role !== "viewer"}
            formError={assetsError}
            projectionSeries={projectionSeries}
            anchorDateYmd={projectionSeries?.anchor_date_ymd ?? null}
            calendarTz={installation?.installation.calendar_tz?.trim() || "UTC"}
            assetModalOpen={assetModalOpen}
            closeAssetModal={() => {
              resetAssetForm();
              setAssetModalOpen(false);
            }}
            openNewAssetModal={() => {
              resetAssetForm();
              setAssetModalOpen(true);
            }}
            assets={assets}
            assetsBusy={assetsBusy}
            assetCategories={assetCategories}
            assetFormCategoryId={assetFormCategoryId}
            setAssetFormCategoryId={setAssetFormCategoryId}
            assetFormName={assetFormName}
            setAssetFormName={setAssetFormName}
            assetFormValue={assetFormValue}
            setAssetFormValue={setAssetFormValue}
            assetFormPurchase={assetFormPurchase}
            setAssetFormPurchase={setAssetFormPurchase}
            assetFormLiquid={assetFormLiquid}
            setAssetFormLiquid={setAssetFormLiquid}
            assetFormExpectedReturn={assetFormExpectedReturn}
            setAssetFormExpectedReturn={setAssetFormExpectedReturn}
            assetFormNotes={assetFormNotes}
            setAssetFormNotes={setAssetFormNotes}
            editingAssetId={editingAssetId}
            assetSaving={assetSaving}
            submitAssetForm={(e) => void submitAssetForm(e)}
            deleteAssetRow={(id) => void deleteAssetRow(id)}
            beginEditAsset={(a) => {
              beginEditAsset(a);
              setAssetModalOpen(true);
            }}
          />
        ) : activeTab === "liabilities" ? (
          <LiabilitiesView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            canEdit={installation?.role !== "viewer"}
            formError={liabilitiesError}
            liabilityModalOpen={liabilityModalOpen}
            closeLiabilityModal={() => {
              resetLiabilityForm();
              setLiabilityModalOpen(false);
            }}
            openNewLiabilityModal={() => {
              resetLiabilityForm();
              setLiabilityModalOpen(true);
            }}
            liabilities={liabilities}
            liabilitiesBusy={liabilitiesBusy}
            liabilityCategories={liabilityCategories}
            liabilityFormCategoryId={liabilityFormCategoryId}
            setLiabilityFormCategoryId={setLiabilityFormCategoryId}
            liabilityFormLabel={liabilityFormLabel}
            setLiabilityFormLabel={setLiabilityFormLabel}
            liabilityFormTypeTag={liabilityFormTypeTag}
            setLiabilityFormTypeTag={setLiabilityFormTypeTag}
            liabilityFormPrincipal={liabilityFormPrincipal}
            setLiabilityFormPrincipal={setLiabilityFormPrincipal}
            liabilityFormApr={liabilityFormApr}
            setLiabilityFormApr={setLiabilityFormApr}
            liabilityFormPaymentAmount={liabilityFormPaymentAmount}
            setLiabilityFormPaymentAmount={setLiabilityFormPaymentAmount}
            liabilityFormPaymentFrequency={liabilityFormPaymentFrequency}
            setLiabilityFormPaymentFrequency={setLiabilityFormPaymentFrequency}
            liabilityFormPaymentEnd={liabilityFormPaymentEnd}
            setLiabilityFormPaymentEnd={setLiabilityFormPaymentEnd}
            liabilityFormNotes={liabilityFormNotes}
            setLiabilityFormNotes={setLiabilityFormNotes}
            liabilityFormDerivePrincipal={liabilityFormDerivePrincipal}
            setLiabilityFormDerivePrincipal={setLiabilityFormDerivePrincipal}
            editingLiabilityId={editingLiabilityId}
            liabilitySaving={liabilitySaving}
            submitLiabilityForm={(e) => void submitLiabilityForm(e)}
            deleteLiabilityRow={(id) => void deleteLiabilityRow(id)}
            beginEditLiability={(row) => {
              beginEditLiability(row);
              setLiabilityModalOpen(true);
            }}
          />
        ) : activeTab === "budget" ? (
          <BudgetView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            canEdit={installation?.role !== "viewer"}
            formError={budgetError}
            budgetModalOpen={budgetModalOpen}
            closeBudgetModal={() => {
              resetBudgetForm();
              setBudgetModalOpen(false);
            }}
            openNewBudgetModal={(scope) => {
              resetBudgetForm(scope);
              setBudgetModalOpen(true);
            }}
            budgetSnapshot={budgetSnapshot}
            budgetLoading={budgetLoading}
            budgetIncomeCategories={budgetIncomeCategories}
            budgetExpenseCategories={budgetExpenseCategories}
            budgetLiabilityCategories={budgetLiabilityCategories}
            budgetFormScope={budgetFormScope}
            setBudgetFormScope={setBudgetFormScope}
            budgetFormCategoryId={budgetFormCategoryId}
            setBudgetFormCategoryId={setBudgetFormCategoryId}
            budgetFormAmount={budgetFormAmount}
            setBudgetFormAmount={setBudgetFormAmount}
            budgetFormNotes={budgetFormNotes}
            setBudgetFormNotes={setBudgetFormNotes}
            budgetFormPersistsAfterRetirement={budgetFormPersistsAfterRetirement}
            setBudgetFormPersistsAfterRetirement={setBudgetFormPersistsAfterRetirement}
            budgetFormExpenseEndType={budgetFormExpenseEndType}
            setBudgetFormExpenseEndType={setBudgetFormExpenseEndType}
            budgetFormExpenseEndDate={budgetFormExpenseEndDate}
            setBudgetFormExpenseEndDate={setBudgetFormExpenseEndDate}
            editingBudgetEntryId={editingBudgetEntryId}
            budgetSaving={budgetSaving}
            submitBudgetForm={(e) => void submitBudgetForm(e)}
            deleteBudgetEntryRow={(id) => void deleteBudgetEntryRow(id)}
            beginEditBudgetEntry={(row) => {
              beginEditBudgetEntry(row);
              setBudgetModalOpen(true);
            }}
            assets={assets}
            allocationRules={allocationRules}
            allocationRulesBusy={allocationRulesBusy}
            allocationRulesError={allocationRulesError}
            allocationPanelOpen={allocationPanelOpen}
            openAllocationPanel={() => setAllocationPanelOpen(true)}
            closeAllocationPanel={() => setAllocationPanelOpen(false)}
            ruleModalOpen={ruleModalOpen}
            openNewRuleModal={() => {
              resetRuleForm();
              setRuleModalOpen(true);
            }}
            closeRuleModal={() => {
              resetRuleForm();
              setRuleModalOpen(false);
            }}
            ruleFormTargetAsset={ruleFormTargetAsset}
            setRuleFormTargetAsset={setRuleFormTargetAsset}
            ruleFormKind={ruleFormKind}
            setRuleFormKind={setRuleFormKind}
            ruleFormAmount={ruleFormAmount}
            setRuleFormAmount={setRuleFormAmount}
            ruleFormCapKind={ruleFormCapKind}
            setRuleFormCapKind={setRuleFormCapKind}
            ruleFormCapValue={ruleFormCapValue}
            setRuleFormCapValue={setRuleFormCapValue}
            editingRuleId={editingRuleId}
            ruleSaving={ruleSaving}
            submitRuleForm={(e) => void submitRuleForm(e)}
            deleteRule={(id) => void deleteRule(id)}
            moveRule={(id, dir) => void moveRule(id, dir)}
            beginEditRule={(r) => {
              beginEditRule(r);
              setRuleModalOpen(true);
            }}
          />
        ) : activeTab === "upcoming" ? (
          <UpcomingView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            canEdit={installation?.role !== "viewer"}
            formError={planningError}
            planningModalOpen={planningModalOpen}
            closePlanningModal={() => {
              resetPlanningFlowForm();
              setPlanningModalOpen(false);
            }}
            openNewPlanningModal={() => {
              resetPlanningFlowForm();
              setPlanningModalOpen(true);
            }}
            planningFlows={planningFlows}
            planningLoading={planningLoading}
            planningIncomeCategories={planningIncomeCategories}
            planningExpenseCategories={planningExpenseCategories}
            planningFormScope={planningFormScope}
            setPlanningFormScope={setPlanningFormScope}
            planningFormCategoryId={planningFormCategoryId}
            setPlanningFormCategoryId={setPlanningFormCategoryId}
            planningFormTitle={planningFormTitle}
            setPlanningFormTitle={setPlanningFormTitle}
            planningFormAmount={planningFormAmount}
            setPlanningFormAmount={setPlanningFormAmount}
            planningFormDue={planningFormDue}
            setPlanningFormDue={setPlanningFormDue}
            planningFormNotes={planningFormNotes}
            setPlanningFormNotes={setPlanningFormNotes}
            planningFormShowInChart={planningFormShowInChart}
            setPlanningFormShowInChart={setPlanningFormShowInChart}
            editingPlanningFlowId={editingPlanningFlowId}
            planningSaving={planningSaving}
            submitPlanningFlowForm={(e) => void submitPlanningFlowForm(e)}
            deletePlanningFlowRow={(id) => void deletePlanningFlowRow(id)}
            beginEditPlanningFlow={(row) => {
              beginEditPlanningFlow(row);
              setPlanningModalOpen(true);
            }}
          />
        ) : activeTab === "retirement" ? (
          <RetirementView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            projectionSeries={projectionSeries}
            projectionBusy={projectionBusy}
            retirementBudgetSnapshot={retirementBudgetSnapshot}
            retirementBusy={retirementBusy}
            retirementError={retirementError}
            user={user}
            calendarTz={installation?.installation.calendar_tz?.trim() || "UTC"}
            canEditFire={installation?.role === "owner"}
            onSaveFire={saveFireSettingsPatch}
            navigate={navigate}
          />
        ) : activeTab === "projection" ? (
          <ProjectionView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            projectionSeries={projectionSeries}
            projectionBusy={projectionBusy}
            projectionError={projectionError}
            userBirthDate={user?.birth_date ?? null}
            calendarTz={installation?.installation.calendar_tz?.trim() || "UTC"}
            planningFlows={planningFlows}
          />
        ) : activeTab === "settings" ? (
          <SettingsView
            installation={installation}
            installationBusy={installationBusy}
            categoryModalOpen={categoryModalOpen}
            categoryRenameModalOpen={categoryRenameModalOpen}
            closeCategoryModal={() => {
              setNewCatName("");
              setCategoryModalOpen(false);
            }}
            openNewCategoryModal={() => {
              setCategoryRenameModalOpen(false);
              setEditingCategoryId(null);
              setEditCategoryName("");
              setNewCatName("");
              setCategoryModalOpen(true);
            }}
            closeRenameCategoryModal={() => {
              setEditingCategoryId(null);
              setEditCategoryName("");
              setCategoryRenameModalOpen(false);
            }}
            openRenameCategoryModal={(row: CategoryRow) => {
              setCategoryModalOpen(false);
              setNewCatName("");
              setEditingCategoryId(row.id);
              setEditCategoryName(row.name);
              setCategoryRenameModalOpen(true);
            }}
            calendarTzDraft={calendarTzDraft}
            setCalendarTzDraft={setCalendarTzDraft}
            calendarTzSaving={calendarTzSaving}
            saveInstallationCalendarTz={(e) =>
              void saveInstallationCalendarTz(e)
            }
            projectionInflationPctDraft={projectionInflationPctDraft}
            setProjectionInflationPctDraft={setProjectionInflationPctDraft}
            showAgeModeDraft={showAgeModeDraft}
            setShowAgeModeDraft={setShowAgeModeDraft}
            installationProjectionSaving={installationProjectionSaving}
            saveInstallationProjection={(e) => void saveInstallationProjection(e)}
            onSaveFire={saveFireSettingsPatch}
            health={health}
            healthError={healthError}
            categoriesError={categoriesError}
            hasMembership={hasMembership}
            canEditCategories={installation?.role !== "viewer"}
            isOwner={installation?.role === "owner"}
            settingsSubTab={settingsSubTab}
            navigateSettingsSubTab={navigateSettingsSubTab}
            visibleSettingsSubTabs={visibleSettingsSubTabs}
            pendingUsers={pendingUsers}
            pendingUsersBusy={pendingUsersBusy}
            approveRoles={approveRoles}
            setApproveRoles={setApproveRoles}
            approveBusy={approveBusy}
            approvePendingUser={(id) => void approvePendingUser(id)}
            categories={categories}
            categoriesBusy={categoriesBusy}
            categoryScopeFilter={categoryScopeFilter}
            setCategoryScopeFilter={setCategoryScopeFilter}
            newCatScope={newCatScope}
            setNewCatScope={setNewCatScope}
            newCatName={newCatName}
            setNewCatName={setNewCatName}
            categorySaving={categorySaving}
            createCategory={(e) => void createCategory(e)}
            openCategoryDeleteModal={(row) => openCategoryDeleteModal(row)}
            categoryDeleteModalOpen={categoryDeleteModalOpen}
            categoryDeletePending={categoryDeletePending}
            categoryRemapToId={categoryRemapToId}
            setCategoryRemapToId={setCategoryRemapToId}
            closeCategoryDeleteModal={closeCategoryDeleteModal}
            confirmDeleteCategory={() => void confirmDeleteCategory()}
            editingCategoryId={editingCategoryId}
            editCategoryName={editCategoryName}
            setEditCategoryName={setEditCategoryName}
            saveCategoryEdit={(id) => void saveCategoryEdit(id)}
            ffbackupExportModalOpen={ffbackupExportModalOpen}
            ffbackupExportPassword={ffbackupExportPassword}
            setFfbackupExportPassword={setFfbackupExportPassword}
            ffbackupExportBusy={ffbackupExportBusy}
            ffbackupExportError={ffbackupExportError}
            openFfbackupExportModal={openFfbackupExportModal}
            closeFfbackupExportModal={closeFfbackupExportModal}
            runFfbackupExport={runFfbackupExport}
            ffbackupImportModalOpen={ffbackupImportModalOpen}
            ffbackupImportFile={ffbackupImportFile}
            setFfbackupImportFile={setFfbackupImportFile}
            ffbackupImportPassword={ffbackupImportPassword}
            setFfbackupImportPassword={setFfbackupImportPassword}
            ffbackupImportBusy={ffbackupImportBusy}
            ffbackupImportError={ffbackupImportError}
            ffbackupImportPreview={ffbackupImportPreview}
            ffbackupImportDone={ffbackupImportDone}
            openFfbackupImportModal={openFfbackupImportModal}
            closeFfbackupImportModal={closeFfbackupImportModal}
            runFfbackupImportPreview={runFfbackupImportPreview}
            runFfbackupImportApply={() => void runFfbackupImportApply()}
          />
        ) : (
          <PlaceholderTab tabLabel={TABS.find((x) => x.id === activeTab)?.label ?? ""} />
        )}
      </main>
        </>
      )}
    </div>
  );
}

/**
 * Filas agrupadas por categoría (orden de ajustes + IDs huérfanos al reunirlos).
 * `sortRowsDescending`: dentro de cada categoría, filas de mayor a menor `value`; empates con `tieBreak`.
 * `categoryTotalDescending`: bloques de categoría de mayor a menor total; empates por nombre de categoría.
 */
function groupRowsByCategoryOrdered<T extends { category_id: string }>(
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

function PlusIcon() {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}

function RowEditIcon() {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
      <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
    </svg>
  );
}

function RowTrashIcon() {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      <line x1="10" y1="11" x2="10" y2="17" />
      <line x1="14" y1="11" x2="14" y2="17" />
    </svg>
  );
}

function GearIcon() {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.09a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.09a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

function AssetsView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  projectionSeries,
  anchorDateYmd,
  calendarTz,
  assetModalOpen,
  closeAssetModal,
  openNewAssetModal,
  assets,
  assetsBusy,
  assetCategories,
  assetFormCategoryId,
  setAssetFormCategoryId,
  assetFormName,
  setAssetFormName,
  assetFormValue,
  setAssetFormValue,
  assetFormPurchase,
  setAssetFormPurchase,
  assetFormLiquid,
  setAssetFormLiquid,
  assetFormExpectedReturn,
  setAssetFormExpectedReturn,
  assetFormNotes,
  setAssetFormNotes,
  editingAssetId,
  assetSaving,
  submitAssetForm,
  deleteAssetRow,
  beginEditAsset,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  projectionSeries: ProjectionSeriesApi | null;
  anchorDateYmd: string | null;
  calendarTz: string;
  assetModalOpen: boolean;
  closeAssetModal: () => void;
  openNewAssetModal: () => void;
  assets: AssetApiRow[];
  assetsBusy: boolean;
  assetCategories: CategoryRow[];
  assetFormCategoryId: string;
  setAssetFormCategoryId: Dispatch<SetStateAction<string>>;
  assetFormName: string;
  setAssetFormName: Dispatch<SetStateAction<string>>;
  assetFormValue: string;
  setAssetFormValue: Dispatch<SetStateAction<string>>;
  assetFormPurchase: string;
  setAssetFormPurchase: Dispatch<SetStateAction<string>>;
  assetFormLiquid: boolean;
  setAssetFormLiquid: Dispatch<SetStateAction<boolean>>;
  assetFormExpectedReturn: string;
  setAssetFormExpectedReturn: Dispatch<SetStateAction<string>>;
  assetFormNotes: string;
  setAssetFormNotes: Dispatch<SetStateAction<string>>;
  editingAssetId: string | null;
  assetSaving: boolean;
  submitAssetForm: (e: FormEvent) => void;
  deleteAssetRow: (id: string) => void;
  beginEditAsset: (a: AssetApiRow) => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const assetMetricsReady = hasMembership && !assetsBusy;
  const assetsTotalVal = assetMetricsReady
    ? assets.reduce(
        (acc, a) => acc + (parseDisplayDecimal(a.current_value) ?? 0),
        0,
      )
    : null;
  const assetsLiquidVal = assetMetricsReady
    ? assets.reduce((acc, a) => {
        if (!a.is_liquid) return acc;
        return acc + (parseDisplayDecimal(a.current_value) ?? 0);
      }, 0)
    : null;
  const liquidPctParen =
    assetMetricsReady &&
    assetsTotalVal !== null &&
    assetsLiquidVal !== null &&
    assetsTotalVal > 0
      ? formatPercentDisplay((assetsLiquidVal / assetsTotalVal) * 100)
      : undefined;

  const assetCostTotals = assetMetricsReady
    ? assetPortfolioCostTotals(assets)
    : null;
  const assetPnlMoney =
    assetCostTotals !== null
      ? assetCostTotals.currentOnCost - assetCostTotals.cost
      : null;
  const assetPnlPctSigned =
    assetCostTotals !== null && assetCostTotals.cost > 0
      ? (assetCostTotals.currentOnCost / assetCostTotals.cost - 1) * 100
      : null;

  const assetTargetReachMap = useMemo<Map<string, string | null>>(() => {
    const out = new Map<string, string | null>();
    const seriesList = projectionSeries?.asset_series ?? [];
    if (seriesList.length === 0) return out;
    const anchorStr =
      anchorDateYmd != null && anchorDateYmd.trim() !== ""
        ? anchorDateYmd.trim()
        : todayYmdInTimeZone(calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (!anchor) return out;
    for (const a of assets) {
      const target = parseDisplayDecimal(
        String(a.contribution_target_amount ?? ""),
      );
      if (target == null || target <= 0) continue;
      const series = seriesList.find((s) => s.asset_id === a.id);
      if (!series) continue;
      let reachedIndex: number | null = null;
      for (let i = 0; i < series.values.length; i++) {
        const v = parseDisplayDecimal(String(series.values[i] ?? ""));
        if (v != null && v >= target) {
          reachedIndex = i;
          break;
        }
      }
      if (reachedIndex == null) {
        out.set(a.id, null);
        continue;
      }
      const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, reachedIndex);
      out.set(a.id, formatProjectionHoverMonthYear(at));
    }
    return out;
  }, [
    projectionSeries?.asset_series,
    assets,
    anchorDateYmd,
    calendarTz,
  ]);

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Activos</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Valor total"
            value={
              assetMetricsReady && assetsTotalVal !== null
                ? formatCurrencyNumber(assetsTotalVal, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Valor líquido"
            value={
              assetMetricsReady && assetsLiquidVal !== null
                ? formatCurrencyNumber(assetsLiquidVal, currencyIso)
                : METRIC_DASH
            }
            parenthetical={liquidPctParen}
          />
          <MetricCard
            label="PnL vs compra"
            value={
              assetMetricsReady && assetPnlMoney !== null
                ? formatCurrencyNumber(assetPnlMoney, currencyIso)
                : METRIC_DASH
            }
            parenthetical={
              assetPnlPctSigned !== null && Number.isFinite(assetPnlPctSigned)
                ? formatPercentDisplaySigned(assetPnlPctSigned)
                : undefined
            }
          />
        </div>
      ) : null}

      {hasMembership && assetCategories.length === 0 && !assetsBusy ? (
        <div className="banner info-banner">
          <strong>Activos</strong> · <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura.
        </p>
      ) : null}

      {canEdit && hasMembership && assetCategories.length > 0 ? (
        <Modal
          title={editingAssetId ? "Editar activo" : "Nuevo activo"}
          open={assetModalOpen}
          onClose={closeAssetModal}
        >
          <form className="asset-form stack" onSubmit={submitAssetForm}>
            <ModalFormError message={formError} />
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={assetFormCategoryId}
                  onChange={(e) => setAssetFormCategoryId(e.target.value)}
                  required
                >
                  {assetCategories.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Nombre</span>
                <input
                  value={assetFormName}
                  onChange={(e) => setAssetFormName(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Fondo índice"
                />
              </label>
              <label className="field">
                <span>Valor actual</span>
                <input
                  value={assetFormValue}
                  onChange={(e) => setAssetFormValue(e.target.value)}
                  required
                  inputMode="decimal"
                  placeholder="0"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Precio compra (opc.)</span>
                <input
                  value={assetFormPurchase}
                  onChange={(e) => setAssetFormPurchase(e.target.value)}
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
              <label className="field checkbox-field">
                <input
                  type="checkbox"
                  checked={assetFormLiquid}
                  onChange={(e) => setAssetFormLiquid(e.target.checked)}
                />
                <span>Líquido</span>
              </label>
              <label className="field">
                <span>Rentab. anual esperada % (opc.)</span>
                <input
                  value={assetFormExpectedReturn}
                  onChange={(e) =>
                    setAssetFormExpectedReturn(e.target.value)
                  }
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={assetFormNotes}
                onChange={(e) => setAssetFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={assetSaving}
              >
                {editingAssetId ? "Guardar cambios" : "Añadir activo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={assetSaving}
                onClick={() => closeAssetModal()}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      <div className="ledger-list-section">
        <div className="ledger-list-toolbar">
          <div className="panel-head-row">
            <h3 className="panel-title">Activos por categoría</h3>
            {canEdit && hasMembership && assetCategories.length > 0 ? (
              <button
                type="button"
                className="btn primary icon-btn ledger-toolbar-add"
                aria-label="Nuevo activo"
                onClick={() => openNewAssetModal()}
              >
                <PlusIcon />
              </button>
            ) : null}
          </div>
          {assetsBusy ? (
            <p className="muted">Cargando…</p>
          ) : assets.length === 0 ? (
            <p className="muted">
              No hay activos registrados en esta instalación.
            </p>
          ) : null}
        </div>
        {!assetsBusy && assets.length > 0 ? (
          <div className="ledger-by-category-stack">
            {groupRowsByCategoryOrdered(assets, assetCategories, {
              sortRowsDescending: {
                value: (a) => parseDisplayDecimal(a.current_value) ?? 0,
                tieBreak: (a, b) => a.name.localeCompare(b.name, "es"),
              },
              categoryTotalDescending: (items) =>
                items.reduce(
                  (acc, a) =>
                    acc + (parseDisplayDecimal(a.current_value) ?? 0),
                  0,
                ),
            }).map((g) => {
              const catTotalVal = g.items.reduce(
                (acc, a) => acc + (parseDisplayDecimal(a.current_value) ?? 0),
                0,
              );
              const showPurchase = g.items.some((a) => {
                const v = parseDisplayDecimal(String(a.purchase_price ?? ""));
                return v != null && v > 0;
              });
              const showReturn = g.items.some(
                (a) =>
                  a.expected_annual_return_percent != null &&
                  String(a.expected_annual_return_percent).trim() !== "",
              );
              const showContribution = g.items.some(
                (a) => assetContributionMonthlyEstimateNum(a) > 0,
              );
              return (
                <section key={g.categoryId} className="panel ledger-category-panel">
                  <div className="panel-head-row">
                    <h3 className="panel-title">{g.label}</h3>
                    <span className="ledger-category-total">
                      {formatCurrencyNumber(catTotalVal, currencyIso)}
                    </span>
                  </div>
                  <div className="table-scroll bordered-top">
                    <table className="assets-table">
                      <thead>
                        <tr>
                          <th>Nombre</th>
                          <th
                            className="num"
                            title="Valor actual. Cuando una regla de asignación apunta a este activo con un tope en € concreto, se muestra como Actual / Target."
                          >
                            Valor
                          </th>
                          {showPurchase ? <th className="num">Compra</th> : null}
                          {showPurchase ? (
                            <th
                              className="num"
                              title="Variación vs precio de compra (no anualizada)"
                            >
                              Δ compra
                            </th>
                          ) : null}
                          {showReturn ? (
                            <th
                              className="num"
                              title="Rentabilidad anual esperada (proyección)"
                            >
                              Rent. % a.a.
                            </th>
                          ) : null}
                          {showContribution ? (
                            <th
                              className="num"
                              title="Aporte estimado del primer mes (suma de reglas de Presupuesto → Asignación del sobrante que apuntan a este activo). Incluye flujos puntuales de Próximos. Cero si todas las reglas anteriores agotan el sobrante antes de llegar a este activo."
                            >
                              Aporte
                            </th>
                          ) : null}
                          {canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((a) => {
                          const target = parseDisplayDecimal(
                            String(a.contribution_target_amount ?? ""),
                          );
                          const currentVal = parseDisplayDecimal(a.current_value);
                          const targetMet =
                            target != null &&
                            currentVal != null &&
                            currentVal >= target;
                          const targetCompact =
                            target != null && target > 0 && !targetMet
                              ? formatProjectionMilestoneCompactLabel(
                                  String(roundUpToHundred(target)),
                                )
                              : null;
                          const targetReachLabel =
                            assetTargetReachMap.get(a.id) ?? null;
                          return (
                          <tr key={a.id}>
                            <td>{a.name}</td>
                            <td className="num">
                              {targetCompact ? (
                                <span
                                  className="asset-target-tag"
                                  title={
                                    targetReachLabel
                                      ? `Objetivo alcanzado en ${targetReachLabel}`
                                      : undefined
                                  }
                                >
                                  (Obj. {targetCompact}){" "}
                                </span>
                              ) : null}
                              {formatCurrencyAmount(a.current_value, currencyIso)}
                            </td>
                            {showPurchase ? (
                              <td className="num">
                                {formatCurrencyOrDash(
                                  a.purchase_price,
                                  currencyIso,
                                )}
                              </td>
                            ) : null}
                            {showPurchase ? (
                              <td className="num muted">
                                {assetImplicitTotalReturnLabel(
                                  a.current_value,
                                  a.purchase_price,
                                ) ?? METRIC_DASH}
                              </td>
                            ) : null}
                            {showReturn ? (
                              <td className="num muted">
                                {a.expected_annual_return_percent != null &&
                                a.expected_annual_return_percent !== ""
                                  ? formatPercentAmount(
                                      a.expected_annual_return_percent,
                                    )
                                  : METRIC_DASH}
                              </td>
                            ) : null}
                            {showContribution ? (
                              <td className="num muted tight">
                                {formatAssetContributionNominalCell(
                                  a,
                                  currencyIso,
                                )}
                              </td>
                            ) : null}
                            {canEdit ? (
                              <td className="asset-actions-cell">
                                <div className="budget-row-actions">
                                  <button
                                    type="button"
                                    className="btn ghost icon-btn"
                                    aria-label="Editar activo"
                                    disabled={assetSaving}
                                    onClick={() => beginEditAsset(a)}
                                  >
                                    <RowEditIcon />
                                  </button>
                                  <button
                                    type="button"
                                    className="btn ghost danger icon-btn"
                                    aria-label="Eliminar activo"
                                    disabled={assetSaving}
                                    onClick={() => deleteAssetRow(a.id)}
                                  >
                                    <RowTrashIcon />
                                  </button>
                                </div>
                              </td>
                            ) : null}
                          </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                </section>
              );
            })}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function LiabilitiesView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  liabilityModalOpen,
  closeLiabilityModal,
  openNewLiabilityModal,
  liabilities,
  liabilitiesBusy,
  liabilityCategories,
  liabilityFormCategoryId,
  setLiabilityFormCategoryId,
  liabilityFormLabel,
  setLiabilityFormLabel,
  liabilityFormTypeTag,
  setLiabilityFormTypeTag,
  liabilityFormPrincipal,
  setLiabilityFormPrincipal,
  liabilityFormApr,
  setLiabilityFormApr,
  liabilityFormPaymentAmount,
  setLiabilityFormPaymentAmount,
  liabilityFormPaymentFrequency,
  setLiabilityFormPaymentFrequency,
  liabilityFormPaymentEnd,
  setLiabilityFormPaymentEnd,
  liabilityFormNotes,
  setLiabilityFormNotes,
  liabilityFormDerivePrincipal,
  setLiabilityFormDerivePrincipal,
  editingLiabilityId,
  liabilitySaving,
  submitLiabilityForm,
  deleteLiabilityRow,
  beginEditLiability,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  liabilityModalOpen: boolean;
  closeLiabilityModal: () => void;
  openNewLiabilityModal: () => void;
  liabilities: LiabilityApiRow[];
  liabilitiesBusy: boolean;
  liabilityCategories: CategoryRow[];
  liabilityFormCategoryId: string;
  setLiabilityFormCategoryId: Dispatch<SetStateAction<string>>;
  liabilityFormLabel: string;
  setLiabilityFormLabel: Dispatch<SetStateAction<string>>;
  liabilityFormTypeTag: string;
  setLiabilityFormTypeTag: Dispatch<SetStateAction<string>>;
  liabilityFormPrincipal: string;
  setLiabilityFormPrincipal: Dispatch<SetStateAction<string>>;
  liabilityFormApr: string;
  setLiabilityFormApr: Dispatch<SetStateAction<string>>;
  liabilityFormPaymentAmount: string;
  setLiabilityFormPaymentAmount: Dispatch<SetStateAction<string>>;
  liabilityFormPaymentFrequency: LiabilityPaymentFreq;
  setLiabilityFormPaymentFrequency: Dispatch<
    SetStateAction<LiabilityPaymentFreq>
  >;
  liabilityFormPaymentEnd: string;
  setLiabilityFormPaymentEnd: Dispatch<SetStateAction<string>>;
  liabilityFormNotes: string;
  setLiabilityFormNotes: Dispatch<SetStateAction<string>>;
  liabilityFormDerivePrincipal: boolean;
  setLiabilityFormDerivePrincipal: Dispatch<SetStateAction<boolean>>;
  editingLiabilityId: string | null;
  liabilitySaving: boolean;
  submitLiabilityForm: (e: FormEvent) => void;
  deleteLiabilityRow: (id: string) => void;
  beginEditLiability: (row: LiabilityApiRow) => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const derivePreview = liabilityDerivedPrincipalPreview(
    liabilityFormPaymentAmount,
    liabilityFormPaymentFrequency,
    liabilityFormPaymentEnd,
    installation?.installation.calendar_tz ?? "UTC",
    currencyIso,
  );

  const liabilityMetricsReady = hasMembership && !liabilitiesBusy;
  const liabilityPrincipalSum = liabilityMetricsReady
    ? liabilities.reduce(
        (acc, r) => acc + (parseDisplayDecimal(r.principal) ?? 0),
        0,
      )
    : null;
  const liabilitiesMonthlyServiceSum = liabilityMetricsReady
    ? liabilities.reduce(
        (acc, r) => acc + liabilityPaymentMonthlyEquivalentNum(r),
        0,
      )
    : null;

  const liabilitiesWeightedApr = liabilityMetricsReady
    ? liabilitiesWeightedAprPercent(liabilities)
    : null;

  const liabilitiesApproxMonthlyInterest = liabilityMetricsReady
    ? liabilitiesApproxMonthlyInterestSum(liabilities)
    : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Pasivos</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Principal total"
            value={
              liabilityMetricsReady && liabilityPrincipalSum !== null
                ? formatCurrencyNumber(liabilityPrincipalSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Servicio mensual equivalente"
            value={
              liabilityMetricsReady && liabilitiesMonthlyServiceSum !== null
                ? formatCurrencyNumber(
                    liabilitiesMonthlyServiceSum,
                    currencyIso,
                  )
                : METRIC_DASH
            }
          />
          <MetricCard
            label="TAE media ponderada"
            value={
              liabilityMetricsReady && liabilitiesWeightedApr !== null
                ? formatPercentDisplay(liabilitiesWeightedApr)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Interés mensual aprox."
            value={
              liabilityMetricsReady && liabilitiesApproxMonthlyInterest !== null
                ? formatCurrencyNumber(
                    liabilitiesApproxMonthlyInterest,
                    currencyIso,
                  )
                : METRIC_DASH
            }
          />
        </div>
      ) : null}

      {hasMembership && liabilityCategories.length === 0 && !liabilitiesBusy ? (
        <div className="banner info-banner">
          <strong>Pasivos</strong> · <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura.
        </p>
      ) : null}

      {canEdit && hasMembership && liabilityCategories.length > 0 ? (
        <Modal
          title={editingLiabilityId ? "Editar pasivo" : "Nuevo pasivo"}
          open={liabilityModalOpen}
          onClose={closeLiabilityModal}
        >
          <form
            className="asset-form stack"
            onSubmit={submitLiabilityForm}
          >
            <ModalFormError message={formError} />
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={liabilityFormCategoryId}
                  onChange={(e) =>
                    setLiabilityFormCategoryId(e.target.value)
                  }
                  required
                >
                  {liabilityCategories.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Etiqueta</span>
                <input
                  value={liabilityFormLabel}
                  onChange={(e) => setLiabilityFormLabel(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Préstamo coche"
                />
              </label>
              <label className="field">
                <span>Tipo (opc.)</span>
                <input
                  value={liabilityFormTypeTag}
                  onChange={(e) => setLiabilityFormTypeTag(e.target.value)}
                  maxLength={120}
                  placeholder="Etiqueta libre"
                />
              </label>
              <label
                className="field"
                style={{
                  gridColumn: "1 / -1",
                  flexDirection: "row",
                  alignItems: "flex-start",
                  gap: "0.5rem",
                }}
              >
                <input
                  type="checkbox"
                  checked={liabilityFormDerivePrincipal}
                  onChange={(e) =>
                    setLiabilityFormDerivePrincipal(e.target.checked)
                  }
                  style={{ marginTop: "0.2rem" }}
                />
                <span className="checkbox-label-with-hint">
                  Derivar principal desde el plan
                  <InlineHint title="Hoy civil = zona Calendario. Semanal ≈ días÷7." />
                </span>
              </label>
              <label className="field">
                <span>
                  Principal
                  {liabilityFormDerivePrincipal ? " (calculado al guardar)" : ""}
                </span>
                <input
                  value={liabilityFormPrincipal}
                  onChange={(e) => setLiabilityFormPrincipal(e.target.value)}
                  required={!liabilityFormDerivePrincipal}
                  disabled={liabilityFormDerivePrincipal}
                  inputMode="decimal"
                  autoComplete="off"
                />
                {liabilityFormDerivePrincipal && derivePreview ? (
                  <span className="muted tight">
                    Vista previa ~{derivePreview} (hoy en{" "}
                    {installation?.installation.calendar_tz ?? "UTC"})
                  </span>
                ) : null}
              </label>
              <label className="field">
                <span>TAE % (opc.)</span>
                <input
                  value={liabilityFormApr}
                  onChange={(e) => setLiabilityFormApr(e.target.value)}
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>
                  Cuota plan
                  {liabilityFormDerivePrincipal ? "" : " (opc.)"}
                </span>
                <input
                  value={liabilityFormPaymentAmount}
                  onChange={(e) =>
                    setLiabilityFormPaymentAmount(e.target.value)
                  }
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Frecuencia</span>
                <select
                  value={liabilityFormPaymentFrequency}
                  onChange={(e) =>
                    setLiabilityFormPaymentFrequency(
                      e.target.value as LiabilityPaymentFreq,
                    )
                  }
                >
                  <option value="">Sin plan</option>
                  <option value="monthly">Mensual</option>
                  <option value="weekly">Semanal</option>
                </select>
              </label>
              <label className="field">
                <span>
                  Fin plan
                  {liabilityFormDerivePrincipal ? "" : " (opc.)"}
                </span>
                <input
                  type="date"
                  value={liabilityFormPaymentEnd}
                  onChange={(e) =>
                    setLiabilityFormPaymentEnd(e.target.value)
                  }
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={liabilityFormNotes}
                onChange={(e) => setLiabilityFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={liabilitySaving}
              >
                {editingLiabilityId ? "Guardar cambios" : "Añadir pasivo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={liabilitySaving}
                onClick={() => closeLiabilityModal()}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      <div className="ledger-list-section">
        <div className="ledger-list-toolbar">
          <div className="panel-head-row">
            <h3 className="panel-title">Pasivos por categoría</h3>
            {canEdit && hasMembership && liabilityCategories.length > 0 ? (
              <button
                type="button"
                className="btn primary icon-btn ledger-toolbar-add"
                aria-label="Nuevo pasivo"
                onClick={() => openNewLiabilityModal()}
              >
                <PlusIcon />
              </button>
            ) : null}
          </div>
          {liabilitiesBusy ? (
            <p className="muted">Cargando…</p>
          ) : liabilities.length === 0 ? (
            <p className="muted">No hay pasivos en esta instalación.</p>
          ) : null}
        </div>
        {!liabilitiesBusy && liabilities.length > 0 ? (
          <div className="ledger-by-category-stack">
            {groupRowsByCategoryOrdered(liabilities, liabilityCategories, {
              sortRowsDescending: {
                value: (row) => parseDisplayDecimal(row.principal) ?? 0,
                tieBreak: (a, b) => a.label.localeCompare(b.label, "es"),
              },
              categoryTotalDescending: (items) =>
                items.reduce(
                  (acc, row) =>
                    acc + (parseDisplayDecimal(row.principal) ?? 0),
                  0,
                ),
            }).map((g) => {
              const catPrincipal = g.items.reduce(
                (acc, row) =>
                  acc + (parseDisplayDecimal(row.principal) ?? 0),
                0,
              );
              return (
                <section key={g.categoryId} className="panel ledger-category-panel">
                  <div className="panel-head-row">
                    <h3 className="panel-title">{g.label}</h3>
                    <span className="ledger-category-total">
                      {formatCurrencyNumber(catPrincipal, currencyIso)}
                    </span>
                  </div>
                  <div className="table-scroll bordered-top">
                    <table className="assets-table">
                      <thead>
                        <tr>
                          <th>Etiqueta</th>
                          <th>Tipo</th>
                          <th className="num">Principal</th>
                          <th className="num">TAE %</th>
                          <th className="num">Cuota</th>
                          <th>Frec.</th>
                          <th>Fin plan</th>
                          {canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((row) => (
                          <tr key={row.id}>
                            <td>{row.label}</td>
                            <td>{row.type_tag ?? METRIC_DASH}</td>
                            <td className="num">
                              {formatCurrencyAmount(row.principal, currencyIso)}
                              {row.principal_derived_from_plan ? (
                                <span
                                  className="muted"
                                  title="Principal derivado del plan"
                                >
                                  {" "}
                                  deriv.
                                </span>
                              ) : null}
                            </td>
                            <td className="num">
                              {row.apr_percent != null &&
                              String(row.apr_percent).trim() !== ""
                                ? formatPercentAmount(row.apr_percent)
                                : METRIC_DASH}
                            </td>
                            <td className="num">
                              {formatCurrencyOrDash(
                                row.payment_amount,
                                currencyIso,
                              )}
                            </td>
                            <td>
                              {row.payment_frequency
                                ? PAYMENT_FREQ_LABEL[row.payment_frequency]
                                : METRIC_DASH}
                            </td>
                            <td>{row.payment_end_date ?? METRIC_DASH}</td>
                            {canEdit ? (
                              <td className="asset-actions-cell">
                                <div className="budget-row-actions">
                                  <button
                                    type="button"
                                    className="btn ghost icon-btn"
                                    aria-label="Editar pasivo"
                                    disabled={liabilitySaving}
                                    onClick={() => beginEditLiability(row)}
                                  >
                                    <RowEditIcon />
                                  </button>
                                  <button
                                    type="button"
                                    className="btn ghost danger icon-btn"
                                    aria-label="Eliminar pasivo"
                                    disabled={liabilitySaving}
                                    onClick={() => deleteLiabilityRow(row.id)}
                                  >
                                    <RowTrashIcon />
                                  </button>
                                </div>
                              </td>
                            ) : null}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </section>
              );
            })}
          </div>
        ) : null}
      </div>
    </div>
  );
}

const BUDGET_SCOPE_LABEL: Record<BudgetScopeToggle, string> = {
  income: "Ingreso",
  expense: "Gasto",
};

const PLANNING_DIRECTION_LABEL: Record<PlanningFlowDirectionApi, string> = {
  inflow: "Entrada",
  outflow: "Salida",
};

function budgetDerivedCatLabel(categories: CategoryRow[], id: string): string {
  return categories.find((x) => x.id === id)?.name ?? id.slice(0, 8);
}

function BudgetView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  budgetModalOpen,
  closeBudgetModal,
  openNewBudgetModal,
  budgetSnapshot,
  budgetLoading,
  budgetIncomeCategories,
  budgetExpenseCategories,
  budgetLiabilityCategories,
  budgetFormScope,
  setBudgetFormScope,
  budgetFormCategoryId,
  setBudgetFormCategoryId,
  budgetFormAmount,
  setBudgetFormAmount,
  budgetFormNotes,
  setBudgetFormNotes,
  budgetFormPersistsAfterRetirement,
  setBudgetFormPersistsAfterRetirement,
  budgetFormExpenseEndType,
  setBudgetFormExpenseEndType,
  budgetFormExpenseEndDate,
  setBudgetFormExpenseEndDate,
  editingBudgetEntryId,
  budgetSaving,
  submitBudgetForm,
  deleteBudgetEntryRow,
  beginEditBudgetEntry,
  assets,
  allocationRules,
  allocationRulesBusy,
  allocationRulesError,
  allocationPanelOpen,
  openAllocationPanel,
  closeAllocationPanel,
  ruleModalOpen,
  openNewRuleModal,
  closeRuleModal,
  ruleFormTargetAsset,
  setRuleFormTargetAsset,
  ruleFormKind,
  setRuleFormKind,
  ruleFormAmount,
  setRuleFormAmount,
  ruleFormCapKind,
  setRuleFormCapKind,
  ruleFormCapValue,
  setRuleFormCapValue,
  editingRuleId,
  ruleSaving,
  submitRuleForm,
  deleteRule,
  moveRule,
  beginEditRule,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  budgetModalOpen: boolean;
  closeBudgetModal: () => void;
  openNewBudgetModal: (scope?: BudgetScopeToggle) => void;
  budgetSnapshot: BudgetSnapshotApi | null;
  budgetLoading: boolean;
  budgetIncomeCategories: CategoryRow[];
  budgetExpenseCategories: CategoryRow[];
  budgetLiabilityCategories: CategoryRow[];
  budgetFormScope: BudgetScopeToggle;
  setBudgetFormScope: Dispatch<SetStateAction<BudgetScopeToggle>>;
  budgetFormCategoryId: string;
  setBudgetFormCategoryId: Dispatch<SetStateAction<string>>;
  budgetFormAmount: string;
  setBudgetFormAmount: Dispatch<SetStateAction<string>>;
  budgetFormNotes: string;
  setBudgetFormNotes: Dispatch<SetStateAction<string>>;
  budgetFormPersistsAfterRetirement: boolean;
  setBudgetFormPersistsAfterRetirement: Dispatch<SetStateAction<boolean>>;
  budgetFormExpenseEndType: "never" | "retirement" | "date";
  setBudgetFormExpenseEndType: Dispatch<SetStateAction<"never" | "retirement" | "date">>;
  budgetFormExpenseEndDate: string;
  setBudgetFormExpenseEndDate: Dispatch<SetStateAction<string>>;
  editingBudgetEntryId: string | null;
  budgetSaving: boolean;
  submitBudgetForm: (e: FormEvent) => void;
  deleteBudgetEntryRow: (id: string) => void;
  beginEditBudgetEntry: (row: BudgetEntryApiRow) => void;
  assets: AssetApiRow[];
  allocationRules: AllocationRuleApiRow[];
  allocationRulesBusy: boolean;
  allocationRulesError: string | null;
  allocationPanelOpen: boolean;
  openAllocationPanel: () => void;
  closeAllocationPanel: () => void;
  ruleModalOpen: boolean;
  openNewRuleModal: () => void;
  closeRuleModal: () => void;
  ruleFormTargetAsset: string;
  setRuleFormTargetAsset: Dispatch<SetStateAction<string>>;
  ruleFormKind: AllocationRuleKind;
  setRuleFormKind: Dispatch<SetStateAction<AllocationRuleKind>>;
  ruleFormAmount: string;
  setRuleFormAmount: Dispatch<SetStateAction<string>>;
  ruleFormCapKind: "none" | AllocationRuleCapKind;
  setRuleFormCapKind: Dispatch<SetStateAction<"none" | AllocationRuleCapKind>>;
  ruleFormCapValue: string;
  setRuleFormCapValue: Dispatch<SetStateAction<string>>;
  editingRuleId: string | null;
  ruleSaving: boolean;
  submitRuleForm: (e: FormEvent) => void;
  deleteRule: (id: string) => void;
  moveRule: (id: string, dir: "up" | "down") => void;
  beginEditRule: (r: AllocationRuleApiRow) => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const categoryMapForSort = budgetCategoryMap(
    budgetIncomeCategories,
    budgetExpenseCategories,
  );

  const budgetEntriesRaw = Array.isArray(budgetSnapshot?.entries)
    ? budgetSnapshot.entries
    : [];

  const sortedEntries =
    budgetSnapshot && !budgetLoading
      ? sortBudgetEntriesMacStyle(budgetEntriesRaw, categoryMapForSort)
      : [];

  const derivedLines = budgetSnapshot?.derived_from_liabilities ?? [];

  const formCats =
    budgetFormScope === "income"
      ? budgetIncomeCategories
      : budgetExpenseCategories;

  const snap =
    hasMembership && !budgetLoading && budgetSnapshot !== null
      ? budgetSnapshot
      : null;

  const incTot = snap
    ? formatCurrencyOrDash(snap.totals?.income_monthly_equivalent, currencyIso)
    : METRIC_DASH;
  const expTot = snap
    ? formatCurrencyOrDash(
        snap.totals?.expense_total_monthly_equivalent,
        currencyIso,
      )
    : METRIC_DASH;
  const netM = snap
    ? formatCurrencyOrDash(snap.totals?.net_monthly_equivalent, currencyIso)
    : METRIC_DASH;

  const incomeEntries = sortedEntries.filter((e) => e.scope === "income");
  const expenseEntries = sortedEntries.filter((e) => e.scope === "expense");

  return (
    <div className="workspace budget-page">
      <div className="workspace-header">
        <h2 className="workspace-title">Presupuesto</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : budgetLoading
                ? "Cargando…"
                : `Mensual · ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership &&
      budgetIncomeCategories.length === 0 &&
      budgetExpenseCategories.length === 0 &&
      !budgetLoading ? (
        <div className="banner info-banner">
          <strong>Ingresos/Gastos</strong> ·{" "}
          <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {hasMembership ? (
        <div
          className="metric-grid workspace-kpi-strip metric-grid--budget-summary"
          aria-label="Resumen del presupuesto"
        >
          <MetricCard label="Ingresos totales" value={incTot} />
          <MetricCard label="Gastos totales" value={expTot} />
          <MetricCard
            label="Neto"
            value={netM}
            action={
              <button
                type="button"
                className="btn ghost icon-btn metric-card__action-btn"
                onClick={openAllocationPanel}
                aria-label="Asignación del sobrante"
                title={`Asignación del sobrante · ${allocationRules.length} ${allocationRules.length === 1 ? "regla" : "reglas"}`}
              >
                <GearIcon />
              </button>
            }
          />
        </div>
      ) : null}

      {hasMembership ? (
        <Modal
          title="Asignación del sobrante"
          open={allocationPanelOpen}
          onClose={closeAllocationPanel}
          wide
        >
          <AllocationRulesPanel
            assets={assets}
            rules={allocationRules}
            busy={allocationRulesBusy}
            error={allocationRulesError}
            canEdit={canEdit}
            currencyIso={currencyIso}
            openNewRuleModal={openNewRuleModal}
            beginEditRule={beginEditRule}
            deleteRule={deleteRule}
            moveRule={moveRule}
            embedded
          />
        </Modal>
      ) : null}

      {canEdit && hasMembership ? (
        <Modal
          title={editingRuleId ? "Editar regla" : "Nueva regla de asignación"}
          open={ruleModalOpen}
          onClose={closeRuleModal}
        >
          <form className="asset-form stack" onSubmit={submitRuleForm}>
            <ModalFormError message={allocationRulesError} />
            <label className="field">
              <span>Destino (activo)</span>
              <select
                value={ruleFormTargetAsset}
                onChange={(e) => setRuleFormTargetAsset(e.target.value)}
                required
                disabled={assets.length === 0}
              >
                {assets.length === 0 ? (
                  <option value="">— Sin activos —</option>
                ) : null}
                {assets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="asset-form-grid">
              <label className="field">
                <span>Tipo</span>
                <select
                  value={ruleFormKind}
                  onChange={(e) =>
                    setRuleFormKind(e.target.value as AllocationRuleKind)
                  }
                >
                  <option value="fixed">Cantidad fija €/mes</option>
                  <option value="percent">% del sobrante restante</option>
                  <option value="remainder">Resto (lo que quede)</option>
                </select>
              </label>
              {ruleFormKind !== "remainder" ? (
                <label className="field">
                  <span>
                    {ruleFormKind === "fixed" ? "Importe €/mes" : "Porcentaje"}
                  </span>
                  <input
                    value={ruleFormAmount}
                    onChange={(e) => setRuleFormAmount(e.target.value)}
                    inputMode="decimal"
                    placeholder="0"
                    required
                    autoComplete="off"
                  />
                </label>
              ) : null}
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Tope opcional</span>
                <select
                  value={ruleFormCapKind}
                  onChange={(e) =>
                    setRuleFormCapKind(
                      e.target.value as "none" | AllocationRuleCapKind,
                    )
                  }
                >
                  <option value="none">Sin tope</option>
                  <option value="amount">Cantidad fija €</option>
                  <option value="months_expense">N × gasto mensual</option>
                  <option value="income_multiple">N × ingreso mensual</option>
                </select>
              </label>
              {ruleFormCapKind !== "none" ? (
                <label className="field">
                  <span>
                    {ruleFormCapKind === "amount"
                      ? "Tope en €"
                      : ruleFormCapKind === "months_expense"
                        ? "N meses de gasto"
                        : "N múltiplo de ingreso"}
                  </span>
                  <input
                    value={ruleFormCapValue}
                    onChange={(e) => setRuleFormCapValue(e.target.value)}
                    inputMode="decimal"
                    placeholder="0"
                    required
                    autoComplete="off"
                  />
                </label>
              ) : null}
            </div>
            <p className="muted tight">
              {ruleFormKind === "remainder" && ruleFormCapKind === "none"
                ? "Esta regla absorberá todo lo que quede del sobrante. Se coloca automáticamente al final del orden; solo puede haber una por usuario."
                : ruleFormKind === "remainder"
                  ? "Esta regla absorbe lo que quede hasta su tope. Se inserta antes del resto sin tope."
                  : ruleFormKind === "fixed"
                    ? "Se aporta esta cantidad fija mensual antes de seguir la cascada. Se inserta antes del resto sin tope."
                    : "Se aporta este % sobre lo que quede del sobrante en este paso de la cascada (no del sobrante total)."}
            </p>
            <div className="asset-form-actions">
              <button type="submit" className="btn primary" disabled={ruleSaving}>
                {editingRuleId ? "Guardar cambios" : "Añadir regla"}
              </button>
              <button
                type="button"
                className="btn ghost"
                onClick={closeRuleModal}
                disabled={ruleSaving}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura.
        </p>
      ) : null}

      {canEdit &&
      hasMembership &&
      (budgetIncomeCategories.length > 0 ||
        budgetExpenseCategories.length > 0) ? (
        <Modal
          title={
            editingBudgetEntryId
              ? "Editar línea de presupuesto"
              : "Nueva línea de presupuesto"
          }
          open={budgetModalOpen}
          onClose={closeBudgetModal}
        >
          <form className="asset-form stack" onSubmit={submitBudgetForm}>
            <ModalFormError message={formError} />
            <div className="segmented" role="tablist" aria-label="Ámbito">
              <button
                type="button"
                role="tab"
                aria-selected={budgetFormScope === "income"}
                className={budgetFormScope === "income" ? "active" : ""}
                onClick={() => setBudgetFormScope("income")}
              >
                Ingreso
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={budgetFormScope === "expense"}
                className={budgetFormScope === "expense" ? "active" : ""}
                onClick={() => setBudgetFormScope("expense")}
              >
                Gasto
              </button>
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría ({BUDGET_SCOPE_LABEL[budgetFormScope]})</span>
                <select
                  value={budgetFormCategoryId}
                  onChange={(e) => setBudgetFormCategoryId(e.target.value)}
                  required
                  disabled={formCats.length === 0}
                >
                  {formCats.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Importe mensual</span>
                <input
                  value={budgetFormAmount}
                  onChange={(e) => setBudgetFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={budgetFormNotes}
                onChange={(e) => setBudgetFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            {budgetFormScope === "income" ? (
              <label className="field field--checkbox">
                <input
                  type="checkbox"
                  checked={budgetFormPersistsAfterRetirement}
                  onChange={(e) => setBudgetFormPersistsAfterRetirement(e.target.checked)}
                />
                <span>Persiste tras jubilación</span>
              </label>
            ) : (
              <>
                <label className="field">
                  <span>Fin del gasto</span>
                  <select
                    value={budgetFormExpenseEndType}
                    onChange={(e) =>
                      setBudgetFormExpenseEndType(
                        e.target.value as "never" | "retirement" | "date"
                      )
                    }
                  >
                    <option value="never">Sin fecha de fin</option>
                    <option value="retirement">Al jubilarse</option>
                    <option value="date">Hasta la fecha…</option>
                  </select>
                </label>
                {budgetFormExpenseEndType === "date" && (
                  <label className="field">
                    <span>Fecha de fin</span>
                    <input
                      type="date"
                      value={budgetFormExpenseEndDate}
                      onChange={(e) => setBudgetFormExpenseEndDate(e.target.value)}
                      required
                    />
                  </label>
                )}
              </>
            )}
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={budgetSaving || formCats.length === 0}
              >
                {editingBudgetEntryId ? "Guardar cambios" : "Añadir línea"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={budgetSaving}
                onClick={() => closeBudgetModal()}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {budgetLoading ? (
        <section className="panel">
          <h3 className="panel-title">Detalle</h3>
          <p className="muted bordered-top">
            Cargando líneas de presupuesto…
          </p>
        </section>
      ) : (
        <div className="budget-two-col">
          <section className="panel budget-col">
            <div className="panel-head-row">
              <h3 className="panel-title">Ingresos</h3>
              {canEdit &&
              hasMembership &&
              budgetIncomeCategories.length > 0 ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nueva línea de ingreso"
                  onClick={() => openNewBudgetModal("income")}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
            {incomeEntries.length === 0 ? (
              <p className="muted bordered-top">
                No hay líneas de ingreso en el presupuesto.
              </p>
            ) : (
              <div className="table-scroll table-scroll--budget-lines bordered-top">
                <table className="assets-table assets-table--budget-lines">
                  <thead>
                    <tr>
                      <th>Categoría</th>
                      <th className="num">Importe mensual</th>
                      {canEdit ? (
                        <th className="asset-actions-cell">
                          <span className="sr-only">Acciones</span>
                        </th>
                      ) : null}
                    </tr>
                  </thead>
                  <tbody>
                    {incomeEntries.map((row) => (
                      <tr key={row.id}>
                        <td>
                          {categoryMapForSort.get(row.category_id)?.name ??
                            row.category_id.slice(0, 8)}
                        </td>
                        <td className="num">
                          {formatCurrencyAmount(row.amount, currencyIso)}
                        </td>
                        {canEdit ? (
                          <td className="asset-actions-cell">
                            <div className="budget-row-actions">
                              <button
                                type="button"
                                className="btn ghost icon-btn"
                                aria-label="Editar línea"
                                disabled={budgetSaving}
                                onClick={() => beginEditBudgetEntry(row)}
                              >
                                <RowEditIcon />
                              </button>
                              <button
                                type="button"
                                className="btn ghost danger icon-btn"
                                aria-label="Eliminar línea"
                                disabled={budgetSaving}
                                onClick={() => deleteBudgetEntryRow(row.id)}
                              >
                                <RowTrashIcon />
                              </button>
                            </div>
                          </td>
                        ) : null}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <div className="budget-expenses-column">
            <section className="panel budget-col">
              <div className="panel-head-row">
                <h3 className="panel-title">Gastos</h3>
                {canEdit &&
                hasMembership &&
                budgetExpenseCategories.length > 0 ? (
                  <button
                    type="button"
                    className="btn primary icon-btn ledger-toolbar-add"
                    aria-label="Nueva línea de gasto"
                    onClick={() => openNewBudgetModal("expense")}
                  >
                    <PlusIcon />
                  </button>
                ) : null}
              </div>
              {expenseEntries.length === 0 ? (
                <p className="muted bordered-top">
                  No hay líneas de gasto recurrentes en el presupuesto.
                </p>
              ) : (
                <div className="table-scroll table-scroll--budget-lines bordered-top">
                  <table className="assets-table assets-table--budget-lines">
                    <thead>
                      <tr>
                        <th>Categoría</th>
                        <th className="num">Importe mensual</th>
                        {canEdit ? (
                          <th className="asset-actions-cell">
                            <span className="sr-only">Acciones</span>
                          </th>
                        ) : null}
                      </tr>
                    </thead>
                    <tbody>
                      {expenseEntries.map((row) => (
                        <tr key={row.id}>
                          <td>
                            {categoryMapForSort.get(row.category_id)?.name ??
                              row.category_id.slice(0, 8)}
                          </td>
                          <td className="num">
                            {formatCurrencyAmount(row.amount, currencyIso)}
                          </td>
                          {canEdit ? (
                            <td className="asset-actions-cell">
                              <div className="budget-row-actions">
                                <button
                                  type="button"
                                  className="btn ghost icon-btn"
                                  aria-label="Editar línea"
                                  disabled={budgetSaving}
                                  onClick={() => beginEditBudgetEntry(row)}
                                >
                                  <RowEditIcon />
                                </button>
                                <button
                                  type="button"
                                  className="btn ghost danger icon-btn"
                                  aria-label="Eliminar línea"
                                  disabled={budgetSaving}
                                  onClick={() => deleteBudgetEntryRow(row.id)}
                                >
                                  <RowTrashIcon />
                                </button>
                              </div>
                            </td>
                          ) : null}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <section className="panel budget-col">
              <h3 className="panel-title">Derivado de pasivos</h3>
              {derivedLines.length === 0 ? (
                <p className="muted bordered-top">
                  No hay cuotas derivadas en este momento.
                </p>
              ) : (
                <div className="table-scroll bordered-top">
                  <table className="assets-table">
                    <thead>
                      <tr>
                        <th>Concepto</th>
                        <th>Categoría pasivo</th>
                        <th className="num">Cuota</th>
                        <th>Frec.</th>
                        <th className="num">Equiv. mensual</th>
                      </tr>
                    </thead>
                    <tbody>
                      {derivedLines.map((row) => (
                        <tr key={row.liability_id}>
                          <td>{row.label}</td>
                          <td>
                            {budgetDerivedCatLabel(
                              budgetLiabilityCategories,
                              row.category_id,
                            )}
                          </td>
                          <td className="num">
                            {formatCurrencyAmount(row.amount, currencyIso)}
                          </td>
                          <td>{PAYMENT_FREQ_LABEL[row.frequency]}</td>
                          <td className="num">
                            {formatCurrencyAmount(
                              row.monthly_equivalent,
                              currencyIso,
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </div>
        </div>
      )}
    </div>
  );
}

function PlanningDirectionChart({
  inflow,
  outflow,
}: {
  inflow: number;
  outflow: number;
}) {
  const sum = inflow + outflow;
  if (!(sum > 0) || !(Number.isFinite(inflow) && Number.isFinite(outflow))) {
    return null;
  }
  const wi = (inflow / sum) * 100;
  const wo = (outflow / sum) * 100;
  return (
    <div className="planning-dir-chart bordered-top">
      <svg
        viewBox="0 0 100 12"
        preserveAspectRatio="none"
        className="planning-dir-svg"
        role="img"
        aria-label="Comparación entradas y salidas planificadas"
      >
        <title>Entradas y salidas</title>
        <rect x="0" y="0" width={wi} height="12" className="planning-dir-bar-in" />
        <rect
          x={wi}
          y="0"
          width={wo}
          height="12"
          className="planning-dir-bar-out"
        />
      </svg>
    </div>
  );
}

function UpcomingView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  planningModalOpen,
  closePlanningModal,
  openNewPlanningModal,
  planningFlows,
  planningLoading,
  planningIncomeCategories,
  planningExpenseCategories,
  planningFormScope,
  setPlanningFormScope,
  planningFormCategoryId,
  setPlanningFormCategoryId,
  planningFormTitle,
  setPlanningFormTitle,
  planningFormAmount,
  setPlanningFormAmount,
  planningFormDue,
  setPlanningFormDue,
  planningFormNotes,
  setPlanningFormNotes,
  planningFormShowInChart,
  setPlanningFormShowInChart,
  editingPlanningFlowId,
  planningSaving,
  submitPlanningFlowForm,
  deletePlanningFlowRow,
  beginEditPlanningFlow,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  planningModalOpen: boolean;
  closePlanningModal: () => void;
  openNewPlanningModal: () => void;
  planningFlows: PlanningFlowApiRow[];
  planningLoading: boolean;
  planningIncomeCategories: CategoryRow[];
  planningExpenseCategories: CategoryRow[];
  planningFormScope: BudgetScopeToggle;
  setPlanningFormScope: Dispatch<SetStateAction<BudgetScopeToggle>>;
  planningFormCategoryId: string;
  setPlanningFormCategoryId: Dispatch<SetStateAction<string>>;
  planningFormTitle: string;
  setPlanningFormTitle: Dispatch<SetStateAction<string>>;
  planningFormAmount: string;
  setPlanningFormAmount: Dispatch<SetStateAction<string>>;
  planningFormDue: string;
  setPlanningFormDue: Dispatch<SetStateAction<string>>;
  planningFormNotes: string;
  setPlanningFormNotes: Dispatch<SetStateAction<string>>;
  planningFormShowInChart: boolean;
  setPlanningFormShowInChart: Dispatch<SetStateAction<boolean>>;
  editingPlanningFlowId: string | null;
  planningSaving: boolean;
  submitPlanningFlowForm: (e: FormEvent) => void;
  deletePlanningFlowRow: (id: string) => void;
  beginEditPlanningFlow: (row: PlanningFlowApiRow) => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;
  const categoryById = budgetCategoryMap(
    planningIncomeCategories,
    planningExpenseCategories,
  );

  const formCats =
    planningFormScope === "income"
      ? planningIncomeCategories
      : planningExpenseCategories;

  const planningInflowSum = planningFlows
    .filter((f) => f.direction === "inflow")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);
  const planningOutflowSum = planningFlows
    .filter((f) => f.direction === "outflow")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);

  const planningWorkspaceSub = installationBusy
    ? "Cargando…"
    : !hasMembership
      ? "Sin acceso hasta aprobación."
      : planningLoading
        ? "Cargando…"
        : `Importes · ${currency}`;

  const flowsNetTotal = !planningLoading
    ? planningFlows.reduce((acc, f) => {
        const amt = parseDisplayDecimal(f.expected_amount) ?? 0;
        return (
          acc +
          (f.direction === "inflow"
            ? amt
            : f.direction === "outflow"
              ? -amt
              : 0)
        );
      }, 0)
    : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Próximos</h2>
        <p className="workspace-sub">{planningWorkspaceSub}</p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {hasMembership ? (
        <div className="metric-grid workspace-kpi-strip planning-direction-strip">
          <MetricCard
            label="Entradas (suma)"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningInflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Salidas (suma)"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningOutflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Neto planificado"
            value={
              flowsNetTotal !== null
                ? formatCurrencyNumber(flowsNetTotal, currencyIso)
                : METRIC_DASH
            }
          />
        </div>
      ) : null}

      {hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Distribución</h3>
          {planningLoading ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : planningFlows.length === 0 ? (
            <p className="muted bordered-top">Sin datos.</p>
          ) : planningInflowSum + planningOutflowSum > 0 ? (
            <PlanningDirectionChart
              inflow={planningInflowSum}
              outflow={planningOutflowSum}
            />
          ) : (
            <p className="muted bordered-top">Sin proporción.</p>
          )}
        </section>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership &&
      planningIncomeCategories.length === 0 &&
      planningExpenseCategories.length === 0 &&
      !planningLoading ? (
        <div className="banner info-banner">
          <strong>Ingresos/Gastos</strong> ·{" "}
          <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura.
        </p>
      ) : null}

      {canEdit &&
      hasMembership &&
      (planningIncomeCategories.length > 0 ||
        planningExpenseCategories.length > 0) ? (
        <Modal
          title={
            editingPlanningFlowId ? "Editar flujo planificado" : "Nuevo flujo"
          }
          open={planningModalOpen}
          onClose={closePlanningModal}
        >
          <form className="asset-form stack" onSubmit={submitPlanningFlowForm}>
            <ModalFormError message={formError} />
            <div className="segmented" role="tablist" aria-label="Dirección">
              <button
                type="button"
                role="tab"
                aria-selected={planningFormScope === "income"}
                className={planningFormScope === "income" ? "active" : ""}
                onClick={() => setPlanningFormScope("income")}
              >
                Entrada (ingreso)
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={planningFormScope === "expense"}
                className={planningFormScope === "expense" ? "active" : ""}
                onClick={() => setPlanningFormScope("expense")}
              >
                Salida (gasto)
              </button>
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={planningFormCategoryId}
                  onChange={(e) => setPlanningFormCategoryId(e.target.value)}
                  required
                  disabled={formCats.length === 0}
                >
                  {formCats.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Título</span>
                <input
                  value={planningFormTitle}
                  onChange={(e) => setPlanningFormTitle(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Nómina marzo"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Importe esperado</span>
                <input
                  value={planningFormAmount}
                  onChange={(e) => setPlanningFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Fecha prevista (opc.)</span>
                <input
                  type="date"
                  value={planningFormDue}
                  onChange={(e) => {
                    setPlanningFormDue(e.target.value);
                    if (e.target.value.trim() === "") {
                      setPlanningFormShowInChart(false);
                    }
                  }}
                />
              </label>
              {planningFormDue.trim() !== "" ? (
                <label className="field checkbox-field">
                  <input
                    type="checkbox"
                    checked={planningFormShowInChart}
                    onChange={(e) =>
                      setPlanningFormShowInChart(e.target.checked)
                    }
                  />
                  <span>Mostrar en la gráfica</span>
                </label>
              ) : null}
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={planningFormNotes}
                onChange={(e) => setPlanningFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={planningSaving || formCats.length === 0}
              >
                {editingPlanningFlowId ? "Guardar cambios" : "Añadir flujo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={planningSaving}
                onClick={() => closePlanningModal()}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      <section className="panel">
        <div className="panel-head-row">
          <h3 className="panel-title">Lista</h3>
          {canEdit &&
          hasMembership &&
          (planningIncomeCategories.length > 0 ||
            planningExpenseCategories.length > 0) ? (
            <button
              type="button"
              className="btn primary icon-btn ledger-toolbar-add"
              aria-label="Nuevo flujo planificado"
              onClick={() => openNewPlanningModal()}
            >
              <PlusIcon />
            </button>
          ) : null}
        </div>
        {planningLoading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : planningFlows.length === 0 ? (
          <p className="muted bordered-top">
            No hay flujos planificados en esta instalación.
          </p>
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>Dirección</th>
                  <th>Categoría</th>
                  <th>Título</th>
                  <th className="num">Importe</th>
                  <th>Fecha prevista</th>
                  {canEdit ? (
                    <th className="asset-actions-cell">
                      <span className="sr-only">Acciones</span>
                    </th>
                  ) : null}
                </tr>
              </thead>
              <tbody>
                {planningFlows.map((row) => (
                  <tr key={row.id}>
                    <td>{PLANNING_DIRECTION_LABEL[row.direction]}</td>
                    <td>
                      {categoryById.get(row.category_id)?.name ??
                        row.category_id.slice(0, 8)}
                    </td>
                    <td>{row.title}</td>
                    <td className="num">
                      {formatCurrencyAmount(row.expected_amount, currencyIso)}
                    </td>
                    <td>{row.due_date ?? METRIC_DASH}</td>
                    {canEdit ? (
                      <td className="asset-actions-cell">
                        <div className="budget-row-actions">
                          <button
                            type="button"
                            className="btn ghost icon-btn"
                            aria-label="Editar flujo planificado"
                            disabled={planningSaving}
                            onClick={() => beginEditPlanningFlow(row)}
                          >
                            <RowEditIcon />
                          </button>
                          <button
                            type="button"
                            className="btn ghost danger icon-btn"
                            aria-label="Eliminar flujo planificado"
                            disabled={planningSaving}
                            onClick={() => deletePlanningFlowRow(row.id)}
                          >
                            <RowTrashIcon />
                          </button>
                        </div>
                      </td>
                    ) : null}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

function summaryDonutGradient(
  rows: { total: string }[],
  totalWhole: string,
  scope: "asset" | "liability",
): string | null {
  const tw = parseDisplayDecimal(totalWhole) ?? 0;
  if (tw <= 0 || rows.length === 0) {
    return null;
  }
  let accPct = 0;
  const stops: string[] = [];
  rows.forEach((r, rowIndex) => {
    const v = parseDisplayDecimal(r.total) ?? 0;
    if (v <= 0) {
      return;
    }
    const pct = Math.min(100 - accPct, (v / tw) * 100);
    const c = summaryChartSliceColor(scope, rowIndex);
    const start = accPct;
    accPct += pct;
    stops.push(`${c} ${start}% ${accPct}%`);
  });
  if (stops.length === 0) {
    return null;
  }
  return `conic-gradient(${stops.join(", ")})`;
}

function SummaryDonutChart({
  title,
  rows,
  totalWhole,
  currencyIso,
  chartScope,
}: {
  title: string;
  rows: { key: string; label: string; total: string }[];
  totalWhole: string;
  currencyIso: string;
  chartScope: "asset" | "liability";
}) {
  const filtered = rows.filter((r) => (parseDisplayDecimal(r.total) ?? 0) > 0);
  const g = summaryDonutGradient(rows, totalWhole, chartScope);
  if (!g || filtered.length === 0) {
    return (
      <div className="summary-donut-card">
        <h4 className="subsection-title">{title}</h4>
        <p className="muted tight">Sin datos.</p>
      </div>
    );
  }
  return (
    <div className="summary-donut-card">
      <h4 className="subsection-title">{title}</h4>
      <div className="summary-donut-inner">
        <div
          className="summary-donut-ring"
          style={{ background: g }}
          role="img"
          aria-label={title}
        />
        <ul className="summary-donut-legend">
          {rows.map((r, rowIndex) => {
            if ((parseDisplayDecimal(r.total) ?? 0) <= 0) {
              return null;
            }
            const sw = summaryChartSliceColor(chartScope, rowIndex);
            return (
              <li key={r.key}>
                <span
                  className="summary-donut-legend-swatch"
                  style={{ background: sw }}
                  aria-hidden
                />
                <span className="summary-donut-legend-label">{r.label}</span>
                <span className="summary-donut-legend-val">
                  {formatCurrencyAmount(r.total, currencyIso)}
                </span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}

function SummaryBreakdownBlock({
  title,
  rows,
  totalWhole,
  currencyIso,
  labelColumn,
  chartScope,
}: {
  title: string;
  rows: { key: string; label: string; total: string }[];
  totalWhole: string;
  currencyIso: string;
  labelColumn: string;
  chartScope: "asset" | "liability";
}) {
  if (rows.length === 0) {
    return (
      <div className="breakdown-block">
        <h4 className="subsection-title">{title}</h4>
        <p className="muted tight">Sin datos.</p>
      </div>
    );
  }
  return (
    <div className="breakdown-block">
      <h4 className="subsection-title">{title}</h4>
      <div className="breakdown-table-wrap bordered-top">
        <table className="breakdown-table">
          <thead>
            <tr>
              <th>{labelColumn}</th>
              <th className="num">Importe</th>
              <th className="num">%</th>
              <th className="breakdown-bar-col" aria-hidden />
            </tr>
          </thead>
          <tbody>
            {rows.map((row, idx) => {
              const pct = breakdownPercentOfTotal(row.total, totalWhole);
              return (
                <tr key={row.key}>
                  <td>{row.label}</td>
                  <td className="num">
                    {formatCurrencyAmount(row.total, currencyIso)}
                  </td>
                  <td className="num muted">
                    {formatBreakdownPct(row.total, totalWhole)}
                  </td>
                  <td className="breakdown-bar-cell">
                    <div className="breakdown-bar-track">
                      <div
                        className="breakdown-bar-fill"
                        style={{
                          width: pct !== null ? `${pct}%` : "0%",
                          background: summaryBreakdownBarGradient(chartScope, idx),
                        }}
                      />
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function SummaryView({
  installation,
  loading,
  hasMembership,
  ledgerPersonScope,
  summary,
  summaryBusy,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const showMetrics =
    hasMembership && !loading && !summaryBusy && summary !== null;
  const currencyIso = installation?.installation.base_currency ?? "";

  const nw = showMetrics
    ? formatCurrencyAmount(summary.net_worth, currencyIso)
    : METRIC_DASH;
  const ta = showMetrics
    ? formatCurrencyAmount(summary.total_assets, currencyIso)
    : METRIC_DASH;
  const tl = showMetrics
    ? formatCurrencyAmount(summary.total_liabilities, currencyIso)
    : METRIC_DASH;
  const dta = showMetrics
    ? formatDebtToAssetsPct(summary.debt_to_assets_ratio)
    : METRIC_DASH;

  const fh = summary?.financial_health;
  const savingsMoneyParen =
    showMetrics && fh
      ? formatCurrencyAmount(fh.monthly_net_excluding_derived_debt, currencyIso)
      : "";
  const savingsMoneyPrimary =
    showMetrics && fh
      ? formatCurrencyAmount(fh.net_monthly_equivalent, currencyIso)
      : METRIC_DASH;
  const showSavingsMoneyTile =
    showMetrics &&
    fh &&
    (!isZeroMoneyMetric(fh.net_monthly_equivalent) ||
      !isZeroMoneyMetric(fh.monthly_net_excluding_derived_debt));
  const savingsMoneyParenthetical =
    showSavingsMoneyTile &&
    savingsMoneyPrimary !== savingsMoneyParen &&
    savingsMoneyParen !== ""
      ? savingsMoneyParen
      : undefined;

  let savingsRatePrimary = METRIC_DASH;
  let savingsRateParenthetical: string | undefined;
  if (showMetrics && fh) {
    const sr = formatFractionAsPercent(fh.savings_rate);
    const srx = formatFractionAsPercent(fh.savings_rate_excluding_derived_debt);
    const showPctTile =
      !isZeroFractionMetric(fh.savings_rate) ||
      !isZeroFractionMetric(fh.savings_rate_excluding_derived_debt);
    if (showPctTile) {
      if (sr !== METRIC_DASH) {
        savingsRatePrimary = sr;
        savingsRateParenthetical =
          srx !== METRIC_DASH && srx !== sr ? srx : undefined;
      } else {
        savingsRatePrimary = srx;
      }
    }
  }
  const showSavingsRateTile =
    showMetrics && fh && savingsRatePrimary !== METRIC_DASH;

  const financialHealthHasAnyTile =
    showMetrics &&
    fh &&
    (showSavingsMoneyTile ||
      showSavingsRateTile ||
      !isZeroMoneyMetric(fh.liquid_assets_total) ||
      !isZeroMoneyMetric(fh.runway_months) ||
      !isZeroFractionMetric(fh.upcoming_coverage_ratio));

  const liquidAssetsPctOfTotalAssets =
    showMetrics && summary && fh
      ? (() => {
          const liq = parseDisplayDecimal(fh.liquid_assets_total);
          const tot = parseDisplayDecimal(summary.total_assets);
          if (liq === null || tot === null || tot <= 0) return null;
          return formatPercentDisplay((liq / tot) * 100);
        })()
      : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Resumen</h2>
        <p className="workspace-sub">
          {loading
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      <div className="metric-grid workspace-kpi-strip">
        {!showMetrics ? (
          <>
            <MetricCard label="Patrimonio neto" value={nw} />
            <MetricCard label="Activos totales" value={ta} />
            <MetricCard label="Pasivos totales" value={tl} />
            <MetricCard label="Ratio deuda / activos" value={dta} />
          </>
        ) : (
          <>
            {!isZeroMoneyMetric(summary.net_worth) ? (
              <MetricCard label="Patrimonio neto" value={nw} />
            ) : null}
            {!isZeroMoneyMetric(summary.total_assets) ? (
              <MetricCard label="Activos totales" value={ta} />
            ) : null}
            {!isZeroMoneyMetric(summary.total_liabilities) ? (
              <MetricCard label="Pasivos totales" value={tl} />
            ) : null}
            {!isZeroFractionMetric(summary.debt_to_assets_ratio) ? (
              <MetricCard label="Ratio deuda / activos" value={dta} />
            ) : null}
          </>
        )}
      </div>

      <section className="panel">
        <h3 className="panel-title">Salud financiera</h3>
        {showMetrics ? (
          financialHealthHasAnyTile ? (
            <div className="metric-grid bordered-top">
              {showSavingsMoneyTile ? (
                <MetricCard
                  label="Ahorro mensual neto"
                  value={savingsMoneyPrimary}
                  parenthetical={savingsMoneyParenthetical}
                />
              ) : null}
              {showSavingsRateTile ? (
                <MetricCard
                  label="Tasa de ahorro"
                  value={savingsRatePrimary}
                  parenthetical={savingsRateParenthetical}
                />
              ) : null}
              {!isZeroMoneyMetric(summary.financial_health.liquid_assets_total) ? (
                <MetricCard
                  label="Activos líquidos"
                  value={formatCurrencyAmount(
                    summary.financial_health.liquid_assets_total,
                    currencyIso,
                  )}
                  parenthetical={
                    liquidAssetsPctOfTotalAssets ?? undefined
                  }
                />
              ) : null}
              {!isZeroMoneyMetric(summary.financial_health.runway_months) ? (
                <MetricCard
                  label="Runway"
                  value={formatMonthsRough(
                    summary.financial_health.runway_months,
                  )}
                />
              ) : null}
            </div>
          ) : (
            <p className="muted bordered-top">Sin datos.</p>
          )
        ) : (
          <p className="muted bordered-top">Sin acceso.</p>
        )}
      </section>

      <section className="panel">
        <h3 className="panel-title">Desglose</h3>
        {showMetrics && summary ? (
          <div className="summary-donuts-row bordered-top">
            <SummaryDonutChart
              title="Activos por categoría"
              currencyIso={currencyIso}
              chartScope="asset"
              totalWhole={summary.total_assets}
              rows={summary.assets_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
            <SummaryDonutChart
              title="Pasivos por categoría"
              currencyIso={currencyIso}
              chartScope="liability"
              totalWhole={summary.total_liabilities}
              rows={summary.liabilities_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
          </div>
        ) : null}
        {showMetrics && summary ? (
          <div className="breakdown-grid">
            <SummaryBreakdownBlock
              title="Activos por categoría"
              labelColumn="Categoría"
              chartScope="asset"
              totalWhole={summary.total_assets}
              currencyIso={currencyIso}
              rows={summary.assets_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
            <SummaryBreakdownBlock
              title="Pasivos por categoría"
              labelColumn="Categoría"
              chartScope="liability"
              totalWhole={summary.total_liabilities}
              currencyIso={currencyIso}
              rows={summary.liabilities_by_category.map((r) => ({
                key: r.category_id,
                label: r.category_name,
                total: r.total,
              }))}
            />
          </div>
        ) : (
          <p className="muted bordered-top">Sin acceso.</p>
        )}
      </section>
    </div>
  );
}

function MetricCard({
  label,
  value,
  suffix,
  parenthetical,
  action,
}: {
  label: string;
  value: string;
  suffix?: string;
  /** Detalle del mismo KPI entre paréntesis (`.metric-value-parenthetical`). */
  parenthetical?: string;
  /** Botón/icono opcional en la esquina superior derecha (p.ej. engranaje de config). */
  action?: ReactNode;
}) {
  return (
    <article className="metric-card">
      <div className="metric-card__header">
        <div className="metric-label">{label}</div>
        {action ? <div className="metric-card__action">{action}</div> : null}
      </div>
      <div className="metric-value-row">
        <span className="metric-value">{value}</span>
        {parenthetical != null && parenthetical !== "" ? (
          <span className="metric-value-parenthetical">
            ({parenthetical})
          </span>
        ) : null}
        {suffix && suffix !== METRIC_DASH ? (
          <span className="metric-suffix">{suffix}</span>
        ) : null}
      </div>
    </article>
  );
}

function BootstrapInstallationPanel({
  installationBusy,
  setupCurrency,
  setSetupCurrency,
  setupCalendarTz,
  setSetupCalendarTz,
  setupInstallation,
}: {
  installationBusy: boolean;
  setupCurrency: "EUR" | "USD" | "GBP";
  setSetupCurrency: (v: "EUR" | "USD" | "GBP") => void;
  setupCalendarTz: string;
  setSetupCalendarTz: Dispatch<SetStateAction<string>>;
  setupInstallation: (e: FormEvent) => void;
}) {
  return (
    <section className="panel">
      <h3 className="panel-title">Inicializar instalación</h3>
      <form className="stack bordered-top" onSubmit={setupInstallation}>
        <label className="field">
          <span>Moneda base</span>
          <select
            value={setupCurrency}
            onChange={(e) =>
              setSetupCurrency(e.target.value as "EUR" | "USD" | "GBP")
            }
          >
            <option value="EUR">EUR</option>
            <option value="USD">USD</option>
            <option value="GBP">GBP</option>
          </select>
        </label>
        <label className="field">
          <span>Zona horaria (IANA)</span>
          <select
            value={
              [
                "UTC",
                "Europe/Madrid",
                "Europe/London",
                "America/New_York",
                "America/Los_Angeles",
              ].includes(setupCalendarTz)
                ? setupCalendarTz
                : "__custom__"
            }
            onChange={(e) => {
              const v = e.target.value;
              if (v === "__custom__") return;
              setSetupCalendarTz(v);
            }}
          >
            <option value="UTC">UTC</option>
            <option value="Europe/Madrid">Europe/Madrid</option>
            <option value="Europe/London">Europe/London</option>
            <option value="America/New_York">America/New_York</option>
            <option value="America/Los_Angeles">America/Los_Angeles</option>
            <option value="__custom__">Otra (editar abajo)</option>
          </select>
        </label>
        <label className="field">
          <span>IANA exacta (opcional)</span>
          <input
            value={setupCalendarTz}
            onChange={(e) => setSetupCalendarTz(e.target.value)}
            placeholder="Europe/Madrid"
            maxLength={64}
            autoComplete="off"
          />
        </label>
        <button type="submit" className="btn primary" disabled={installationBusy}>
          Crear instalación
        </button>
      </form>
    </section>
  );
}

function SettingsView({
  installation,
  installationBusy,
  categoryModalOpen,
  categoryRenameModalOpen,
  closeCategoryModal,
  openNewCategoryModal,
  closeRenameCategoryModal,
  openRenameCategoryModal,
  calendarTzDraft,
  setCalendarTzDraft,
  calendarTzSaving,
  saveInstallationCalendarTz,
  projectionInflationPctDraft,
  setProjectionInflationPctDraft,
  showAgeModeDraft,
  setShowAgeModeDraft,
  installationProjectionSaving,
  saveInstallationProjection,
  onSaveFire,
  health,
  healthError,
  categoriesError,
  hasMembership,
  canEditCategories,
  isOwner,
  settingsSubTab,
  navigateSettingsSubTab,
  visibleSettingsSubTabs,
  pendingUsers,
  pendingUsersBusy,
  approveRoles,
  setApproveRoles,
  approveBusy,
  approvePendingUser,
  categories,
  categoriesBusy,
  categoryScopeFilter,
  setCategoryScopeFilter,
  newCatScope,
  setNewCatScope,
  newCatName,
  setNewCatName,
  categorySaving,
  createCategory,
  openCategoryDeleteModal,
  categoryDeleteModalOpen,
  categoryDeletePending,
  categoryRemapToId,
  setCategoryRemapToId,
  closeCategoryDeleteModal,
  confirmDeleteCategory,
  editingCategoryId,
  editCategoryName,
  setEditCategoryName,
  saveCategoryEdit,
  ffbackupExportModalOpen,
  ffbackupExportPassword,
  setFfbackupExportPassword,
  ffbackupExportBusy,
  ffbackupExportError,
  openFfbackupExportModal,
  closeFfbackupExportModal,
  runFfbackupExport,
  ffbackupImportModalOpen,
  ffbackupImportFile,
  setFfbackupImportFile,
  ffbackupImportPassword,
  setFfbackupImportPassword,
  ffbackupImportBusy,
  ffbackupImportError,
  ffbackupImportPreview,
  ffbackupImportDone,
  openFfbackupImportModal,
  closeFfbackupImportModal,
  runFfbackupImportPreview,
  runFfbackupImportApply,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  categoryModalOpen: boolean;
  categoryRenameModalOpen: boolean;
  closeCategoryModal: () => void;
  openNewCategoryModal: () => void;
  closeRenameCategoryModal: () => void;
  openRenameCategoryModal: (row: CategoryRow) => void;
  calendarTzDraft: string;
  setCalendarTzDraft: Dispatch<SetStateAction<string>>;
  calendarTzSaving: boolean;
  saveInstallationCalendarTz: (e: FormEvent) => void;
  projectionInflationPctDraft: string;
  setProjectionInflationPctDraft: Dispatch<SetStateAction<string>>;
  showAgeModeDraft: "dates" | "ages";
  setShowAgeModeDraft: Dispatch<SetStateAction<"dates" | "ages">>;
  installationProjectionSaving: boolean;
  saveInstallationProjection: (e: FormEvent) => void;
  onSaveFire: (fs: FireSettingsApi) => Promise<void>;
  health: HealthResponse | null;
  healthError: string | null;
  categoriesError: string | null;
  hasMembership: boolean;
  canEditCategories: boolean;
  isOwner: boolean;
  settingsSubTab: SettingsSubTabId;
  navigateSettingsSubTab: (id: SettingsSubTabId) => void;
  visibleSettingsSubTabs: SettingsSubTabId[];
  pendingUsers: UserResponse[];
  pendingUsersBusy: boolean;
  approveRoles: Record<string, "member" | "viewer">;
  setApproveRoles: Dispatch<
    SetStateAction<Record<string, "member" | "viewer">>
  >;
  approveBusy: boolean;
  approvePendingUser: (userId: string) => void;
  categories: CategoryRow[];
  categoriesBusy: boolean;
  categoryScopeFilter: CategoryScope | "all";
  setCategoryScopeFilter: Dispatch<
    SetStateAction<CategoryScope | "all">
  >;
  newCatScope: CategoryScope;
  setNewCatScope: Dispatch<SetStateAction<CategoryScope>>;
  newCatName: string;
  setNewCatName: Dispatch<SetStateAction<string>>;
  categorySaving: boolean;
  createCategory: (e: FormEvent) => void;
  openCategoryDeleteModal: (row: CategoryRow) => void;
  categoryDeleteModalOpen: boolean;
  categoryDeletePending: CategoryRow | null;
  categoryRemapToId: string;
  setCategoryRemapToId: Dispatch<SetStateAction<string>>;
  closeCategoryDeleteModal: () => void;
  confirmDeleteCategory: () => void;
  editingCategoryId: string | null;
  editCategoryName: string;
  setEditCategoryName: Dispatch<SetStateAction<string>>;
  saveCategoryEdit: (id: string) => void;
  ffbackupExportModalOpen: boolean;
  ffbackupExportPassword: string;
  setFfbackupExportPassword: Dispatch<SetStateAction<string>>;
  ffbackupExportBusy: boolean;
  ffbackupExportError: string | null;
  openFfbackupExportModal: () => void;
  closeFfbackupExportModal: () => void;
  runFfbackupExport: (e: FormEvent) => void;
  ffbackupImportModalOpen: boolean;
  ffbackupImportFile: File | null;
  setFfbackupImportFile: Dispatch<SetStateAction<File | null>>;
  ffbackupImportPassword: string;
  setFfbackupImportPassword: Dispatch<SetStateAction<string>>;
  ffbackupImportBusy: boolean;
  ffbackupImportError: string | null;
  ffbackupImportPreview: FfbackupImportPreviewResponse | null;
  ffbackupImportDone: string | null;
  openFfbackupImportModal: () => void;
  closeFfbackupImportModal: () => void;
  runFfbackupImportPreview: (e: FormEvent) => void;
  runFfbackupImportApply: () => void;
}) {
  const renamingCat =
    editingCategoryId === null
      ? undefined
      : categories.find((x) => x.id === editingCategoryId);

  const filteredCategories =
    categoryScopeFilter === "all"
      ? categories
      : categories.filter((c) => c.scope === categoryScopeFilter);

  const settingsSubTabs = useMemo(
    () =>
      visibleSettingsSubTabs.map((id) => ({
        id,
        label: SETTINGS_SUBTAB_LABEL[id],
      })),
    [visibleSettingsSubTabs],
  );

  const [fireTaxDraft, setFireTaxDraft] = useState<FireSettingsApi>(() =>
    normalizeInstallationFireSettings(installation?.installation.fire_settings),
  );
  const [fireTaxSaving, setFireTaxSaving] = useState(false);
  const lastSavedFireTaxPayloadRef = useRef<string>("");
  const skipFireTaxAutosaveRef = useRef(true);

  useEffect(() => {
    const serverFs = normalizeInstallationFireSettings(
      installation?.installation.fire_settings,
    );
    setFireTaxDraft(serverFs);
    lastSavedFireTaxPayloadRef.current = JSON.stringify(serverFs);
    skipFireTaxAutosaveRef.current = true;
  }, [installation?.installation.id]);

  useEffect(() => {
    if (!hasMembership || !isOwner) return;
    if (skipFireTaxAutosaveRef.current) {
      skipFireTaxAutosaveRef.current = false;
      return;
    }
    const payloadJson = JSON.stringify(fireTaxDraft);
    if (payloadJson === lastSavedFireTaxPayloadRef.current) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setFireTaxSaving(true);
      void onSaveFire(fireTaxDraft).finally(() => {
        if (!cancelled) {
          lastSavedFireTaxPayloadRef.current = payloadJson;
          setFireTaxSaving(false);
        }
      });
    }, 420);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [fireTaxDraft, hasMembership, isOwner, onSaveFire]);

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Ajustes</h2>
      </div>

      <nav
        className="tab-bar settings-subtab-bar"
        aria-label="Subsecciones de ajustes"
      >
        {settingsSubTabs.map((t) => (
          <button
            key={t.id}
            type="button"
            className={`tab-btn ${settingsSubTab === t.id ? "active" : ""}`}
            onClick={() => navigateSettingsSubTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {settingsSubTab === "access" && isOwner ? (
        <section className="panel">
          <h3 className="panel-title">Aprobar acceso</h3>
          {pendingUsersBusy ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : pendingUsers.length === 0 ? (
            <p className="muted bordered-top">Nadie pendiente.</p>
          ) : (
            <ul className="pending-users-list">
              {pendingUsers.map((u) => (
                <li key={u.id} className="pending-user-row">
                  <span className="pending-user-name">{u.username}</span>
                  <div className="pending-user-actions">
                    <label className="field inline-role">
                      <span className="sr-only">Rol</span>
                      <select
                        value={approveRoles[u.id] ?? "member"}
                        onChange={(e) =>
                          setApproveRoles((prev) => ({
                            ...prev,
                            [u.id]: e.target.value as "member" | "viewer",
                          }))
                        }
                      >
                        <option value="member">Miembro</option>
                        <option value="viewer">Visor</option>
                      </select>
                    </label>
                    <button
                      type="button"
                      className="btn primary"
                      disabled={approveBusy}
                      onClick={() => approvePendingUser(u.id)}
                    >
                      Aprobar
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      {settingsSubTab === "calendar" && hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Zona horaria del calendario</h3>
          {isOwner ? (
            <form
              className="stack bordered-top"
              onSubmit={saveInstallationCalendarTz}
            >
              <label className="field">
                <span>IANA (p. ej. Europe/Madrid)</span>
                <input
                  value={calendarTzDraft}
                  onChange={(e) => setCalendarTzDraft(e.target.value)}
                  maxLength={64}
                  placeholder="Europe/Madrid"
                  autoComplete="off"
                />
              </label>
              <button
                type="submit"
                className="btn primary"
                disabled={calendarTzSaving}
              >
                Guardar zona horaria
              </button>
            </form>
          ) : (
            <p className="muted bordered-top">
              <strong>{installation?.installation.calendar_tz ?? "UTC"}</strong>{" "}
              · solo lectura
            </p>
          )}
        </section>
      ) : null}

      {settingsSubTab === "projection" && hasMembership ? (
        isOwner ? (
        <section className="panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          <form
            className="stack bordered-top"
            onSubmit={saveInstallationProjection}
          >
            <label className="field">
              <span>Inflación anual %</span>
              <input
                value={projectionInflationPctDraft}
                onChange={(e) =>
                  setProjectionInflationPctDraft(e.target.value)
                }
                inputMode="decimal"
                placeholder="2,5"
                autoComplete="off"
              />
              <small className="muted">
                Se aplica al target FIRE para preservar tu poder adquisitivo. Los
                ingresos, gastos y aportaciones se mantienen constantes en euros
                — refleja «hacer lo que haces ahora». Usa <code>0</code> para
                desactivar.
              </small>
            </label>
            <label className="field">
              <span>Modo edad en la interfaz</span>
              <select
                value={showAgeModeDraft}
                onChange={(e) =>
                  setShowAgeModeDraft(
                    e.target.value === "ages" ? "ages" : "dates",
                  )
                }
              >
                <option value="dates">Fechas</option>
                <option value="ages">Edades</option>
              </select>
            </label>
            <button
              type="submit"
              className="btn primary"
              disabled={installationProjectionSaving}
            >
              Guardar proyección
            </button>
          </form>
        </section>
        ) : (
        <section className="panel muted-panel">
          <h3 className="panel-title">Proyección y modo de edad</h3>
          <p className="muted tight">Solo lectura.</p>
        </section>
        )
      ) : null}

      {settingsSubTab === "retirement" && hasMembership ? (
        isOwner ? (
          <section className="panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <p className="muted tight">
              {fireTaxSaving ? "Guardando…" : "Guardado automático."}
            </p>
            <div className="stack bordered-top">
              <label className="field checkbox-field">
                <input
                  type="checkbox"
                  checked={fireTaxDraft.taxes_enabled}
                  onChange={(e) =>
                    setFireTaxDraft((p) => ({
                      ...p,
                      taxes_enabled: e.target.checked,
                    }))
                  }
                />
                <span>Aplicar IRPF del ahorro</span>
              </label>
              <fieldset disabled={!fireTaxDraft.taxes_enabled} className="stack">
                <div className="stack tight">
                  <button
                    type="button"
                    className="btn ghost"
                    onClick={() =>
                      setFireTaxDraft((p) => ({
                        ...p,
                        tax_brackets: DEFAULT_ES_TAX_BRACKETS_API.map((b) => ({
                          up_to: b.up_to,
                          pct: b.pct,
                        })),
                      }))
                    }
                  >
                    Restaurar España
                  </button>
                  <div className="table-scroll">
                    <table className="table">
                      <thead>
                        <tr>
                          <th>Hasta base (€)</th>
                          <th>Tipo (%)</th>
                        </tr>
                      </thead>
                      <tbody>
                        {fireTaxDraft.tax_brackets.map((row, idx) => (
                          <tr key={`tax-br-${idx}`}>
                            <td>
                              <input
                                placeholder={
                                  idx === fireTaxDraft.tax_brackets.length - 1 ? "∞" : ""
                                }
                                value={row.up_to ?? ""}
                                onChange={(e) => {
                                  const t = e.target.value.trim();
                                  setFireTaxDraft((p) => {
                                    const next = [...p.tax_brackets];
                                    next[idx] = {
                                      ...next[idx],
                                      up_to:
                                        t === ""
                                          ? idx === p.tax_brackets.length - 1
                                            ? null
                                            : next[idx].up_to
                                          : t.replace(",", "."),
                                    };
                                    return { ...p, tax_brackets: next };
                                  });
                                }}
                              />
                            </td>
                            <td>
                              <input
                                value={row.pct}
                                onChange={(e) => {
                                  const t = e.target.value.replace(",", ".");
                                  setFireTaxDraft((p) => {
                                    const next = [...p.tax_brackets];
                                    next[idx] = { ...next[idx], pct: t };
                                    return { ...p, tax_brackets: next };
                                  });
                                }}
                              />
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              </fieldset>
            </div>
          </section>
        ) : (
          <section className="panel muted-panel">
            <h3 className="panel-title">Fiscalidad (IRPF ahorro)</h3>
            <p className="muted tight">Solo lectura.</p>
          </section>
        )
      ) : null}

      {settingsSubTab === "categories" && hasMembership ? (
        <section className="panel">
          <div className="panel-head-row">
            <h3 className="panel-title">Categorías</h3>
            {canEditCategories ? (
              <button
                type="button"
                className="btn primary"
                onClick={() => openNewCategoryModal()}
              >
                Nueva categoría
              </button>
            ) : null}
          </div>
          <div className="category-toolbar bordered-top">
            <label className="field inline-role">
              <span className="sr-only">Filtrar por ámbito</span>
              <select
                value={categoryScopeFilter}
                onChange={(e) => {
                  const v = e.target.value;
                  setCategoryScopeFilter(
                    v === "all" ? "all" : (v as CategoryScope),
                  );
                }}
              >
                <option value="all">Todos los ámbitos</option>
                {CATEGORY_SCOPES.map((s) => (
                  <option key={s} value={s}>
                    {CATEGORY_SCOPE_LABEL[s]}
                  </option>
                ))}
              </select>
            </label>
          </div>
          {!canEditCategories ? (
            <p className="muted bordered-top">
              Solo lectura.
            </p>
          ) : null}
          {categoriesBusy ? (
            <p className="muted bordered-top">Cargando categorías…</p>
          ) : (
            <ul className="category-list">
              {filteredCategories.map((c) => (
                <li key={c.id} className="category-row">
                  <span className="category-scope-tag">
                    {CATEGORY_SCOPE_LABEL[c.scope]}
                  </span>
                  <span className="category-name">{c.name}</span>
                  {canEditCategories ? (
                    <div className="category-row-actions budget-row-actions">
                      <button
                        type="button"
                        className="btn ghost icon-btn"
                        aria-label="Renombrar categoría"
                        disabled={categorySaving}
                        onClick={() => openRenameCategoryModal(c)}
                      >
                        <RowEditIcon />
                      </button>
                      <button
                        type="button"
                        className="btn ghost danger icon-btn"
                        aria-label="Eliminar categoría"
                        disabled={categorySaving}
                        onClick={() => openCategoryDeleteModal(c)}
                      >
                        <RowTrashIcon />
                      </button>
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
          {!categoriesBusy && filteredCategories.length === 0 ? (
            <p className="muted bordered-top">Vacío.</p>
          ) : null}
        </section>
      ) : null}

      {settingsSubTab === "data" ? (
        <>
          {hasMembership ? (
            <section className="panel">
              <h3 className="panel-title">Backup personal (.ffbackup)</h3>
              <p className="muted compact bordered-top">
                Exporta o restaura un archivo cifrado con tu contraseña que
                contiene solo tus datos: activos, pasivos, presupuesto,
                planificación, categorías usadas, fecha de nacimiento y
                preferencias UI. El archivo es portable entre instalaciones.
              </p>
              {ffbackupImportDone ? (
                <p className="muted compact bordered-top">
                  {ffbackupImportDone}
                </p>
              ) : null}
              <div className="bordered-top settings-backup-actions">
                <button
                  type="button"
                  className="btn primary"
                  onClick={() => openFfbackupExportModal()}
                >
                  Exportar mis datos (.ffbackup)
                </button>
                <button
                  type="button"
                  className="btn"
                  onClick={() => openFfbackupImportModal()}
                >
                  Importar backup (.ffbackup)
                </button>
              </div>
            </section>
          ) : null}

          <section className="panel">
            <h3 className="panel-title">Instalación</h3>
            {installationBusy ? (
              <p className="muted">Cargando…</p>
            ) : installation ? (
              <dl className="settings-meta-dl">
                <div>
                  <dt>Moneda base</dt>
                  <dd>{installation.installation.base_currency}</dd>
                </div>
                <div>
                  <dt>Tu rol</dt>
                  <dd>{installation.role}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted tight">Sin acceso.</p>
            )}
          </section>

          <section className="panel dev-panel">
            <h3 className="panel-title">Estado del sistema</h3>
            {healthError ? (
              <p className="error compact">
                <code>/v1/health</code>: {healthError}
              </p>
            ) : health ? (
              <dl className="health-dl">
                <div>
                  <dt>Servicio</dt>
                  <dd>{health.service}</dd>
                </div>
                <div>
                  <dt>Versión API</dt>
                  <dd>{health.version}</dd>
                </div>
                <div>
                  <dt>Estado</dt>
                  <dd>{health.status}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted">Comprobando…</p>
            )}
          </section>
        </>
      ) : null}

      {canEditCategories ? (
        <>
          <Modal
            title="Nueva categoría"
            open={categoryModalOpen}
            onClose={closeCategoryModal}
          >
            <form className="asset-form stack" onSubmit={createCategory}>
              <ModalFormError message={categoriesError} />
              <label className="field">
                <span>Ámbito</span>
                <select
                  value={newCatScope}
                  onChange={(e) =>
                    setNewCatScope(e.target.value as CategoryScope)
                  }
                >
                  {CATEGORY_SCOPES.map((s) => (
                    <option key={s} value={s}>
                      {CATEGORY_SCOPE_LABEL[s]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Nombre</span>
                <input
                  value={newCatName}
                  onChange={(e) => setNewCatName(e.target.value)}
                  maxLength={200}
                  placeholder="p. ej. Efectivo"
                  autoComplete="off"
                />
              </label>
              <div className="asset-form-actions">
                <button
                  type="submit"
                  className="btn primary"
                  disabled={categorySaving}
                >
                  Añadir
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={categorySaving}
                  onClick={() => closeCategoryModal()}
                >
                  Cancelar
                </button>
              </div>
            </form>
          </Modal>
          <Modal
            title="Renombrar categoría"
            open={categoryRenameModalOpen && editingCategoryId !== null}
            onClose={closeRenameCategoryModal}
          >
            {renamingCat ? (
              <form
                className="asset-form stack"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (editingCategoryId) {
                    void saveCategoryEdit(editingCategoryId);
                  }
                }}
              >
                <ModalFormError message={categoriesError} />
                <p className="muted tight">
                  Ámbito:{" "}
                  <strong>{CATEGORY_SCOPE_LABEL[renamingCat.scope]}</strong>
                </p>
                <label className="field">
                  <span>Nombre</span>
                  <input
                    value={editCategoryName}
                    onChange={(e) => setEditCategoryName(e.target.value)}
                    maxLength={200}
                    aria-label="Nuevo nombre"
                    autoComplete="off"
                  />
                </label>
                <div className="asset-form-actions">
                  <button
                    type="submit"
                    className="btn primary"
                    disabled={categorySaving || !editCategoryName.trim()}
                  >
                    Guardar
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={categorySaving}
                    onClick={() => closeRenameCategoryModal()}
                  >
                    Cancelar
                  </button>
                </div>
              </form>
            ) : (
              <p className="muted tight">Categoría no encontrada.</p>
            )}
          </Modal>
          <Modal
            title="Eliminar categoría"
            open={categoryDeleteModalOpen && categoryDeletePending !== null}
            onClose={closeCategoryDeleteModal}
          >
            {categoryDeletePending ? (
              <div className="stack">
                <ModalFormError message={categoriesError} />
                <p className="muted tight">
                  Se eliminará{" "}
                  <strong>{categoryDeletePending.name}</strong> (
                  {CATEGORY_SCOPE_LABEL[categoryDeletePending.scope]}).
                </p>
                {(() => {
                  const siblings = categories.filter(
                    (x) =>
                      x.scope === categoryDeletePending.scope &&
                      x.id !== categoryDeletePending.id,
                  );
                  if (siblings.length === 0) {
                    return (
                      <p className="hint">Sin categoría sustituta en el ámbito.</p>
                    );
                  }
                  return (
                    <label className="field">
                      <span>Reasignar a</span>
                      <select
                        value={categoryRemapToId}
                        onChange={(e) => setCategoryRemapToId(e.target.value)}
                        aria-label="Categoría destino para remap"
                      >
                        {siblings.map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
                          </option>
                        ))}
                      </select>
                    </label>
                  );
                })()}
                <div className="asset-form-actions">
                  <button
                    type="button"
                    className="btn ghost danger"
                    disabled={categorySaving}
                    onClick={() => confirmDeleteCategory()}
                  >
                    Eliminar
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={categorySaving}
                    onClick={() => closeCategoryDeleteModal()}
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            ) : (
              <p className="muted tight">Nada seleccionado.</p>
            )}
          </Modal>
        </>
      ) : null}

      <Modal
        title="Exportar backup personal"
        open={ffbackupExportModalOpen}
        onClose={closeFfbackupExportModal}
      >
        <form className="stack" onSubmit={runFfbackupExport}>
          <ModalFormError message={ffbackupExportError} />
          <p className="muted tight">
            El archivo .ffbackup quedará cifrado con tu contraseña actual.
            Guárdalo en un sitio seguro y recuerda la contraseña — sin ella
            no se puede restaurar.
          </p>
          <label className="field">
            <span>Tu contraseña</span>
            <input
              type="password"
              autoComplete="current-password"
              value={ffbackupExportPassword}
              onChange={(e) => setFfbackupExportPassword(e.target.value)}
              disabled={ffbackupExportBusy}
              required
            />
          </label>
          <div className="asset-form-actions">
            <button
              type="submit"
              className="btn primary"
              disabled={ffbackupExportBusy}
            >
              {ffbackupExportBusy ? "Generando…" : "Descargar .ffbackup"}
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={ffbackupExportBusy}
              onClick={() => closeFfbackupExportModal()}
            >
              Cancelar
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        title="Importar backup personal"
        open={ffbackupImportModalOpen}
        onClose={closeFfbackupImportModal}
      >
        <div className="stack">
          <ModalFormError message={ffbackupImportError} />
          {ffbackupImportPreview === null ? (
            <form className="stack" onSubmit={runFfbackupImportPreview}>
              <p className="muted tight">
                Sube un archivo .ffbackup y la contraseña con la que se
                generó. Verás un resumen antes de aplicar nada.
              </p>
              <label className="field">
                <span>Archivo .ffbackup</span>
                <input
                  type="file"
                  accept=".ffbackup"
                  disabled={ffbackupImportBusy}
                  onChange={(e) => {
                    const f = e.target.files && e.target.files[0];
                    setFfbackupImportFile(f ?? null);
                  }}
                />
              </label>
              <label className="field">
                <span>Contraseña del backup</span>
                <input
                  type="password"
                  autoComplete="off"
                  value={ffbackupImportPassword}
                  onChange={(e) =>
                    setFfbackupImportPassword(e.target.value)
                  }
                  disabled={ffbackupImportBusy}
                  required
                />
              </label>
              <div className="asset-form-actions">
                <button
                  type="submit"
                  className="btn primary"
                  disabled={ffbackupImportBusy || !ffbackupImportFile}
                >
                  {ffbackupImportBusy ? "Leyendo…" : "Previsualizar"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={ffbackupImportBusy}
                  onClick={() => closeFfbackupImportModal()}
                >
                  Cancelar
                </button>
              </div>
            </form>
          ) : (
            <div className="stack">
              <p className="muted tight">
                Backup de <strong>{ffbackupImportPreview.username_original}</strong>{" "}
                exportado el {ffbackupImportPreview.exported_at} (app{" "}
                {ffbackupImportPreview.app_version}, schema v
                {ffbackupImportPreview.schema_version}).
              </p>
              <ul className="muted tight" style={{ paddingLeft: "1.2em" }}>
                <li>{ffbackupImportPreview.counts.assets} activos</li>
                <li>{ffbackupImportPreview.counts.liabilities} pasivos</li>
                <li>
                  {ffbackupImportPreview.counts.budget_entries} entradas de
                  presupuesto
                </li>
                <li>
                  {ffbackupImportPreview.counts.planning_flows} flujos
                  planificados
                </li>
                <li>
                  Categorías:{" "}
                  {ffbackupImportPreview.counts.categories_in_backup} (
                  {ffbackupImportPreview.counts.categories_already_present} ya
                  existen,{" "}
                  {ffbackupImportPreview.counts.categories_to_create} se
                  crearán)
                </li>
                {ffbackupImportPreview.birth_date_will_change ? (
                  <li>Tu fecha de nacimiento se actualizará.</li>
                ) : null}
                {ffbackupImportPreview.ui_preferences_present ? (
                  <li>Se restaurarán tus preferencias UI.</li>
                ) : null}
              </ul>
              <p className="error compact">
                Al continuar se eliminarán todos tus activos, pasivos,
                presupuesto y planificación actuales y serán reemplazados por
                los del backup. Operación atómica e irreversible.
              </p>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn ghost danger"
                  disabled={ffbackupImportBusy}
                  onClick={() => runFfbackupImportApply()}
                >
                  {ffbackupImportBusy
                    ? "Importando…"
                    : "Confirmar reemplazo"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={ffbackupImportBusy}
                  onClick={() => closeFfbackupImportModal()}
                >
                  Cancelar
                </button>
              </div>
            </div>
          )}
        </div>
      </Modal>
    </div>
  );
}

const ASSET_LINE_COLORS = [
  "#2563eb",
  "#0891b2",
  "#059669",
  "#7c3aed",
  "#0f766e",
  "#1d4ed8",
  "#15803d",
  "#0ea5e9",
];

function ProjectionNetWorthChart({
  series,
  milestones,
  focusMode,
  inflationAdjusted,
  installationInflationPct,
  currencyIso,
  ledgerPersonScope,
  inflationPctDisplay,
  ageUiMode,
  userBirthDate,
  anchorDateYmd,
  calendarTz,
  planningFlows,
}: {
  series: ProjectionSeriesApi;
  milestones: ProjectionMilestoneApi[];
  focusMode: boolean;
  /** Cuando true (default), las series del chart se deflactan visualmente — la matemática del
   *  motor sigue siendo nominal, pero el eje Y muestra "dinero de hoy" y el target FIRE base es plano. */
  inflationAdjusted: boolean;
  installationInflationPct: number;
  currencyIso: string;
  ledgerPersonScope: LedgerPersonScope;
  inflationPctDisplay: string | null;
  ageUiMode: "dates" | "ages";
  userBirthDate: string | null;
  /** Mes 0 del motor (YYYY-MM-DD); prioridad sobre reloj cliente. */
  anchorDateYmd: string | null;
  calendarTz: string;
  planningFlows: PlanningFlowApiRow[];
}) {
  const pts = series.points;
  const gid = useId().replace(/:/g, "");
  const svgRef = useRef<SVGSVGElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const yAxisAnimRef = useRef<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [tipOffset, setTipOffset] = useState({ x: 0, y: 0 });
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 });
  const [viewWindow, setViewWindow] = useState({ start: 0, count: pts.length });
  const [animatedYDomain, setAnimatedYDomain] = useState<{
    min: number;
    max: number;
  } | null>(null);
  const animatedYDomainRef = useRef<{ min: number; max: number } | null>(null);

  useLayoutEffect(() => {
    const node = wrapRef.current;
    if (!node) return;
    const measure = () => {
      const rect = node.getBoundingClientRect();
      if (rect.width > 0) {
        setContainerSize((prev) => {
          const nextW = Math.round(rect.width);
          const nextH = Math.max(0, Math.round(rect.height));
          return prev.width === nextW && prev.height === nextH
            ? prev
            : { width: nextW, height: nextH };
        });
      }
    };
    measure();
    const ro = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect && rect.width > 0) {
        setContainerSize((prev) => {
          const nextW = Math.round(rect.width);
          const nextH = Math.max(0, Math.round(rect.height));
          return prev.width === nextW && prev.height === nextH
            ? prev
            : { width: nextW, height: nextH };
        });
      }
    });
    ro.observe(node);
    return () => ro.disconnect();
  }, []);

  const focusWindow = useMemo(() => {
    if (pts.length <= 0) return null;
    const nextMonetaryMilestones = milestones
      .filter((m) => m.reached_month_index >= 0)
      .sort((a, b) => a.reached_month_index - b.reached_month_index)
      .slice(0, 3);
    const focusEnd = nextMonetaryMilestones.at(-1)?.reached_month_index;
    if (focusEnd == null) return null;
    const clampedEnd = Math.max(0, Math.min(pts.length - 1, focusEnd));
    return { start: 0, count: clampedEnd + 1 };
  }, [milestones, pts.length]);

  useEffect(() => {
    setViewWindow((prev) => {
      if (pts.length <= 0) return { start: 0, count: 0 };
      const next = focusMode && focusWindow
        ? focusWindow
        : { start: 0, count: pts.length };
      if (prev.start === next.start && prev.count === next.count) {
        return prev;
      }
      return next;
    });
  }, [focusMode, focusWindow, pts.length]);

  const hasFireTargetSeries = useMemo(() => {
    const f = series.fire_target_series;
    return Array.isArray(f) && f.length === series.points.length && f.length > 0;
  }, [series.fire_target_series, series.points.length]);

  const legendLabels = useMemo(() => {
    const assetNames = (series.asset_series ?? []).map((as) => as.asset_name);
    const labels: string[] = ["Patrimonio neto", "Capital aportado"];
    if (hasFireTargetSeries) labels.push("Target FIRE");
    labels.push(...assetNames);
    return labels;
  }, [series.asset_series, hasFireTargetSeries]);

  const layoutDims = useMemo(
    () =>
      buildProjectionChartLayout(
        containerSize.width > 0 ? containerSize.width : 1040,
        containerSize.height > 0 ? containerSize.height : undefined,
        legendLabels,
      ),
    [containerSize.height, containerSize.width, legendLabels],
  );

  const model = useMemo(() => {
    if (pts.length < 2) return null;
    const deflate = inflationAdjusted && installationInflationPct > 0;
    const deflator = (monthIndex: number) =>
      deflate
        ? 1 / Math.pow(1 + installationInflationPct / 100, monthIndex / 12)
        : 1;
    const nw = pts.map(
      (p, i) => (parseDisplayDecimal(p.net_worth) ?? 0) * deflator(i),
    );
    const cc = pts.map(
      (p, i) => (parseDisplayDecimal(p.contributed_capital) ?? 0) * deflator(i),
    );
    const fireRaw = series.fire_target_series;
    const fireTarget =
      Array.isArray(fireRaw) && fireRaw.length === pts.length
        ? fireRaw.map((v, i) => (parseDisplayDecimal(v) ?? 0) * deflator(i))
        : null;
    const assetSeries = (series.asset_series ?? [])
      .map((as) => {
        const values = as.values.map(
          (v, i) => (parseDisplayDecimal(v) ?? 0) * deflator(i),
        );
        return {
          id: as.asset_id,
          name: as.asset_name,
          values,
          peak: values.length > 0 ? Math.max(0, ...values) : 0,
        };
      })
      .sort((a, b) => {
        if (a.peak !== b.peak) return a.peak - b.peak;
        return a.name.localeCompare(b.name);
      });
    const monthCount = pts.length;
    const assetSums: number[] = new Array(monthCount).fill(0);
    for (let k = 0; k < monthCount; k++) {
      let s = 0;
      for (const as of assetSeries) s += Math.max(0, as.values[k] ?? 0);
      assetSums[k] = s;
    }
    const assetStacks = assetSeries.map(() => ({
      bottoms: new Array(monthCount).fill(0),
      tops: new Array(monthCount).fill(0),
    }));
    for (let k = 0; k < monthCount; k++) {
      const baseTotal = Math.max(0, nw[k] ?? 0);
      const sum = assetSums[k];
      let cursor = 0;
      for (let i = 0; i < assetSeries.length; i++) {
        const v = Math.max(0, assetSeries[i].values[k] ?? 0);
        const height = sum > 0 ? baseTotal * (v / sum) : 0;
        assetStacks[i].bottoms[k] = cursor;
        cursor += height;
        assetStacks[i].tops[k] = cursor;
      }
    }
    const minVisiblePoints = Math.min(pts.length, 12);
    const visibleCount = Math.max(
      minVisiblePoints,
      Math.min(pts.length, Math.round(viewWindow.count)),
    );
    const maxStart = Math.max(0, pts.length - visibleCount);
    const visibleStart = Math.max(
      0,
      Math.min(maxStart, Math.round(viewWindow.start)),
    );
    const visibleEnd = visibleStart + visibleCount - 1;
    const nwVisible = nw.slice(visibleStart, visibleEnd + 1);
    const ccVisible = cc.slice(visibleStart, visibleEnd + 1);
    const fireTargetVisible = fireTarget
      ? fireTarget.slice(visibleStart, visibleEnd + 1)
      : null;
    const assetVisibleValues = assetSeries.flatMap((as) =>
      as.values.slice(visibleStart, visibleEnd + 1),
    );
    const startNwParsed = parseDisplayDecimal(series.starting_net_worth);
    const startNw =
      startNwParsed !== null ? startNwParsed : nw[0] ?? 0;
    const allowNegativeAxis = startNw < 0;

    // El target FIRE inflado puede crecer muy por encima del patrimonio en horizontes largos;
    // dejarlo fuera del rango del eje Y para no aplastar la curva del patrimonio. La línea se
    // recorta visualmente por el clipPath del plot si excede.
    const dataMin = Math.min(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const dataMax = Math.max(...nwVisible, ...ccVisible, ...assetVisibleValues);
    const rawSpan = dataMax - dataMin;
    const padY =
      rawSpan > 0
        ? rawSpan * 0.07
        : Math.max(Math.abs(dataMax) * 0.06, 1);

    let plotMin = dataMin - padY;
    let plotMax = dataMax + padY;
    if (!allowNegativeAxis) {
      plotMin = Math.max(0, plotMin);
    }

    let yTicksRaw = niceYTicks(plotMin, plotMax, 6);
    let yTicks = allowNegativeAxis
      ? yTicksRaw
      : yTicksRaw.filter((t) => t >= 0);
    if (!allowNegativeAxis && yTicks.length < 2) {
      yTicks = niceYTicks(Math.max(0, plotMin), plotMax, 6);
    }
    let yMin = yTicks[0] ?? (allowNegativeAxis ? plotMin : Math.max(0, plotMin));
    let yMax = yTicks[yTicks.length - 1] ?? plotMax;
    if (!allowNegativeAxis && yMin < 0) {
      yMin = 0;
    }
    const xTicksAll = projectionXTicks(
      series.months,
      {
        ageUiMode,
        birthDateIso: userBirthDate,
        anchorDateYmd,
        calendarTz,
      },
      { plotWidthPx: layoutDims.pw },
    );
    const xTicks = xTicksAll.filter(
      (tick) => tick.monthIndex >= visibleStart && tick.monthIndex <= visibleEnd,
    );
    if (xTicks.length === 0 && visibleEnd > visibleStart) {
      xTicks.push({
        monthIndex: visibleEnd,
        label: projectionXTickLabel(visibleEnd, series.months, {
          ageUiMode,
          birthDateIso: userBirthDate,
          anchorDateYmd,
          calendarTz,
        }),
      });
    }

    const tickSpanPx =
      xTicks.length > 1 ? layoutDims.pw / (xTicks.length - 1) : layoutDims.pw;
    const rotateXLabels =
      xTicks.length > 11 || (xTicks.length > 5 && tickSpanPx < 46);
    const xAxisExtraBottom = rotateXLabels ? 38 : 0;
    const viewHeight = layoutDims.H + xAxisExtraBottom;

    const { W, H, ml, mr, mt, mb, pw, ph } = layoutDims;

    const xScale = (i: number) => {
      const local = i - visibleStart;
      return ml + (local / Math.max(1, visibleCount - 1)) * pw;
    };
    const compoundOutpaceMonth =
      series.compound_outpaces_true_savings_month_index ?? null;
    const visibleMilestones = milestones.filter(
      (m) =>
        m.reached_month_index >= visibleStart && m.reached_month_index <= visibleEnd,
    );
    const showCompoundOutpaceMarker =
      compoundOutpaceMonth != null &&
      compoundOutpaceMonth >= visibleStart &&
      compoundOutpaceMonth <= visibleEnd;
    const chartPlanningMarkers = (() => {
      const anchor = anchorDateYmd ? parseYmdComponents(anchorDateYmd) : null;
      if (!anchor) return [];
      const out: Array<{
        id: string;
        mi: number;
        title: string;
        direction: PlanningFlowDirectionApi;
      }> = [];
      for (const f of planningFlows) {
        if (!f.show_in_chart || !f.due_date) continue;
        const d = parseYmdComponents(f.due_date);
        if (!d) continue;
        const mi = (d.y - anchor.y) * 12 + (d.m - anchor.m);
        if (mi < visibleStart || mi > visibleEnd) continue;
        out.push({ id: f.id, mi, title: f.title, direction: f.direction });
      }
      return out;
    })();
    return {
      nw,
      cc,
      fireTargetVisible,
      assetSeries,
      assetStacks,
      nwVisible,
      ccVisible,
      allowNegativeAxis,
      targetYMin: yMin,
      targetYMax: yMax,
      xTicks,
      xScale,
      compoundOutpaceMonth,
      showCompoundOutpaceMarker,
      visibleMilestones,
      chartPlanningMarkers,
      pw,
      ph,
      ml,
      mr,
      mt,
      mb,
      W,
      H,
      rotateXLabels,
      viewHeight,
      visibleStart,
      visibleEnd,
      visibleCount,
    };
  }, [
    pts,
    series.months,
    series.starting_net_worth,
    series.asset_series,
    series.fire_target_series,
    layoutDims,
    ageUiMode,
    userBirthDate,
    anchorDateYmd,
    calendarTz,
    milestones,
    focusMode,
    inflationAdjusted,
    installationInflationPct,
    viewWindow.count,
    viewWindow.start,
    planningFlows,
  ]);

  if (!model) {
    return null;
  }

  const {
    nw,
    cc,
    fireTargetVisible,
    assetSeries,
    assetStacks,
    nwVisible,
    ccVisible,
    allowNegativeAxis,
    targetYMin,
    targetYMax,
    xTicks,
    xScale,
    compoundOutpaceMonth,
    showCompoundOutpaceMarker,
    visibleMilestones,
    chartPlanningMarkers,
    pw,
    ph,
    ml,
    mt,
    W,
    rotateXLabels,
    viewHeight,
    visibleStart,
    visibleEnd,
    visibleCount,
  } = model;

  useEffect(() => {
    animatedYDomainRef.current = animatedYDomain;
  }, [animatedYDomain]);

  useEffect(() => {
    if (targetYMax <= targetYMin) {
      setAnimatedYDomain({ min: targetYMin, max: targetYMax + 1 });
      return;
    }
    if (yAxisAnimRef.current != null) {
      cancelAnimationFrame(yAxisAnimRef.current);
      yAxisAnimRef.current = null;
    }
    const from = animatedYDomainRef.current ?? { min: targetYMin, max: targetYMax };
    const to = { min: targetYMin, max: targetYMax };
    const start = performance.now();
    const durationMs = 170;
    const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;
    const tick = (now: number) => {
      const t = Math.max(0, Math.min(1, (now - start) / durationMs));
      const eased = easeOutCubic(t);
      setAnimatedYDomain({
        min: from.min + (to.min - from.min) * eased,
        max: from.max + (to.max - from.max) * eased,
      });
      if (t < 1) {
        yAxisAnimRef.current = requestAnimationFrame(tick);
      } else {
        yAxisAnimRef.current = null;
      }
    };
    yAxisAnimRef.current = requestAnimationFrame(tick);
    return () => {
      if (yAxisAnimRef.current != null) {
        cancelAnimationFrame(yAxisAnimRef.current);
        yAxisAnimRef.current = null;
      }
    };
  }, [targetYMax, targetYMin]);

  const yMin = animatedYDomain?.min ?? targetYMin;
  const yMax = animatedYDomain?.max ?? targetYMax;
  const spanY = Math.max(1, yMax - yMin);
  const yScale = (v: number) => mt + ph - ((v - yMin) / spanY) * ph;
  const yTicksRaw = niceYTicks(yMin, yMax, 6);
  let yTicks = allowNegativeAxis ? yTicksRaw : yTicksRaw.filter((t) => t >= 0);
  if (!allowNegativeAxis && yTicks.length < 2) {
    yTicks = niceYTicks(Math.max(0, yMin), yMax, 6);
  }
  const nwPoints = nwVisible
    .map((v, i) =>
      `${xScale(visibleStart + i)},${yScale(v)}`,
    )
    .join(" ");
  const ccPoints = ccVisible
    .map((v, i) =>
      `${xScale(visibleStart + i)},${yScale(v)}`,
    )
    .join(" ");
  const firePoints = fireTargetVisible
    ? fireTargetVisible
        .map((v, i) => `${xScale(visibleStart + i)},${yScale(v)}`)
        .join(" ")
    : null;
  let areaD = "";
  if (nwVisible.length > 0) {
    const parts: string[] = [];
    nwVisible.forEach((v, i) => {
      const globalIndex = visibleStart + i;
      parts.push(
        `${i === 0 ? "M" : "L"} ${xScale(globalIndex)} ${yScale(v)}`,
      );
    });
    const xLast = xScale(visibleEnd);
    const x0 = xScale(visibleStart);
    const yBase = mt + ph;
    parts.push(`L ${xLast} ${yBase}`);
    parts.push(`L ${x0} ${yBase} Z`);
    areaD = parts.join(" ");
  }

  function pointerToIndex(clientX: number): number {
    const svg = svgRef.current;
    if (!svg) return 0;
    const rect = svg.getBoundingClientRect();
    const vb = svg.viewBox.baseVal;
    const xSvg = ((clientX - rect.left) / Math.max(rect.width, 1)) * vb.width;
    const local = xSvg - ml;
    const idx = Math.round((local / pw) * Math.max(1, visibleCount - 1)) + visibleStart;
    return Math.max(0, Math.min(pts.length - 1, idx));
  }

  function onPointerMove(e: PointerEvent<SVGSVGElement>) {
    const i = pointerToIndex(e.clientX);
    setHover(i);
    const wrap = wrapRef.current;
    if (wrap) {
      const r = wrap.getBoundingClientRect();
      setTipOffset({ x: e.clientX - r.left, y: e.clientY - r.top });
    }
  }

  function onPointerLeave() {
    setHover(null);
  }

  function onWheel(e: WheelEvent<SVGSVGElement>) {
    e.preventDefault();
    if (pts.length < 2) return;
    const panInput = e.shiftKey ? e.deltaY : e.deltaX;
    const isPan = Math.abs(panInput) > Math.abs(e.deltaY) || e.shiftKey;
    const minVisiblePoints = Math.min(pts.length, 12);
    if (isPan && visibleCount < pts.length) {
      const step = Math.max(1, Math.round(visibleCount * 0.08));
      const direction = panInput > 0 ? 1 : -1;
      const maxStart = Math.max(0, pts.length - visibleCount);
      const nextStart = Math.max(
        0,
        Math.min(maxStart, visibleStart + direction * step),
      );
      if (nextStart !== visibleStart) {
        setViewWindow({ start: nextStart, count: visibleCount });
      }
      return;
    }
    if (e.deltaY === 0) return;
    const zoomFactor = e.deltaY < 0 ? 0.88 : 1.14;
    const nextCountRaw = Math.round(visibleCount * zoomFactor);
    const nextCount = Math.max(
      minVisiblePoints,
      Math.min(pts.length, nextCountRaw),
    );
    if (nextCount === visibleCount) return;
    const anchorIndex = pointerToIndex(e.clientX);
    const ratioWithinWindow =
      (anchorIndex - visibleStart) / Math.max(1, visibleCount - 1);
    const nextStartRaw = Math.round(
      anchorIndex - ratioWithinWindow * (nextCount - 1),
    );
    const maxStart = Math.max(0, pts.length - nextCount);
    const nextStart = Math.max(0, Math.min(maxStart, nextStartRaw));
    setViewWindow({ start: nextStart, count: nextCount });
  }

  const horizonLine = formatProjectionChartHorizonLine(series);
  const deltaStr = formatCurrencyAmount(series.monthly_delta_assumption, currencyIso);
  const scopeShort = ledgerPersonScope === "mine" ? "Mi vista" : "Hogar";
  const inflationShort =
    installationInflationPct > 0
      ? inflationAdjusted
        ? `Dinero de hoy (deflactado ~${inflationPctDisplay ?? `${installationInflationPct}%`} anual)`
        : `Patrimonio nominal · target FIRE +${inflationPctDisplay ?? `${installationInflationPct}%`} anual`
      : "Sin inflación · target FIRE plano";

  return (
    <div
      ref={wrapRef}
      className="projection-chart-root projection-chart-root--fullbleed bordered-top"
    >
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${viewHeight}`}
        preserveAspectRatio="xMidYMid meet"
        className="projection-chart-svg"
        style={{
          aspectRatio: `${W} / ${viewHeight}`,
        }}
        role="application"
        aria-label="Proyección de patrimonio neto y capital aportado acumulado"
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
        onWheel={onWheel}
      >
        <title>Patrimonio neto y capital aportado en el tiempo</title>
        <defs>
          <linearGradient id={`nwFill-${gid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#10b981" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#10b981" stopOpacity="0.03" />
          </linearGradient>
          <clipPath id={`projectionPlotClip-${gid}`}>
            <rect x={ml} y={mt} width={pw} height={ph} rx={6} />
          </clipPath>
        </defs>

        <text x={ml} y={layoutDims.headlineBlockTopY} className="projection-chart-headline">
          {scopeShort}
        </text>
        <text x={ml} y={layoutDims.headlineBlockTopY + 22} className="projection-chart-meta">
          {horizonLine}
        </text>
        <text x={ml} y={layoutDims.headlineBlockTopY + 40} className="projection-chart-meta">
          {inflationShort} · Δ regular presup. {deltaStr}/mes
        </text>

        <g
          transform={`translate(${layoutDims.legend.x}, ${layoutDims.legend.y})`}
          className="projection-chart-legend"
        >
          {(() => {
            const p = layoutDims.legendPlacements;
            const items: ReactNode[] = [];
            if (p[0]) {
              items.push(
                <g key="legend-nw" transform={`translate(${p[0].x}, ${p[0].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#047857" strokeWidth={3} strokeLinecap="round" />
                  <text x={28} y={15}>Patrimonio neto</text>
                </g>,
              );
            }
            if (p[1]) {
              items.push(
                <g key="legend-cc" transform={`translate(${p[1].x}, ${p[1].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#b45309" strokeWidth={2.25} strokeDasharray="6 5" strokeLinecap="round" />
                  <text x={28} y={15}>Capital aportado</text>
                </g>,
              );
            }
            const fireOffset = hasFireTargetSeries ? 1 : 0;
            if (hasFireTargetSeries && p[2]) {
              items.push(
                <g key="legend-fire" transform={`translate(${p[2].x}, ${p[2].y})`}>
                  <line x1={0} y1={11} x2={22} y2={11} stroke="#7c3aed" strokeWidth={1.5} strokeDasharray="3 4" strokeLinecap="round" opacity={0.5} />
                  <text x={28} y={15}>Target FIRE</text>
                </g>,
              );
            }
            assetSeries.forEach((as, idx) => {
              const pos = p[2 + fireOffset + idx];
              if (!pos) return;
              const color = ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length];
              items.push(
                <g key={`legend-${as.id}`} transform={`translate(${pos.x}, ${pos.y})`}>
                  <rect
                    x={0}
                    y={6}
                    width={20}
                    height={11}
                    rx={2}
                    fill={color}
                    fillOpacity={0.14}
                    stroke={color}
                    strokeOpacity={0.4}
                    strokeWidth={0.8}
                  />
                  <text x={26} y={15}>{as.name}</text>
                </g>,
              );
            });
            return items;
          })()}
        </g>

        {yTicks.map((yt) => (
          <g key={`gy-${yt}`}>
            <line
              x1={ml}
              y1={yScale(yt)}
              x2={ml + pw}
              y2={yScale(yt)}
              className="projection-chart-grid"
            />
            <text
              x={ml - 10}
              y={yScale(yt)}
              textAnchor="end"
              dominantBaseline="middle"
              className="projection-chart-tick"
            >
              {formatAxisMoney(yt, currencyIso)}
            </text>
          </g>
        ))}

        {xTicks.map(({ monthIndex, label }) => {
          const cx = xScale(monthIndex);
          const tickY =
            mt + ph + (layoutDims.narrow ? 12 : 14) + (rotateXLabels ? 8 : 0);
          return (
            <text
              key={`gx-${monthIndex}`}
              transform={
                rotateXLabels
                  ? `rotate(38 ${cx.toFixed(2)} ${tickY.toFixed(2)})`
                  : undefined
              }
              x={cx}
              y={tickY}
              textAnchor="start"
              dominantBaseline={rotateXLabels ? "middle" : "auto"}
              className={`projection-chart-tick${rotateXLabels ? " projection-chart-tick--xrot" : ""}`}
            >
              {label}
            </text>
          );
        })}

        <rect
          x={ml}
          y={mt}
          width={pw}
          height={ph}
          fill="#ffffff"
          fillOpacity={0.35}
          rx={6}
          className="projection-chart-plot-bg"
        />

        <g clipPath={`url(#projectionPlotClip-${gid})`}>
          {assetSeries.length === 0 ? (
            <path d={areaD} fill={`url(#nwFill-${gid})`} stroke="none" />
          ) : (
            assetSeries.map((as, idx) => {
              const color = ASSET_LINE_COLORS[idx % ASSET_LINE_COLORS.length];
              const stack = assetStacks[idx];
              const topParts: string[] = [];
              const botParts: string[] = [];
              for (let k = visibleStart; k <= visibleEnd; k++) {
                topParts.push(`${xScale(k)},${yScale(stack.tops[k])}`);
              }
              for (let k = visibleEnd; k >= visibleStart; k--) {
                botParts.push(`${xScale(k)},${yScale(stack.bottoms[k])}`);
              }
              const d = `M ${topParts.join(" L ")} L ${botParts.join(" L ")} Z`;
              return (
                <path
                  key={as.id}
                  d={d}
                  fill={color}
                  fillOpacity={0.14}
                  stroke={color}
                  strokeWidth={0.8}
                  strokeOpacity={0.4}
                />
              );
            })
          )}
          <polyline
            points={nwPoints}
            fill="none"
            stroke="#047857"
            strokeWidth={2.85}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <polyline
            points={ccPoints}
            fill="none"
            stroke="#b45309"
            strokeWidth={2.1}
            strokeDasharray="7 5"
            strokeLinecap="round"
            strokeLinejoin="round"
            opacity={0.92}
          />
          {firePoints ? (
            <polyline
              points={firePoints}
              fill="none"
              stroke="#7c3aed"
              strokeWidth={1.2}
              strokeDasharray="3 4"
              strokeLinecap="round"
              strokeLinejoin="round"
              opacity={0.225}
            />
          ) : null}
          {visibleMilestones.map((m) => {
            const x = xScale(m.reached_month_index);
            const y0 = mt + ph;
            const nwAtMilestone = nw[m.reached_month_index] ?? null;
            const nwY =
              nwAtMilestone != null && Number.isFinite(nwAtMilestone)
                ? yScale(nwAtMilestone)
                : null;
            // Mantiene la marca siempre por encima de la curva de patrimonio neto.
            const y1Floor = mt + 12;
            const y1FromNetWorth = nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
            const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
            const isJubilacion = m.target === "jubilacion";
            const targetNum = parseDisplayDecimal(m.target);
            const label =
              targetNum != null
                ? formatCurrencyNumber(targetNum, currencyIso)
                : formatProjectionMilestoneCompactLabel(m.target);
            return (
              <g key={`ms-${m.target}-${m.reached_month_index}`}>
                <line
                  x1={x}
                  x2={x}
                  y1={y0}
                  y2={y1}
                  className={isJubilacion ? "projection-chart-jubilacion-line" : "projection-chart-milestone-line"}
                />
                <text
                  x={x}
                  y={y1 - 6}
                  textAnchor="middle"
                  className={isJubilacion ? "projection-chart-jubilacion-label" : "projection-chart-milestone-label"}
                >
                  {label}
                </text>
              </g>
            );
          })}
          {chartPlanningMarkers.map((m) => {
            const x = xScale(m.mi);
            const y0 = mt + ph;
            const nwAtMi = nw[m.mi] ?? null;
            const nwY =
              nwAtMi != null && Number.isFinite(nwAtMi) ? yScale(nwAtMi) : null;
            const y1Floor = mt + 12;
            const y1FromNetWorth =
              nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
            const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
            const isInflow = m.direction === "inflow";
            const label =
              m.title.length > 18 ? m.title.slice(0, 17) + "…" : m.title;
            return (
              <g key={`pf-${m.id}`}>
                <line
                  x1={x}
                  x2={x}
                  y1={y0}
                  y2={y1}
                  className={
                    isInflow
                      ? "projection-chart-planning-inflow-line"
                      : "projection-chart-planning-outflow-line"
                  }
                />
                <text
                  x={x}
                  y={y1 - 6}
                  textAnchor="middle"
                  className={
                    isInflow
                      ? "projection-chart-planning-inflow-label"
                      : "projection-chart-planning-outflow-label"
                  }
                >
                  {label}
                </text>
              </g>
            );
          })}
          {showCompoundOutpaceMarker && compoundOutpaceMonth != null
            ? (() => {
                const x = xScale(compoundOutpaceMonth);
                const y0 = mt + ph;
                const nwAtMs = nw[compoundOutpaceMonth] ?? null;
                const nwY =
                  nwAtMs != null && Number.isFinite(nwAtMs)
                    ? yScale(nwAtMs)
                    : null;
                const y1Floor = mt + 12;
                const y1FromNetWorth =
                  nwY != null ? nwY - 12 : y0 - Math.min(44, ph * 0.22);
                const y1 = Math.max(y1Floor, Math.min(y0 - 8, y1FromNetWorth));
                return (
                  <g>
                    <line
                      x1={x}
                      x2={x}
                      y1={y0}
                      y2={y1}
                      className="projection-chart-milestone-line"
                    />
                    <text
                      x={x}
                      y={y1 - 6}
                      textAnchor="middle"
                      className="projection-chart-milestone-label"
                    >
                      Interés &gt; ahorro
                    </text>
                  </g>
                );
              })()
            : null}

          {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
            <line
              x1={xScale(hover)}
              x2={xScale(hover)}
              y1={mt}
              y2={mt + ph}
              className="projection-chart-crosshair"
            />
          ) : null}
          {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
            <>
              <circle cx={xScale(hover)} cy={yScale(nw[hover])} r={6} className="projection-chart-dot-nw" />
              <circle cx={xScale(hover)} cy={yScale(cc[hover])} r={5} className="projection-chart-dot-cc" />
            </>
          ) : null}
        </g>

        <text
          transform={`translate(${Math.min(30, ml * 0.32)}, ${mt + ph / 2}) rotate(-90)`}
          textAnchor="middle"
          className="projection-chart-axis-caption"
        >
          {normalizeCurrencyIso(currencyIso) ?? "Importe"}
        </text>
      </svg>

      {hover !== null && hover >= visibleStart && hover <= visibleEnd ? (
        <div
          className="projection-chart-tooltip"
          style={{
            left: tipOffset.x,
            top: tipOffset.y,
          }}
        >
          <div className="projection-chart-tooltip-title">
            {projectionHoverTitle(
              hover,
              ageUiMode,
              userBirthDate,
              calendarTz,
              anchorDateYmd,
            )}
          </div>
          <div>
            Patrimonio neto —{" "}
            {formatCurrencyOrDash(pts[hover]?.net_worth, currencyIso)}
          </div>
          <div>
            Capital aportado —{" "}
            {formatCurrencyOrDash(
              pts[hover]?.contributed_capital,
              currencyIso,
            )}
          </div>
          {assetSeries.map((as) => (
            <div key={as.id}>
              {as.name} —{" "}
              {formatCurrencyOrDash(
                series.asset_series?.find((s) => s.asset_id === as.id)?.values[hover] ?? undefined,
                currencyIso,
              )}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function RetirementView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  projectionSeries,
  projectionBusy,
  retirementBudgetSnapshot,
  retirementBusy,
  retirementError,
  user,
  calendarTz,
  canEditFire,
  onSaveFire,
  navigate,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  projectionBusy: boolean;
  retirementBudgetSnapshot: BudgetSnapshotApi | null;
  retirementBusy: boolean;
  retirementError: string | null;
  user: UserResponse | null;
  calendarTz: string;
  canEditFire: boolean;
  onSaveFire: (fs: FireSettingsApi) => Promise<void>;
  navigate: (path: string, replace?: boolean) => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const [fireDraft, setFireDraft] = useState<FireSettingsApi>(() =>
    defaultFireSettingsApi(),
  );
  const lastSavedFirePayloadRef = useRef<string>("");
  const fireSaveTimerRef = useRef(0);
  const fireSaveSeqRef = useRef(0);

  useEffect(() => {
    setFireDraft(
      normalizeInstallationFireSettings(
        installation?.installation.fire_settings,
      ),
    );
    const serverFs = normalizeInstallationFireSettings(
      installation?.installation.fire_settings,
    );
    lastSavedFirePayloadRef.current = JSON.stringify(serverFs);
  }, [installation?.installation.id]);

  const axisAgeMode = projectionSeries
    ? resolveProjectionAxisAgeMode(projectionSeries, installation)
    : "dates";
  const axisBirth = (() => {
    const fromApi = projectionSeries?.viewer_birth_date?.trim();
    const fromUser = user?.birth_date?.trim();
    const pick =
      fromApi && fromApi.length > 0
        ? fromApi
        : fromUser && fromUser.length > 0
          ? fromUser
          : null;
    return pick;
  })();
  const axisAnchor = projectionSeries?.anchor_date_ymd?.trim() || null;

  const retirementMetricsReady =
    hasMembership &&
    !projectionBusy &&
    !retirementBusy &&
    projectionSeries != null &&
    retirementBudgetSnapshot != null;

  const installationInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    return n != null && n > 0 ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  const fireKpis = useMemo(() => {
    const expenseM =
      retirementBudgetSnapshot?.totals.expense_regular_monthly_equivalent;
    const incomeM =
      retirementBudgetSnapshot?.totals.income_monthly_equivalent;
    const incomeRetM =
      retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent;
    const needAnnual = computeFireAnnualNeedNetEur(
      fireDraft,
      expenseM,
      incomeM,
      incomeRetM,
    );
    const swrN = parseDisplayDecimal(fireDraft.swr_pct);
    const brackets = fireDraft.tax_brackets;
    const taxOn = fireDraft.taxes_enabled;

    let targetNoPen: number | null = null;
    let grossNoPen: number | null = null;

    if (needAnnual !== null && needAnnual > 0 && swrN !== null && swrN > 0) {
      grossNoPen = grossUpNetAnnualFire(needAnnual, brackets, taxOn);
      targetNoPen = grossNoPen / (swrN / 100);
    }

    const pts = projectionSeries?.points ?? [];
    const mc = projectionSeries?.months ?? 0;

    let miNo: number | null = null;
    if (targetNoPen !== null && targetNoPen > 0) {
      miNo = findFirstMonthNetWorthAtLeastInflated(
        pts,
        targetNoPen,
        installationInflationPct,
      );
    }

    let targetAtCross: number | null = null;
    if (
      targetNoPen !== null &&
      targetNoPen > 0 &&
      miNo !== null &&
      installationInflationPct > 0
    ) {
      targetAtCross =
        targetNoPen * Math.pow(1 + installationInflationPct / 100, miNo / 12);
    }

    return {
      needAnnual,
      swrN,
      targetNoPen,
      targetAtCross,
      miNo,
      mc,
    };
  }, [
    fireDraft,
    installationInflationPct,
    retirementBudgetSnapshot?.totals.expense_regular_monthly_equivalent,
    retirementBudgetSnapshot?.totals.income_monthly_equivalent,
    retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent,
    projectionSeries?.points,
    projectionSeries?.months,
  ]);

  const renderRetirementAmount = useCallback(
    (annual: number, monthly: number): ReactNode => (
      <>
        {formatCurrencyNumber(annual, currencyIso)}{" "}
        <span className="retirement-mode-monthly">
          ({formatCurrencyNumber(monthly, currencyIso)}/mes)
        </span>
      </>
    ),
    [currencyIso],
  );

  const retirementObjectiveManualAnnualDisplay = useMemo<ReactNode>(() => {
    const m = parseDisplayDecimal(
      String(fireDraft.fire_number_manual_amount ?? ""),
    );
    if (!(m !== null && m > 0)) return METRIC_DASH;
    return renderRetirementAmount(m, m / 12);
  }, [fireDraft.fire_number_manual_amount, renderRetirementAmount]);

  const retirementObjectiveExpenseAnnualDisplay = useMemo<ReactNode>(() => {
    const baseM = parseDisplayDecimal(
      String(
        retirementBudgetSnapshot?.totals.expense_regular_monthly_equivalent ??
          "",
      ),
    );
    if (!(baseM !== null && baseM >= 0)) return METRIC_DASH;
    return renderRetirementAmount(baseM * 12, baseM);
  }, [
    retirementBudgetSnapshot?.totals.expense_regular_monthly_equivalent,
    renderRetirementAmount,
  ]);

  const retirementObjectiveIncomeAnnualDisplay = useMemo<ReactNode>(() => {
    const incM = parseDisplayDecimal(
      String(retirementBudgetSnapshot?.totals.income_monthly_equivalent ?? ""),
    );
    if (!(incM !== null && incM >= 0)) return METRIC_DASH;
    return renderRetirementAmount(incM * 12, incM);
  }, [
    retirementBudgetSnapshot?.totals.income_monthly_equivalent,
    renderRetirementAmount,
  ]);

  const skipFireAutosaveRef = useRef(true);

  useEffect(() => {
    skipFireAutosaveRef.current = true;
  }, [installation?.installation.id]);

  const runFireSave = useCallback(() => {
    if (!hasMembership || !canEditFire) return;
    const swrN = parseDisplayDecimal(fireDraft.swr_pct);
    if (swrN === null || swrN < 0 || swrN > 4) {
      return;
    }
    if (
      fireDraft.fire_number_mode === "manual" &&
      (fireDraft.fire_number_manual_amount == null ||
        String(fireDraft.fire_number_manual_amount).trim() === "")
    ) {
      return;
    }
    const payloadJson = JSON.stringify(fireDraft);
    if (payloadJson === lastSavedFirePayloadRef.current) return;
    const seq = ++fireSaveSeqRef.current;
    void onSaveFire(fireDraft)
      .then(() => {
        if (seq !== fireSaveSeqRef.current) return;
        lastSavedFirePayloadRef.current = payloadJson;
      })
      .catch(() => {});
  }, [fireDraft, hasMembership, canEditFire, onSaveFire]);

  const queueFireSave = useCallback(
    (delayMs: number) => {
      window.clearTimeout(fireSaveTimerRef.current);
      fireSaveTimerRef.current = window.setTimeout(() => {
        fireSaveTimerRef.current = 0;
        runFireSave();
      }, delayMs);
    },
    [runFireSave],
  );

  useEffect(() => {
    if (!hasMembership || !canEditFire) return;
    if (skipFireAutosaveRef.current) {
      skipFireAutosaveRef.current = false;
      return;
    }
    queueFireSave(420);
    return () => {
      window.clearTimeout(fireSaveTimerRef.current);
    };
  }, [fireDraft, hasMembership, canEditFire, queueFireSave]);

  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState !== "hidden") return;
      window.clearTimeout(fireSaveTimerRef.current);
      runFireSave();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [runFireSave]);

  const lblOpts = {
    birthDateIso: axisBirth,
    anchorDateYmd: axisAnchor,
    calendarTz,
  };

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Jubilación</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {retirementError ? (
        <div className="banner error-banner">{retirementError}</div>
      ) : null}

      {installationInflationPct <= 0 ? (
        <div className="banner info-banner">
          Inflación a 0%: el target FIRE queda plano en euros de hoy. La fecha objetivo puede ser optimista respecto a tu poder adquisitivo real.{" "}
          <a
            href={TAB_PATH.settings}
            onClick={(e) => {
              if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                return;
              e.preventDefault();
              navigate(TAB_PATH.settings);
            }}
          >
            Ajustes
          </a>
          .
        </div>
      ) : null}

      {hasMembership ? (
        <>
          <div className="metric-grid workspace-kpi-strip">
            <MetricCard
              label="Patrimonio objetivo (Actual + Inf. Adj.)"
              value={
                retirementMetricsReady &&
                fireKpis.targetNoPen !== null &&
                fireKpis.targetNoPen > 0
                  ? formatCurrencyNumber(fireKpis.targetNoPen, currencyIso)
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.targetAtCross !== null &&
                fireKpis.targetAtCross > 0
                  ? formatCurrencyNumber(fireKpis.targetAtCross, currencyIso)
                  : undefined
              }
            />
            <MetricCard
              label="Primer cruce"
              value={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0
                  ? `~${projectionXTickLabel(fireKpis.miNo, fireKpis.mc, {
                      ageUiMode: axisAgeMode,
                      birthDateIso: axisBirth,
                      anchorDateYmd: axisAnchor,
                      calendarTz,
                    })}`
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0
                  ? complementaryProjectionTickLabel(
                      fireKpis.miNo,
                      fireKpis.mc,
                      axisAgeMode,
                      lblOpts,
                    )
                  : ""
              }
            />
            <MetricCard
              label="Años hasta el cruce"
              value={
                retirementMetricsReady && fireKpis.miNo !== null
                  ? formatYearsEsFromMonths(fireKpis.miNo)
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0 &&
                fireKpis.miNo > fireKpis.mc
                  ? "Fuera del horizonte de la proyección actual."
                  : ""
              }
            />
          </div>
          {retirementMetricsReady &&
          fireKpis.swrN !== null &&
          fireKpis.swrN <= 0 ? (
            <p className="muted tight">
              SWR 0 %: no se calcula fecha de cruce.
            </p>
          ) : null}
        </>
      ) : null}

      {!canEditFire ? (
        <p className="muted tight">
          Solo el propietario puede editar esta configuración.
        </p>
      ) : null}

      <section className="panel">
        <h3 className="panel-title">Objetivo anual <span className="muted">(en dinero de hoy)</span></h3>
        <div className="stack bordered-top retirement-config-stack">
          <fieldset disabled={!canEditFire} className="stack retirement-config-stack">
            <div className="retirement-mode-grid" role="radiogroup" aria-label="Modo objetivo anual">
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "manual" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "manual"}
                  onChange={() =>
                    setFireDraft((p) => ({ ...p, fire_number_mode: "manual" }))
                  }
                />
                <span className="retirement-mode-name">Manual</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveManualAnnualDisplay}
                </span>
              </label>
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "annual_expense" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "annual_expense"}
                  onChange={() =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_mode: "annual_expense",
                    }))
                  }
                />
                <span className="retirement-mode-name">Gasto actual</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveExpenseAnnualDisplay}
                </span>
              </label>
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "current_income" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "current_income"}
                  onChange={() =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_mode: "current_income",
                    }))
                  }
                />
                <span className="retirement-mode-name">Ingresos actuales</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveIncomeAnnualDisplay}
                </span>
              </label>
            </div>

            {fireDraft.fire_number_mode === "manual" ? (
              <label className="field">
                <span>Gasto anual neto objetivo</span>
                <input
                  inputMode="decimal"
                  value={fireDraft.fire_number_manual_amount ?? ""}
                  onChange={(e) =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_manual_amount:
                        e.target.value.trim() === ""
                          ? null
                          : e.target.value.replace(",", "."),
                    }))
                  }
                  onBlur={() => queueFireSave(0)}
                />
              </label>
            ) : null}
          </fieldset>
        </div>
      </section>

      <section className="panel">
        <h3 className="panel-title">Retirada</h3>
        <div className="stack bordered-top retirement-config-stack">
          <fieldset disabled={!canEditFire} className="stack retirement-config-stack">
            <label className="field">
              <span>Retirada anual (SWR)</span>
              <input
                type="range"
                min={0}
                max={40}
                step={1}
                value={Math.round(
                  (parseDisplayDecimal(fireDraft.swr_pct) ?? 0) * 10,
                )}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  setFireDraft((p) => ({
                    ...p,
                    swr_pct: String(v / 10),
                  }));
                }}
                onBlur={() => queueFireSave(0)}
              />
              <span className="muted tight">
                {formatPercentAmount(fireDraft.swr_pct)}
              </span>
            </label>
          </fieldset>
        </div>
      </section>

      {hasMembership &&
      !projectionBusy &&
      !retirementBusy &&
      (!projectionSeries || !retirementBudgetSnapshot) ? (
        <div className="banner info-banner">Sin datos.</div>
      ) : null}
    </div>
  );
}


function ProjectionView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  projectionSeries,
  projectionBusy,
  projectionError,
  userBirthDate,
  calendarTz,
  planningFlows,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  projectionBusy: boolean;
  projectionError: string | null;
  userBirthDate: string | null;
  calendarTz: string;
  planningFlows: PlanningFlowApiRow[];
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const inflationPctRaw =
    installation?.installation.annual_inflation_assumption_percent;
  const inflationPctDisplay =
    inflationPctRaw != null && String(inflationPctRaw).trim() !== ""
      ? formatPercentAmount(String(inflationPctRaw))
      : null;

  const axisAgeMode = projectionSeries
    ? resolveProjectionAxisAgeMode(projectionSeries, installation)
    : "dates";
  const axisBirth = (() => {
    const fromApi = projectionSeries?.viewer_birth_date?.trim();
    const fromUser = userBirthDate?.trim();
    const pick =
      fromApi && fromApi.length > 0
        ? fromApi
        : fromUser && fromUser.length > 0
          ? fromUser
          : null;
    return pick;
  })();
  const axisAnchor =
    projectionSeries?.anchor_date_ymd?.trim() || null;
  const jubilacionMiNo = projectionSeries?.jubilacion_month_index ?? null;
  const jubilacionTargetNoPen =
    projectionSeries?.jubilacion_target_net_worth != null
      ? parseDisplayDecimal(projectionSeries.jubilacion_target_net_worth)
      : null;

  const nextMilestones: ProjectionMilestoneApi[] = (() => {
    const base = projectionSeries?.milestones ?? [];
    if (jubilacionMiNo !== null) {
      return [
        ...base,
        { target: "jubilacion", reached_month_index: jubilacionMiNo, reached_date_ymd: "" },
      ];
    }
    return base;
  })();
  const [focusMode, setFocusMode] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    try {
      return window.localStorage.getItem(PROJECTION_FOCUS_STORAGE_KEY) === "1";
    } catch {
      return false;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(
        PROJECTION_FOCUS_STORAGE_KEY,
        focusMode ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }, [focusMode]);

  const [inflationAdjusted, setInflationAdjusted] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    try {
      const v = window.localStorage.getItem(
        PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
      );
      return v == null ? true : v === "1";
    } catch {
      return true;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(
        PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
        inflationAdjusted ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }, [inflationAdjusted]);

  const projectionInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    return n != null && n > 0 ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  return (
    <div className="workspace workspace--projection-fullwidth">
      <div className="workspace-header">
        <div className="projection-header-main">
          <h2 className="workspace-title">Proyección</h2>
          <label className="projection-focus-toggle">
            <span className="projection-focus-toggle-label">Focus</span>
            <input
              type="checkbox"
              role="switch"
              checked={focusMode}
              onChange={(e) => setFocusMode(e.target.checked)}
              aria-label="Activar focus en la proyección"
            />
            <span className="projection-focus-toggle-track" aria-hidden="true">
              <span className="projection-focus-toggle-thumb" />
            </span>
          </label>
          <label className="projection-focus-toggle">
            <span className="projection-focus-toggle-label">Inflation Adjusted</span>
            <input
              type="checkbox"
              role="switch"
              checked={inflationAdjusted}
              onChange={(e) => setInflationAdjusted(e.target.checked)}
              aria-label="Mostrar la proyección ajustada a inflación (en dinero de hoy)"
            />
            <span className="projection-focus-toggle-track" aria-hidden="true">
              <span className="projection-focus-toggle-thumb" />
            </span>
          </label>
        </div>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {projectionError ? (
        <div className="banner error-banner">{projectionError}</div>
      ) : null}

      {hasMembership && projectionBusy ? (
        <p className="muted tight">Cargando serie…</p>
      ) : null}

      {hasMembership && !projectionBusy && projectionSeries ? (
        <section className="panel">
          <h3 className="panel-title">Trayectoria proyectada</h3>
          {nextMilestones.length > 0 ? (
            <div className="metric-grid workspace-kpi-strip">
              {nextMilestones.map((m) => {
                const isJubilacion = m.target === "jubilacion";
                return (
                  <MetricCard
                    key={`${m.target}-${m.reached_month_index}`}
                    label={isJubilacion ? "Jubilación" : "Milestone"}
                    value={
                      isJubilacion
                        ? (jubilacionTargetNoPen !== null
                            ? formatCurrencyNumber(jubilacionTargetNoPen, currencyIso)
                            : METRIC_DASH)
                        : formatProjectionMilestoneCompactLabel(m.target)
                    }
                    parenthetical={`~${projectionXTickLabel(
                      m.reached_month_index,
                      projectionSeries.months,
                      {
                        ageUiMode: axisAgeMode,
                        birthDateIso: axisBirth,
                        anchorDateYmd: axisAnchor,
                        calendarTz,
                      },
                    )}`}
                  />
                );
              })}
            </div>
          ) : null}
          <ProjectionNetWorthChart
            series={projectionSeries}
            milestones={nextMilestones}
            focusMode={focusMode}
            inflationAdjusted={inflationAdjusted}
            installationInflationPct={projectionInflationPct}
            currencyIso={currencyIso}
            ledgerPersonScope={ledgerPersonScope}
            inflationPctDisplay={inflationPctDisplay}
            ageUiMode={axisAgeMode}
            userBirthDate={axisBirth}
            anchorDateYmd={axisAnchor}
            calendarTz={calendarTz}
            planningFlows={planningFlows}
          />
        </section>
      ) : null}
    </div>
  );
}

function formatAllocationCap(
  capKind: AllocationRuleCapKind | null | undefined,
  capValue: string | null | undefined,
  currencyIso: string,
): string {
  if (!capKind || capValue == null) return "—";
  const n = parseDisplayDecimal(String(capValue));
  if (n == null) return "—";
  switch (capKind) {
    case "amount":
      return formatCurrencyNumber(n, currencyIso);
    case "months_expense":
      return `${formatPercentAmount(n.toString()).replace(" %", "")} × gasto`;
    case "income_multiple":
      return `${formatPercentAmount(n.toString()).replace(" %", "")} × ingreso`;
    default:
      return "—";
  }
}

function formatAllocationAmount(
  kind: AllocationRuleKind,
  amount: string | null | undefined,
  currencyIso: string,
): string {
  if (kind === "remainder") return "Resto";
  if (amount == null) return "—";
  const n = parseDisplayDecimal(String(amount));
  if (n == null) return "—";
  if (kind === "fixed") return formatCurrencyNumber(n, currencyIso);
  if (kind === "percent") return formatPercentDisplay(n);
  return "—";
}

function AllocationRulesPanel({
  assets,
  rules,
  busy,
  error,
  canEdit,
  currencyIso,
  openNewRuleModal,
  beginEditRule,
  deleteRule,
  moveRule,
  embedded = false,
}: {
  assets: AssetApiRow[];
  rules: AllocationRuleApiRow[];
  busy: boolean;
  error: string | null;
  canEdit: boolean;
  currencyIso: string;
  openNewRuleModal: () => void;
  beginEditRule: (r: AllocationRuleApiRow) => void;
  deleteRule: (id: string) => void;
  moveRule: (id: string, dir: "up" | "down") => void;
  embedded?: boolean;
}) {
  const assetById = new Map(assets.map((a) => [a.id, a.name]));
  const sinkIndex = rules.findIndex(
    (r) => r.kind === "remainder" && !r.cap_kind,
  );
  const hasSink = sinkIndex >= 0;

  return (
    <section
      className={
        embedded
          ? "allocation-rules-panel allocation-rules-panel--embedded"
          : "panel allocation-rules-panel"
      }
    >
      {embedded ? (
        canEdit ? (
          <div className="panel-head-row">
            <span />
            <button
              type="button"
              className="btn primary icon-btn"
              aria-label="Nueva regla"
              onClick={openNewRuleModal}
              disabled={assets.length === 0}
            >
              <PlusIcon />
            </button>
          </div>
        ) : null
      ) : (
        <div className="panel-head-row">
          <h3 className="panel-title">Asignación del sobrante</h3>
          {canEdit ? (
            <button
              type="button"
              className="btn primary icon-btn"
              aria-label="Nueva regla"
              onClick={openNewRuleModal}
              disabled={assets.length === 0}
            >
              <PlusIcon />
            </button>
          ) : null}
        </div>
      )}
      <p className="muted tight">
        Cascada en orden ascendente. Cada mes, sobre el sobrante (ingresos −
        gastos − cuotas de deuda + flujos puntuales de Próximos), cada regla
        coge su parte y lo que queda baja a la siguiente:
      </p>
      <ul className="muted tight allocation-rules-help">
        <li>
          <strong>Fija (€)</strong>: aporta exactamente esa cantidad si hay
          sobrante disponible.
        </li>
        <li>
          <strong>%</strong>: aporta ese porcentaje del sobrante que queda
          <em> en ese paso de la cascada</em> (no del sobrante total inicial).
        </li>
        <li>
          <strong>Resto</strong>: absorbe lo que quede después de las anteriores.
          La regla resto <em>sin tope</em> es única por usuario y siempre va al
          final. Puedes poner varias reglas resto <em>con tope</em> antes (p.ej.
          "fondo emergencia hasta 3 meses de gasto") y la cascada saltará la
          regla cuando se llene.
        </li>
      </ul>
      {error ? <p className="form-error">{error}</p> : null}
      {assets.length === 0 ? (
        <p className="muted">Crea activos primero para poder asignar reglas.</p>
      ) : busy ? (
        <p className="muted">Cargando…</p>
      ) : rules.length === 0 ? (
        <p className="muted">
          Sin reglas. El sobrante mensual quedará como efectivo.
        </p>
      ) : (
        <>
          {!hasSink ? (
            <div className="banner info-banner">
              Falta una regla <strong>Resto sin tope</strong> al final: el
              sobrante no asignado quedará como efectivo.
            </div>
          ) : hasSink && sinkIndex !== rules.length - 1 ? (
            <div className="banner info-banner">
              La regla <strong>Resto sin tope</strong> debe ser la última. Las
              reglas posteriores (#{sinkIndex + 2}…) recibirán 0&nbsp;€.
            </div>
          ) : null}
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Destino</th>
                  <th>Tipo</th>
                  <th className="num">Cantidad</th>
                  <th className="num">Tope</th>
                  {canEdit ? (
                    <th className="asset-actions-cell">
                      <span className="sr-only">Acciones</span>
                    </th>
                  ) : null}
                </tr>
              </thead>
              <tbody>
                {rules.map((r, i) => (
                  <tr key={r.id}>
                    <td>{i + 1}</td>
                    <td>{assetById.get(r.target_asset_id) ?? "—"}</td>
                    <td>
                      {r.kind === "fixed"
                        ? "Fija"
                        : r.kind === "percent"
                          ? "Porcentaje"
                          : "Resto"}
                    </td>
                    <td className="num">
                      {formatAllocationAmount(r.kind, r.amount, currencyIso)}
                    </td>
                    <td className="num muted">
                      {formatAllocationCap(r.cap_kind, r.cap_value, currencyIso)}
                    </td>
                    {canEdit ? (
                      <td className="asset-actions-cell">
                        <div className="budget-row-actions">
                          <button
                            type="button"
                            className="btn ghost icon-btn"
                            aria-label="Subir prioridad"
                            disabled={i === 0}
                            onClick={() => moveRule(r.id, "up")}
                          >
                            ▲
                          </button>
                          <button
                            type="button"
                            className="btn ghost icon-btn"
                            aria-label="Bajar prioridad"
                            disabled={i === rules.length - 1}
                            onClick={() => moveRule(r.id, "down")}
                          >
                            ▼
                          </button>
                          <button
                            type="button"
                            className="btn ghost icon-btn"
                            aria-label="Editar regla"
                            onClick={() => beginEditRule(r)}
                          >
                            <RowEditIcon />
                          </button>
                          <button
                            type="button"
                            className="btn ghost danger icon-btn"
                            aria-label="Eliminar regla"
                            onClick={() => deleteRule(r.id)}
                          >
                            <RowTrashIcon />
                          </button>
                        </div>
                      </td>
                    ) : null}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}

function PlaceholderTab({ tabLabel }: { tabLabel: string }) {
  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">{tabLabel}</h2>
        <p className="workspace-sub">Próximamente.</p>
      </div>
      <div
        className="panel placeholder-hero"
        aria-label={`${tabLabel}: pendiente`}
      />
    </div>
  );
}
