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
    }
  }, [user, loadInstallation]);

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
    if (!user) {
      setCategories([]);
      setCategoriesError(null);
      setEditingCategoryId(null);
      setEditCategoryName("");
      setNewCatName("");
    }
  }, [user]);

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
          />
        ) : activeTab === "settings" ? (
          <SettingsView
            installation={installation}
            installationBusy={installationBusy}
            setupCurrency={setupCurrency}
            setSetupCurrency={setSetupCurrency}
            setupInstallation={(e) => void setupInstallation(e)}
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

function SummaryView({
  installation,
  loading,
  hasMembership,
}: {
  installation: InstallationAccess | null;
  loading: boolean;
  hasMembership: boolean;
}) {
  const currency =
    installation?.installation.base_currency ?? METRIC_DASH;

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
          ) : (
            <>
              Moneda base <strong>{currency}</strong>. Métricas en vivo cuando
              el backend exponga activos y pasivos.
            </>
          )}
        </p>
      </div>

      <div className="metric-grid">
        <MetricCard
          label="Patrimonio neto"
          value={METRIC_DASH}
          suffix={currency}
          hint="Conectará con activos y pasivos cuando la API exponga totales."
        />
        <MetricCard
          label="Activos totales"
          value={METRIC_DASH}
          suffix={currency}
        />
        <MetricCard
          label="Pasivos totales"
          value={METRIC_DASH}
          suffix={currency}
        />
        <MetricCard
          label="Ratio deuda / activos"
          value={METRIC_DASH}
          hint="Se calculará con los mismos criterios que el cliente de referencia."
        />
      </div>

      <section className="panel">
        <h3 className="panel-title">Salud financiera</h3>
        <p className="muted tight">
          Aquí irán ingresos, gastos, ahorro, runway y coberturas — datos en
          vivo enlazados a esta instalación (pendiente de backend).
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
          Gráficos donut y reparto por categoría (activos / pasivos) — mismo
          papel que en la app de escritorio.
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
  setupInstallation,
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
  setupInstallation: (e: FormEvent) => void;
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
