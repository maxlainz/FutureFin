import { describe, expect, it } from "vitest";
import { deflationFactorAt, projectionXTicks } from "./projection-chart";

describe("deflationFactorAt", () => {
  it("mes 0 → 1 (sin ajuste)", () => {
    expect(deflationFactorAt(0, 3)).toBe(1);
  });
  it("12 meses al 3% → 1/1.03", () => {
    expect(deflationFactorAt(12, 3)).toBeCloseTo(1 / 1.03, 10);
  });
  it("−12 meses (pasado) al 3% → amplifica ≈ 1.03", () => {
    expect(deflationFactorAt(-12, 3)).toBeCloseTo(1.03, 10);
  });
  it("inflación 0 → 1 en cualquier mes", () => {
    expect(deflationFactorAt(12, 0)).toBe(1);
    expect(deflationFactorAt(-24, 0)).toBe(1);
  });
  it("inflación negativa → 1 (sin ajuste)", () => {
    expect(deflationFactorAt(12, -2)).toBe(1);
  });
});

const DATES_OPTS = {
  ageUiMode: "dates" as const,
  anchorDateYmd: "2026-07-06",
  calendarTz: "UTC",
};

describe("projectionXTicks — retrocompatibilidad (startMonth por defecto)", () => {
  it("fallback sin opts: salida idéntica a la conocida (solo futuro)", () => {
    const ticks = projectionXTicks(24);
    expect(ticks).toEqual([
      { monthIndex: 0, label: "Mes 0" },
      { monthIndex: 2, label: "Mes 2" },
      { monthIndex: 4, label: "Mes 4" },
      { monthIndex: 6, label: "Mes 6" },
      { monthIndex: 8, label: "Mes 8" },
      { monthIndex: 10, label: "Mes 10" },
      { monthIndex: 12, label: "Mes 12" },
      { monthIndex: 14, label: "Mes 14" },
      { monthIndex: 16, label: "Mes 16" },
      { monthIndex: 18, label: "Mes 18" },
      { monthIndex: 20, label: "Mes 20" },
      { monthIndex: 22, label: "Mes 22" },
      { monthIndex: 24, label: "Mes 24" },
    ]);
  });

  it("modo fechas: solo ticks de futuro (mes 0 excluido, pertenece a «Hoy»)", () => {
    const ticks = projectionXTicks(36, DATES_OPTS);
    expect(ticks).toEqual([
      { monthIndex: 6, label: "2027" },
      { monthIndex: 18, label: "2028" },
      { monthIndex: 30, label: "2029" },
    ]);
  });

  it("omitir startMonth === pasar startMonth = 0 (fallback y fechas)", () => {
    expect(projectionXTicks(24)).toEqual(
      projectionXTicks(24, undefined, undefined, 0),
    );
    expect(projectionXTicks(36, DATES_OPTS)).toEqual(
      projectionXTicks(36, DATES_OPTS, undefined, 0),
    );
  });
});

describe("projectionXTicks — con historia (startMonth < 0)", () => {
  it("modo fechas emite ticks negativos y excluye el mes 0", () => {
    const ticks = projectionXTicks(24, DATES_OPTS, undefined, -24);
    const idx = ticks.map((t) => t.monthIndex);
    expect(idx).toEqual([-18, -6, 6, 18]);
    expect(idx.some((m) => m < 0)).toBe(true);
    expect(idx.every((m) => m !== 0)).toBe(true);
  });

  it("fallback sin opts también cubre el pasado e incluye el 0 (divisor)", () => {
    const idx = projectionXTicks(24, undefined, undefined, -24).map(
      (t) => t.monthIndex,
    );
    expect(idx.some((m) => m < 0)).toBe(true);
    expect(idx).toContain(0);
    // Ordenado ascendente.
    expect(idx).toEqual([...idx].sort((a, b) => a - b));
  });
});
