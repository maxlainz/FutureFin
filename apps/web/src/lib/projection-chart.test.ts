import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  buildWithdrawalTooltipRows,
  deflationFactorAt,
  formatYearsEsFromMonths,
  lastPointIndexAtOrBeforeMonth,
  projectionMaxXTicks,
  projectionXTicks,
  thinTicksFromEnd,
} from "./projection-chart";

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
  // INVERTIDO en 4.9.0 (#146): hasta 4.8.0 una inflación negativa devolvía 1 («sin ajuste»);
  // ahora compone — los euros de un mundo deflacionario valen MÁS en euros de hoy.
  it("inflación negativa → factor > 1 en meses futuros (12m a −2% → 1/0.98)", () => {
    expect(deflationFactorAt(12, -2)).toBeCloseTo(1 / 0.98, 10);
  });
  it("inflación negativa en el pasado → factor < 1 (espejo)", () => {
    expect(deflationFactorAt(-12, -2)).toBeCloseTo(0.98, 10);
  });
});

// #136-4b: fixture cruzado con `deflator_at_month_index` del servidor (suite Rust
// `deflator_parity.rs`). Si un lado cambia sin actualizar el JSON, SU suite falla — el fixture
// haciendo su trabajo. Dominio compartido k >= 0 entero; k < 0 y meses fraccionarios son
// TS-only (divergencia aceptada, declarada en financial-contracts §4).
describe("paridad del deflactor con el servidor (#136-4b)", () => {
  const dirname = path.dirname(fileURLToPath(import.meta.url));
  const fixturePath = path.resolve(
    dirname,
    "../../../api/tests/fixtures/deflator-parity.json",
  );
  type Case = {
    annual_inflation_percent: string;
    month_index: number;
    expected_deflator: string;
  };
  const cases = (
    JSON.parse(readFileSync(fixturePath, "utf-8")) as { cases: Case[] }
  ).cases;

  it("el fixture no está vacío", () => {
    expect(cases.length).toBeGreaterThan(0);
  });

  for (const c of cases) {
    it(`k=${c.month_index} al ${c.annual_inflation_percent} %`, () => {
      const got = deflationFactorAt(
        c.month_index,
        Number(c.annual_inflation_percent),
      );
      expect(Math.abs(got - Number(c.expected_deflator))).toBeLessThan(1e-9);
    });
  }
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

/**
 * Serie decimada tal y como la sirve `?density=hybrid`: los 13 primeros meses uno a uno y
 * después uno por año. La POSICIÓN 13 es el mes 24, no el 13 — ese desajuste es el que hacía
 * que un `clampToMonth` no recortara nada y que el eje X repartiera 70 años a distancias iguales.
 */
const HYBRID_POINTS = [
  ...Array.from({ length: 13 }, (_v, i) => ({ month_index: i })),
  ...Array.from({ length: 10 }, (_v, i) => ({ month_index: 24 + i * 12 })),
];

describe("lastPointIndexAtOrBeforeMonth", () => {
  it("serie mensual: la posición ES el mes", () => {
    const monthly = Array.from({ length: 60 }, (_v, i) => ({ month_index: i }));
    expect(lastPointIndexAtOrBeforeMonth(monthly, 0)).toBe(0);
    expect(lastPointIndexAtOrBeforeMonth(monthly, 11)).toBe(11);
    expect(lastPointIndexAtOrBeforeMonth(monthly, 59)).toBe(59);
  });

  it("serie hybrid: traduce el mes a su posición diezmada", () => {
    // Mes 12 = última posición del tramo mensual.
    expect(lastPointIndexAtOrBeforeMonth(HYBRID_POINTS, 12)).toBe(12);
    // Mes 30 cae entre el 24 (pos 13) y el 36 (pos 14) → se queda en el 24.
    expect(lastPointIndexAtOrBeforeMonth(HYBRID_POINTS, 30)).toBe(13);
    expect(HYBRID_POINTS[13]!.month_index).toBe(24);
    expect(lastPointIndexAtOrBeforeMonth(HYBRID_POINTS, 36)).toBe(14);
  });

  it("un mes más allá del último punto devuelve el último, no desborda", () => {
    const last = HYBRID_POINTS.length - 1;
    expect(lastPointIndexAtOrBeforeMonth(HYBRID_POINTS, 100_000)).toBe(last);
  });

  it("un mes anterior al primer punto devuelve 0: siempre hay algo que pintar", () => {
    expect(lastPointIndexAtOrBeforeMonth(HYBRID_POINTS, -5)).toBe(0);
    expect(lastPointIndexAtOrBeforeMonth([], 12)).toBe(0);
  });
});

describe("projectionMaxXTicks — techo de etiquetas por ancho", () => {
  it("plots estrechos (<560) exigen más aire por etiqueta → menos ticks", () => {
    // fechas: 340/52 → 6; 1300/34 → 18 (techo). edades: 340/44 → 7.
    expect(projectionMaxXTicks(340, "dates")).toBe(6);
    expect(projectionMaxXTicks(340, "ages")).toBe(7);
    expect(projectionMaxXTicks(1300, "dates")).toBe(18);
  });

  it("acotado a [5, 18] en los extremos", () => {
    expect(projectionMaxXTicks(60, "dates")).toBe(5);
    expect(projectionMaxXTicks(4000, "dates")).toBe(18);
  });
});

describe("formatYearsEsFromMonths — cinco casos del issue #132", () => {
  it("mes 0 → «Ya alcanzado» (ya jubilado, no «0 años»)", () => {
    expect(formatYearsEsFromMonths(0)).toBe("Ya alcanzado");
  });
  it("mes 5 → «5 meses» (no «0 años»)", () => {
    expect(formatYearsEsFromMonths(5)).toBe("5 meses");
  });
  it("mes 12 → «1 año» (singular, sin resto)", () => {
    expect(formatYearsEsFromMonths(12)).toBe("1 año");
  });
  it("mes 17 → «1 año y 5 meses» (no «1 años»)", () => {
    expect(formatYearsEsFromMonths(17)).toBe("1 año y 5 meses");
  });
  it("mes 199 → «16 años y 7 meses» (no «17 años»)", () => {
    expect(formatYearsEsFromMonths(199)).toBe("16 años y 7 meses");
  });
});

describe("thinTicksFromEnd — diezmado de los ticks visibles", () => {
  const years = Array.from({ length: 54 }, (_, i) => ({ monthIndex: 6 + i * 12 }));

  it("recorta a ≤ maxTicks con hueco uniforme y conserva el último visible", () => {
    const thinned = thinTicksFromEnd(years, 6);
    expect(thinned.length).toBeLessThanOrEqual(6);
    // El fin de la ventana sigue etiquetado (se diezma desde el final)…
    expect(thinned[thinned.length - 1]).toEqual(years[years.length - 1]);
    // …y todos los huecos son idénticos (step·12 meses).
    const gaps = new Set(
      thinned.slice(1).map((t, k) => t.monthIndex - thinned[k]!.monthIndex),
    );
    expect(gaps.size).toBe(1);
  });

  it("sin exceso devuelve los mismos ticks (copia)", () => {
    expect(thinTicksFromEnd(years.slice(0, 5), 6)).toEqual(years.slice(0, 5));
    expect(thinTicksFromEnd([], 6)).toEqual([]);
  });

  it("cap < 1 se trata como 1: sobrevive solo el último", () => {
    expect(thinTicksFromEnd(years, 0)).toEqual([years[years.length - 1]]);
  });
});

/**
 * Filas de flujos de retirada del tooltip (5.0.0 §B.8 + pase de correcciones §F).
 *
 * Lo que estos tests fijan no es el formato: es que **«Recorte» y «No financiado» son cosas
 * distintas y pueden estar las dos a la vez**. Hasta el pase el tooltip solo enseñaba el
 * recorte de la REGLA, así que un mes en que la cartera no dio para pagar el gasto se veía
 * idéntico a un mes normal — quedarse sin capital parecía un problema de configuración.
 */
describe("buildWithdrawalTooltipRows — flujos del mes jubilado", () => {
  const point = {
    month_index: 300,
    withdrawal: 2000,
    withdrawal_shortfall: 150,
    unmet_need: 400,
    withdrawal_excess: 0,
  };

  it("antes de la jubilación no pinta ninguna fila", () => {
    expect(buildWithdrawalTooltipRows({ ...point, month_index: 299 }, 300, 1)).toEqual([]);
    // Sin jubilación en el horizonte tampoco: no hay meses jubilados que describir.
    expect(buildWithdrawalTooltipRows(point, null, 1)).toEqual([]);
    expect(buildWithdrawalTooltipRows(point, undefined, 1)).toEqual([]);
  });

  it("«No financiado» va DESPUÉS de «Recorte» y no lo sustituye", () => {
    const rows = buildWithdrawalTooltipRows(point, 300, 1);
    expect(rows.map((r) => r.key)).toEqual(["withdrawal", "shortfall", "unmet"]);
    expect(rows.map((r) => r.label)).toEqual([
      "Retirada del mes",
      "Recorte",
      "No financiado",
    ]);
    expect(rows.find((r) => r.key === "unmet")!.amount).toBe(400);
  });

  it("un descubierto sin recorte se enseña igual (es la mitad que faltaba)", () => {
    const rows = buildWithdrawalTooltipRows(
      { month_index: 300, withdrawal: 1000, withdrawal_shortfall: 0, unmet_need: 900 },
      300,
      1,
    );
    expect(rows.map((r) => r.key)).toEqual(["withdrawal", "unmet"]);
  });

  it("un cero no se pinta: afirmaría que se midió un descubierto", () => {
    const rows = buildWithdrawalTooltipRows(
      { month_index: 300, withdrawal: 0, withdrawal_shortfall: 0, unmet_need: 0 },
      300,
      1,
    );
    // La retirada SÍ, aunque sea cero: ahí el cero es el dato (ese mes no vendiste nada).
    expect(rows.map((r) => r.key)).toEqual(["withdrawal"]);
  });

  it("un backend sin `unmet_need` no pinta la fila (nunca un 0 tranquilizador)", () => {
    const rows = buildWithdrawalTooltipRows(
      { month_index: 300, withdrawal: 1000, withdrawal_shortfall: 200 },
      300,
      1,
    );
    expect(rows.map((r) => r.key)).toEqual(["withdrawal", "shortfall"]);
  });

  it("las cuatro filas comparten el MISMO deflactor del patrimonio de arriba", () => {
    const f = deflationFactorAt(300, 2.5);
    const rows = buildWithdrawalTooltipRows(
      { ...point, withdrawal_excess: 90 },
      300,
      f,
    );
    expect(rows.map((r) => r.key)).toEqual([
      "withdrawal",
      "shortfall",
      "unmet",
      "excess",
    ]);
    for (const [key, nominal] of [
      ["withdrawal", 2000],
      ["shortfall", 150],
      ["unmet", 400],
      ["excess", 90],
    ] as const) {
      expect(rows.find((r) => r.key === key)!.amount).toBeCloseTo(nominal * f, 9);
    }
  });
});
