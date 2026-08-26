import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "./styles/theme.css";
import "./index.css";
import App from "./App.tsx";
import { stripBase } from "./lib/basePath";
import { chartPerf } from "./lib/perf";

// La pantalla de autorización OAuth se resuelve AQUÍ, no en el router de App.tsx: su
// useLayoutEffect canonicaliza cualquier path desconocido a /resumen y perdería los
// query params del authorization request. Chunk lazy: los usuarios normales no lo bajan.
const OAuthAuthorizeView = lazy(() =>
  import("./views/OAuthAuthorizeView").then((m) => ({
    default: m.OAuthAuthorizeView,
  })),
);
const isOAuthAuthorize =
  stripBase(window.location.pathname).replace(/\/+$/, "") === "/oauth/authorize";

// Detecta tasks que bloquean el main thread durante el flujo del chart.
// Solo activo bajo `?perf=1` o `localStorage.debug:chart-perf=1`.
if (chartPerf.enabled && "PerformanceObserver" in window) {
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        if (e.duration > 50) {
          console.warn(`[chart:perf] long task ${e.duration.toFixed(0)} ms`, e);
        }
      }
    }).observe({ entryTypes: ["longtask"] });
  } catch {
    /* algunos navegadores (Firefox) no soportan longtask todavía */
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isOAuthAuthorize ? (
      <Suspense fallback={null}>
        <OAuthAuthorizeView />
      </Suspense>
    ) : (
      <App />
    )}
  </StrictMode>
);
