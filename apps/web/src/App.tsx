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

const defaultFetchInit: RequestInit = {
  credentials: "include",
};

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
    }
  }, [user, loadHouseholds]);

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

  return (
    <main className="layout">
      <header className="page-header">
        <h1>FutureFin</h1>
        <p className="muted tight">
          En Docker, interfaz y API comparten{" "}
          <code>http://127.0.0.1:8080</code>. En desarrollo con Vite, la
          interfaz suele ir en <code>:8080</code> y la API en{" "}
          <code>:8081</code> (proxy); si el puerto está ocupado, Vite puede usar
          otro — revisa la consola.
        </p>
      </header>

      {sessionBusy ? (
        <section className="card">
          <p className="muted">Comprobando sesión…</p>
        </section>
      ) : user ? (
        <section className="card">
          <div className="row-between">
            <div>
              <h2>Sesión</h2>
              <p className="muted tight">
                Conectado como <strong>{user.username}</strong>
              </p>
            </div>
            <button
              type="button"
              className="btn secondary"
              disabled={authBusy}
              onClick={() => void logout()}
            >
              Cerrar sesión
            </button>
          </div>
        </section>
      ) : (
        <section className="card">
          <h2>Acceso</h2>
          <div className="segmented" role="tablist" aria-label="Modo de acceso">
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
                  authMode === "register" ? "new-password" : "current-password"
                }
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                minLength={12}
                maxLength={256}
              />
            </label>
            <button type="submit" className="btn primary" disabled={authBusy}>
              {authMode === "register" ? "Registrarse y entrar" : "Entrar"}
            </button>
          </form>
          <p className="hint">
            Tras registrarte se hace login automático con la misma contraseña.
            Usuario: 3–64 caracteres; solo letras, dígitos,{" "}
            <code>.</code>, <code>_</code>, <code>-</code>. Contraseña: mínimo 12
            caracteres.
          </p>
        </section>
      )}

      {sessionError ? (
        <p className="error banner">{sessionError}</p>
      ) : null}

      {user ? (
        <section className="card">
          <h2>Hogares</h2>
          {householdsError ? (
            <p className="error">{householdsError}</p>
          ) : null}
          {hhBusy && households.length === 0 ? (
            <p className="muted">Cargando…</p>
          ) : households.length === 0 ? (
            <p className="muted tight">
              Aún no tienes hogares. Crea uno para empezar a modelar datos.
            </p>
          ) : (
            <ul className="household-list">
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
          <form className="stack bordered" onSubmit={(e) => void createHousehold(e)}>
            <h3 className="subsection-title">Nuevo hogar</h3>
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
      ) : null}

      <section className="card">
        <h2>Estado de la API</h2>
        {healthError ? (
          <p className="error">
            No se pudo contactar <code>/v1/health</code>: {healthError}
          </p>
        ) : health ? (
          <pre className="mono">{JSON.stringify(health, null, 2)}</pre>
        ) : (
          <p className="muted">Comprobando…</p>
        )}
      </section>
    </main>
  );
}
