import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
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

type HouseholdSummary = {
  id: string;
  name: string;
  base_currency: string;
  role: "owner" | "member" | "viewer";
};

type PersonRow = {
  id: string;
  household_id: string;
  display_name: string;
  is_primary: boolean;
  birth_date?: string | null;
};

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

  const [households, setHouseholds] = useState<HouseholdSummary[]>([]);
  const [householdsError, setHouseholdsError] = useState<string | null>(null);
  const [hhBusy, setHhBusy] = useState(false);
  const [hhName, setHhName] = useState("");
  const [hhCurrency, setHhCurrency] = useState<"EUR" | "USD" | "GBP">("EUR");
  const [selectedHouseholdId, setSelectedHouseholdId] = useState<string | null>(
    null,
  );

  const [persons, setPersons] = useState<PersonRow[]>([]);
  const [personsError, setPersonsError] = useState<string | null>(null);
  const [personsBusy, setPersonsBusy] = useState(false);
  const [personDisplayName, setPersonDisplayName] = useState("");
  const [personBirthDate, setPersonBirthDate] = useState("");
  const [personIsPrimary, setPersonIsPrimary] = useState(false);
  const [personMutating, setPersonMutating] = useState(false);

  const [activeTab, setActiveTab] = useState<TabId>("summary");

  const selectedHousehold =
    households.find((h) => h.id === selectedHouseholdId) ?? null;

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

  const loadHouseholds = useCallback(async () => {
    setHhBusy(true);
    setHouseholdsError(null);
    try {
      const res = await fetch("/v1/households", defaultFetchInit);
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const list = (await res.json()) as HouseholdSummary[];
      setHouseholds(list);
    } catch (e: unknown) {
      setHouseholds([]);
      setHouseholdsError(e instanceof Error ? e.message : String(e));
    } finally {
      setHhBusy(false);
    }
  }, []);

  const loadPersons = useCallback(async (householdId: string) => {
    setPersonsBusy(true);
    setPersonsError(null);
    try {
      const res = await fetch(
        `/v1/households/${encodeURIComponent(householdId)}/persons`,
        defaultFetchInit,
      );
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      const list = (await res.json()) as PersonRow[];
      setPersons(list);
    } catch (e: unknown) {
      setPersons([]);
      setPersonsError(e instanceof Error ? e.message : String(e));
    } finally {
      setPersonsBusy(false);
    }
  }, []);

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
      void loadHouseholds();
    } else {
      setHouseholds([]);
      setHouseholdsError(null);
      setSelectedHouseholdId(null);
      setPersons([]);
      setPersonsError(null);
    }
  }, [user, loadHouseholds]);

  useEffect(() => {
    if (!selectedHouseholdId) {
      setPersons([]);
      setPersonsError(null);
      return;
    }
    void loadPersons(selectedHouseholdId);
  }, [selectedHouseholdId, loadPersons]);

  useEffect(() => {
    if (households.length === 0) {
      setSelectedHouseholdId(null);
      return;
    }
    setSelectedHouseholdId((prev) => {
      if (prev && households.some((h) => h.id === prev)) {
        return prev;
      }
      return households[0]?.id ?? null;
    });
  }, [households]);

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
      setHouseholds([]);
      setSelectedHouseholdId(null);
      setPersons([]);
      setPersonsError(null);
      setActiveTab("summary");
    } catch (e: unknown) {
      setSessionError(e instanceof Error ? e.message : String(e));
    } finally {
      setAuthBusy(false);
    }
  }

  async function createHousehold(ev: FormEvent) {
    ev.preventDefault();
    setHhBusy(true);
    setHouseholdsError(null);
    try {
      const res = await fetch("/v1/households", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: hhName,
          base_currency: hhCurrency,
          projection_includes_inflation: false,
          projection_target_age: null,
          show_age_mode: "dates",
        }),
      });
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setHhName("");
      await loadHouseholds();
    } catch (e: unknown) {
      setHouseholdsError(e instanceof Error ? e.message : String(e));
    } finally {
      setHhBusy(false);
    }
  }

  async function createPerson(ev: FormEvent) {
    ev.preventDefault();
    if (!selectedHouseholdId) {
      return;
    }
    setPersonMutating(true);
    setPersonsError(null);
    try {
      const payload: {
        display_name: string;
        is_primary: boolean;
        birth_date?: string;
      } = {
        display_name: personDisplayName,
        is_primary: personIsPrimary,
      };
      if (personBirthDate.trim()) {
        payload.birth_date = personBirthDate.trim();
      }
      const res = await fetch(
        `/v1/households/${encodeURIComponent(selectedHouseholdId)}/persons`,
        {
          ...defaultFetchInit,
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        },
      );
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      setPersonDisplayName("");
      setPersonBirthDate("");
      setPersonIsPrimary(false);
      await loadPersons(selectedHouseholdId);
    } catch (e: unknown) {
      setPersonsError(e instanceof Error ? e.message : String(e));
    } finally {
      setPersonMutating(false);
    }
  }

  async function deletePerson(personId: string) {
    if (!selectedHouseholdId) {
      return;
    }
    setPersonMutating(true);
    setPersonsError(null);
    try {
      const res = await fetch(
        `/v1/households/${encodeURIComponent(selectedHouseholdId)}/persons/${encodeURIComponent(personId)}`,
        { ...defaultFetchInit, method: "DELETE" },
      );
      if (!res.ok && res.status !== 204) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadPersons(selectedHouseholdId);
    } catch (e: unknown) {
      setPersonsError(e instanceof Error ? e.message : String(e));
    } finally {
      setPersonMutating(false);
    }
  }

  async function makePersonPrimary(personId: string) {
    if (!selectedHouseholdId) {
      return;
    }
    setPersonMutating(true);
    setPersonsError(null);
    try {
      const res = await fetch(
        `/v1/households/${encodeURIComponent(selectedHouseholdId)}/persons/${encodeURIComponent(personId)}`,
        {
          ...defaultFetchInit,
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ is_primary: true }),
        },
      );
      if (!res.ok) {
        throw new Error(await errorMessageFromResponse(res));
      }
      await loadPersons(selectedHouseholdId);
    } catch (e: unknown) {
      setPersonsError(e instanceof Error ? e.message : String(e));
    } finally {
      setPersonMutating(false);
    }
  }

  const householdCanWrite = selectedHousehold?.role !== "viewer";

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
              <code>-</code>). Contraseña ≥ 12 caracteres.
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
        <div className="app-header-center">
          <label className="hh-select-label">
            <span className="sr-only">Hogar activo</span>
            <select
              className="hh-select"
              value={selectedHouseholdId ?? ""}
              onChange={(e) =>
                setSelectedHouseholdId(e.target.value || null)
              }
              disabled={households.length === 0}
            >
              {households.length === 0 ? (
                <option value="">Sin hogares — créalo en Ajustes</option>
              ) : (
                households.map((h) => (
                  <option key={h.id} value={h.id}>
                    {h.name} ({h.base_currency})
                  </option>
                ))
              )}
            </select>
          </label>
        </div>
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

        {householdsError ? (
          <div className="banner error-banner">{householdsError}</div>
        ) : null}

        {personsError ? (
          <div className="banner error-banner">{personsError}</div>
        ) : null}

        {activeTab === "summary" ? (
          <SummaryView
            household={selectedHousehold}
            hhBusy={hhBusy}
            hasHouseholds={households.length > 0}
          />
        ) : activeTab === "settings" ? (
          <SettingsView
            households={households}
            hhBusy={hhBusy}
            hhName={hhName}
            setHhName={setHhName}
            hhCurrency={hhCurrency}
            setHhCurrency={setHhCurrency}
            createHousehold={(e) => void createHousehold(e)}
            health={health}
            healthError={healthError}
            selectedHouseholdId={selectedHouseholdId}
            householdCanWrite={householdCanWrite}
            persons={persons}
            personsBusy={personsBusy}
            personDisplayName={personDisplayName}
            setPersonDisplayName={setPersonDisplayName}
            personBirthDate={personBirthDate}
            setPersonBirthDate={setPersonBirthDate}
            personIsPrimary={personIsPrimary}
            setPersonIsPrimary={setPersonIsPrimary}
            personMutating={personMutating}
            createPerson={(e) => void createPerson(e)}
            deletePerson={(id) => void deletePerson(id)}
            makePersonPrimary={(id) => void makePersonPrimary(id)}
          />
        ) : (
          <PlaceholderTab tabLabel={TABS.find((x) => x.id === activeTab)?.label ?? ""} />
        )}
      </main>
    </div>
  );
}

function SummaryView({
  household,
  hhBusy,
  hasHouseholds,
}: {
  household: HouseholdSummary | null;
  hhBusy: boolean;
  hasHouseholds: boolean;
}) {
  const currency = household?.base_currency ?? "—";

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Resumen</h2>
        <p className="workspace-sub">
          {hhBusy ? (
            "Cargando hogares…"
          ) : !hasHouseholds ? (
            <>
              Crea un hogar en <strong>Ajustes</strong> para fijar moneda y
              contexto.
            </>
          ) : household ? (
            <>
              Vista del hogar <strong>{household.name}</strong> · rol{" "}
              <span className="role-pill">{household.role}</span>
            </>
          ) : null}
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
          vivo respecto al hogar seleccionado (pendiente de backend).
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
  households,
  hhBusy,
  hhName,
  setHhName,
  hhCurrency,
  setHhCurrency,
  createHousehold,
  health,
  healthError,
  selectedHouseholdId,
  householdCanWrite,
  persons,
  personsBusy,
  personDisplayName,
  setPersonDisplayName,
  personBirthDate,
  setPersonBirthDate,
  personIsPrimary,
  setPersonIsPrimary,
  personMutating,
  createPerson,
  deletePerson,
  makePersonPrimary,
}: {
  households: HouseholdSummary[];
  hhBusy: boolean;
  hhName: string;
  setHhName: (v: string) => void;
  hhCurrency: "EUR" | "USD" | "GBP";
  setHhCurrency: (v: "EUR" | "USD" | "GBP") => void;
  createHousehold: (e: FormEvent) => void;
  health: HealthResponse | null;
  healthError: string | null;
  selectedHouseholdId: string | null;
  householdCanWrite: boolean;
  persons: PersonRow[];
  personsBusy: boolean;
  personDisplayName: string;
  setPersonDisplayName: (v: string) => void;
  personBirthDate: string;
  setPersonBirthDate: (v: string) => void;
  personIsPrimary: boolean;
  setPersonIsPrimary: (v: boolean) => void;
  personMutating: boolean;
  createPerson: (e: FormEvent) => void;
  deletePerson: (id: string) => void;
  makePersonPrimary: (id: string) => void;
}) {
  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Ajustes</h2>
        <p className="workspace-sub">
          Hogares, moneda base y personas del hogar activo (cabecera). Los
          visores solo leen.
        </p>
      </div>

      <section className="panel">
        <h3 className="panel-title">Personas del hogar activo</h3>
        {!selectedHouseholdId ? (
          <p className="muted tight">
            Elige un hogar en la cabecera para ver y editar personas.
          </p>
        ) : personsBusy ? (
          <p className="muted">Cargando personas…</p>
        ) : persons.length === 0 ? (
          <p className="muted tight">
            Ninguna persona todavía. Añade la primera (titular, cónyuge, hijos…).
          </p>
        ) : (
          <ul className="person-list">
            {persons.map((p) => (
              <li key={p.id} className="person-row">
                <div className="person-main">
                  <span className="person-name">{p.display_name}</span>
                  {p.birth_date ? (
                    <span className="person-meta">nac. {p.birth_date}</span>
                  ) : null}
                  {p.is_primary ? (
                    <span className="primary-badge">Principal</span>
                  ) : null}
                </div>
                {householdCanWrite ? (
                  <div className="person-actions">
                    {!p.is_primary ? (
                      <button
                        type="button"
                        className="btn ghost text"
                        disabled={personMutating}
                        onClick={() => makePersonPrimary(p.id)}
                      >
                        Marcar principal
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="btn ghost danger"
                      disabled={personMutating}
                      onClick={() => {
                        if (
                          window.confirm(
                            `¿Eliminar a «${p.display_name}» del hogar?`,
                          )
                        ) {
                          deletePerson(p.id);
                        }
                      }}
                    >
                      Eliminar
                    </button>
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
        {selectedHouseholdId && householdCanWrite ? (
          <form className="stack bordered-top" onSubmit={createPerson}>
            <h4 className="subsection-title">Nueva persona</h4>
            <label className="field">
              <span>Nombre</span>
              <input
                value={personDisplayName}
                onChange={(e) => setPersonDisplayName(e.target.value)}
                required
                maxLength={128}
                placeholder="Ej. María"
              />
            </label>
            <label className="field">
              <span>Fecha de nacimiento (opcional)</span>
              <input
                type="date"
                value={personBirthDate}
                onChange={(e) => setPersonBirthDate(e.target.value)}
              />
            </label>
            <label className="field checkbox-field">
              <input
                type="checkbox"
                checked={personIsPrimary}
                onChange={(e) => setPersonIsPrimary(e.target.checked)}
              />
              <span>Persona principal del hogar</span>
            </label>
            <button
              type="submit"
              className="btn primary"
              disabled={personMutating || personsBusy}
            >
              Añadir persona
            </button>
          </form>
        ) : selectedHouseholdId && !householdCanWrite ? (
          <p className="hint bordered-top">
            Tu rol en este hogar es solo lectura; no puedes cambiar personas.
          </p>
        ) : null}
      </section>

      <section className="panel">
        <h3 className="panel-title">Tus hogares</h3>
        {hhBusy && households.length === 0 ? (
          <p className="muted">Cargando…</p>
        ) : households.length === 0 ? (
          <p className="muted tight">Ninguno todavía.</p>
        ) : (
          <ul className="household-list roomy">
            {households.map((h) => (
              <li key={h.id}>
                <span className="hh-name">{h.name}</span>
                <span className="hh-meta">
                  {h.base_currency} · {h.role}
                </span>
              </li>
            ))}
          </ul>
        )}
        <form className="stack bordered-top" onSubmit={createHousehold}>
          <h4 className="subsection-title">Nuevo hogar</h4>
          <label className="field">
            <span>Nombre</span>
            <input
              value={hhName}
              onChange={(e) => setHhName(e.target.value)}
              required
              maxLength={128}
              placeholder="Ej. Casa principal"
            />
          </label>
          <label className="field">
            <span>Moneda base</span>
            <select
              value={hhCurrency}
              onChange={(e) =>
                setHhCurrency(e.target.value as "EUR" | "USD" | "GBP")
              }
            >
              <option value="EUR">EUR</option>
              <option value="USD">USD</option>
              <option value="GBP">GBP</option>
            </select>
          </label>
          <button type="submit" className="btn primary" disabled={hhBusy}>
            Crear hogar
          </button>
        </form>
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
          filtros por persona y KPIs reactivos al editar campos (sin botón global
          «calcular»).
        </p>
      </div>
    </div>
  );
}
