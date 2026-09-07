import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  NEEDED_CAPITAL_SERIES,
  buildWithdrawalTooltipRows,
  deflationFactorAt,
  formatYearsEsFromMonths,
  lastPointIndexAtOrBeforeMonth,
  neededCapitalAtRetirement,
  neededCurveForChart,
  projectionMaxXTicks,
  projectionXTicks,
  resolveDeflationAnnualPct,
  successStripForChart,
  thinTicksFromEnd,
  type NeededCapitalAtRetirementSeries,
  type NeededCurveSeries,
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

// ─────────────────────────────────────────────────────────────────────────────────────────────
describe("resolveDeflationAnnualPct", () => {
  it("la tasa de la respuesta manda cuando es un número finito", () => {
    expect(resolveDeflationAnnualPct("2.5", 9)).toBe(2.5);
    expect(resolveDeflationAnnualPct("0", 9)).toBe(0);
    expect(resolveDeflationAnnualPct("-1.25", 9)).toBe(-1.25);
  });

  it("sin campo (backend < 4.6.0) cae a la tasa de la instalación", () => {
    expect(resolveDeflationAnnualPct(undefined, 3.1)).toBe(3.1);
  });

  it("un valor no numérico cae a la tasa de la instalación en vez de propagar NaN", () => {
    expect(resolveDeflationAnnualPct("no-soy-un-numero", 3.1)).toBe(3.1);
  });

  // Fija el borde heredado del memo que esta función deduplica (`Number("") === 0`): la cadena
  // vacía se lee como «0 %», no como «campo ausente». No es lo que el backend emite, pero es lo
  // que el chart hacía y este extract NO cambia comportamiento.
  it("la cadena vacía se lee como 0 (borde heredado del memo del chart)", () => {
    expect(resolveDeflationAnnualPct("", 3.1)).toBe(0);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
/**
 * `neededCurveForChart` — la curva «Capital necesario» del chart (modelo v2, C4).
 *
 * Sustituye a la línea del objetivo FIRE, y hereda de ella la única propiedad que importaba: se
 * alinea con `points[]` por POSICIÓN y se deflacta por el `month_index` REAL de cada punto. Lo que
 * añade son dos guardas que la línea vieja no necesitaba, porque su array siempre estaba completo:
 * el ESTADO (`computing` ⇒ no hay curva) y los nodos `null` (el nivel 2 aún no los ha resuelto),
 * que se conservan como `null` y nunca como cero.
 *
 * **Supuesto declarado**: la curva viaja NOMINAL. El contrato de `api/types.ts` no lo dice con esas
 * palabras; lo que dice es que se compara con la línea de patrimonio, que es nominal —y que **NO
 * tiene por qué cruzarla en la fecha válida**, porque la fecha la deciden los caminos que aguantan y
 * no un cruce—. Si el servidor la publicara en euros de hoy, el toggle la deflactaría dos veces.
 */
describe("neededCurveForChart", () => {
  const pts = (months: number[]) =>
    months.map((m) => ({
      month_index: m,
      net_worth: 0,
      contributed_capital: 0,
    }));

  const series = (over: Partial<NeededCurveSeries> = {}): NeededCurveSeries => ({
    points: pts([0, 12, 24]),
    needed_capital_curve: [100, 200, 300],
    needed_capital_curve_state: "ready",
    ...over,
  });

  const identity = () => 1;

  it("deflacta cada nodo con el MES real del punto, no con su posición", () => {
    // `density=hybrid`: la posición 2 es el mes 24. Deflactar por la posición aplicaría el factor
    // de 2 meses a un importe de dos años vista.
    const got = neededCurveForChart(series(), (mi) => deflationFactorAt(mi, 3));
    expect(got).not.toBeNull();
    expect(got![0]).toBeCloseTo(100, 9);
    expect(got![1]).toBeCloseTo(200 * deflationFactorAt(12, 3), 9);
    expect(got![2]).toBeCloseTo(300 * deflationFactorAt(24, 3), 9);
  });

  it("un nodo sin resolver se conserva como null (jamás como 0)", () => {
    const got = neededCurveForChart(
      series({ needed_capital_curve: [100, null, 300] }),
      identity,
    );
    expect(got).toEqual([100, null, 300]);
  });

  it("estado `computing` o `unavailable` ⇒ no hay curva, aunque llegara un array", () => {
    for (const state of ["computing", "unavailable"] as const) {
      expect(
        neededCurveForChart(
          series({ needed_capital_curve_state: state }),
          identity,
        ),
      ).toBeNull();
    }
  });

  // Media curva alineada y media desplazada es peor que ninguna: nada en pantalla diría cuál de
  // las dos mitades es la buena.
  it("longitud distinta de `points[]` ⇒ se descarta ENTERA", () => {
    expect(
      neededCurveForChart(series({ needed_capital_curve: [100, 200] }), identity),
    ).toBeNull();
    expect(
      neededCurveForChart(
        series({ needed_capital_curve: [100, 200, 300, 400] }),
        identity,
      ),
    ).toBeNull();
  });

  it("curva ausente, nula o vacía, serie nula, o sin puntos ⇒ null", () => {
    expect(neededCurveForChart(series({ needed_capital_curve: null }), identity)).toBeNull();
    expect(
      neededCurveForChart(series({ needed_capital_curve: undefined }), identity),
    ).toBeNull();
    expect(
      neededCurveForChart(
        series({ points: [], needed_capital_curve: [] }),
        identity,
      ),
    ).toBeNull();
    expect(neededCurveForChart(null, identity)).toBeNull();
    expect(neededCurveForChart(undefined, identity)).toBeNull();
  });

  // Backend anterior al campo de estado: se juzga solo por el array, sin dar por hecho que falta.
  it("sin `needed_capital_curve_state` la curva vale si el array cuadra", () => {
    const got = neededCurveForChart(
      series({ needed_capital_curve_state: undefined }),
      identity,
    );
    expect(got).toEqual([100, 200, 300]);
  });

  it("un nodo no finito (NaN/Infinity) se trata como no resuelto", () => {
    expect(
      neededCurveForChart(
        series({ needed_capital_curve: [Number.NaN, Number.POSITIVE_INFINITY, 300] }),
        identity,
      ),
    ).toEqual([null, null, 300]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
/**
 * `successStripForChart` — la tira de éxito por año de jubilación que va bajo el eje X.
 *
 * Cada celda contesta una pregunta distinta de la que contesta la banda: no «qué le pasa a este
 * plan» sino «qué pasaría si me fuera ese año». Por eso descarta en vez de rellenar: una celda que
 * no se puede colorear no se pinta, porque un verde inventado ahí diría que irse ese año sale bien.
 */
describe("successStripForChart", () => {
  it("ordena por mes y conserva la fracción tal cual", () => {
    expect(
      successStripForChart({
        success_by_retirement_year: [
          { month_index: 24, success: 0.5 },
          { month_index: 12, success: 0.78 },
        ],
      }),
    ).toEqual([
      { monthIndex: 12, success: 0.78 },
      { monthIndex: 24, success: 0.5 },
    ]);
  });

  it("nivel 2 aún sin resolver (null/ausente) ⇒ tira vacía, no una tira a medias", () => {
    expect(successStripForChart({ success_by_retirement_year: null })).toEqual([]);
    expect(successStripForChart({ success_by_retirement_year: undefined })).toEqual([]);
    expect(successStripForChart(null)).toEqual([]);
    expect(successStripForChart(undefined)).toEqual([]);
  });

  // No se clampa: un 1,4 no es «éxito total», es un valor que este chart no sabe leer.
  it("descarta el éxito fuera de [0, 1] y el no finito, en vez de recortarlo", () => {
    expect(
      successStripForChart({
        success_by_retirement_year: [
          { month_index: 12, success: 1.4 },
          { month_index: 24, success: -0.1 },
          { month_index: 36, success: Number.NaN },
          { month_index: 48, success: 0 },
          { month_index: 60, success: 1 },
        ],
      }),
    ).toEqual([
      { monthIndex: 48, success: 0 },
      { monthIndex: 60, success: 1 },
    ]);
  });

  it("descarta el mes no finito y desduplica quedándose con la PRIMERA aparición", () => {
    expect(
      successStripForChart({
        success_by_retirement_year: [
          { month_index: Number.NaN, success: 0.9 },
          { month_index: 12, success: 0.7 },
          { month_index: 12, success: 0.3 },
        ],
      }),
    ).toEqual([{ monthIndex: 12, success: 0.7 }]);
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
/**
 * `neededCapitalAtRetirement` — la SEGUNDA línea del tile «Capital necesario hoy».
 *
 * La primera línea (`needed_capital_today`) va en euros de hoy por contrato y no pasa por aquí.
 * Esta sí sigue el toggle, y por eso **la base viaja pegada al importe**: «toggle activo con
 * inflación 0» y «toggle apagado» dan el MISMO número con bases distintas, y solo el campo
 * `basis` los separa (arqueología §2.26 — una base re-derivada de una segunda comparación acaba
 * discrepando del número que rotula).
 */
describe("neededCapitalAtRetirement", () => {
  const series = (
    over: Partial<NeededCapitalAtRetirementSeries> = {},
  ): NeededCapitalAtRetirementSeries => ({
    points: [0, 12, 24].map((m) => ({
      month_index: m,
      net_worth: 0,
      contributed_capital: 0,
    })),
    needed_capital_curve: [100_000, 200_000, 300_000],
    needed_capital_curve_state: "ready",
    safe_date_series_position: 2,
    deflation_annual_inflation_percent: "2.5",
    ...over,
  });

  it("toggle activo: el nodo de la fecha válida deflactado a su MES, base «today»", () => {
    const got = neededCapitalAtRetirement(series(), true, 0);
    expect(got.basis).toBe("today");
    expect(got.amount).toBeCloseTo(300_000 * deflationFactorAt(24, 2.5), 9);
  });

  it("toggle apagado: el nominal tal cual, base «nominal»", () => {
    const got = neededCapitalAtRetirement(series(), false, 0);
    expect(got.basis).toBe("nominal");
    expect(got.amount).toBeCloseTo(300_000, 9);
  });

  it("toggle activo con inflación 0: mismo número que apagado, y base «nominal»", () => {
    const got = neededCapitalAtRetirement(
      series({ deflation_annual_inflation_percent: "0" }),
      true,
      0,
    );
    expect(got.basis).toBe("nominal");
    expect(got.amount).toBeCloseTo(300_000, 9);
  });

  it("la tasa de la RESPUESTA manda sobre la de la instalación", () => {
    const got = neededCapitalAtRetirement(series(), true, 9);
    expect(got.amount).toBeCloseTo(300_000 * deflationFactorAt(24, 2.5), 9);
  });

  it("sin tasa en la respuesta cae a la de la instalación (backend < 4.6.0)", () => {
    const got = neededCapitalAtRetirement(
      series({ deflation_annual_inflation_percent: undefined }),
      true,
      3,
    );
    expect(got.amount).toBeCloseTo(300_000 * deflationFactorAt(24, 3), 9);
  });

  // Los tres huecos dan `null`, NUNCA 0: un «0 €» en esta línea diría que llegado el día no
  // necesitas nada.
  it("sin curva lista, sin fecha válida, o con ese nodo sin resolver ⇒ importe null", () => {
    expect(
      neededCapitalAtRetirement(
        series({ needed_capital_curve_state: "computing" }),
        true,
        0,
      ).amount,
    ).toBeNull();
    expect(
      neededCapitalAtRetirement(
        series({ safe_date_series_position: null }),
        true,
        0,
      ).amount,
    ).toBeNull();
    expect(
      neededCapitalAtRetirement(
        series({ needed_capital_curve: [100_000, 200_000, null] }),
        true,
        0,
      ).amount,
    ).toBeNull();
    expect(neededCapitalAtRetirement(null, true, 2.5).amount).toBeNull();
  });

  // Una posición fuera del array no es el último punto: es una respuesta que no cuadra, y
  // rotular el nodo equivocado sería peor que no rotular ninguno.
  it("una posición fuera de rango no se recorta al último nodo", () => {
    expect(
      neededCapitalAtRetirement(
        series({ safe_date_series_position: 9 }),
        true,
        0,
      ).amount,
    ).toBeNull();
  });

  // La base se declara aunque no haya importe: el tile decide con ella si escribe «(euros de
  // hoy)» y no puede quedarse sin respuesta por un nodo ausente.
  it("la base se publica aunque el importe sea null", () => {
    expect(
      neededCapitalAtRetirement(series({ safe_date_series_position: null }), true, 0)
        .basis,
    ).toBe("today");
  });
});

// ─────────────────────────────────────────────────────────────────────────────────────────────
/** La curva y su leyenda salen de UNA constante: dos definiciones del rótulo o del color es cómo
 *  una leyenda acaba nombrando una serie que ya no está. */
describe("NEEDED_CAPITAL_SERIES", () => {
  it("rótulo, token y guion, sin un solo hex", () => {
    expect(NEEDED_CAPITAL_SERIES.label).toBe("Capital necesario");
    expect(NEEDED_CAPITAL_SERIES.color).toBe("var(--proj-required)");
    expect(NEEDED_CAPITAL_SERIES.color.startsWith("var(--")).toBe(true);
    expect(NEEDED_CAPITAL_SERIES.dash).toBe("6 4");
  });
});
