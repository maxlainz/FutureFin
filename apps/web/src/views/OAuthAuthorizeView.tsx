/**
 * Pantalla de autorización OAuth (`/oauth/authorize`) — la sirve el fallback SPA y se
 * monta desde `main.tsx` FUERA de App.tsx (el router de pestañas redirigiría a /resumen
 * y perdería los query params). Los params OAuth de la URL no se tocan en ningún
 * momento: el login es un fetch + re-render, y la única navegación es el
 * `window.location.assign(redirect_to)` final.
 *
 * Máquina de estados: loading → disabled | invalid | redirecting | login | consent
 * (→ submitting → pending | error). Errores FATALES (cliente desconocido, redirect sin
 * match) se PINTAN y jamás redirigen — redirigir sería un open redirect.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import "../App.css";
import { defaultFetchInit } from "../api/client";
import type {
  OAuthAuthorizeDecisionApi,
  OAuthAuthorizeDetailsApi,
  UserResponse,
} from "../api/types";
import { LoginPanel } from "../auth/LoginPanel";
import { apiUrl } from "../lib/basePath";
import { isoTimestampDmy } from "../lib/format";
import {
  authorizeErrorMessage,
  parseAuthorizeParams,
  redirectHostLabel,
  type AuthorizeParams,
} from "../lib/oauth";
import {
  applyTheme,
  loadThemePref,
  subscribeSystemThemeChanges,
} from "../lib/theme";

type Phase =
  | { kind: "loading" }
  | { kind: "disabled" }
  | { kind: "invalid"; message: string }
  | { kind: "redirecting" }
  | { kind: "login" }
  | { kind: "consent"; details: OAuthAuthorizeDetailsApi; user: UserResponse }
  | { kind: "submitting"; details: OAuthAuthorizeDetailsApi; user: UserResponse }
  | { kind: "pending" }
  | { kind: "error"; message: string };

export function OAuthAuthorizeView() {
  const params = useMemo(
    () => parseAuthorizeParams(window.location.search),
    [],
  );
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  // La vista vive fuera de App.tsx: aplica el tema ella misma (claro/oscuro/auto).
  useEffect(() => {
    applyTheme(loadThemePref());
    return subscribeSystemThemeChanges(() => applyTheme(loadThemePref()));
  }, []);

  const evaluate = useCallback(async () => {
    if (!params) {
      setPhase({
        kind: "invalid",
        message:
          "La solicitud de autorización está incompleta. Vuelve a intentar la conexión desde el cliente (p. ej. claude.ai).",
      });
      return;
    }
    setPhase({ kind: "loading" });
    try {
      const details = await fetchAuthorizeDetails(params);
      if (details === "disabled") {
        setPhase({ kind: "disabled" });
        return;
      }
      if (details.status === "invalid_request") {
        setPhase({
          kind: "invalid",
          message: authorizeErrorMessage(details.error_code ?? "invalid_request"),
        });
        return;
      }
      if (details.status === "redirect_error") {
        setPhase({ kind: "redirecting" });
        if (details.redirect_to) window.location.replace(details.redirect_to);
        return;
      }
      // status === "consent": ¿hay sesión?
      const me = await fetch(apiUrl("/v1/auth/me"), defaultFetchInit);
      if (me.status === 401) {
        setPhase({ kind: "login" });
        return;
      }
      if (!me.ok) {
        throw new Error(`HTTP ${me.status}`);
      }
      const user = (await me.json()) as UserResponse;
      setPhase({ kind: "consent", details, user });
    } catch (e: unknown) {
      setPhase({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [params]);

  useEffect(() => {
    void evaluate();
  }, [evaluate]);

  const decide = async (approve: boolean) => {
    if (phase.kind !== "consent" || !params) return;
    setPhase({ kind: "submitting", details: phase.details, user: phase.user });
    try {
      const res = await fetch(apiUrl("/v1/oauth/authorize"), {
        ...defaultFetchInit,
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ approve, ...params }),
      });
      if (res.status === 401) {
        setPhase({ kind: "login" });
        return;
      }
      if (res.status === 403) {
        setPhase({ kind: "pending" });
        return;
      }
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      const body = (await res.json()) as OAuthAuthorizeDecisionApi;
      window.location.assign(body.redirect_to);
    } catch (e: unknown) {
      setPhase({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  const changeUser = async () => {
    try {
      await fetch(apiUrl("/v1/auth/logout"), { ...defaultFetchInit, method: "POST" });
    } catch {
      // Da igual: sin cookie válida el siguiente estado es login de todos modos.
    }
    setPhase({ kind: "login" });
  };

  return (
    <div className="auth-screen">
      <div className="auth-brand">
        <div className="auth-brand-inner">
          <span className="logo-mark">FF</span>
          <h1>FutureFin</h1>
        </div>
      </div>
      <div className="auth-panel-wrap">{renderPhase()}</div>
    </div>
  );

  function renderPhase() {
    switch (phase.kind) {
      case "loading":
      case "redirecting":
        return (
          <div className="auth-panel card-elevated">
            <div className="app-loading-inner">
              <span className="spinner" aria-hidden />
              <p className="muted">
                {phase.kind === "loading" ? "Comprobando la solicitud…" : "Redirigiendo…"}
              </p>
            </div>
          </div>
        );
      case "disabled":
        return (
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">Acceso deshabilitado</h2>
            <p className="muted">
              El acceso MCP está deshabilitado en esta instalación
              (<code>FUTUREFIN_MCP_ENABLED=0</code>).
            </p>
          </div>
        );
      case "invalid":
        return (
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">Solicitud no válida</h2>
            <p className="error compact" role="alert">
              {phase.message}
            </p>
          </div>
        );
      case "login":
        return (
          // El `next` conserva los params OAuth: a la vuelta del rodeo por Home Assistant el
          // navegador aterriza otra vez en /oauth/authorize?<mismos params>, `evaluate()` vuelve
          // a correr y —ya con sesión— cae en la pantalla de consentimiento.
          <LoginPanel
            intro="Una aplicación quiere conectarse a tu FutureFin con tu rol (lectura, y escritura si está permitida en Ajustes → MCP). Inicia sesión para revisar y autorizar el acceso."
            onAuthenticated={() => void evaluate()}
            haLoginNext={window.location.pathname + window.location.search}
          />
        );
      case "pending":
        return (
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">Cuenta pendiente</h2>
            <p className="muted">
              Tu cuenta está pendiente de aprobación del propietario de la
              instalación. Hasta entonces no puedes autorizar conexiones.
            </p>
          </div>
        );
      case "error":
        return (
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">Algo ha fallado</h2>
            <p className="error compact" role="alert">
              {phase.message}
            </p>
            <button type="button" className="btn ghost" onClick={() => void evaluate()}>
              Reintentar
            </button>
          </div>
        );
      case "consent":
      case "submitting": {
        const { details, user } = phase;
        const busy = phase.kind === "submitting";
        const host =
          details.redirect_host ??
          (params ? redirectHostLabel(params.redirect_uri) : "");
        return (
          <div className="auth-panel card-elevated">
            <h2 className="auth-panel-title">
              {details.client_name ?? "Una aplicación"} quiere acceder a tu FutureFin
            </h2>
            <p className="muted tight">
              Volverás a: <strong>{host}</strong>
              <br />
              <span className="muted">
                («{details.client_name}» es el nombre que declara la propia
                aplicación; el destino de arriba es el dato verificado.)
              </span>
            </p>
            <div className="stack">
              <p className="tight">
                Podrá <strong>leer</strong> tu resumen, proyección, presupuesto,
                movimientos, histórico, activos y pasivos, y{" "}
                <strong>registrar o modificar datos en tu nombre</strong> si tu rol
                escribe y el interruptor de escritura está activado (Ajustes → MCP).
                Puedes revocar el acceso cuando quieras en Ajustes → MCP.
              </p>
              {details.already_connected ? (
                <p className="muted tight">
                  Ya conectaste esta aplicación
                  {details.connected_at
                    ? ` el ${isoTimestampDmy(details.connected_at)}`
                    : ""}
                  ; al continuar se renovará el acceso.
                </p>
              ) : null}
              <p className="muted tight">
                Autorizas como <strong>{user.username}</strong>.{" "}
                <button
                  type="button"
                  className="btn ghost"
                  disabled={busy}
                  onClick={() => void changeUser()}
                >
                  Cambiar de usuario
                </button>
              </p>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn primary"
                  disabled={busy}
                  onClick={() => void decide(true)}
                >
                  {busy ? "Un momento…" : "Autorizar"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={busy}
                  onClick={() => void decide(false)}
                >
                  Cancelar
                </button>
              </div>
            </div>
          </div>
        );
      }
    }
  }
}

/** GET authorize-details con la misma query de la URL; "disabled" si el kill-switch (404). */
async function fetchAuthorizeDetails(
  params: AuthorizeParams,
): Promise<OAuthAuthorizeDetailsApi | "disabled"> {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v) q.set(k, v);
  }
  const res = await fetch(
    apiUrl(`/v1/oauth/authorize-details?${q.toString()}`),
    defaultFetchInit,
  );
  if (res.status === 404) return "disabled";
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return (await res.json()) as OAuthAuthorizeDetailsApi;
}
