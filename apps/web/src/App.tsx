import {
  useCallback,
  useEffect,
  useState,
  type Dispatch,
  type FormEvent,
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
};

type InstallationSnapshot = {
  id: string;
  base_currency: string;
  /** IANA TZ for civil "today" (liability derive, etc.) */
  calendar_tz?: string;
  projection_includes_inflation: boolean;
  projection_target_age: number | null;
  show_age_mode: string;
};

type InstallationAccess = {
  installation: InstallationSnapshot;
  role: "owner" | "member" | "viewer";
};

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
  notes: string | null;
  sort_index: number;
};

type SummaryResponse = {
  total_assets: string;
  total_liabilities: string;
  net_worth: string;
  debt_to_assets_ratio: string | null;
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
  label: string | null;
  amount: string;
  frequency: "monthly" | "weekly";
  monthly_equivalent: string;
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

function liabilityDerivedPrincipalPreview(
  amountStr: string,
  freq: LiabilityPaymentFreq,
  endYmd: string,
  installationCalendarTz: string,
): string | null {
  if (!freq || !endYmd.trim()) return null;
  const startYmd = todayYmdInTimeZone(installationCalendarTz);
  const n = paymentIntervalCountUtc(freq, startYmd, endYmd.trim());
  if (n === null || n <= 0) return null;
  const amount = Number(amountStr.trim().replace(",", "."));
  if (!Number.isFinite(amount) || amount <= 0) return null;
  const total = amount * n;
  return total.toLocaleString(undefined, { maximumFractionDigits: 4 });
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

type TabId =
  | "summary"
  | "assets"
  | "liabilities"
  | "budget"
  | "upcoming"
  | "retirement"
  | "projection"
  | "settings";

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

const defaultFetchInit: RequestInit = {
  credentials: "include",
};

const METRIC_DASH = "—";

function formatSummaryAmount(s: string): string {
  const n = Number(s.replace(",", "."));
  if (!Number.isFinite(n)) return s;
  return n.toLocaleString(undefined, { maximumFractionDigits: 4 });
}

function formatDebtToAssetsPct(ratio: string | null | undefined): string {
  if (ratio == null || ratio === "") return METRIC_DASH;
  const r = Number(String(ratio).replace(",", "."));
  if (!Number.isFinite(r)) return METRIC_DASH;
  return `${(r * 100).toLocaleString(undefined, { maximumFractionDigits: 2 })} %`;
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
    Number(e.monthly_equivalent.replace(",", "."));
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
    return (a.label ?? "").localeCompare(b.label ?? "", "es");
  });
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
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);

  const [sessionBusy, setSessionBusy] = useState(true);
  const [user, setUser] = useState<UserResponse | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);

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
  const [setupCurrency, setSetupCurrency] = useState<"EUR" | "USD" | "GBP">(
    "EUR",
  );
  const [setupCalendarTz, setSetupCalendarTz] = useState("UTC");
  const [calendarTzDraft, setCalendarTzDraft] = useState("UTC");
  const [calendarTzSaving, setCalendarTzSaving] = useState(false);

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
  const [editingCategoryId, setEditingCategoryId] = useState<string | null>(
    null,
  );
  const [editCategoryName, setEditCategoryName] = useState("");

  const [assets, setAssets] = useState<AssetApiRow[]>([]);
  const [assetsBusy, setAssetsBusy] = useState(false);
  const [assetsError, setAssetsError] = useState<string | null>(null);
  const [assetCategories, setAssetCategories] = useState<CategoryRow[]>([]);
  const [assetFormCategoryId, setAssetFormCategoryId] = useState("");
  const [assetFormName, setAssetFormName] = useState("");
  const [assetFormValue, setAssetFormValue] = useState("");
  const [assetFormPurchase, setAssetFormPurchase] = useState("");
  const [assetFormLiquid, setAssetFormLiquid] = useState(true);
  const [assetFormNotes, setAssetFormNotes] = useState("");
  const [editingAssetId, setEditingAssetId] = useState<string | null>(null);
  const [assetSaving, setAssetSaving] = useState(false);

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
  const [editingBudgetEntryId, setEditingBudgetEntryId] = useState<
    string | null
  >(null);
  const [budgetFormScope, setBudgetFormScope] =
    useState<BudgetScopeToggle>("expense");
  const [budgetFormCategoryId, setBudgetFormCategoryId] = useState("");
  const [budgetFormLabel, setBudgetFormLabel] = useState("");
  const [budgetFormAmount, setBudgetFormAmount] = useState("");
  const [budgetFormFrequency, setBudgetFormFrequency] = useState<
    "monthly" | "weekly"
  >("monthly");
  const [budgetFormNotes, setBudgetFormNotes] = useState("");

  const [summary, setSummary] = useState<SummaryResponse | null>(null);
  const [summaryBusy, setSummaryBusy] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);

  const [activeTab, setActiveTab] = useState<TabId>("summary");

  const hasMembership = installation !== null;

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

  const loadInstallation = useCallback(async () => {
    setInstallationBusy(true);
    setInstallationError(null);
    try {
      const res = await fetch("/v1/installation", defaultFetchInit);
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const body = (await res.json()) as InstallationAccess | null;
      setInstallation(body);
    } catch (e: unknown) {
      setInstallation(null);
      setInstallationError(e instanceof Error ? e.message : String(e));
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
        fetch("/v1/assets", defaultFetchInit),
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
  }, []);

  const loadLiabilitiesPage = useCallback(async () => {
    setLiabilitiesBusy(true);
    setLiabilitiesError(null);
    try {
      const [catRes, libRes] = await Promise.all([
        fetch("/v1/categories?scope=liability", defaultFetchInit),
        fetch("/v1/liabilities", defaultFetchInit),
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
  }, []);

  const loadBudgetPage = useCallback(async () => {
    setBudgetLoading(true);
    setBudgetError(null);
    try {
      const [budRes, incRes, expRes, libCatRes] = await Promise.all([
        fetch("/v1/budget", defaultFetchInit),
        fetch("/v1/categories?scope=income", defaultFetchInit),
        fetch("/v1/categories?scope=expense", defaultFetchInit),
        fetch("/v1/categories?scope=liability", defaultFetchInit),
      ]);

      if (budRes.status === 403 || budRes.status === 404) {
        setBudgetSnapshot(null);
      } else if (!budRes.ok) {
        throw new Error(await errorMessageFromResponse(budRes));
      } else {
        setBudgetSnapshot((await budRes.json()) as BudgetSnapshotApi);
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
  }, []);

  const loadSummaryPage = useCallback(async () => {
    setSummaryBusy(true);
    setSummaryError(null);
    try {
      const res = await fetch("/v1/summary", defaultFetchInit);
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
  }, []);

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
    if (!user || !hasMembership || activeTab !== "summary") {
      return;
    }
    void loadSummaryPage();
  }, [user, hasMembership, activeTab, loadSummaryPage]);

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
      setBudgetFormLabel("");
      setBudgetFormAmount("");
      setBudgetFormFrequency("monthly");
      setBudgetFormNotes("");
    }
  }, [user]);

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
      setPendingUsers([]);
      setPendingUsersError(null);
      setActiveTab("summary");
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
      await loadInstallation();
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
      await loadInstallation();
    } catch (e: unknown) {
      setInstallationError(e instanceof Error ? e.message : String(e));
    } finally {
      setCalendarTzSaving(false);
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
      await loadCategories();
    } catch (e: unknown) {
      setCategoriesError(e instanceof Error ? e.message : String(e));
    } finally {
      setCategorySaving(false);
    }
  }

  async function deleteCategory(id: string) {
    setCategorySaving(true);
    setCategoriesError(null);
    try {
      const res = await fetch(`/v1/categories/${encodeURIComponent(id)}`, {
        ...defaultFetchInit,
        method: "DELETE",
      });
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      if (editingCategoryId === id) {
        setEditingCategoryId(null);
        setEditCategoryName("");
      }
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
      const pp = assetFormPurchase.trim();
      if (pp) {
        base.purchase_price = pp;
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
    setAssetFormValue(a.current_value);
    setAssetFormPurchase(a.purchase_price ?? "");
    setAssetFormLiquid(a.is_liquid);
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
    setLiabilityFormPrincipal(row.principal);
    setLiabilityFormApr(row.apr_percent ?? "");
    setLiabilityFormPaymentAmount(row.payment_amount ?? "");
    setLiabilityFormPaymentFrequency(row.payment_frequency ?? "");
    setLiabilityFormPaymentEnd(row.payment_end_date ?? "");
    setLiabilityFormNotes(row.notes ?? "");
    setLiabilityFormDerivePrincipal(row.principal_derived_from_plan ?? false);
  }

  function resetBudgetForm() {
    setEditingBudgetEntryId(null);
    const cats =
      budgetFormScope === "income"
        ? budgetIncomeCategories
        : budgetExpenseCategories;
    setBudgetFormCategoryId(cats[0]?.id ?? "");
    setBudgetFormLabel("");
    setBudgetFormAmount("");
    setBudgetFormFrequency("monthly");
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
        frequency: budgetFormFrequency,
      };
      const lb = budgetFormLabel.trim();
      if (lb) {
        base.label = lb;
      }
      const nt = budgetFormNotes.trim();
      if (nt) {
        base.notes = nt;
      }

      if (editingBudgetEntryId) {
        const patchBody = {
          category_id: budgetFormCategoryId,
          label: budgetFormLabel.trim(),
          amount: amt,
          frequency: budgetFormFrequency,
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
    setBudgetFormLabel(row.label ?? "");
    setBudgetFormAmount(row.amount);
    setBudgetFormFrequency(row.frequency);
    setBudgetFormNotes(row.notes ?? "");
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
            <p>
              Patrimonio, presupuesto, planificación y FIRE — mismo modelo que
              el cliente de referencia, en el navegador.
            </p>
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
            <p className="hint">
              Usuario 3–64 caracteres (<code>.</code> <code>_</code>{" "}
              <code>-</code>). Contraseña ≥ 12 caracteres. Cada usuario se crea
              aquí; el propietario de la instalación debe aprobar el acceso en{" "}
              <strong>Ajustes</strong>.
            </p>
          </div>
          <footer className="auth-footer muted">
            Tras entrar verás el escritorio con pestañas (Resumen, Activos…).
          </footer>
        </div>
      </div>
    );
  }

  return (
    <div className="app-root">
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
          <span className="user-chip">{user.username}</span>
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

      <nav className="tab-bar" aria-label="Secciones">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            className={`tab-btn ${activeTab === t.id ? "active" : ""}`}
            onClick={() => setActiveTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      <main className="app-main">
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

        {summaryError ? (
          <div className="banner error-banner">{summaryError}</div>
        ) : null}

        {!installationBusy &&
        user &&
        !hasMembership &&
        activeTab !== "settings" ? (
          <div className="banner info-banner">
            Tu cuenta está registrada pero el propietario aún no ha aprobado tu
            acceso. Avísale para que te conceda entrada en{" "}
            <strong>Ajustes</strong>.
          </div>
        ) : null}

        {activeTab === "summary" ? (
          <SummaryView
            installation={installation}
            loading={installationBusy}
            hasMembership={hasMembership}
            summary={summary}
            summaryBusy={summaryBusy}
          />
        ) : activeTab === "assets" ? (
          <AssetsView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            canEdit={installation?.role !== "viewer"}
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
            assetFormNotes={assetFormNotes}
            setAssetFormNotes={setAssetFormNotes}
            editingAssetId={editingAssetId}
            assetSaving={assetSaving}
            submitAssetForm={(e) => void submitAssetForm(e)}
            deleteAssetRow={(id) => void deleteAssetRow(id)}
            beginEditAsset={beginEditAsset}
            resetAssetForm={resetAssetForm}
          />
        ) : activeTab === "liabilities" ? (
          <LiabilitiesView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            canEdit={installation?.role !== "viewer"}
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
            beginEditLiability={beginEditLiability}
            resetLiabilityForm={resetLiabilityForm}
          />
        ) : activeTab === "budget" ? (
          <BudgetView
            installation={installation}
            installationBusy={installationBusy}
            hasMembership={hasMembership}
            canEdit={installation?.role !== "viewer"}
            budgetSnapshot={budgetSnapshot}
            budgetLoading={budgetLoading}
            budgetIncomeCategories={budgetIncomeCategories}
            budgetExpenseCategories={budgetExpenseCategories}
            budgetLiabilityCategories={budgetLiabilityCategories}
            budgetFormScope={budgetFormScope}
            setBudgetFormScope={setBudgetFormScope}
            budgetFormCategoryId={budgetFormCategoryId}
            setBudgetFormCategoryId={setBudgetFormCategoryId}
            budgetFormLabel={budgetFormLabel}
            setBudgetFormLabel={setBudgetFormLabel}
            budgetFormAmount={budgetFormAmount}
            setBudgetFormAmount={setBudgetFormAmount}
            budgetFormFrequency={budgetFormFrequency}
            setBudgetFormFrequency={setBudgetFormFrequency}
            budgetFormNotes={budgetFormNotes}
            setBudgetFormNotes={setBudgetFormNotes}
            editingBudgetEntryId={editingBudgetEntryId}
            budgetSaving={budgetSaving}
            submitBudgetForm={(e) => void submitBudgetForm(e)}
            deleteBudgetEntryRow={(id) => void deleteBudgetEntryRow(id)}
            beginEditBudgetEntry={beginEditBudgetEntry}
            resetBudgetForm={resetBudgetForm}
          />
        ) : activeTab === "settings" ? (
          <SettingsView
            installation={installation}
            installationBusy={installationBusy}
            setupCurrency={setupCurrency}
            setSetupCurrency={setSetupCurrency}
            setupCalendarTz={setupCalendarTz}
            setSetupCalendarTz={setSetupCalendarTz}
            setupInstallation={(e) => void setupInstallation(e)}
            calendarTzDraft={calendarTzDraft}
            setCalendarTzDraft={setCalendarTzDraft}
            calendarTzSaving={calendarTzSaving}
            saveInstallationCalendarTz={(e) =>
              void saveInstallationCalendarTz(e)
            }
            health={health}
            healthError={healthError}
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
            deleteCategory={(id) => void deleteCategory(id)}
            editingCategoryId={editingCategoryId}
            setEditingCategoryId={setEditingCategoryId}
            editCategoryName={editCategoryName}
            setEditCategoryName={setEditCategoryName}
            saveCategoryEdit={(id) => void saveCategoryEdit(id)}
          />
        ) : (
          <PlaceholderTab tabLabel={TABS.find((x) => x.id === activeTab)?.label ?? ""} />
        )}
      </main>
    </div>
  );
}

function assetCategoryLabel(categories: CategoryRow[], id: string): string {
  const c = categories.find((x) => x.id === id);
  return c?.name ?? id.slice(0, 8);
}

function AssetsView({
  installation,
  installationBusy,
  hasMembership,
  canEdit,
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
  assetFormNotes,
  setAssetFormNotes,
  editingAssetId,
  assetSaving,
  submitAssetForm,
  deleteAssetRow,
  beginEditAsset,
  resetAssetForm,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  canEdit: boolean;
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
  assetFormNotes: string;
  setAssetFormNotes: Dispatch<SetStateAction<string>>;
  editingAssetId: string | null;
  assetSaving: boolean;
  submitAssetForm: (e: FormEvent) => void;
  deleteAssetRow: (id: string) => void;
  beginEditAsset: (a: AssetApiRow) => void;
  resetAssetForm: () => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Activos</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando instalación…"
            : !hasMembership
              ? "Sin acceso a datos hasta que un propietario apruebe tu cuenta."
              : `Valores en moneda de la instalación (${currency}). Las categorías deben ser del ámbito Activos.`}
        </p>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">
          Cuando tengas acceso podrás registrar activos aquí.
        </div>
      ) : null}

      {hasMembership && assetCategories.length === 0 && !assetsBusy ? (
        <div className="banner info-banner">
          Aún no hay categorías de <strong>Activos</strong>. Créalas en{" "}
          <strong>Ajustes → Categorías</strong> antes de registrar posiciones.
        </div>
      ) : null}

      {canEdit && hasMembership && assetCategories.length > 0 ? (
        <section className="panel">
          <h3 className="panel-title">
            {editingAssetId ? "Editar activo" : "Nuevo activo"}
          </h3>
          <form className="asset-form stack bordered-top" onSubmit={submitAssetForm}>
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
              {editingAssetId ? (
                <button
                  type="button"
                  className="btn ghost"
                  disabled={assetSaving}
                  onClick={() => resetAssetForm()}
                >
                  Cancelar edición
                </button>
              ) : null}
            </div>
          </form>
        </section>
      ) : !canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura: tu rol no permite crear ni editar activos.
        </p>
      ) : null}

      <section className="panel">
        <h3 className="panel-title">Posiciones</h3>
        {assetsBusy ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : assets.length === 0 ? (
          <p className="muted bordered-top">
            No hay activos registrados en esta instalación.
          </p>
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>Nombre</th>
                  <th>Categoría</th>
                  <th className="num">
                    Valor ({currency})
                  </th>
                  <th className="num">Compra</th>
                  <th>Líquido</th>
                  <th>Notas</th>
                  {canEdit ? <th /> : null}
                </tr>
              </thead>
              <tbody>
                {assets.map((a) => (
                  <tr key={a.id}>
                    <td>{a.name}</td>
                    <td>{assetCategoryLabel(assetCategories, a.category_id)}</td>
                    <td className="num">{a.current_value}</td>
                    <td className="num">
                      {a.purchase_price ?? METRIC_DASH}
                    </td>
                    <td>{a.is_liquid ? "Sí" : "No"}</td>
                    <td className="asset-notes-cell">
                      {a.notes ?? METRIC_DASH}
                    </td>
                    {canEdit ? (
                      <td className="asset-actions-cell">
                        <button
                          type="button"
                          className="btn ghost"
                          disabled={assetSaving}
                          onClick={() => beginEditAsset(a)}
                        >
                          Editar
                        </button>
                        <button
                          type="button"
                          className="btn ghost danger"
                          disabled={assetSaving}
                          onClick={() => deleteAssetRow(a.id)}
                        >
                          Eliminar
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

function liabilityCatLabel(categories: CategoryRow[], id: string): string {
  return categories.find((x) => x.id === id)?.name ?? id.slice(0, 8);
}

function LiabilitiesView({
  installation,
  installationBusy,
  hasMembership,
  canEdit,
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
  resetLiabilityForm,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  canEdit: boolean;
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
  resetLiabilityForm: () => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const derivePreview = liabilityDerivedPrincipalPreview(
    liabilityFormPaymentAmount,
    liabilityFormPaymentFrequency,
    liabilityFormPaymentEnd,
    installation?.installation.calendar_tz ?? "UTC",
  );

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Pasivos</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando instalación…"
            : !hasMembership
              ? "Sin acceso a datos hasta que un propietario apruebe tu cuenta."
              : `Principal y planes de pago en ${currency}. Puedes derivar el principal desde la cuota y la fecha fin (como en el cliente Mac). Las categorías deben ser del ámbito Pasivos. Al cargar la lista, los planes con fecha fin pasada se eliminan (tipo arranque Mac).`}
        </p>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">
          Cuando tengas acceso podrás registrar pasivos aquí.
        </div>
      ) : null}

      {hasMembership && liabilityCategories.length === 0 && !liabilitiesBusy ? (
        <div className="banner info-banner">
          Aún no hay categorías de <strong>Pasivos</strong>. Créalas en{" "}
          <strong>Ajustes → Categorías</strong>.
        </div>
      ) : null}

      {canEdit && hasMembership && liabilityCategories.length > 0 ? (
        <section className="panel">
          <h3 className="panel-title">
            {editingLiabilityId ? "Editar pasivo" : "Nuevo pasivo"}
          </h3>
          <form
            className="asset-form stack bordered-top"
            onSubmit={submitLiabilityForm}
          >
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
                <span>
                  Derivar principal desde el plan (cuota × intervalos hasta la
                  fecha fin; mismo criterio que el cliente Mac)
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
                    Vista previa ~{derivePreview} {currency} (hoy en{" "}
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
              {editingLiabilityId ? (
                <button
                  type="button"
                  className="btn ghost"
                  disabled={liabilitySaving}
                  onClick={() => resetLiabilityForm()}
                >
                  Cancelar edición
                </button>
              ) : null}
            </div>
          </form>
        </section>
      ) : !canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura: tu rol no permite crear ni editar pasivos.
        </p>
      ) : null}

      <section className="panel">
        <h3 className="panel-title">Lista</h3>
        {liabilitiesBusy ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : liabilities.length === 0 ? (
          <p className="muted bordered-top">
            No hay pasivos en esta instalación.
          </p>
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>Etiqueta</th>
                  <th>Categoría</th>
                  <th>Tipo</th>
                  <th className="num">Principal</th>
                  <th className="num">TAE %</th>
                  <th className="num">Cuota</th>
                  <th>Frec.</th>
                  <th>Fin plan</th>
                  <th>Notas</th>
                  {canEdit ? <th /> : null}
                </tr>
              </thead>
              <tbody>
                {liabilities.map((row) => (
                  <tr key={row.id}>
                    <td>{row.label}</td>
                    <td>
                      {liabilityCatLabel(liabilityCategories, row.category_id)}
                    </td>
                    <td>{row.type_tag ?? METRIC_DASH}</td>
                    <td className="num">
                      {row.principal}
                      {row.principal_derived_from_plan ? (
                        <span className="muted" title="Principal derivado del plan">
                          {" "}
                          deriv.
                        </span>
                      ) : null}
                    </td>
                    <td className="num">{row.apr_percent ?? METRIC_DASH}</td>
                    <td className="num">
                      {row.payment_amount ?? METRIC_DASH}
                    </td>
                    <td>
                      {row.payment_frequency
                        ? PAYMENT_FREQ_LABEL[row.payment_frequency]
                        : METRIC_DASH}
                    </td>
                    <td>{row.payment_end_date ?? METRIC_DASH}</td>
                    <td className="asset-notes-cell">
                      {row.notes ?? METRIC_DASH}
                    </td>
                    {canEdit ? (
                      <td className="asset-actions-cell">
                        <button
                          type="button"
                          className="btn ghost"
                          disabled={liabilitySaving}
                          onClick={() => beginEditLiability(row)}
                        >
                          Editar
                        </button>
                        <button
                          type="button"
                          className="btn ghost danger"
                          disabled={liabilitySaving}
                          onClick={() => deleteLiabilityRow(row.id)}
                        >
                          Eliminar
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

const BUDGET_SCOPE_LABEL: Record<BudgetScopeToggle, string> = {
  income: "Ingreso",
  expense: "Gasto",
};

function budgetDerivedCatLabel(categories: CategoryRow[], id: string): string {
  return categories.find((x) => x.id === id)?.name ?? id.slice(0, 8);
}

function BudgetView({
  installation,
  installationBusy,
  hasMembership,
  canEdit,
  budgetSnapshot,
  budgetLoading,
  budgetIncomeCategories,
  budgetExpenseCategories,
  budgetLiabilityCategories,
  budgetFormScope,
  setBudgetFormScope,
  budgetFormCategoryId,
  setBudgetFormCategoryId,
  budgetFormLabel,
  setBudgetFormLabel,
  budgetFormAmount,
  setBudgetFormAmount,
  budgetFormFrequency,
  setBudgetFormFrequency,
  budgetFormNotes,
  setBudgetFormNotes,
  editingBudgetEntryId,
  budgetSaving,
  submitBudgetForm,
  deleteBudgetEntryRow,
  beginEditBudgetEntry,
  resetBudgetForm,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  canEdit: boolean;
  budgetSnapshot: BudgetSnapshotApi | null;
  budgetLoading: boolean;
  budgetIncomeCategories: CategoryRow[];
  budgetExpenseCategories: CategoryRow[];
  budgetLiabilityCategories: CategoryRow[];
  budgetFormScope: BudgetScopeToggle;
  setBudgetFormScope: Dispatch<SetStateAction<BudgetScopeToggle>>;
  budgetFormCategoryId: string;
  setBudgetFormCategoryId: Dispatch<SetStateAction<string>>;
  budgetFormLabel: string;
  setBudgetFormLabel: Dispatch<SetStateAction<string>>;
  budgetFormAmount: string;
  setBudgetFormAmount: Dispatch<SetStateAction<string>>;
  budgetFormFrequency: "monthly" | "weekly";
  setBudgetFormFrequency: Dispatch<
    SetStateAction<"monthly" | "weekly">
  >;
  budgetFormNotes: string;
  setBudgetFormNotes: Dispatch<SetStateAction<string>>;
  editingBudgetEntryId: string | null;
  budgetSaving: boolean;
  submitBudgetForm: (e: FormEvent) => void;
  deleteBudgetEntryRow: (id: string) => void;
  beginEditBudgetEntry: (row: BudgetEntryApiRow) => void;
  resetBudgetForm: () => void;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const categoryMapForSort = budgetCategoryMap(
    budgetIncomeCategories,
    budgetExpenseCategories,
  );

  const sortedEntries =
    budgetSnapshot && !budgetLoading
      ? sortBudgetEntriesMacStyle(budgetSnapshot.entries, categoryMapForSort)
      : [];

  const formCats =
    budgetFormScope === "income"
      ? budgetIncomeCategories
      : budgetExpenseCategories;

  const snap =
    hasMembership && !budgetLoading && budgetSnapshot !== null
      ? budgetSnapshot
      : null;

  const incM = snap
    ? formatSummaryAmount(snap.totals.income_monthly_equivalent)
    : METRIC_DASH;
  const expReg = snap
    ? formatSummaryAmount(snap.totals.expense_regular_monthly_equivalent)
    : METRIC_DASH;
  const expDer = snap
    ? formatSummaryAmount(snap.totals.expense_derived_monthly_equivalent)
    : METRIC_DASH;
  const expTot = snap
    ? formatSummaryAmount(snap.totals.expense_total_monthly_equivalent)
    : METRIC_DASH;
  const netM = snap
    ? formatSummaryAmount(snap.totals.net_monthly_equivalent)
    : METRIC_DASH;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Presupuesto</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando instalación…"
            : !hasMembership
              ? "Sin acceso a datos hasta que un propietario apruebe tu cuenta."
              : budgetLoading
                ? "Cargando presupuesto y categorías…"
                : `Totales en equivalencia mensual (${currency}). Las cuotas de pasivos con plan activo aparecen como líneas derivadas (solo lectura), alineadas al cliente Mac.`}
        </p>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">
          Cuando tengas acceso podrás ver y editar el presupuesto aquí.
        </div>
      ) : null}

      {hasMembership &&
      budgetIncomeCategories.length === 0 &&
      budgetExpenseCategories.length === 0 &&
      !budgetLoading ? (
        <div className="banner info-banner">
          Aún no hay categorías de <strong>Ingresos</strong> ni{" "}
          <strong>Gastos</strong>. Créalas en <strong>Ajustes → Categorías</strong>.
        </div>
      ) : null}

      <div className="metric-grid">
        <MetricCard
          label="Ingresos (equiv. mensual)"
          value={incM}
          suffix={currency}
          hint="Entradas recurrentes guardadas en presupuesto."
        />
        <MetricCard
          label="Gastos recurrentes"
          value={expReg}
          suffix={currency}
        />
        <MetricCard
          label="Gastos derivados (pasivos)"
          value={expDer}
          suffix={currency}
          hint="Cuotas de planes activos en Pasivos."
        />
        <MetricCard
          label="Gastos totales"
          value={expTot}
          suffix={currency}
        />
        <MetricCard
          label="Neto mensual"
          value={netM}
          suffix={currency}
          hint="Ingresos − gastos totales."
        />
      </div>

      {canEdit &&
      hasMembership &&
      (budgetIncomeCategories.length > 0 ||
        budgetExpenseCategories.length > 0) ? (
        <section className="panel">
          <h3 className="panel-title">
            {editingBudgetEntryId ? "Editar línea" : "Nueva línea de presupuesto"}
          </h3>
          <form
            className="asset-form stack bordered-top"
            onSubmit={submitBudgetForm}
          >
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
                <span>Etiqueta (opc.)</span>
                <input
                  value={budgetFormLabel}
                  onChange={(e) => setBudgetFormLabel(e.target.value)}
                  maxLength={200}
                  placeholder="Detalle opcional"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Importe</span>
                <input
                  value={budgetFormAmount}
                  onChange={(e) => setBudgetFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Frecuencia</span>
                <select
                  value={budgetFormFrequency}
                  onChange={(e) =>
                    setBudgetFormFrequency(
                      e.target.value as "monthly" | "weekly",
                    )
                  }
                >
                  <option value="monthly">Mensual</option>
                  <option value="weekly">Semanal</option>
                </select>
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
              {editingBudgetEntryId ? (
                <button
                  type="button"
                  className="btn ghost"
                  disabled={budgetSaving}
                  onClick={() => resetBudgetForm()}
                >
                  Cancelar edición
                </button>
              ) : null}
            </div>
          </form>
        </section>
      ) : !canEdit && hasMembership ? (
        <p className="muted tight">
          Solo lectura: tu rol no permite crear ni editar líneas de presupuesto.
        </p>
      ) : null}

      <section className="panel">
        <h3 className="panel-title">Líneas guardadas</h3>
        {budgetLoading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : sortedEntries.length === 0 ? (
          <p className="muted bordered-top">
            No hay líneas de presupuesto guardadas.
          </p>
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>Ámbito</th>
                  <th>Categoría</th>
                  <th>Etiqueta</th>
                  <th className="num">Importe</th>
                  <th>Frec.</th>
                  <th className="num">Equiv. mensual</th>
                  <th>Notas</th>
                  {canEdit ? <th /> : null}
                </tr>
              </thead>
              <tbody>
                {sortedEntries.map((row) => (
                  <tr key={row.id}>
                    <td>{BUDGET_SCOPE_LABEL[row.scope]}</td>
                    <td>
                      {categoryMapForSort.get(row.category_id)?.name ??
                        row.category_id.slice(0, 8)}
                    </td>
                    <td>{row.label ?? METRIC_DASH}</td>
                    <td className="num">{row.amount}</td>
                    <td>{PAYMENT_FREQ_LABEL[row.frequency]}</td>
                    <td className="num">{row.monthly_equivalent}</td>
                    <td className="asset-notes-cell">
                      {row.notes ?? METRIC_DASH}
                    </td>
                    {canEdit ? (
                      <td className="asset-actions-cell">
                        <button
                          type="button"
                          className="btn ghost"
                          disabled={budgetSaving}
                          onClick={() => beginEditBudgetEntry(row)}
                        >
                          Editar
                        </button>
                        <button
                          type="button"
                          className="btn ghost danger"
                          disabled={budgetSaving}
                          onClick={() => deleteBudgetEntryRow(row.id)}
                        >
                          Eliminar
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

      <section className="panel">
        <h3 className="panel-title">Derivado de pasivos (solo lectura)</h3>
        <p className="muted tight">
          Cuotas de pasivos con plan completo y fecha fin posterior a hoy en la
          zona civil de la instalación.
        </p>
        {budgetLoading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : !budgetSnapshot ||
          budgetSnapshot.derived_from_liabilities.length === 0 ? (
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
                  <th>Notas</th>
                </tr>
              </thead>
              <tbody>
                {budgetSnapshot.derived_from_liabilities.map((row) => (
                  <tr key={row.liability_id}>
                    <td>{row.label}</td>
                    <td>
                      {budgetDerivedCatLabel(
                        budgetLiabilityCategories,
                        row.category_id,
                      )}
                    </td>
                    <td className="num">{row.amount}</td>
                    <td>{PAYMENT_FREQ_LABEL[row.frequency]}</td>
                    <td className="num">{row.monthly_equivalent}</td>
                    <td className="asset-notes-cell">{row.notes}</td>
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

function SummaryView({
  installation,
  loading,
  hasMembership,
  summary,
  summaryBusy,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
  summary: SummaryResponse | null;
  summaryBusy: boolean;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

  const showMetrics =
    hasMembership && !loading && !summaryBusy && summary !== null;
  const nw = showMetrics ? formatSummaryAmount(summary.net_worth) : METRIC_DASH;
  const ta = showMetrics ? formatSummaryAmount(summary.total_assets) : METRIC_DASH;
  const tl = showMetrics
    ? formatSummaryAmount(summary.total_liabilities)
    : METRIC_DASH;
  const dta = showMetrics
    ? formatDebtToAssetsPct(summary.debt_to_assets_ratio)
    : METRIC_DASH;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Resumen</h2>
        <p className="workspace-sub">
          {loading ? (
            "Cargando datos de la instalación…"
          ) : !hasMembership ? (
            <>
              Regístrate en la pantalla de acceso; cuando el propietario apruebe
              tu usuario en <strong>Ajustes</strong>, verás datos aquí. Para
              recuperar una base vacía, usa la inicialización en Ajustes.
            </>
          ) : summaryBusy ? (
            <>
              Actualizando métricas desde activos y pasivos… Moneda base{" "}
              <strong>{currency}</strong>.
            </>
          ) : (
            <>
              Moneda base <strong>{currency}</strong>. Totales alineados al
              checklist Summary (purga de pasivos con plan vencido antes de
              sumar, igual que la lista de pasivos).
            </>
          )}
        </p>
      </div>

      <div className="metric-grid">
        <MetricCard
          label="Patrimonio neto"
          value={nw}
          suffix={currency}
          hint="Activos − pasivos (principal registrado)."
        />
        <MetricCard
          label="Activos totales"
          value={ta}
          suffix={currency}
        />
        <MetricCard
          label="Pasivos totales"
          value={tl}
          suffix={currency}
        />
        <MetricCard
          label="Ratio deuda / activos"
          value={dta}
          hint="Pasivos ÷ activos; vacío si activos = 0."
        />
      </div>

      <section className="panel">
        <h3 className="panel-title">Salud financiera</h3>
        <p className="muted tight">
          Pendiente de paridad completa: ingresos/gastos recurrentes, tasa de
          ahorro, runway y cobertura próxima (presupuesto y planeación).
        </p>
        <div className="placeholder-chips">
          <span className="chip">Ingresos recurrentes</span>
          <span className="chip">Gastos</span>
          <span className="chip">Tasa de ahorro</span>
          <span className="chip">Runway</span>
        </div>
      </section>

      <section className="panel muted-panel">
        <h3 className="panel-title">Desglose</h3>
        <p className="muted tight">
          Donut / categorías como en el cliente Mac: usa las pestañas Activos y
          Pasivos por ahora; gráficos enlazados aquí vendrán después.
        </p>
      </section>
    </div>
  );
}

function MetricCard({
  label,
  value,
  suffix,
  hint,
}: {
  label: string;
  value: string;
  suffix?: string;
  hint?: string;
}) {
  return (
    <article className="metric-card">
      <div className="metric-label">{label}</div>
      <div className="metric-value-row">
        <span className="metric-value">{value}</span>
        {suffix && suffix !== METRIC_DASH ? (
          <span className="metric-suffix">{suffix}</span>
        ) : null}
      </div>
      {hint ? <p className="metric-hint">{hint}</p> : null}
    </article>
  );
}

function SettingsView({
  installation,
  installationBusy,
  setupCurrency,
  setSetupCurrency,
  setupCalendarTz,
  setSetupCalendarTz,
  setupInstallation,
  calendarTzDraft,
  setCalendarTzDraft,
  calendarTzSaving,
  saveInstallationCalendarTz,
  health,
  healthError,
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
  deleteCategory,
  editingCategoryId,
  setEditingCategoryId,
  editCategoryName,
  setEditCategoryName,
  saveCategoryEdit,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  setupCurrency: "EUR" | "USD" | "GBP";
  setSetupCurrency: (v: "EUR" | "USD" | "GBP") => void;
  setupCalendarTz: string;
  setSetupCalendarTz: Dispatch<SetStateAction<string>>;
  setupInstallation: (e: FormEvent) => void;
  calendarTzDraft: string;
  setCalendarTzDraft: Dispatch<SetStateAction<string>>;
  calendarTzSaving: boolean;
  saveInstallationCalendarTz: (e: FormEvent) => void;
  health: HealthResponse | null;
  healthError: string | null;
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
  deleteCategory: (id: string) => void;
  editingCategoryId: string | null;
  setEditingCategoryId: Dispatch<SetStateAction<string | null>>;
  editCategoryName: string;
  setEditCategoryName: Dispatch<SetStateAction<string>>;
  saveCategoryEdit: (id: string) => void;
}) {
  const filteredCategories =
    categoryScopeFilter === "all"
      ? categories
      : categories.filter((c) => c.scope === categoryScopeFilter);

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Ajustes</h2>
        <p className="workspace-sub">
          Los usuarios se registran en la pantalla de acceso. El propietario
          concede acceso aquí; los visores solo leen.
        </p>
      </div>

      {!installationBusy && !hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Acceso</h3>
          <p className="muted tight">
            Ya tienes cuenta pero el propietario debe aprobarte en esta
            instalación.
          </p>
          <p className="hint bordered-top">
            Recuperación: si la base no tiene instalación inicializada pero sí
            usuarios, puedes crearla aquí (solo cuando la tabla está vacía).
          </p>
          <form className="stack bordered-top" onSubmit={setupInstallation}>
            <h4 className="subsection-title">Inicializar instalación</h4>
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
            <button
              type="submit"
              className="btn primary"
              disabled={installationBusy}
            >
              Crear instalación
            </button>
          </form>
        </section>
      ) : null}

      {isOwner ? (
        <section className="panel">
          <h3 className="panel-title">Aprobar acceso</h3>
          <p className="muted tight">
            Usuarios registrados que aún no tienen acceso a esta instalación.
            Elige rol y aprueba.
          </p>
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

      {hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Zona horaria del calendario</h3>
          <p className="muted tight">
            Define el día civil «hoy» para derivar principal en pasivos (paridad
            con calendario local tipo Mac). Debe ser un identificador IANA válido.
          </p>
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
              Actual:{" "}
              <strong>{installation?.installation.calendar_tz ?? "UTC"}</strong>
              . Solo el propietario puede cambiarla.
            </p>
          )}
        </section>
      ) : null}

      {hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Categorías</h3>
          <p className="muted tight">
            Ámbitos alineados al cliente de referencia. No hay categorías por
            defecto: créalas aquí o al importar datos.
          </p>
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
          {canEditCategories ? (
            <form
              className="category-add-row bordered-top"
              onSubmit={createCategory}
            >
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
              <label className="field category-add-name">
                <span>Nombre</span>
                <input
                  value={newCatName}
                  onChange={(e) => setNewCatName(e.target.value)}
                  maxLength={200}
                  placeholder="p. ej. Efectivo"
                />
              </label>
              <button
                type="submit"
                className="btn primary category-add-submit"
                disabled={categorySaving}
              >
                Añadir
              </button>
            </form>
          ) : (
            <p className="muted bordered-top">
              Solo lectura: tu rol no permite crear ni editar categorías.
            </p>
          )}
          {categoriesBusy ? (
            <p className="muted bordered-top">Cargando categorías…</p>
          ) : (
            <ul className="category-list">
              {filteredCategories.map((c) => (
                <li key={c.id} className="category-row">
                  <span className="category-scope-tag">
                    {CATEGORY_SCOPE_LABEL[c.scope]}
                  </span>
                  {editingCategoryId === c.id ? (
                    <div className="category-edit-row">
                      <input
                        className="category-edit-input"
                        value={editCategoryName}
                        onChange={(e) => setEditCategoryName(e.target.value)}
                        maxLength={200}
                        aria-label="Nuevo nombre"
                      />
                      <button
                        type="button"
                        className="btn primary"
                        disabled={categorySaving}
                        onClick={() => saveCategoryEdit(c.id)}
                      >
                        Guardar
                      </button>
                      <button
                        type="button"
                        className="btn ghost"
                        disabled={categorySaving}
                        onClick={() => {
                          setEditingCategoryId(null);
                          setEditCategoryName("");
                        }}
                      >
                        Cancelar
                      </button>
                    </div>
                  ) : (
                    <>
                      <span className="category-name">{c.name}</span>
                      {canEditCategories ? (
                        <div className="category-row-actions">
                          <button
                            type="button"
                            className="btn ghost"
                            disabled={categorySaving}
                            onClick={() => {
                              setEditingCategoryId(c.id);
                              setEditCategoryName(c.name);
                            }}
                          >
                            Renombrar
                          </button>
                          <button
                            type="button"
                            className="btn ghost danger"
                            disabled={categorySaving}
                            onClick={() => deleteCategory(c.id)}
                          >
                            Eliminar
                          </button>
                        </div>
                      ) : null}
                    </>
                  )}
                </li>
              ))}
            </ul>
          )}
          {!categoriesBusy && filteredCategories.length === 0 ? (
            <p className="muted bordered-top">
              No hay categorías en este filtro.
            </p>
          ) : null}
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
              <dt>Modo edad</dt>
              <dd>{installation.installation.show_age_mode}</dd>
            </div>
          </dl>
        ) : (
          <p className="muted tight">
            Sin acceso — revisa la sección de acceso arriba si necesitas
            recuperar la instalación.
          </p>
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
    </div>
  );
}

function PlaceholderTab({ tabLabel }: { tabLabel: string }) {
  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">{tabLabel}</h2>
        <p className="workspace-sub">
          Esta sección replica las capacidades del cliente de referencia;
          conectaremos formularios y datos cuando el backend exponga el modelo.
        </p>
      </div>
      <div className="panel placeholder-hero">
        <p className="muted tight">
          Aquí irá la vista completa de <strong>{tabLabel}</strong> — tablas,
          filtros y KPIs reactivos al editar campos (sin botón global
          «calcular»).
        </p>
      </div>
    </div>
  );
}
