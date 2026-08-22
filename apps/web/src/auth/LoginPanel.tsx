/**
 * Formulario de login compacto y autocontenido — lo consume la pantalla de autorización
 * OAuth (`/oauth/authorize`), que vive FUERA de App.tsx y solo necesita iniciar sesión
 * (registrarse ahí no tiene sentido: la instalación aprueba miembros desde Ajustes).
 *
 * Deliberadamente NO es una extracción del formulario de App.tsx (plan B autorizado):
 * el estado de auth de App.tsx está entrelazado con logout/refresh y moverlo arriesgaba
 * regresión sin cambiar nada observable. Si algún día App.tsx adelgaza, este panel es
 * el punto de aterrizaje natural.
 */

import { useState, type FormEvent, type ReactNode } from "react";
import {
  ApiRequestError,
  apiErrorFromResponse,
  apiFetch,
  defaultFetchInit,
} from "../api/client";
import { LOGIN_INVALID_CREDENTIALS } from "../lib/errorMessages";
import type { UserResponse } from "../api/types";

export function LoginPanel(props: {
  intro?: ReactNode;
  onAuthenticated: (user: UserResponse) => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (ev: FormEvent) => {
    ev.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const res = await apiFetch("/v1/auth/login", {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password }),
      });
      if (!res.ok) {
        throw await apiErrorFromResponse(res);
      }
      const me = (await res.json()) as UserResponse;
      setPassword("");
      props.onAuthenticated(me);
    } catch (e: unknown) {
      // Un 401 al ENTRAR son credenciales incorrectas, no una sesión caducada (ver
      // LOGIN_INVALID_CREDENTIALS): aquí todavía no hay sesión que caducar.
      setError(
        e instanceof ApiRequestError && e.status === 401
          ? LOGIN_INVALID_CREDENTIALS
          : e instanceof Error
            ? e.message
            : String(e),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="auth-panel card-elevated">
      <h2 className="auth-panel-title">Iniciar sesión</h2>
      {props.intro ? <p className="muted">{props.intro}</p> : null}
      <form className="stack" onSubmit={(e) => void submit(e)}>
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
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            minLength={12}
            maxLength={256}
          />
        </label>
        <button type="submit" className="btn primary wide" disabled={busy}>
          Entrar
        </button>
      </form>
      {error ? <p className="error compact">{error}</p> : null}
    </div>
  );
}
