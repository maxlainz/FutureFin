import { describe, expect, it } from "vitest";
import {
  clampWindowStart,
  domainMonthsOf,
  minSpanOf,
  panWindow,
  pinchWindow,
  type ChartDomain,
} from "./chart-gestures";

/**
 * Reproducción INLINE de la aritmética de `onWheel` (ProjectionNetWorthChart),
 * la fuente de verdad que estos helpers deben espejar. Copiada literalmente del
 * componente para que el test falle si los helpers divergen del wheel.
 */
function wheelZoom(
  visibleMonthStart: number,
  monthSpan: number,
  anchorMonth: number,
  deltaY: number,
  totalMonths: number,
  historyStartMonth: number,
): { startMonth: number; monthSpan: number } {
  const domainMonths = totalMonths - historyStartMonth;
  const minSpan = Math.min(totalMonths, 12);
  const zoomFactor = deltaY < 0 ? 0.88 : 1.14;
  const nextSpanRaw = Math.round(monthSpan * zoomFactor);
  const nextSpan = Math.max(minSpan, Math.min(domainMonths, nextSpanRaw));
  const ratio = (anchorMonth - visibleMonthStart) / Math.max(1, monthSpan - 1);
  const nextStartRaw = Math.round(anchorMonth - ratio * (nextSpan - 1));
  const maxStart = Math.max(historyStartMonth, totalMonths - nextSpan);
  const nextStart = Math.max(historyStartMonth, Math.min(maxStart, nextStartRaw));
  return { startMonth: nextStart, monthSpan: nextSpan };
}

/** Clamp de pan del wheel (branch isPan), aislado. */
function wheelPanClamp(
  candidateStart: number,
  monthSpan: number,
  totalMonths: number,
  historyStartMonth: number,
): number {
  const maxStart = Math.max(historyStartMonth, totalMonths - monthSpan);
  return Math.max(historyStartMonth, Math.min(maxStart, candidateStart));
}

describe("domain helpers mirror the wheel", () => {
  it("domainMonthsOf = totalMonths - historyStartMonth", () => {
    expect(domainMonthsOf({ totalMonths: 840, historyStartMonth: -24 })).toBe(864);
    expect(domainMonthsOf({ totalMonths: 120, historyStartMonth: 0 })).toBe(120);
  });
  it("minSpanOf = min(totalMonths, 12)", () => {
    expect(minSpanOf({ totalMonths: 840, historyStartMonth: -24 })).toBe(12);
    expect(minSpanOf({ totalMonths: 6, historyStartMonth: 0 })).toBe(6);
  });
});

describe("clampWindowStart == wheel pan clamp", () => {
  const domains: ChartDomain[] = [
    { totalMonths: 840, historyStartMonth: 0 },
    { totalMonths: 840, historyStartMonth: -24 },
    { totalMonths: 120, historyStartMonth: -6 },
  ];
  for (const d of domains) {
    for (const span of [12, 60, 240]) {
      for (const raw of [-100, -24, -6, 0, 50, 400, 900]) {
        it(`clamp raw=${raw} span=${span} d=${d.totalMonths}/${d.historyStartMonth}`, () => {
          expect(clampWindowStart(raw, span, d)).toBe(
            wheelPanClamp(raw, span, d.totalMonths, d.historyStartMonth),
          );
        });
      }
    }
  }
});

describe("pinchWindow == wheel zoom (scale = 1/zoomFactor)", () => {
  const d: ChartDomain = { totalMonths: 840, historyStartMonth: -24 };
  const cases = [
    { start: 0, span: 864, anchor: 100, deltaY: -1 }, // zoom in
    { start: 0, span: 864, anchor: 100, deltaY: 1 }, // zoom out
    { start: -24, span: 200, anchor: -10, deltaY: -1 }, // anchored in the past
    { start: 300, span: 120, anchor: 360, deltaY: 1 },
    { start: 12, span: 12, anchor: 18, deltaY: -1 }, // already at min span
    { start: 0, span: 864, anchor: 0, deltaY: 1 }, // already at full domain
  ];
  for (const c of cases) {
    it(`start=${c.start} span=${c.span} anchor=${c.anchor} deltaY=${c.deltaY}`, () => {
      const zoomFactor = c.deltaY < 0 ? 0.88 : 1.14;
      const scale = 1 / zoomFactor;
      const got = pinchWindow(c.start, c.span, c.anchor, scale, d);
      const expected = wheelZoom(
        c.start,
        c.span,
        c.anchor,
        c.deltaY,
        d.totalMonths,
        d.historyStartMonth,
      );
      expect(got).toEqual(expected);
    });
  }
});

describe("panWindow: manipulación directa 1:1 + clamp", () => {
  const d: ChartDomain = { totalMonths: 840, historyStartMonth: -24 };

  it("dedo a la derecha (dxPx > 0) → startMonth DECRECE (aparece el pasado)", () => {
    const start = 300;
    const next = panWindow(start, 120, 100, 0.5, d); // dxPx=100, monthsPerPx=0.5 → -50 meses
    expect(next).toBe(250);
  });

  it("dedo a la izquierda (dxPx < 0) → startMonth CRECE (avanza al futuro)", () => {
    const start = 300;
    const next = panWindow(start, 120, -100, 0.5, d); // +50 meses
    expect(next).toBe(350);
  });

  it("clampa contra el borde histórico izquierdo (no expulsa el pasado)", () => {
    const start = -24;
    const next = panWindow(start, 120, 5000, 0.5, d); // empuja muy a la izquierda
    expect(next).toBe(-24); // historyStartMonth
  });

  it("clampa contra el borde derecho (maxStart = totalMonths - span)", () => {
    const start = 700;
    const next = panWindow(start, 120, -5000, 0.5, d); // empuja muy a la derecha
    expect(next).toBe(840 - 120); // 720
  });

  it("dxPx=0 mantiene la ventana (redondeo estable)", () => {
    expect(panWindow(100, 60, 0, 0.5, d)).toBe(100);
  });

  it("comparte clamp con clampWindowStart", () => {
    const start = 300;
    const dxPx = -100;
    const monthsPerPx = 0.5;
    const span = 120;
    const raw = Math.round(start - dxPx * monthsPerPx);
    expect(panWindow(start, span, dxPx, monthsPerPx, d)).toBe(
      clampWindowStart(raw, span, d),
    );
  });
});
