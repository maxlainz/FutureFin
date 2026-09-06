/**
 * El degradado de la banda de riesgo fijado (5.0.0, V2/V5/P1 + modelo v2 C3).
 *
 * Lo que aquí puede romperse en silencio son tres cosas y ninguna se ve mirando la pantalla:
 *
 *  1. Que el color se reparta por POSICIÓN del array en vez de por mes — la rejilla del servidor
 *     salta de cinco en cinco años, así que el desplazamiento sería de décadas y el chart seguiría
 *     pareciendo un chart correcto.
 *  2. Que una probabilidad ausente se pinte de verde en vez de saltarse.
 *  3. Que la escala deje de salir del UMBRAL del perfil. Con umbral 100 los cortes tienen que ser
 *     **idénticos** a los constantes de antes del modelo v2 (0,05 / 0,10): un rediseño no puede
 *     cambiar el color de un plan que no ha cambiado. Y con umbral 80 tienen que ablandarse, o le
 *     estaríamos pintando de ruina a alguien el plan que él mismo eligió.
 */

import { describe, expect, it } from "vitest";
import type { FailureProbabilityPointApi } from "../api/types";
import {
  RISK_RED_FLOOR,
  failureKindsAtMonth,
  failureProbabilityAtMonth,
  riskColorForProbability,
  riskCutoffsForThreshold,
  riskGradientStops,
} from "./risk-gradient";

const pt = (
  month_index: number,
  probability: number | null,
  by_kind: [number, number, number] | null = null,
  age: number | null = null,
): FailureProbabilityPointApi => ({ month_index, age, probability, by_kind });

/** La escala del umbral por defecto (95) y la del más exigente (100): son la MISMA. */
const C95 = riskCutoffsForThreshold(95);
const C100 = riskCutoffsForThreshold(100);
/** La de quien acepta que dos de cada diez escenarios fallen. */
const C80 = riskCutoffsForThreshold(80);

describe("riskCutoffsForThreshold — la escala sale del umbral del perfil (C3)", () => {
  it("con umbral 100 los cortes son EXACTAMENTE los de antes del modelo v2", () => {
    // El pin que impide que el rediseño cambie de color un plan que no ha cambiado.
    expect(C100).toEqual({ amber: 0.05, red: 0.1 });
  });

  it("el suelo del rojo es el 10 %: por encima del 90 % de éxito el listón deja de moverse", () => {
    expect(RISK_RED_FLOOR).toBe(0.1);
    // 95 y 90 caerían en 0,05 y 0,10 sin el suelo; con él, los tres umbrales altos comparten escala.
    expect(C95).toEqual({ amber: 0.05, red: 0.1 });
    expect(riskCutoffsForThreshold(90)).toEqual({ amber: 0.05, red: 0.1 });
    expect(riskCutoffsForThreshold(99)).toEqual({ amber: 0.05, red: 0.1 });
  });

  it("por debajo del 90 % la escala se ablanda con el umbral", () => {
    expect(C80).toEqual({ amber: 0.1, red: 0.2 });
    const c85 = riskCutoffsForThreshold(85);
    expect(c85.red).toBeCloseTo(0.15, 10);
    expect(c85.amber).toBeCloseTo(0.075, 10);
  });

  it("el ámbar es SIEMPRE la mitad del rojo, sea cual sea el umbral", () => {
    for (const u of [80, 85, 90, 95, 100]) {
      const c = riskCutoffsForThreshold(u);
      expect(c.amber, `umbral ${u}`).toBeCloseTo(c.red / 2, 12);
    }
  });

  it("un umbral que no llegó NO ablanda la escala: se trata como el 100", () => {
    expect(riskCutoffsForThreshold(null)).toEqual(C100);
    expect(riskCutoffsForThreshold(undefined)).toEqual(C100);
    expect(riskCutoffsForThreshold(Number.NaN)).toEqual(C100);
  });

  it("un umbral fuera de rango se acota en vez de inventar una escala", () => {
    // Sin acotar, un −1000 pondría el rojo en el 1.100 % y nada sería rojo jamás.
    expect(riskCutoffsForThreshold(-1000)).toEqual({ amber: 0.5, red: 1 });
    expect(riskCutoffsForThreshold(500)).toEqual(C100);
  });
});

describe("riskColorForProbability — los tres peldaños, con la escala que se le pase", () => {
  it("los tres peldaños son tokens PUROS: la leyenda tiene que poder repetirlos", () => {
    expect(riskColorForProbability(0, C100)).toBe("var(--ff-pos)");
    expect(riskColorForProbability(C100.amber, C100)).toBe("var(--ff-warn)");
    expect(riskColorForProbability(C100.red, C100)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.5, C100)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(1, C100)).toBe("var(--ff-neg)");
  });

  it("entre 0 y el ámbar mezcla verde→ámbar de forma progresiva", () => {
    expect(riskColorForProbability(0.025, C100)).toBe(
      "color-mix(in oklch, var(--ff-warn) 50%, var(--ff-pos))",
    );
    expect(riskColorForProbability(0.01, C100)).toBe(
      "color-mix(in oklch, var(--ff-warn) 20%, var(--ff-pos))",
    );
  });

  it("entre ámbar y rojo mezcla ámbar→rojo", () => {
    expect(riskColorForProbability(0.075, C100)).toBe(
      "color-mix(in oklch, var(--ff-neg) 50%, var(--ff-warn))",
    );
    expect(riskColorForProbability(0.06, C100)).toBe(
      "color-mix(in oklch, var(--ff-neg) 20%, var(--ff-warn))",
    );
  });

  it("la MISMA probabilidad cambia de color con el umbral, que es el objetivo de C3", () => {
    // Uno de cada diez escenarios fallidos: ruina para quien pidió el 95 %, ámbar exacto para
    // quien pidió el 80 % — que es justo lo que aceptó al pedirlo.
    expect(riskColorForProbability(0.1, C95)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.1, C80)).toBe("var(--ff-warn)");
    // Y el 5 % pasa de ámbar puro a la mitad del tramo verde→ámbar.
    expect(riskColorForProbability(0.05, C95)).toBe("var(--ff-warn)");
    expect(riskColorForProbability(0.05, C80)).toBe(
      "color-mix(in oklch, var(--ff-warn) 50%, var(--ff-pos))",
    );
  });

  it("el rojo empieza exactamente donde el veredicto del servidor deja de cumplir el umbral", () => {
    expect(riskColorForProbability(0.0999, C95)).not.toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.1001, C95)).toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.1999, C80)).not.toBe("var(--ff-neg)");
    expect(riskColorForProbability(0.2001, C80)).toBe("var(--ff-neg)");
  });

  it("cero es cero: no hay mezcla al 0 %, hay verde", () => {
    expect(riskColorForProbability(0, C80)).not.toContain("color-mix");
  });
});

describe("failureProbabilityAtMonth — plana fuera, lineal dentro", () => {
  const points = [pt(240, 0), pt(300, 0.1), pt(360, 0.2)];

  it("antes de la primera muestra vale la primera, no una rampa inventada", () => {
    // La rejilla del servidor arranca en la jubilación: el tramo de acumulación no tiene muestra
    // propia, y suponerle una rampa sería dibujar un riesgo que nadie ha calculado.
    expect(failureProbabilityAtMonth(points, 0)).toBe(0);
    expect(failureProbabilityAtMonth(points, 239)).toBe(0);
    expect(failureProbabilityAtMonth(points, 240)).toBe(0);
  });

  it("interpola linealmente entre dos muestras", () => {
    expect(failureProbabilityAtMonth(points, 270)).toBeCloseTo(0.05, 10);
    expect(failureProbabilityAtMonth(points, 330)).toBeCloseTo(0.15, 10);
  });

  it("después de la última se queda plana", () => {
    expect(failureProbabilityAtMonth(points, 360)).toBe(0.2);
    expect(failureProbabilityAtMonth(points, 999)).toBe(0.2);
  });

  it("sin muestras es `null`, que NO es cero", () => {
    expect(failureProbabilityAtMonth([], 100)).toBeNull();
    expect(failureProbabilityAtMonth(null, 100)).toBeNull();
    expect(failureProbabilityAtMonth(undefined, 100)).toBeNull();
    expect(failureProbabilityAtMonth([pt(240, null)], 240)).toBeNull();
  });

  it("cuenta las TRES formas de fallo, no solo agotar la cartera", () => {
    // El campo es `failure_probability_by_age` y su `probability` ya suma F1+F2+F3: aquí no se
    // recompone nada desde `by_kind`. Un plan que falla al 100 % por tasa inicial (F2) en el mes
    // de jubilación tiene que salir rojo, no verde por no haber agotado nada.
    const soloF2 = [pt(240, 1, [0, 2500, 0]), pt(360, 1, [0, 2500, 0])];
    expect(failureProbabilityAtMonth(soloF2, 300)).toBe(1);
  });
});

describe("failureKindsAtMonth — el desglose del hover, sin interpolar", () => {
  const points = [
    pt(240, 0.02, [50, 0, 0]),
    pt(300, 0.1, [200, 30, 20]),
    pt(360, 0.2, [400, 60, 40]),
  ];

  it("devuelve el desglose de la muestra más cercana, tal cual", () => {
    expect(failureKindsAtMonth(points, 300)).toEqual([200, 30, 20]);
    expect(failureKindsAtMonth(points, 310)).toEqual([200, 30, 20]);
    expect(failureKindsAtMonth(points, 350)).toEqual([400, 60, 40]);
    expect(failureKindsAtMonth(points, 0)).toEqual([50, 0, 0]);
    expect(failureKindsAtMonth(points, 9999)).toEqual([400, 60, 40]);
  });

  it("NO interpola: el trío que enseña es de una muestra real", () => {
    // A mitad de camino entre [50,0,0] y [200,30,20], una interpolación daría [125,15,10] — un
    // desglose que no cuadraría con la acumulada que el tooltip enseña al lado.
    const mid = failureKindsAtMonth(points, 270);
    expect(mid).toEqual([50, 0, 0]);
    expect(mid).not.toEqual([125, 15, 10]);
  });

  it("en un empate gana la muestra ANTERIOR: las tres series son acumuladas", () => {
    // El mes 270 equidista de 240 y 300. La anterior no atribuye a este mes fallos que aún no han
    // ocurrido; la posterior sí lo haría.
    expect(failureKindsAtMonth(points, 270)).toEqual([50, 0, 0]);
  });

  it("sin desglose publicado es `null`, no un trío de ceros", () => {
    expect(failureKindsAtMonth([pt(240, 0.02), pt(300, 0.1)], 260)).toBeNull();
    expect(failureKindsAtMonth([], 100)).toBeNull();
    expect(failureKindsAtMonth(null, 100)).toBeNull();
    expect(failureKindsAtMonth(points, Number.NaN)).toBeNull();
  });

  it("salta las muestras sin desglose y usa la más cercana que sí lo tiene", () => {
    const conHueco = [pt(240, 0.02), pt(300, 0.1, [200, 30, 20])];
    expect(failureKindsAtMonth(conHueco, 245)).toEqual([200, 30, 20]);
  });
});

describe("riskGradientStops", () => {
  const points = [pt(240, 0), pt(300, 0.05), pt(360, 0.2)];
  const stopsOf = (over: Partial<Parameters<typeof riskGradientStops>[0]> = {}) =>
    riskGradientStops({ points, monthStart: 0, monthEnd: 480, cutoffs: C100, ...over });

  it("las paradas van ordenadas y dentro de [0, 1]", () => {
    const stops = stopsOf();
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
    const interiores = stopsOf().filter((s) => s.offset > 0 && s.offset < 1);
    expect(interiores.map((s) => s.offset)).toEqual([240 / 480, 300 / 480, 360 / 480]);
  });

  it("extiende PLANA a los dos lados con el valor de la muestra del extremo", () => {
    const stops = stopsOf();
    expect(stops[0]).toEqual({ offset: 0, color: "var(--ff-pos)" });
    // A la derecha, 0,20 ya es rojo puro y así se queda hasta el borde.
    expect(stops[stops.length - 1]).toEqual({ offset: 1, color: "var(--ff-neg)" });
  });

  it("la parada de una muestra usa SU probabilidad, con el color exacto de la escala", () => {
    const enLa300 = stopsOf().find((s) => s.offset === 300 / 480);
    expect(enLa300?.color).toBe("var(--ff-warn)");
  });

  it("el mismo sorteo se tiñe distinto según el umbral del perfil", () => {
    // 0,05 es ámbar puro al 95 % y apenas verde-tirando-a-ámbar al 80 %; 0,20 es rojo en las dos,
    // pero por razones distintas (en la de 80 es justo el corte).
    expect(stopsOf({ cutoffs: C95 }).find((s) => s.offset === 300 / 480)?.color).toBe(
      "var(--ff-warn)",
    );
    expect(stopsOf({ cutoffs: C80 }).find((s) => s.offset === 300 / 480)?.color).toBe(
      "color-mix(in oklch, var(--ff-warn) 50%, var(--ff-pos))",
    );
  });

  it("una muestra sin probabilidad se SALTA: no vale 0 y no pinta verde", () => {
    const conHueco = [pt(240, 0.2), pt(300, null), pt(360, 0.3)];
    const stops = stopsOf({ points: conHueco });
    expect(stops.map((s) => s.offset)).toEqual([0, 240 / 480, 360 / 480, 1]);
    for (const s of stops) expect(s.color).toBe("var(--ff-neg)");
  });

  it("las muestras fuera de la ventana no generan parada propia", () => {
    const stops = stopsOf({ monthStart: 280, monthEnd: 340 });
    expect(stops.map((s) => s.offset)).toEqual([0, (300 - 280) / 60, 1]);
  });

  it("menos de dos muestras utilizables → sin degradado (el chart vuelve al acento plano)", () => {
    expect(stopsOf({ points: [pt(240, 0.1)] })).toEqual([]);
    expect(stopsOf({ points: [] })).toEqual([]);
    expect(stopsOf({ points: null })).toEqual([]);
    expect(stopsOf({ points: [pt(240, null), pt(300, null)] })).toEqual([]);
  });

  it("una ventana degenerada o no finita → sin degradado", () => {
    expect(stopsOf({ monthStart: 100, monthEnd: 100 })).toEqual([]);
    expect(stopsOf({ monthStart: 200, monthEnd: 100 })).toEqual([]);
    expect(stopsOf({ monthStart: Number.NaN })).toEqual([]);
  });

  it("el color de una parada coincide con lo que el hover diría en ese mes", () => {
    // La invariante que impide que el tooltip y el tinte se contradigan: los dos salen de
    // `failureProbabilityAtMonth`.
    for (const m of [240, 270, 300, 330, 360]) {
      const p = failureProbabilityAtMonth(points, m)!;
      const stops = stopsOf({ monthStart: m, monthEnd: m + 12 });
      expect(stops[0]!.color, `mes ${m}`).toBe(riskColorForProbability(p, C100));
    }
  });
});
