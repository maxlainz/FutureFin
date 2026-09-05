/**
 * El degradado de la banda de riesgo fijado (5.0.0, V2/V5/P1).
 *
 * Lo que aquí puede romperse en silencio son dos cosas y ninguna se ve mirando la pantalla: que
 * el color se reparta por POSICIÓN del array en vez de por mes (la rejilla del servidor salta de
 * cinco en cinco años, así que el desplazamiento sería de décadas y el chart seguiría pareciendo
 * un chart correcto) y que una probabilidad ausente se pinte de verde en vez de saltarse. Los dos
 * casos tienen su test aquí abajo.
 */

import { describe, expect, it } from "vitest";
import type { DepletionProbabilityPointApi } from "../api/types";
import {
  RISK_AMBER_AT,
  RISK_RED_AT,
  depletionProbabilityAtMonth,
  riskColorForProbability,
  riskGradientStops,
} from "./risk-gradient";

const pt = (
  month_index: number,
  probability: string | null,
  age: number | null = null,
): DepletionProbabilityPointApi => ({ month_index, age, probability });

describe("riskColorForProbability — los cortes ABSOLUTOS de P1", () => {
  it("los tres peldaños son tokens PUROS: la leyenda tiene que poder repetirlos", () => {
    expect(riskColorForProbability(0)).toBe("var(--ff-pos)");
    expect(riskColorForProbability(RISK_AMBER_AT)).toBe("var(--ff-warn)");
    expect(riskColorForProbability(RISK_RED_AT)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.5)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(1)).toBe("var(--ff-neg)");
  });

  it("entre 0 y el 5 % mezcla verde→ámbar de forma progresiva", () => {
    expect(riskColorForProbability(0.025)).toBe(
      "color-mix(in oklch, var(--ff-warn) 50%, var(--ff-pos))",
    );
    expect(riskColorForProbability(0.01)).toBe(
      "color-mix(in oklch, var(--ff-warn) 20%, var(--ff-pos))",
    );
  });

  it("entre el 5 % y el 10 % mezcla ámbar→rojo", () => {
    expect(riskColorForProbability(0.075)).toBe(
      "color-mix(in oklch, var(--ff-neg) 50%, var(--ff-warn))",
    );
    expect(riskColorForProbability(0.06)).toBe(
      "color-mix(in oklch, var(--ff-neg) 20%, var(--ff-warn))",
    );
  });

  it("el rojo empieza exactamente donde el veredicto del servidor: éxito < 90 %", () => {
    expect(RISK_RED_AT).toBe(0.1);
    expect(riskColorForProbability(0.0999)).not.toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.1001)).toBe("var(--ff-neg)");
  });

  it("cero es cero: no hay mezcla al 0 %, hay verde", () => {
    expect(riskColorForProbability(0)).not.toContain("color-mix");
  });
});

describe("depletionProbabilityAtMonth — plana fuera, lineal dentro", () => {
  const points = [pt(240, "0.0000"), pt(300, "0.1000"), pt(360, "0.2000")];

  it("antes de la primera muestra vale la primera, no una rampa inventada", () => {
    // La rejilla del servidor arranca en la jubilación: el tramo de acumulación no tiene muestra
    // propia, y suponerle una rampa sería dibujar un riesgo que nadie ha calculado.
    expect(depletionProbabilityAtMonth(points, 0)).toBe(0);
    expect(depletionProbabilityAtMonth(points, 239)).toBe(0);
    expect(depletionProbabilityAtMonth(points, 240)).toBe(0);
  });

  it("interpola linealmente entre dos muestras", () => {
    expect(depletionProbabilityAtMonth(points, 270)).toBeCloseTo(0.05, 10);
    expect(depletionProbabilityAtMonth(points, 330)).toBeCloseTo(0.15, 10);
  });

  it("después de la última se queda plana", () => {
    expect(depletionProbabilityAtMonth(points, 360)).toBe(0.2);
    expect(depletionProbabilityAtMonth(points, 999)).toBe(0.2);
  });

  it("sin muestras es `null`, que NO es cero", () => {
    expect(depletionProbabilityAtMonth([], 100)).toBeNull();
    expect(depletionProbabilityAtMonth(null, 100)).toBeNull();
    expect(depletionProbabilityAtMonth(undefined, 100)).toBeNull();
    expect(depletionProbabilityAtMonth([pt(240, null)], 240)).toBeNull();
  });
});

describe("riskGradientStops", () => {
  const points = [pt(240, "0.0000"), pt(300, "0.0500"), pt(360, "0.2000")];

  it("las paradas van ordenadas y dentro de [0, 1]", () => {
    const stops = riskGradientStops({ points, monthStart: 0, monthEnd: 480 });
    expect(stops.length).toBeGreaterThanOrEqual(2);
    for (const s of stops) {
      expect(s.offset).toBeGreaterThanOrEqual(0);
      expect(s.offset).toBeLessThanOrEqual(1);
    }
    expect(stops.map((s) => s.offset)).toEqual(
      [...stops.map((s) => s.offset)].sort((a, b) => a - b),
    );
  });

  it("mapea por MES: una rejilla que salta de 60 en 60 no se reparte a intervalos iguales", () => {
    // Si alguien indexara por POSICIÓN, las tres muestras caerían en 0 · 0,5 · 1 y este test se
    // pondría rojo. Con meses, la de 300 cae en (300−0)/480 = 0,625.
    const stops = riskGradientStops({ points, monthStart: 0, monthEnd: 480 });
    const interiores = stops.filter((s) => s.offset > 0 && s.offset < 1);
    expect(interiores.map((s) => s.offset)).toEqual([240 / 480, 300 / 480, 360 / 480]);
  });

  it("extiende PLANA a los dos lados con el valor de la muestra del extremo", () => {
    const stops = riskGradientStops({ points, monthStart: 0, monthEnd: 480 });
    expect(stops[0]).toEqual({ offset: 0, color: "var(--ff-pos)" });
    // A la derecha, 0,20 ya es rojo puro y así se queda hasta el borde.
    expect(stops[stops.length - 1]).toEqual({ offset: 1, color: "var(--ff-neg)" });
  });

  it("la parada de una muestra usa SU probabilidad, con el color exacto de la escala", () => {
    const stops = riskGradientStops({ points, monthStart: 0, monthEnd: 480 });
    const enLa300 = stops.find((s) => s.offset === 300 / 480);
    expect(enLa300?.color).toBe("var(--ff-warn)");
  });

  it("una muestra sin probabilidad se SALTA: no vale 0 y no pinta verde", () => {
    const conHueco = [pt(240, "0.2000"), pt(300, null), pt(360, "0.3000")];
    const stops = riskGradientStops({ points: conHueco, monthStart: 0, monthEnd: 480 });
    expect(stops.map((s) => s.offset)).toEqual([0, 240 / 480, 360 / 480, 1]);
    for (const s of stops) expect(s.color).toBe("var(--ff-neg)");
  });

  it("las muestras fuera de la ventana no generan parada propia", () => {
    const stops = riskGradientStops({ points, monthStart: 280, monthEnd: 340 });
    expect(stops.map((s) => s.offset)).toEqual([0, (300 - 280) / 60, 1]);
  });

  it("menos de dos muestras utilizables → sin degradado (el chart vuelve al acento plano)", () => {
    expect(riskGradientStops({ points: [pt(240, "0.10")], monthStart: 0, monthEnd: 480 })).toEqual(
      [],
    );
    expect(riskGradientStops({ points: [], monthStart: 0, monthEnd: 480 })).toEqual([]);
    expect(riskGradientStops({ points: null, monthStart: 0, monthEnd: 480 })).toEqual([]);
    expect(
      riskGradientStops({ points: [pt(240, null), pt(300, null)], monthStart: 0, monthEnd: 480 }),
    ).toEqual([]);
  });

  it("una ventana degenerada o no finita → sin degradado", () => {
    expect(riskGradientStops({ points, monthStart: 100, monthEnd: 100 })).toEqual([]);
    expect(riskGradientStops({ points, monthStart: 200, monthEnd: 100 })).toEqual([]);
    expect(riskGradientStops({ points, monthStart: Number.NaN, monthEnd: 480 })).toEqual([]);
  });

  it("el color de una parada coincide con lo que el hover diría en ese mes", () => {
    // La invariante que impide que el tooltip y el tinte se contradigan: los dos salen de
    // `depletionProbabilityAtMonth`.
    for (const m of [240, 270, 300, 330, 360]) {
      const p = depletionProbabilityAtMonth(points, m)!;
      const stops = riskGradientStops({ points, monthStart: m, monthEnd: m + 12 });
      expect(stops[0]!.color, `mes ${m}`).toBe(riskColorForProbability(p));
    }
  });
});
