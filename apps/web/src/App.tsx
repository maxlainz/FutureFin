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

type InstallationSnapshot = {
  id: string;
  base_currency: string;
  /** IANA TZ for civil "today" (liability derive, etc.) */
  calendar_tz?: string;
  projection_includes_inflation: boolean;
  /** Supuesto anual % cuando inflación está activa (API como decimal string). */
  annual_inflation_assumption_percent?: string | null;
  projection_target_age: number | null;
  show_age_mode: string;
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
  monthly_contribution_fixed: string;
  contribution_remainder_weight: string;
  contribution_frequency?: AssetContributionFreq;
  /** Primer mes motor: fija escalada + remanente nominal (API); ausente en clientes antiguos. */
  contribution_nominal_monthly?: string;
  notes: string | null;
  sort_index: number;
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

type AssetContributionFreq = "monthly" | "weekly";

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
  | "settings";

type SettingsSubTabId =
  | "access"
  | "calendar"
  | "projection"
  | "categories"
  | "data";

const TABS: { id: TabId; label: string }[] = [
  { id: "summary", label: "Resumen" },
  { id: "assets", label: "Activos" },
  { id: "liabilities", label: "Pasivos" },
  { id: "budget", label: "Presupuesto" },
  { id: "upcoming", label: "Próximos" },
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
  settings: "/ajustes",
};

function normalizeAppPath(pathname: string): string {
  const p = pathname.replace(/\/+$/, "") || "/";
  return p;
}

function tabFromPathname(pathname: string): TabId | null {
  const p = normalizeAppPath(pathname);
  const ids = Object.keys(TAB_PATH) as TabId[];
  for (const id of ids) {
    if (TAB_PATH[id] === p) return id;
  }
  return null;
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

function formatMoneyAmountOrDash(s: string | null | undefined): string {
  if (s == null || String(s).trim() === "") return METRIC_DASH;
  return formatMoneyAmount(String(s));
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

/** Equivalente mensual nominal de la cuota fija (semanal ×52/12, mismo criterio que el motor). */
function assetFixedMonthlyEquivalentNum(a: AssetApiRow): number {
  const fixed = parseDisplayDecimal(
    String(a.monthly_contribution_fixed ?? "0").trim(),
  );
  if (fixed === null || fixed <= 0) return 0;
  if (a.contribution_frequency === "weekly") {
    return (fixed * 52) / 12;
  }
  return fixed;
}

/** Importe nominal mensual (primer mes del motor): cuota fija escalada + remanente; incluye ajuste del primer mes por Próximos (fecha o reparto 90 días). */
function formatAssetContributionNominalCell(
  a: AssetApiRow,
  currencyIso: string,
): string {
  const raw = a.contribution_nominal_monthly;
  if (raw != null && String(raw).trim() !== "") {
    const n = parseDisplayDecimal(String(raw).trim());
    if (n !== null && n > 0) {
      return formatCurrencyAmount(String(raw).trim(), currencyIso);
    }
    return METRIC_DASH;
  }
  const eq = assetFixedMonthlyEquivalentNum(a);
  return eq > 0 ? formatCurrencyNumber(eq, currencyIso) : METRIC_DASH;
}

/** Misma lógica que la celda «Aporte»: número mensual estimado por activo. */
function assetContributionMonthlyEstimateNum(a: AssetApiRow): number {
  const raw = a.contribution_nominal_monthly;
  if (raw != null && String(raw).trim() !== "") {
    const n = parseDisplayDecimal(String(raw).trim());
    if (n !== null && n > 0) return n;
  }
  return assetFixedMonthlyEquivalentNum(a);
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

function formatProjectionMilestoneCompactLabel(target: string): string {
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
function formatProjectionChartHorizonLine(
  series: ProjectionSeriesApi,
  projectionTargetAge: number | null,
): string {
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
    case "mac_target_age": {
      const parts: string[] = [];
      if (projectionTargetAge != null) {
        parts.push(`Edad objetivo ${projectionTargetAge} años`);
      }
      if (endYearStr != null) {
        parts.push(`fin de serie ${endYearStr}`);
      }
      if (parts.length > 0) {
        return parts.join(" · ");
      }
      return `Horizonte ${spanYears} años`;
    }
    case "mac_fallback_no_demographics": {
      const tail = "sin DOB / sin edad objetivo fija";
      if (endYearStr != null) {
        return `Fin de serie ${endYearStr} · ${tail}`;
      }
      return `Horizonte ${spanYears} años · ${tail}`;
    }
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
    const annual = [12, 24, 36, 48, 60, 120, 240];
    const yAligned = Math.max(12, Math.ceil(roughStep / 12) * 12);
    step = annual.find((s) => s >= yAligned) ?? yAligned;
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

/** Una marca por año civil al menos (modo fechas); mantiene el último mes del horizonte. */
function thinProjectionTicksUniqueYears(
  ticks: number[],
  anchor: { y: number; m: number; d: number },
  monthEnd: number,
): number[] {
  const sorted = [...new Set(ticks)].sort((a, b) => a - b);
  const out: number[] = [];
  let prevY: number | null = null;
  for (const m of sorted) {
    const y = addMonthsCivil(anchor.y, anchor.m, anchor.d, m).y;
    if (prevY === null || y !== prevY) {
      out.push(m);
      prevY = y;
    }
  }
  if (out[out.length - 1] !== monthEnd) {
    const yEnd = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthEnd).y;
    const yLast = addMonthsCivil(
      anchor.y,
      anchor.m,
      anchor.d,
      out[out.length - 1],
    ).y;
    if (yEnd !== yLast) {
      out.push(monthEnd);
    } else {
      out[out.length - 1] = monthEnd;
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

  let ticks = buildProjectionMonthTickIndices(mc, maxTicks);

  if (opts?.ageUiMode === "dates") {
    const anchorStr =
      opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
        ? opts.anchorDateYmd.trim()
        : todayYmdInTimeZone(opts.calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (anchor) {
      ticks = thinProjectionTicksUniqueYears(ticks, anchor, mc);
    }
  }

  return ticks.map((m) => ({
    monthIndex: m,
    label: projectionXTickLabel(m, mc, opts),
  }));
}

/** Geometría del SVG en unidades de usuario: depende del ancho CSS real del contenedor. */
function buildProjectionChartLayout(
  containerCssWidth: number,
  containerCssHeight?: number,
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
  let mt = narrow ? 94 : 88;
  const mb = narrow ? 42 : 52;

  let pw = W - ml - mr;
  const legendOnRight = pw >= 280;
  if (!legendOnRight) {
    mt = narrow ? 122 : 112;
    if (pw < 440) {
      mt += 12;
    }
  }

  pw = W - ml - mr;
  const ph = H - mt - mb;

  const legend = legendOnRight
    ? {
        x: ml + Math.max(0, pw - Math.min(210, Math.round(pw * 0.42))),
        y: 16,
      }
    : { x: ml, y: narrow ? 92 : 82 };

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
    legendOnRight,
    legend,
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

/** Retorno acumulado (valor/compra − 1); no es TAE. Paridad MVP con «retorno implícito» del Mac cuando hay precio de compra. */
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

/** MVP equivalente a InlineHelpIcon del Mac: tooltip nativo al pasar el cursor o foco. */
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
}: {
  title: string;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
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
        className="modal-dialog card-elevated"
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
  const [projectionInflationDraft, setProjectionInflationDraft] =
    useState(false);
  const [projectionInflationPctDraft, setProjectionInflationPctDraft] =
    useState("");
  const [projectionTargetAgeDraft, setProjectionTargetAgeDraft] =
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
  const [assetFormCategoryId, setAssetFormCategoryId] = useState("");
  const [assetFormName, setAssetFormName] = useState("");
  const [assetFormValue, setAssetFormValue] = useState("");
  const [assetFormPurchase, setAssetFormPurchase] = useState("");
  const [assetFormLiquid, setAssetFormLiquid] = useState(true);
  const [assetFormExpectedReturn, setAssetFormExpectedReturn] = useState("");
  const [assetFormMonthlyFixed, setAssetFormMonthlyFixed] = useState("");
  const [assetFormContributionFrequency, setAssetFormContributionFrequency] =
    useState<AssetContributionFreq>("monthly");
  const [assetFormRemainderWeight, setAssetFormRemainderWeight] =
    useState("");
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

  const [summary, setSummary] = useState<SummaryResponse | null>(null);
  const [summaryBusy, setSummaryBusy] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);

  const [projectionSeries, setProjectionSeries] =
    useState<ProjectionSeriesApi | null>(null);
  const [projectionBusy, setProjectionBusy] = useState(false);
  const [projectionError, setProjectionError] = useState<string | null>(null);

  const [userProfileOpen, setUserProfileOpen] = useState(false);
  const [userBirthDraft, setUserBirthDraft] = useState("");
  const [userProfileSaving, setUserProfileSaving] = useState(false);
  const [userProfileError, setUserProfileError] = useState<string | null>(null);
  const [backupZipBusy, setBackupZipBusy] = useState(false);
  const [backupZipError, setBackupZipError] = useState<string | null>(null);

  const [pathname, navigate] = useAppPathNavigation();
  const activeTab = useMemo(
    () => tabFromPathname(pathname) ?? "summary",
    [pathname],
  );

  const hasMembership = installation !== null;

  useLayoutEffect(() => {
    if (!user) return;
    const p = normalizeAppPath(pathname);
    if (p === "/") {
      navigate("/resumen", true);
      return;
    }
    if (tabFromPathname(pathname) === null) {
      navigate("/resumen", true);
    }
  }, [user, pathname, navigate]);

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
      setProjectionInflationDraft(false);
      setProjectionInflationPctDraft("");
      setProjectionTargetAgeDraft("");
      setShowAgeModeDraft("dates");
      return;
    }
    const inst = installation.installation;
    setProjectionInflationDraft(inst.projection_includes_inflation);
    setProjectionInflationPctDraft(
      formatEditableDecimalString(inst.annual_inflation_assumption_percent),
    );
    setProjectionTargetAgeDraft(
      inst.projection_target_age != null
        ? String(inst.projection_target_age)
        : "",
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
  }, [user, hasMembership, activeTab, loadBudgetPage]);

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
      setAssetModalOpen(false);
      setLiabilityModalOpen(false);
      setBudgetModalOpen(false);
      setPlanningModalOpen(false);
      setCategoryModalOpen(false);
      setCategoryRenameModalOpen(false);
      setProjectionSeries(null);
      setProjectionError(null);
      setBackupZipError(null);
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
          body: JSON.stringify({ username, password }),
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
          projection_includes_inflation: false,
          projection_target_age: null,
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
    const ageTrim = projectionTargetAgeDraft.trim();
    let projection_target_age: number | null;
    if (ageTrim === "") {
      projection_target_age = null;
    } else {
      const n = Number(ageTrim);
      if (!Number.isFinite(n) || !Number.isInteger(n) || n < 65 || n > 105) {
        setInstallationError(
          "Edad objetivo de horizonte: entero entre 65 y 105, o vacío para limpiar.",
        );
        return;
      }
      projection_target_age = n;
    }
    const pctTrim = projectionInflationPctDraft.trim().replace(",", ".");
    if (pctTrim !== "") {
      const n = Number(pctTrim);
      if (!Number.isFinite(n) || n < 0 || n > 50) {
        setInstallationError(
          "Supuesto de inflación anual: número entre 0 y 50, o vacío para borrar.",
        );
        return;
      }
    }
    setInstallationProjectionSaving(true);
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation", {
        ...defaultFetchInit,
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          projection_includes_inflation: projectionInflationDraft,
          projection_target_age,
          show_age_mode: showAgeModeDraft,
          annual_inflation_assumption_percent:
            pctTrim === "" ? null : pctTrim,
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
    setAssetFormMonthlyFixed("");
    setAssetFormContributionFrequency("monthly");
    setAssetFormRemainderWeight("");
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
      const mf = assetFormMonthlyFixed.trim().replace(",", ".");
      base.monthly_contribution_fixed = mf === "" ? "0" : mf;
      base.contribution_frequency = assetFormContributionFrequency;
      const rw = assetFormRemainderWeight.trim().replace(",", ".");
      base.contribution_remainder_weight = rw === "" ? "0" : rw;
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
    setAssetFormMonthlyFixed(
      formatEditableDecimalString(a.monthly_contribution_fixed ?? "0"),
    );
    setAssetFormContributionFrequency(
      a.contribution_frequency === "weekly" ? "weekly" : "monthly",
    );
    setAssetFormRemainderWeight(
      formatEditableDecimalString(a.contribution_remainder_weight ?? "0"),
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
        const patchBody = {
          category_id: budgetFormCategoryId,
          amount: amt,
          notes: budgetFormNotes.trim(),
        };
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
      if (editingPlanningFlowId) {
        const patchBody: Record<string, unknown> = {
          category_id: planningFormCategoryId,
          title: tit,
          expected_amount: amt,
          due_date: dueTrim === "" ? null : dueTrim,
          notes: planningFormNotes.trim(),
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
          birth_date: trimmed === "" ? null : trimmed,
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

  async function exportBackupZipFile() {
    setBackupZipBusy(true);
    setBackupZipError(null);
    try {
      const res = await fetch("/v1/backup/export.zip", defaultFetchInit);
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "futurefin-export.zip";
      a.rel = "noopener";
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } catch (e: unknown) {
      setBackupZipError(e instanceof Error ? e.message : String(e));
    } finally {
      setBackupZipBusy(false);
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
            <span>Fecha de nacimiento (opcional)</span>
            <input
              type="date"
              value={userBirthDraft}
              onChange={(e) => setUserBirthDraft(e.target.value)}
              max={new Date().toISOString().slice(0, 10)}
              autoComplete="bday"
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
            assetFormMonthlyFixed={assetFormMonthlyFixed}
            setAssetFormMonthlyFixed={setAssetFormMonthlyFixed}
            assetFormContributionFrequency={assetFormContributionFrequency}
            setAssetFormContributionFrequency={setAssetFormContributionFrequency}
            assetFormRemainderWeight={assetFormRemainderWeight}
            setAssetFormRemainderWeight={setAssetFormRemainderWeight}
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
            editingBudgetEntryId={editingBudgetEntryId}
            budgetSaving={budgetSaving}
            submitBudgetForm={(e) => void submitBudgetForm(e)}
            deleteBudgetEntryRow={(id) => void deleteBudgetEntryRow(id)}
            beginEditBudgetEntry={(row) => {
              beginEditBudgetEntry(row);
              setBudgetModalOpen(true);
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
            editingPlanningFlowId={editingPlanningFlowId}
            planningSaving={planningSaving}
            submitPlanningFlowForm={(e) => void submitPlanningFlowForm(e)}
            deletePlanningFlowRow={(id) => void deletePlanningFlowRow(id)}
            beginEditPlanningFlow={(row) => {
              beginEditPlanningFlow(row);
              setPlanningModalOpen(true);
            }}
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
            projectionInflationDraft={projectionInflationDraft}
            setProjectionInflationDraft={setProjectionInflationDraft}
            projectionInflationPctDraft={projectionInflationPctDraft}
            setProjectionInflationPctDraft={setProjectionInflationPctDraft}
            projectionTargetAgeDraft={projectionTargetAgeDraft}
            setProjectionTargetAgeDraft={setProjectionTargetAgeDraft}
            showAgeModeDraft={showAgeModeDraft}
            setShowAgeModeDraft={setShowAgeModeDraft}
            installationProjectionSaving={installationProjectionSaving}
            saveInstallationProjection={(e) => void saveInstallationProjection(e)}
            health={health}
            healthError={healthError}
            categoriesError={categoriesError}
            hasMembership={hasMembership}
            canEditCategories={installation?.role !== "viewer"}
            isOwner={installation?.role === "owner"}
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
            backupZipBusy={backupZipBusy}
            backupZipError={backupZipError}
            exportBackupZip={() => void exportBackupZipFile()}
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

function AssetsView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
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
  assetFormMonthlyFixed,
  setAssetFormMonthlyFixed,
  assetFormContributionFrequency,
  setAssetFormContributionFrequency,
  assetFormRemainderWeight,
  setAssetFormRemainderWeight,
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
  assetFormMonthlyFixed: string;
  setAssetFormMonthlyFixed: Dispatch<SetStateAction<string>>;
  assetFormContributionFrequency: AssetContributionFreq;
  setAssetFormContributionFrequency: Dispatch<
    SetStateAction<AssetContributionFreq>
  >;
  assetFormRemainderWeight: string;
  setAssetFormRemainderWeight: Dispatch<SetStateAction<string>>;
  assetFormNotes: string;
  setAssetFormNotes: Dispatch<SetStateAction<string>>;
  editingAssetId: string | null;
  assetSaving: boolean;
  submitAssetForm: (e: FormEvent) => void;
  deleteAssetRow: (id: string) => void;
  beginEditAsset: (a: AssetApiRow) => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;
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

  const assetsMonthlyContributionSum = assetMetricsReady
    ? assets.reduce((acc, a) => acc + assetContributionMonthlyEstimateNum(a), 0)
    : null;

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
          <MetricCard
            label="Aporte mensual (est.)"
            value={
              assetMetricsReady && assetsMonthlyContributionSum !== null
                ? formatCurrencyNumber(assetsMonthlyContributionSum, currencyIso)
                : METRIC_DASH
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
            <div className="asset-auto-contrib-section">
              <h4 className="asset-auto-contrib-heading">
                Aportación automática
              </h4>
              <div className="asset-form-grid">
                <label className="field">
                  <span>Frecuencia cuota fija</span>
                  <select
                    value={assetFormContributionFrequency}
                    onChange={(e) =>
                      setAssetFormContributionFrequency(
                        e.target.value as AssetContributionFreq,
                      )
                    }
                  >
                    <option value="monthly">Mensual</option>
                    <option value="weekly">Semanal (×52÷12 en proyección)</option>
                  </select>
                </label>
                <label className="field">
                  <span>
                    Importe cuota fija (
                    {assetFormContributionFrequency === "weekly"
                      ? "por semana"
                      : "por mes"}
                    )
                  </span>
                  <input
                    value={assetFormMonthlyFixed}
                    onChange={(e) =>
                      setAssetFormMonthlyFixed(e.target.value)
                    }
                    inputMode="decimal"
                    placeholder="0"
                    autoComplete="off"
                  />
                </label>
                <label className="field">
                  <span>Peso sobre remanente (%)</span>
                  <input
                    value={assetFormRemainderWeight}
                    onChange={(e) =>
                      setAssetFormRemainderWeight(e.target.value)
                    }
                    inputMode="decimal"
                    placeholder="0"
                    autoComplete="off"
                  />
                </label>
              </div>
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
                          <th className="num">Valor</th>
                          <th className="num">Compra</th>
                          <th
                            className="num"
                            title="Variación vs precio de compra (no anualizada)"
                          >
                            Δ compra
                          </th>
                          <th>Líquido</th>
                          <th
                            className="num"
                            title="Rentabilidad anual esperada (proyección)"
                          >
                            Rent. % a.a.
                          </th>
                          <th
                            className="num"
                            title="Aporte estimado mes 1 (cuota fija + remanente por pesos)"
                          >
                            Aporte
                          </th>
                          {canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((a) => (
                          <tr key={a.id}>
                            <td>{a.name}</td>
                            <td className="num">
                              {formatCurrencyAmount(
                                a.current_value,
                                currencyIso,
                              )}
                            </td>
                            <td className="num">
                              {formatCurrencyOrDash(
                                a.purchase_price,
                                currencyIso,
                              )}
                            </td>
                            <td className="num muted">
                              {assetImplicitTotalReturnLabel(
                                a.current_value,
                                a.purchase_price,
                              ) ?? METRIC_DASH}
                            </td>
                            <td>{a.is_liquid ? "Sí" : "No"}</td>
                            <td className="num muted">
                              {a.expected_annual_return_percent != null &&
                              a.expected_annual_return_percent !== ""
                                ? formatPercentAmount(
                                    a.expected_annual_return_percent,
                                  )
                                : METRIC_DASH}
                            </td>
                            <td className="num muted tight">
                              {formatAssetContributionNominalCell(
                                a,
                                currencyIso,
                              )}
                            </td>
                            {canEdit ? (
                              <td className="asset-actions-cell budget-row-actions">
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
                              <td className="asset-actions-cell budget-row-actions">
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
  editingBudgetEntryId,
  budgetSaving,
  submitBudgetForm,
  deleteBudgetEntryRow,
  beginEditBudgetEntry,
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
  editingBudgetEntryId: string | null;
  budgetSaving: boolean;
  submitBudgetForm: (e: FormEvent) => void;
  deleteBudgetEntryRow: (id: string) => void;
  beginEditBudgetEntry: (row: BudgetEntryApiRow) => void;
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
          <MetricCard label="Neto" value={netM} />
        </div>
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
                          <td className="asset-actions-cell budget-row-actions">
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
                            <td className="asset-actions-cell budget-row-actions">
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
          <h3 className="panel-title">Panorama</h3>
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
                  onChange={(e) => setPlanningFormDue(e.target.value)}
                />
              </label>
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
                      <td className="asset-actions-cell budget-row-actions">
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
}: {
  label: string;
  value: string;
  suffix?: string;
  /** Detalle del mismo KPI entre paréntesis (`.metric-value-parenthetical`). */
  parenthetical?: string;
}) {
  return (
    <article className="metric-card">
      <div className="metric-label">{label}</div>
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
  projectionInflationDraft,
  setProjectionInflationDraft,
  projectionInflationPctDraft,
  setProjectionInflationPctDraft,
  projectionTargetAgeDraft,
  setProjectionTargetAgeDraft,
  showAgeModeDraft,
  setShowAgeModeDraft,
  installationProjectionSaving,
  saveInstallationProjection,
  health,
  healthError,
  categoriesError,
  hasMembership,
  canEditCategories,
  isOwner,
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
  backupZipBusy,
  backupZipError,
  exportBackupZip,
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
  projectionInflationDraft: boolean;
  setProjectionInflationDraft: Dispatch<SetStateAction<boolean>>;
  projectionInflationPctDraft: string;
  setProjectionInflationPctDraft: Dispatch<SetStateAction<string>>;
  projectionTargetAgeDraft: string;
  setProjectionTargetAgeDraft: Dispatch<SetStateAction<string>>;
  showAgeModeDraft: "dates" | "ages";
  setShowAgeModeDraft: Dispatch<SetStateAction<"dates" | "ages">>;
  installationProjectionSaving: boolean;
  saveInstallationProjection: (e: FormEvent) => void;
  health: HealthResponse | null;
  healthError: string | null;
  categoriesError: string | null;
  hasMembership: boolean;
  canEditCategories: boolean;
  isOwner: boolean;
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
  backupZipBusy: boolean;
  backupZipError: string | null;
  exportBackupZip: () => void;
}) {
  const renamingCat =
    editingCategoryId === null
      ? undefined
      : categories.find((x) => x.id === editingCategoryId);

  const filteredCategories =
    categoryScopeFilter === "all"
      ? categories
      : categories.filter((c) => c.scope === categoryScopeFilter);

  const settingsSubTabs = useMemo(() => {
    const out: { id: SettingsSubTabId; label: string }[] = [];
    if (isOwner) {
      out.push({ id: "access", label: "Acceso" });
    }
    if (hasMembership) {
      out.push({ id: "calendar", label: "Calendario" });
      out.push({ id: "projection", label: "Proyección" });
      out.push({ id: "categories", label: "Categorías" });
    }
    out.push({ id: "data", label: "Datos y sistema" });
    return out;
  }, [isOwner, hasMembership]);

  const [settingsSubTab, setSettingsSubTab] =
    useState<SettingsSubTabId>("calendar");

  useEffect(() => {
    if (!settingsSubTabs.some((t) => t.id === settingsSubTab)) {
      setSettingsSubTab(settingsSubTabs[0]?.id ?? "data");
    }
  }, [settingsSubTabs, settingsSubTab]);

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
            onClick={() => setSettingsSubTab(t.id)}
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
            <label className="field checkbox-field">
              <input
                type="checkbox"
                checked={projectionInflationDraft}
                onChange={(e) =>
                  setProjectionInflationDraft(e.target.checked)
                }
              />
              <span>Incluir inflación en la proyección de patrimonio</span>
            </label>
            <label className="field">
              <span>Supuesto inflación anual % (opc.)</span>
              <input
                value={projectionInflationPctDraft}
                onChange={(e) =>
                  setProjectionInflationPctDraft(e.target.value)
                }
                inputMode="decimal"
                placeholder="2,5"
                autoComplete="off"
              />
            </label>
            <label className="field">
              <span>Edad objetivo del horizonte (opc.)</span>
              <input
                value={projectionTargetAgeDraft}
                onChange={(e) => setProjectionTargetAgeDraft(e.target.value)}
                inputMode="numeric"
                placeholder="—"
                autoComplete="off"
              />
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
          {hasMembership && isOwner ? (
            <section className="panel">
              <h3 className="panel-title">Copia CSV (ZIP)</h3>
              {backupZipError ? (
                <p className="error compact bordered-top">{backupZipError}</p>
              ) : null}
              <div className="bordered-top">
                <button
                  type="button"
                  className="btn primary"
                  disabled={backupZipBusy}
                  onClick={() => exportBackupZip()}
                >
                  {backupZipBusy ? "Generando…" : "Descargar ZIP"}
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
                <div>
                  <dt>Zona horaria (IANA)</dt>
                  <dd>
                    {installation.installation.calendar_tz?.trim()
                      ? installation.installation.calendar_tz.trim()
                      : "UTC"}
                  </dd>
                </div>
                <div>
                  <dt>Modo edad (UI)</dt>
                  <dd>{installation.installation.show_age_mode}</dd>
                </div>
                <div>
                  <dt>Inflación en proyección</dt>
                  <dd>
                    {installation.installation.projection_includes_inflation
                      ? "sí"
                      : "no"}
                  </dd>
                </div>
                <div>
                  <dt>Supuesto inflación anual</dt>
                  <dd>
                    {installation.installation.annual_inflation_assumption_percent !=
                    null &&
                    String(
                      installation.installation.annual_inflation_assumption_percent,
                    ).trim() !== ""
                      ? formatPercentAmount(
                          String(
                            installation.installation
                              .annual_inflation_assumption_percent,
                          ),
                        )
                      : "—"}
                  </dd>
                </div>
                <div>
                  <dt>Edad objetivo horizonte</dt>
                  <dd>
                    {installation.installation.projection_target_age != null
                      ? String(installation.installation.projection_target_age)
                      : "—"}
                  </dd>
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
    </div>
  );
}

function ProjectionNetWorthChart({
  series,
  milestones,
  currencyIso,
  ledgerPersonScope,
  inflationActive,
  inflationPctDisplay,
  ageUiMode,
  userBirthDate,
  anchorDateYmd,
  calendarTz,
  projectionTargetAge,
}: {
  series: ProjectionSeriesApi;
  milestones: ProjectionMilestoneApi[];
  currencyIso: string;
  ledgerPersonScope: LedgerPersonScope;
  inflationActive: boolean;
  inflationPctDisplay: string | null;
  ageUiMode: "dates" | "ages";
  userBirthDate: string | null;
  /** Mes 0 del motor (YYYY-MM-DD); prioridad sobre reloj cliente. */
  anchorDateYmd: string | null;
  calendarTz: string;
  /** Edad objetivo de instalación (no confundir con la duración del horizonte en años). */
  projectionTargetAge: number | null;
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

  useEffect(() => {
    setViewWindow((prev) => {
      if (pts.length <= 0) return { start: 0, count: 0 };
      if (prev.start === 0 && prev.count === pts.length) {
        return prev;
      }
      return { start: 0, count: pts.length };
    });
  }, [pts.length]);

  const layoutDims = useMemo(
    () =>
      buildProjectionChartLayout(
        containerSize.width > 0 ? containerSize.width : 1040,
        containerSize.height > 0 ? containerSize.height : undefined,
      ),
    [containerSize.height, containerSize.width],
  );

  const model = useMemo(() => {
    if (pts.length < 2) return null;
    const nw = pts.map((p) => parseDisplayDecimal(p.net_worth) ?? 0);
    const cc = pts.map((p) => parseDisplayDecimal(p.contributed_capital) ?? 0);
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
    const startNwParsed = parseDisplayDecimal(series.starting_net_worth);
    const startNw =
      startNwParsed !== null ? startNwParsed : nw[0] ?? 0;
    const allowNegativeAxis = startNw < 0;

    const dataMin = Math.min(...nwVisible, ...ccVisible);
    const dataMax = Math.max(...nwVisible, ...ccVisible);
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
    if (xTicks.length === 0) {
      xTicks.push({
        monthIndex: visibleStart,
        label: projectionXTickLabel(visibleStart, series.months, {
          ageUiMode,
          birthDateIso: userBirthDate,
          anchorDateYmd,
          calendarTz,
        }),
      });
      if (visibleEnd !== visibleStart) {
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
    return {
      nw,
      cc,
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
    layoutDims,
    ageUiMode,
    userBirthDate,
    anchorDateYmd,
    calendarTz,
    milestones,
    viewWindow.count,
    viewWindow.start,
  ]);

  if (!model) {
    return null;
  }

  const {
    nw,
    cc,
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
    pw,
    ph,
    ml,
    mr,
    mt,
    W,
    H,
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

  const horizonLine = formatProjectionChartHorizonLine(series, projectionTargetAge);
  const deltaStr = formatCurrencyAmount(series.monthly_delta_assumption, currencyIso);
  const scopeShort = ledgerPersonScope === "mine" ? "Mi vista" : "Hogar";
  const inflationShort = inflationActive
    ? inflationPctDisplay != null && inflationPctDisplay.length > 0
      ? `Dinero de hoy (~${inflationPctDisplay} anual)`
      : "Dinero de hoy (inflación activa)"
    : "Serie nominal";

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

        <text x={ml} y={34} className="projection-chart-headline">
          {scopeShort}
        </text>
        <text x={ml} y={56} className="projection-chart-meta">
          {horizonLine}
        </text>
        <text x={ml} y={74} className="projection-chart-meta">
          {inflationShort} · Δ regular presup. {deltaStr}/mes
        </text>

        <g
          transform={`translate(${layoutDims.legend.x}, ${layoutDims.legend.y})`}
          className={`projection-chart-legend${layoutDims.legendOnRight ? "" : " projection-chart-legend--stacked"}`}
        >
          <line x1={0} y1={7} x2={26} y2={7} stroke="#047857" strokeWidth={3} strokeLinecap="round" />
          <text x={34} y={11}>Patrimonio neto</text>
          <line x1={0} y1={29} x2={26} y2={29} stroke="#b45309" strokeWidth={2.25} strokeDasharray="6 5" strokeLinecap="round" />
          <text x={34} y={33}>Capital aportado</text>
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
            mt + ph + (layoutDims.narrow ? 22 : 26) + (rotateXLabels ? 10 : 0);
          return (
            <text
              key={`gx-${monthIndex}`}
              transform={
                rotateXLabels
                  ? `rotate(-38 ${cx.toFixed(2)} ${tickY.toFixed(2)})`
                  : undefined
              }
              x={cx}
              y={tickY}
              textAnchor={rotateXLabels ? "end" : "middle"}
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
          <path d={areaD} fill={`url(#nwFill-${gid})`} stroke="none" />
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
                  className="projection-chart-milestone-line"
                />
                <text
                  x={x}
                  y={y1 - 6}
                  textAnchor="middle"
                  className="projection-chart-milestone-label"
                >
                  {label}
                </text>
              </g>
            );
          })}
          {showCompoundOutpaceMarker && compoundOutpaceMonth != null ? (
            <g>
              <line
                x1={xScale(compoundOutpaceMonth)}
                x2={xScale(compoundOutpaceMonth)}
                y1={mt + 2}
                y2={mt + ph}
                className="projection-chart-compound-marker"
              />
              <text
                x={xScale(compoundOutpaceMonth)}
                y={mt + 14}
                textAnchor="middle"
                className="projection-chart-compound-label"
              >
                Interés &gt; ahorro
              </text>
            </g>
          ) : null}

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
        </div>
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
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;
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
  const nextMilestones = projectionSeries?.milestones ?? [];
  const compoundOutpaceMonth =
    projectionSeries?.compound_outpaces_true_savings_month_index ?? null;

  return (
    <div className="workspace workspace--projection-fullwidth">
      <div className="workspace-header">
        <h2 className="workspace-title">Proyección</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
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
          {nextMilestones.length > 0 || compoundOutpaceMonth != null ? (
            <div className="metric-grid workspace-kpi-strip">
              {nextMilestones.map((m) => (
                <MetricCard
                  key={`${m.target}-${m.reached_month_index}`}
                  label="Milestone"
                  value={formatProjectionMilestoneCompactLabel(m.target)}
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
              ))}
              {compoundOutpaceMonth != null ? (
                <MetricCard
                  label="Interés compuesto"
                  value="Supera al ahorro"
                  parenthetical={`~${projectionXTickLabel(
                    compoundOutpaceMonth,
                    projectionSeries.months,
                    {
                      ageUiMode: axisAgeMode,
                      birthDateIso: axisBirth,
                      anchorDateYmd: axisAnchor,
                      calendarTz,
                    },
                  )}`}
                />
              ) : null}
            </div>
          ) : null}
          <ProjectionNetWorthChart
            series={projectionSeries}
            milestones={nextMilestones}
            currencyIso={currencyIso}
            ledgerPersonScope={ledgerPersonScope}
            inflationActive={
              installation?.installation.projection_includes_inflation ?? false
            }
            inflationPctDisplay={inflationPctDisplay}
            ageUiMode={axisAgeMode}
            userBirthDate={axisBirth}
            anchorDateYmd={axisAnchor}
            calendarTz={calendarTz}
            projectionTargetAge={
              installation?.installation.projection_target_age ?? null
            }
          />
        </section>
      ) : null}
    </div>
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
