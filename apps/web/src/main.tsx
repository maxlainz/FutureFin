import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/theme.css";
import "./index.css";
import App from "./App.tsx";
import { chartPerf } from "./lib/perf";

// Detecta tasks que bloquean el main thread durante el flujo del chart.
// Solo activo bajo `?perf=1` o `localStorage.debug:chart-perf=1`.
if (chartPerf.enabled && "PerformanceObserver" in window) {
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        if (e.duration > 50) {
          // eslint-disable-next-line no-console
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
    <App />
  </StrictMode>
);
