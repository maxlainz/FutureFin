import {
  Suspense,
  lazy,
  startTransition,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import "./App.css";
import { formatEditableDecimalString, parseDisplayDecimal } from "./lib/format";
import { apiGet, defaultFetchInit, errorMessageFromResponse } from "./api/client";
import { Modal, ModalFormError } from "./components/Modal";
import { SnapshotPromptModal } from "./components/SnapshotPromptModal";
import {
  liquidCoverageComplete,
  pruneEditLog,
  type EditLog,
} from "./lib/snapshot-tracker";
import { TopBar } from "./components/TopBar";
import { MobileNavDrawer } from "./components/MobileNavDrawer";
import { SummaryView } from "./views/SummaryView";
import type { BudgetScopeToggle } from "./views/BudgetView";
import { BootstrapInstallationPanel } from "./auth/BootstrapInstallationPanel";
import { ledgerViewQs } from "./lib/ledger";
import { savingsSourceUsesTransactions } from "./lib/fire";
import { readFileAsBase64 } from "./lib/files";
import { chartPerf } from "./lib/perf";
import { PROJECTION_FOCUS_STORAGE_KEY } from "./lib/projection-chart";
import type { LedgerPersonScope } from "./lib/ledger";
import {
  TABS,
  TAB_PATH,
  type SettingsSubTabId,
  type TabId,
  normalizeAppPath,
  settingsSubTabFromPathname,
  settingsSubTabPath,
  tabFromPathname,
} from "./lib/navigation";
import {
  applyTheme,
  loadThemePref,
  saveThemePref,
  subscribeSystemThemeChanges,
  type ThemePref,
} from "./lib/theme";

/**
 * Two-phase fetch de `/v1/projection/series`. Primero pide `?density=hybrid`
 * (~82 puntos, JSON ~5 KB) y dispara `onData` cuando llega; luego, en
 * paralelo, pide la versión `monthly` (full ~841 puntos) y, al recibirla,
 * dispara `onData` otra vez dentro de `startTransition` para que el render
 * del SVG denso no bloquee el hilo principal.
 *
 * Si el cache server está warm (post-login warm-up dual), ambos GET son
 * cache hit y llegan en <10 ms cada uno → el hybrid no añade latencia
 * perceptible. En cold cache, el hybrid mejora el tiempo al primer paint.
 */
function projectionSeriesUrl(scope: LedgerPersonScope, density?: "hybrid"): string {
  const params = new URLSearchParams();
  if (scope === "mine") params.set("view", "mine");
  if (density) params.set("density", density);
  const q = params.toString();
  return q ? `/v1/projection/series?${q}` : "/v1/projection/series";
}

async function fetchProjectionTwoPhase(
  scope: LedgerPersonScope,
  onData: (data: ProjectionSeriesApi) => void,
): Promise<void> {
  chartPerf.mark("fetch-start");
  // Lanzamos ambos en paralelo. El hybrid suele llegar antes (menor JSON y
  // mismo compute server con cache hit), pero si por algún motivo el monthly
  // llega antes, useTransition garantiza coherencia (la última asignación
  // gana).
  const hybridPromise = fetch(
    projectionSeriesUrl(scope, "hybrid"),
    defaultFetchInit,
  );
  const monthlyPromise = fetch(
    projectionSeriesUrl(scope),
    defaultFetchInit,
  );

  const hybridRes = await hybridPromise;
  chartPerf.mark("fetch-response");
  if (hybridRes.ok) {
    const data = (await hybridRes.json()) as ProjectionSeriesApi;
    chartPerf.mark("fetch-end");
    onData(data);
  } else if (
    hybridRes.status === 403 ||
    hybridRes.status === 404
  ) {
    // Sesión inválida o sin acceso: nada que hacer; el caller resuelve.
    throw new Error(`projection hybrid: ${hybridRes.status}`);
  }

  // Phase 2 en background: cuando llega el monthly, reemplaza con
  // startTransition para que el re-render denso no bloquee inputs.
  const monthlyRes = await monthlyPromise;
  if (monthlyRes.ok) {
    const full = (await monthlyRes.json()) as ProjectionSeriesApi;
    startTransition(() => onData(full));
  }
}

// Lazy-loaded views: each becomes a separate Vite chunk, only fetched when the user navigates
// to the tab. SummaryView stays eager because it's the landing page after auth.
const AssetsView = lazy(() =>
  import("./views/AssetsView").then((m) => ({ default: m.AssetsView })),
);
const LiabilitiesView = lazy(() =>
  import("./views/LiabilitiesView").then((m) => ({ default: m.LiabilitiesView })),
);
const BudgetView = lazy(() =>
  import("./views/BudgetView").then((m) => ({ default: m.BudgetView })),
);
const GastosView = lazy(() =>
  import("./views/GastosView").then((m) => ({ default: m.GastosView })),
);
const UpcomingView = lazy(() =>
  import("./views/UpcomingView").then((m) => ({ default: m.UpcomingView })),
);
const RetirementView = lazy(() =>
  import("./views/RetirementView").then((m) => ({ default: m.RetirementView })),
);
const ProjectionView = lazy(() =>
  import("./views/ProjectionView").then((m) => ({ default: m.ProjectionView })),
);
const SettingsView = lazy(() =>
  import("./views/SettingsView").then((m) => ({ default: m.SettingsView })),
);
import type {
  AllocationRuleApiRow,
  AllocationRuleCapKind,
  AllocationRuleKind,
  AssetApiRow,
  BudgetEntryApiRow,
  BudgetSnapshotApi,
  CategoryRow,
  CategoryScope,
  FireSettingsApi,
  HealthResponse,
  HistoryCashflowApi,
  HistorySeriesApi,
  HistorySnapshotKindApi,
  InstallationAccess,
  InstallationGate,
  InstallationSessionContext,
  LiabilityApiRow,
  PlanningFlowApiRow,
  ProjectionSeriesApi,
  SummaryResponse,
  UserResponse,
} from "./api/types";

type LiabilityPaymentFreq = "" | "monthly" | "weekly";

/**
 * Paletas por ámbito: cada entrada es un color bien diferenciado dentro de la misma
 * familia (fríos = activos, cálidos = pasivos). Se cicla si hay más categorías.
 */

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

const LEDGER_PERSON_SCOPE_STORAGE_KEY = "futurefin-ledger-person-scope";

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

  // Serie histórica (snapshots pasados). Es un enhancement del chart: cualquier
  // fallo cae a `null` en silencio (sin busy/error state) y el chart degrada a
  // solo-futuro. Se carga junto a la proyección en la pestaña Proyección.
  const [historySeries, setHistorySeries] = useState<HistorySeriesApi | null>(
    null,
  );

  // Cash-flow histórico (transacciones): agregado mensual + serie fina anclada.
  // Mismo contrato de degradación que historySeries: fallo → null en silencio y
  // el chart pinta el pasado solo con la serie mensual. `cashflowDaily` es el
  // detalle diario (ventana 6 meses) que se fetchea lazy cuando el chart hace
  // zoom a la zona histórica reciente; se purga con cada recarga del weekly.
  const [cashflowSeries, setCashflowSeries] =
    useState<HistoryCashflowApi | null>(null);
  const [cashflowDaily, setCashflowDaily] =
    useState<HistoryCashflowApi | null>(null);
  // Anti-bucle del fetch lazy diario: el chart puede pedirlo en cada re-render
  // mientras no llegue; con esto solo se dispara una vez por (scope, recarga).
  const cashflowDailyRequestedRef = useRef(false);

  // Trigger del modal «¿Guardar snapshot?» (ver lib/snapshot-tracker.ts y plan §5.2). Todo el
  // estado del disparo vive en refs (no provoca re-render); solo el paso visible del modal y su
  // busy son estado. `liquidEditLogRef`: ediciones de valor de activos líquidos propios dentro de
  // la ventana rodante. `snapshotPromptFiredRef`: ya se ofreció una vez (se rearma al vaciarse la
  // ventana). `liabilityEditedAtRef` / `liabilitySnapshotSavedAtRef`: para ofrecer el paso de
  // pasivos solo si hay cambios sin snapshotear.
  const liquidEditLogRef = useRef<EditLog>(new Map());
  const snapshotPromptFiredRef = useRef(false);
  const liabilityEditedAtRef = useRef<number | null>(null);
  const liabilitySnapshotSavedAtRef = useRef<number | null>(null);
  const [snapshotPromptStep, setSnapshotPromptStep] = useState<
    "closed" | "assets" | "liabilities"
  >("closed");
  const [snapshotPromptBusy, setSnapshotPromptBusy] = useState(false);

  const [retirementBudgetSnapshot, setRetirementBudgetSnapshot] =
    useState<BudgetSnapshotApi | null>(null);
  const [retirementBusy, setRetirementBusy] = useState(false);
  const [retirementError, setRetirementError] = useState<string | null>(null);

  const [userProfileOpen, setUserProfileOpen] = useState(false);
  const [userBirthDraft, setUserBirthDraft] = useState("");
  const [userProfileSaving, setUserProfileSaving] = useState(false);
  const [userProfileError, setUserProfileError] = useState<string | null>(null);

  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [themePref, setThemePref] = useState<ThemePref>(loadThemePref);
  useLayoutEffect(() => {
    applyTheme(themePref);
    saveThemePref(themePref);
  }, [themePref]);
  useEffect(() => {
    if (themePref !== "auto") return;
    return subscribeSystemThemeChanges(() => applyTheme("auto"));
  }, [themePref]);
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
      out.push("calendar", "projection", "retirement", "categories", "history");
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

  // Devuelve las filas recién cargadas para que los callers que necesitan el estado fresco
  // inmediatamente (p. ej. el trigger del snapshot en `submitAssetForm`, ya que `setAssets` es
  // asíncrono) no dependan de un re-render. Los demás callers ignoran el retorno.
  const loadAssetsPage = useCallback(async (): Promise<AssetApiRow[]> => {
    setAssetsBusy(true);
    setAssetsError(null);
    let loaded: AssetApiRow[] = [];
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
        loaded = (await astRes.json()) as AssetApiRow[];
        setAssets(loaded);
      }
    } catch (e: unknown) {
      loaded = [];
      setAssets([]);
      setAssetCategories([]);
      setAssetsError(e instanceof Error ? e.message : String(e));
    } finally {
      setAssetsBusy(false);
    }
    return loaded;
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
    const qs = ledgerViewQs(ledgerPersonScope);
    // /v1/summary y /v1/projection/series en paralelo, pero con flows
    // INDEPENDIENTES: en cuanto llega summary liberamos summaryBusy (KPIs y
    // Salud financiera pintan ya). El MiniProjection del Resumen aparece
    // después, cuando llegue projection-series, sin bloquear el resto.
    const summaryFlow = (async () => {
      try {
        const res = await fetch(`/v1/summary${qs}`, defaultFetchInit);
        if (res.status === 403 || res.status === 404) {
          setSummary(null);
        } else if (!res.ok) {
          throw new Error(await errorMessageFromResponse(res));
        } else {
          setSummary((await res.json()) as SummaryResponse);
        }
      } catch (e: unknown) {
        setSummary(null);
        setSummaryError(e instanceof Error ? e.message : String(e));
      } finally {
        setSummaryBusy(false);
      }
    })();
    const projectionFlow = (async () => {
      try {
        await fetchProjectionTwoPhase(ledgerPersonScope, (data) => {
          setProjectionSeries(data);
        });
      } catch {
        setProjectionSeries(null);
      }
    })();
    await Promise.all([summaryFlow, projectionFlow]);
  }, [ledgerPersonScope]);

  const loadProjectionSeriesPage = useCallback(async () => {
    setProjectionBusy(true);
    setProjectionError(null);
    try {
      await fetchProjectionTwoPhase(ledgerPersonScope, (data) => {
        setProjectionSeries(data);
      });
    } catch (e: unknown) {
      setProjectionSeries(null);
      setProjectionError(e instanceof Error ? e.message : String(e));
    } finally {
      setProjectionBusy(false);
    }
  }, [ledgerPersonScope]);

  const loadHistorySeries = useCallback(async () => {
    // Purga síncrona antes del await: en un toggle Hogar↔Mío la serie del scope anterior no debe
    // sobrevivir la ventana de fetch (se fusionaría con la proyección del nuevo scope → empalme
    // cruzado). Al vaciarla, el chart degrada a solo-futuro hasta que resuelva el nuevo fetch.
    setHistorySeries(null);
    try {
      const data = await apiGet<HistorySeriesApi>(
        `/v1/history/series${ledgerViewQs(ledgerPersonScope)}`,
      );
      setHistorySeries(data);
    } catch {
      // El histórico es opcional: cualquier fallo degrada al chart solo-futuro.
      setHistorySeries(null);
    }
  }, [ledgerPersonScope]);

  const loadCashflowSeries = useCallback(async () => {
    // Misma purga síncrona que loadHistorySeries: el overlay del scope anterior no debe
    // sobrevivir la ventana de fetch. También purga el detalle diario (queda stale tras
    // cualquier mutación de transacciones o cambio de scope) y rearma su fetch lazy.
    setCashflowSeries(null);
    setCashflowDaily(null);
    cashflowDailyRequestedRef.current = false;
    try {
      const qs = ledgerViewQs(ledgerPersonScope);
      const data = await apiGet<HistoryCashflowApi>(
        `/v1/history/cashflow${qs}${qs ? "&" : "?"}window_months=24`,
      );
      setCashflowSeries(data);
    } catch {
      // El cash-flow es un enhancement: cualquier fallo degrada al pasado solo-mensual.
      setCashflowSeries(null);
    }
  }, [ledgerPersonScope]);

  // Detalle diario lazy (el endpoint limita daily a ventana ≤ 6 meses). Lo pide el chart
  // cuando la vista hace zoom a la zona histórica reciente; un fallo se ignora (weekly sigue).
  const loadCashflowDaily = useCallback(async () => {
    if (cashflowDailyRequestedRef.current) return;
    cashflowDailyRequestedRef.current = true;
    try {
      const qs = ledgerViewQs(ledgerPersonScope);
      const data = await apiGet<HistoryCashflowApi>(
        `/v1/history/cashflow${qs}${qs ? "&" : "?"}window_months=6&resolution=daily`,
      );
      setCashflowDaily(data);
    } catch {
      /* weekly sigue siendo el fallback */
    }
  }, [ledgerPersonScope]);

  // Captura un snapshot de hoy (upsert silencioso, sin confirmación). Resuelve (void) si se guardó;
  // **lanza** un `Error` con el mensaje de la API si falla (el llamador — SnapshotButton o los
  // handlers del modal — decide cómo mostrarlo). Los snapshots no son inputs del engine, así que
  // no invalidan la cache de proyección; solo refrescamos la serie histórica.
  const saveSnapshotNow = useCallback(
    async (kinds: HistorySnapshotKindApi[]): Promise<void> => {
      const res = await fetch("/v1/history/snapshots/capture", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ kinds }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      void loadHistorySeries();
      // Los snapshots son las anclas de la serie fina del cash-flow: refrescarla también.
      void loadCashflowSeries();
    },
    [loadHistorySeries, loadCashflowSeries],
  );

  const loadRetirementPage = useCallback(
    async (opts?: { silent?: boolean; skipProjection?: boolean }) => {
      const silent = opts?.silent === true;
      // skipProjection: usado por el prefetch tras login para no volver a pegar
      // a /v1/projection/series (lo hace loadSummaryPage). Refetcheamos
      // proyección en navegación normal (refresh tras mutaciones).
      const skipProjection = opts?.skipProjection === true;
      if (!silent) {
        setRetirementBusy(true);
        setRetirementError(null);
        setProjectionError(null);
      }
      try {
        const qs =
          ledgerPersonScope === "mine" ? "?view=mine" : "";
        const budPromise = fetch(`/v1/budget${qs}`, defaultFetchInit);
        const projPromise = skipProjection
          ? Promise.resolve(null as Response | null)
          : fetch(`/v1/projection/series${qs}`, defaultFetchInit);
        // El preview del target en los modos con promedio (B y C) consume los equivalentes
        // efectivos del summary; lo cargamos aquí para que sea fresco y coherente con el scope
        // activo. Independiente y silencioso: cualquier fallo degrada el preview al
        // presupuesto (modo A), sin bloquear budget/proyección. Solo se lanza en B/C: en
        // presupuesto RetirementView usa la base del budget y este fetch sería inútil.
        if (
          savingsSourceUsesTransactions(
            installation?.installation?.fire_settings?.savings_source,
          )
        ) {
          void (async () => {
            try {
              const res = await fetch(`/v1/summary${qs}`, defaultFetchInit);
              if (res.status === 403 || res.status === 404) {
                setSummary(null);
              } else if (res.ok) {
                setSummary((await res.json()) as SummaryResponse);
              }
            } catch {
              /* stale summary tolerado: el preview degrada a presupuesto */
            }
          })();
        }
        const [budRes, projRes] = await Promise.all([budPromise, projPromise]);
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
        if (projRes) {
          if (projRes.status === 403 || projRes.status === 404) {
            setProjectionSeries(null);
          } else if (!projRes.ok) {
            throw new Error(await errorMessageFromResponse(projRes));
          } else {
            setProjectionSeries((await projRes.json()) as ProjectionSeriesApi);
          }
        }
      } catch (e: unknown) {
        if (!silent) {
          setRetirementBudgetSnapshot(null);
          if (!skipProjection) setProjectionSeries(null);
          setRetirementError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (!silent) {
          setRetirementBusy(false);
        }
      }
    },
    [ledgerPersonScope, installation],
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

  // Tras login: esperamos a que la pestaña actual termine su carga (via el
  // useEffect[activeTab === "xxx"] correspondiente) y luego encadenamos en
  // serie el prefetch de los chunks JS + loaders del resto. Así no saturamos
  // ancho de banda ni CPU del API al inicio. Excluye la pestaña actual (sus
  // datos ya están en estado) y `summary` (ya pre-fetcha projection-series en
  // su propio Promise.all).
  const currentTabBusy = useMemo(() => {
    switch (activeTab) {
      case "summary":
        return summaryBusy;
      case "assets":
        return assetsBusy;
      case "liabilities":
        return liabilitiesBusy;
      case "budget":
        return budgetLoading || assetsBusy || allocationRulesBusy;
      case "upcoming":
        return planningLoading;
      case "retirement":
        return retirementBusy;
      case "projection":
        return projectionBusy;
      case "settings":
        return categoriesBusy;
      default:
        return false;
    }
  }, [
    activeTab,
    summaryBusy,
    assetsBusy,
    liabilitiesBusy,
    budgetLoading,
    allocationRulesBusy,
    planningLoading,
    retirementBusy,
    projectionBusy,
    categoriesBusy,
  ]);

  const prefetchOtherViews = useCallback(
    async (currentTab: TabId, signal: AbortSignal) => {
      // Orden por uso esperado: Proyección (con su chart grande aparte), luego
      // las tablas y por último Ajustes.
      type PrefetchTask = {
        tab: TabId;
        importChunk: () => Promise<unknown>;
        loadData?: () => Promise<unknown>;
      };
      const tasks: PrefetchTask[] = [
        {
          tab: "projection",
          // ProjectionNetWorthChart está incluido en el chunk de ProjectionView
          // (import directo, no lazy interno).
          importChunk: () => import("./views/ProjectionView"),
          // Datos cubiertos por loadSummaryPage (Promise.all summary+projection).
        },
        {
          tab: "assets",
          importChunk: () => import("./views/AssetsView"),
          loadData: () => loadAssetsPage(),
        },
        {
          tab: "liabilities",
          importChunk: () => import("./views/LiabilitiesView"),
          loadData: () => loadLiabilitiesPage(),
        },
        {
          tab: "budget",
          importChunk: () => import("./views/BudgetView"),
          loadData: async () => {
            await Promise.all([loadBudgetPage(), loadAllocationRules()]);
          },
        },
        {
          tab: "expenses",
          // Vista autónoma: solo calentamos el chunk; los datos los carga ella al montar.
          importChunk: () => import("./views/GastosView"),
        },
        {
          tab: "retirement",
          importChunk: () => import("./views/RetirementView"),
          // skipProjection: loadSummaryPage ya cargó /v1/projection/series; un
          // segundo fetch saturaría el endpoint pesado y dispararía re-render
          // del chart con la misma data.
          loadData: () =>
            loadRetirementPage({ silent: true, skipProjection: true }),
        },
        {
          tab: "upcoming",
          importChunk: () => import("./views/UpcomingView"),
          loadData: () => loadPlanningPage(),
        },
        {
          tab: "settings",
          importChunk: () => import("./views/SettingsView"),
          // Categorías se cargan al entrar en Ajustes (no merece bandwidth aquí).
        },
      ];
      for (const t of tasks) {
        if (signal.aborted) return;
        if (t.tab === currentTab) continue;
        try {
          await t.importChunk();
        } catch {
          /* chunk fetch failure: dejamos que el lazy on-demand reintente */
        }
        if (signal.aborted) return;
        if (t.loadData) {
          try {
            await t.loadData();
          } catch {
            /* errores los maneja cada loader internamente */
          }
        }
      }
    },
    [
      loadAssetsPage,
      loadLiabilitiesPage,
      loadBudgetPage,
      loadAllocationRules,
      loadPlanningPage,
      loadRetirementPage,
    ],
  );

  const prefetchedRef = useRef(false);
  useEffect(() => {
    prefetchedRef.current = false;
  }, [user?.id]);

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
    void loadHistorySeries();
    void loadCashflowSeries();
  }, [
    user,
    hasMembership,
    activeTab,
    loadProjectionSeriesPage,
    loadHistorySeries,
    loadCashflowSeries,
  ]);

  useEffect(() => {
    if (!user || !hasMembership || activeTab !== "retirement") {
      return;
    }
    void loadRetirementPage();
    // Solo recargar bloqueante al cambiar sesión / pestaña / vista; no cuando `user` muta tras PATCH pensión.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user?.id, hasMembership, activeTab, loadRetirementPage]);

  useEffect(() => {
    if (!user || !hasMembership) return;
    if (currentTabBusy) return;
    if (prefetchedRef.current) return;
    prefetchedRef.current = true;
    const win = window as Window & {
      requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number;
      cancelIdleCallback?: (handle: number) => void;
    };
    const ctrl = new AbortController();
    const run = () => {
      void prefetchOtherViews(activeTab, ctrl.signal);
    };
    const supportsIdle = typeof win.requestIdleCallback === "function";
    const handle = supportsIdle
      ? win.requestIdleCallback!(run, { timeout: 2000 })
      : window.setTimeout(run, 200);
    return () => {
      ctrl.abort();
      if (supportsIdle) {
        win.cancelIdleCallback?.(handle);
      } else {
        window.clearTimeout(handle);
      }
    };
  }, [user, hasMembership, currentTabBusy, activeTab, prefetchOtherViews]);

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
      setHistorySeries(null);
      // Reset del trigger de snapshot (per-sesión).
      liquidEditLogRef.current = new Map();
      snapshotPromptFiredRef.current = false;
      liabilityEditedAtRef.current = null;
      liabilitySnapshotSavedAtRef.current = null;
      setSnapshotPromptStep("closed");
      setSnapshotPromptBusy(false);
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
      // El target FIRE móvil es recalculado por el backend a partir de la
      // configuración FIRE. Recargamos la serie de proyección (silencioso)
      // para que el chart de Jubilación / Resumen / Proyección reflejen los
      // nuevos parámetros sin esperar a un cambio de pestaña.
      void loadProjectionSeriesPage();
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

  // ¿Es un activo líquido cuyo titular es el usuario de la sesión? Regla del plan §5.2:
  // `owner_user_id === user.id`; si el campo está ausente (endpoint/cliente antiguo), solo cuenta
  // en vista «mío» (donde todas las filas son propias por construcción).
  function isOwnLiquidAsset(row: {
    is_liquid: boolean;
    owner_user_id?: string;
  }): boolean {
    if (!row.is_liquid) return false;
    if (row.owner_user_id != null) {
      return user != null && row.owner_user_id === user.id;
    }
    return ledgerPersonScope === "mine";
  }

  // Evalúa si toca ofrecer el modal de snapshot tras una edición de activo. Poda la ventana
  // rodante; si queda vacía, rearma el disparo; si todos los activos líquidos propios (según la
  // lista recién recargada) tienen edición reciente y aún no se ofreció, dispara el paso «assets».
  function evaluateSnapshotPrompt(reloadedAssets: AssetApiRow[]) {
    const now = Date.now();
    const pruned = pruneEditLog(liquidEditLogRef.current, now);
    liquidEditLogRef.current = pruned;
    if (pruned.size === 0) {
      snapshotPromptFiredRef.current = false;
    }
    const ownLiquidIds = reloadedAssets
      .filter((a) => isOwnLiquidAsset(a))
      .map((a) => a.id);
    if (
      !snapshotPromptFiredRef.current &&
      liquidCoverageComplete(pruned, ownLiquidIds, now)
    ) {
      snapshotPromptFiredRef.current = true;
      setSnapshotPromptStep("assets");
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
    // Capturado ANTES de mutar/recargar (resetAssetForm limpia el formulario y editingAssetId):
    // la fila previa (para saber si el valor cambió de verdad en una edición) y el valor enviado.
    const editingId = editingAssetId;
    const submittedValueNum = parseDisplayDecimal(assetFormValue.trim());
    const previousRow = editingId
      ? assets.find((a) => a.id === editingId) ?? null
      : null;
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

      let createdAsset: AssetApiRow | null = null;
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
        // 201 → devuelve el activo creado (con id/owner_user_id); lo necesitamos para el trigger.
        createdAsset = (await res.json()) as AssetApiRow;
      }
      resetAssetForm();
      setAssetModalOpen(false);
      const reloaded = await loadAssetsPage();

      // Trigger del snapshot: registra la edición si el valor cambió de verdad (creaciones siempre
      // cuentan) y la fila recargada es un activo líquido propio; luego re-evalúa la cobertura.
      const savedId = editingId ?? createdAsset?.id ?? null;
      if (savedId) {
        const valueChanged =
          editingId === null ||
          previousRow == null ||
          parseDisplayDecimal(previousRow.current_value) !== submittedValueNum;
        const savedRow = reloaded.find((a) => a.id === savedId) ?? null;
        if (valueChanged && savedRow != null && isOwnLiquidAsset(savedRow)) {
          liquidEditLogRef.current.set(savedId, Date.now());
        }
        evaluateSnapshotPrompt(reloaded);
      }
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
      // El borrado no dispara ni evalúa el modal; solo saca el id de la ventana de ediciones para
      // que no bloquee la cobertura ni cuente como líquido editado.
      liquidEditLogRef.current.delete(id);
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
      // Marca que hay cambios de pasivos sin snapshotear (para el paso «pasivos» del modal).
      liabilityEditedAtRef.current = Date.now();
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
      // Un borrado también es un cambio de pasivos sin snapshotear.
      liabilityEditedAtRef.current = Date.now();
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

  // Acciones del modal «¿Guardar snapshot?». Best-effort: si la captura falla la cerramos sin
  // ruido (el modal es tonto, sin superficie de error; la captura casi nunca falla — el usuario ya
  // tiene rol de escritura por haber editado). onDismiss deja `snapshotPromptFiredRef` en true, de
  // modo que no se vuelve a ofrecer hasta que la ventana de ediciones se vacíe y se rellene.
  async function handleSnapshotPromptSaveAssets() {
    setSnapshotPromptBusy(true);
    try {
      await saveSnapshotNow(["asset"]);
      // ¿Hay cambios de pasivos posteriores al último snapshot de pasivos? Entonces ofrece el
      // paso 2; si no, cierra.
      const editedAt = liabilityEditedAtRef.current;
      const savedAt = liabilitySnapshotSavedAtRef.current;
      if (editedAt != null && (savedAt == null || savedAt < editedAt)) {
        setSnapshotPromptStep("liabilities");
      } else {
        setSnapshotPromptStep("closed");
      }
    } catch {
      setSnapshotPromptStep("closed");
    } finally {
      setSnapshotPromptBusy(false);
    }
  }

  async function handleSnapshotPromptSaveLiabilities() {
    setSnapshotPromptBusy(true);
    try {
      await saveSnapshotNow(["liability"]);
      liabilitySnapshotSavedAtRef.current = Date.now();
    } catch {
      /* best-effort: cerramos igualmente abajo */
    } finally {
      setSnapshotPromptBusy(false);
      setSnapshotPromptStep("closed");
    }
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
      <TopBar
        activeTab={activeTab}
        navigate={navigate}
        health={health}
        healthError={healthError}
        onMobileMenuOpen={() => setMobileNavOpen(true)}
        extras={
          installationGate === "member" && hasMembership ? (
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
          ) : null
        }
      />
      <MobileNavDrawer
        open={mobileNavOpen}
        onClose={() => setMobileNavOpen(false)}
        activeTab={activeTab}
        navigate={navigate}
      />

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

      <SnapshotPromptModal
        step={snapshotPromptStep}
        busy={snapshotPromptBusy}
        onSaveAssets={() => void handleSnapshotPromptSaveAssets()}
        onSaveLiabilities={() => void handleSnapshotPromptSaveLiabilities()}
        onDismiss={() => setSnapshotPromptStep("closed")}
      />

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

        <Suspense fallback={<p className="muted tight">Cargando…</p>}>
        {activeTab === "summary" ? (
          <SummaryView
            installation={installation}
            loading={installationBusy}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            summary={summary}
            summaryBusy={summaryBusy}
            projectionSeries={projectionSeries}
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
            onSaveSnapshot={
              // Solo ofrece capturar si el usuario tiene ≥1 activo propio en la lista visible: la
              // captura copia únicamente las filas del usuario de sesión, así que sin filas propias
              // generaría un snapshot vacío (miembro en vista Hogar sin activos suyos).
              assets.some((a) => a.owner_user_id === user.id)
                ? () => saveSnapshotNow(["asset"])
                : undefined
            }
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
            onSaveSnapshot={
              // `LiabilityApiRow` no trae `owner_user_id`, así que no podemos comprobar propiedad
              // fila a fila; solo ofrecemos la captura en vista «Mío» (donde toda fila es propia por
              // construcción) para no generar snapshots de pasivos vacíos en vista Hogar.
              ledgerPersonScope === "mine"
                ? async () => {
                    // Guardado manual desde la vista Pasivos: también marca el snapshot de pasivos
                    // como realizado, para que el paso 2 del modal no lo vuelva a ofrecer.
                    await saveSnapshotNow(["liability"]);
                    liabilitySnapshotSavedAtRef.current = Date.now();
                  }
                : undefined
            }
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
        ) : activeTab === "expenses" ? (
          <GastosView
            installation={installation}
            hasMembership={hasMembership}
            ledgerPersonScope={ledgerPersonScope}
            canEdit={installation?.role !== "viewer"}
            user={user}
            onCashflowMutated={() => void loadCashflowSeries()}
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
            summary={summary}
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
            historySeries={historySeries}
            cashflowSeries={cashflowSeries}
            cashflowDaily={cashflowDaily}
            onRequestDailyCashflow={() => void loadCashflowDaily()}
            projectionBusy={projectionBusy}
            projectionError={projectionError}
            userBirthDate={user?.birth_date ?? null}
            calendarTz={installation?.installation.calendar_tz?.trim() || "UTC"}
            planningFlows={planningFlows}
          />
        ) : activeTab === "settings" ? (
          <SettingsView
            user={user}
            themePref={themePref}
            onChangeTheme={setThemePref}
            onLogout={() => void logout()}
            onEditAccount={() => {
              setUserProfileError(null);
              setUserBirthDraft(user.birth_date?.trim() ?? "");
              setUserProfileOpen(true);
            }}
            authBusy={authBusy}
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
            canEditHistory={installation?.role !== "viewer"}
            currencyIso={installation?.installation.base_currency ?? ""}
            calendarTz={installation?.installation.calendar_tz?.trim() || "UTC"}
            onHistoryMutated={() => {
              void loadHistorySeries();
              // Los snapshots anclan la serie fina del cash-flow: refrescarla también.
              void loadCashflowSeries();
            }}
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
        </Suspense>
      </main>
        </>
      )}
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
