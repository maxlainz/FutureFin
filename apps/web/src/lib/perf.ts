/**
 * Instrumentación temporal del flujo de Proyección. Opt-in con `?perf=1` en la
 * URL o `localStorage.setItem("debug:chart-perf","1")`. Sin coste runtime
 * cuando está apagado (todos los métodos hacen early return).
 *
 * Uso típico:
 *   chartPerf.mark("baseSeries-start");
 *   ... compute ...
 *   chartPerf.mark("baseSeries-end");
 *   chartPerf.measure("baseSeries-memo", "baseSeries-start", "baseSeries-end");
 *   chartPerf.report();
 */

const ENABLED =
  typeof window !== "undefined" &&
  (window.location.search.includes("perf=1") ||
    (typeof window.localStorage !== "undefined" &&
      window.localStorage.getItem("debug:chart-perf") === "1"));

export const chartPerf = {
  enabled: ENABLED,

  mark(name: string): void {
    if (!ENABLED) return;
    try {
      performance.mark(`chart:${name}`);
    } catch {
      /* algunos navegadores tiran si la cadena de marcas es inválida — silenciar */
    }
  },

  measure(name: string, start: string, end: string): void {
    if (!ENABLED) return;
    try {
      performance.measure(`chart:${name}`, `chart:${start}`, `chart:${end}`);
    } catch {
      /* la marca de start o end puede no existir si algún paso se saltó */
    }
  },

  report(): void {
    if (!ENABLED) return;
    const measures = performance
      .getEntriesByType("measure")
      .filter((e) => e.name.startsWith("chart:"));
    if (measures.length === 0) return;
    const summary = measures.map((e) => ({
      step: e.name.replace("chart:", ""),
      ms: Math.round(e.duration * 10) / 10,
    }));
    console.table(summary);
    const total = summary.reduce((s, e) => s + e.ms, 0);
    console.info(`[chart:perf] total ≈ ${total.toFixed(1)} ms`);
    performance.clearMarks();
    performance.clearMeasures();
  },
};
