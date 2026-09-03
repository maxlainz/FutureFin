/**
 * Geometría de la tira de fases (D29).
 *
 * Lo que estos tests protegen, por orden de gravedad:
 *  1. **Que nada se calcule sobre posiciones de array.** Con `density=hybrid` el servidor decima
 *     la serie y la posición 13 es el mes 24: un corte de fase colocado por índice caería años
 *     fuera de sitio. La forma comprobable de la afirmación es la invariancia: mismas
 *     transiciones ⇒ mismos segmentos, tenga la serie 841 puntos o 42.
 *  2. La contigüidad de los tramos (un tramo acaba el mes anterior al siguiente) y el recorte a
 *     la ventana visible, que es lo que el chart traduce a píxeles.
 *  3. La regla del «Cruce»: solo cuando la edad manda y cae en un mes distinto del de la
 *     jubilación efectiva. Es la que evita rotular dos veces el mismo instante.
 */

import { describe, expect, it } from "vitest";
import type { HouseholdMemberProjectionApi, PhaseTransitionApi } from "../api/types";
import { buildPhaseMarks, buildPhaseSegments, phaseAtMonth } from "./phase-strip";

const FULL: PhaseTransitionApi[] = [
  { phase: "accumulating", month_index: 0 },
  { phase: "partial", month_index: 120 },
  { phase: "retired", month_index: 235 },
];

describe("buildPhaseSegments", () => {
  it("parte el horizonte en tramos contiguos que acaban un mes antes del siguiente", () => {
    const segs = buildPhaseSegments(FULL, { startMonth: 0, endMonth: 360 });
    expect(segs.map((s) => [s.phase, s.startMonth, s.endMonth])).toEqual([
      ["accumulating", 0, 119],
      ["partial", 120, 234],
      ["retired", 235, 360],
    ]);
    expect(segs.map((s) => s.label)).toEqual([
      "Trabajo",
      "Media jornada",
      "Jubilado",
    ]);
    expect(segs.map((s) => s.shortLabel)).toEqual(["Trab.", "½ jorn.", "Jub."]);
  });

  it("da EXACTAMENTE los mismos tramos con la serie mensual y con la decimada (hybrid)", () => {
    // El punto entero de la función: no recibe `points`, así que la densidad no puede moverla.
    // Se construyen las dos rejillas de verdad para que el test falle el día que alguien meta
    // aritmética de posiciones aquí dentro.
    const monthly = Array.from({ length: 361 }, (_, i) => ({ month_index: i }));
    const hybrid = [
      ...Array.from({ length: 13 }, (_, i) => ({ month_index: i })),
      ...Array.from({ length: 29 }, (_, i) => ({ month_index: (i + 2) * 12 })),
      { month_index: 360 },
    ];
    expect(hybrid.length).toBeLessThan(monthly.length / 5);

    const windowFrom = (pts: { month_index: number }[]) => ({
      startMonth: pts[0]!.month_index,
      endMonth: pts[pts.length - 1]!.month_index,
    });
    expect(buildPhaseSegments(FULL, windowFrom(hybrid))).toEqual(
      buildPhaseSegments(FULL, windowFrom(monthly)),
    );
  });

  it("mantiene el mes exacto de una transición que NO tiene punto servido en hybrid", () => {
    // El mes 235 no es múltiplo de 12: en `hybrid` no existe como punto. El tramo tiene que
    // seguir empezando en 235, no en el punto anual más cercano (228 o 240).
    const segs = buildPhaseSegments(FULL, { startMonth: 0, endMonth: 360 });
    const retired = segs.find((s) => s.phase === "retired")!;
    expect(retired.startMonth).toBe(235);
    expect(retired.transitionMonth).toBe(235);
  });

  it("recorta a la ventana visible y conserva el mes real de la transición", () => {
    const segs = buildPhaseSegments(FULL, { startMonth: 200, endMonth: 300 });
    expect(segs.map((s) => [s.phase, s.startMonth, s.endMonth])).toEqual([
      ["partial", 200, 234],
      ["retired", 235, 300],
    ]);
    // El tramo visible empieza en 200, pero la fase empezó en 120: el borde rotulable no miente.
    expect(segs[0]!.transitionMonth).toBe(120);
  });

  it("deja fuera los tramos que caen enteros más allá de la ventana", () => {
    const segs = buildPhaseSegments(FULL, { startMonth: 0, endMonth: 60 });
    expect(segs.map((s) => s.phase)).toEqual(["accumulating"]);
    expect(segs[0]!.endMonth).toBe(60);
  });

  it("con ventana que empieza en el pasado (histórico), el tramo arranca en su mes real", () => {
    const segs = buildPhaseSegments(FULL, { startMonth: -60, endMonth: 130 });
    expect(segs[0]!.startMonth).toBe(0);
    expect(segs.map((s) => s.phase)).toEqual(["accumulating", "partial"]);
  });

  it("sin transiciones (vista Hogar) no hay tira", () => {
    expect(buildPhaseSegments([], { startMonth: 0, endMonth: 360 })).toEqual([]);
    expect(buildPhaseSegments(undefined, { startMonth: 0, endMonth: 360 })).toEqual([]);
    expect(buildPhaseSegments(null, { startMonth: 0, endMonth: 360 })).toEqual([]);
  });

  it("es robusta ante una respuesta desordenada, repetida o con literales desconocidos", () => {
    const messy = [
      { phase: "retired", month_index: 235 },
      { phase: "accumulating", month_index: 0 },
      { phase: "sabbatical", month_index: 50 },
      { phase: "accumulating", month_index: 30 },
      { phase: "partial", month_index: 120 },
    ] as unknown as PhaseTransitionApi[];
    expect(
      buildPhaseSegments(messy, { startMonth: 0, endMonth: 360 }).map((s) => [
        s.phase,
        s.startMonth,
      ]),
    ).toEqual([
      ["accumulating", 0],
      ["partial", 120],
      ["retired", 235],
    ]);
  });

  it("dos fases el mismo mes: gana la más avanzada y no queda un tramo de cero meses", () => {
    const same: PhaseTransitionApi[] = [
      { phase: "accumulating", month_index: 0 },
      { phase: "partial", month_index: 200 },
      { phase: "retired", month_index: 200 },
    ];
    const segs = buildPhaseSegments(same, { startMonth: 0, endMonth: 300 });
    expect(segs.map((s) => [s.phase, s.startMonth, s.endMonth])).toEqual([
      ["accumulating", 0, 199],
      ["retired", 200, 300],
    ]);
  });

  it("ventana invertida o no finita ⇒ nada que pintar (nunca un rect de ancho negativo)", () => {
    expect(buildPhaseSegments(FULL, { startMonth: 100, endMonth: 10 })).toEqual([]);
    expect(
      buildPhaseSegments(FULL, { startMonth: 0, endMonth: Number.NaN }),
    ).toEqual([]);
  });
});

describe("phaseAtMonth", () => {
  const segs = buildPhaseSegments(FULL, { startMonth: 0, endMonth: 360 });
  it("resuelve la fase de un mes cualquiera, con o sin punto servido", () => {
    expect(phaseAtMonth(segs, 0)).toBe("accumulating");
    expect(phaseAtMonth(segs, 119)).toBe("accumulating");
    expect(phaseAtMonth(segs, 120)).toBe("partial");
    expect(phaseAtMonth(segs, 234)).toBe("partial");
    expect(phaseAtMonth(segs, 235)).toBe("retired");
    expect(phaseAtMonth(segs, 271)).toBe("retired");
  });
  it("fuera de los tramos devuelve null, no la primera fase", () => {
    expect(phaseAtMonth(segs, -12)).toBeNull();
    expect(phaseAtMonth(segs, 999)).toBeNull();
  });
});

describe("buildPhaseMarks", () => {
  const win = { startMonth: 0, endMonth: 360 };

  it("con `asap` el cruce NO se rotula aparte: es el mismo instante que la jubilación", () => {
    const marks = buildPhaseMarks({
      retirementTrigger: "liquid_crossing",
      liquidCrossingMonthIndex: 235,
      retirementMonthIndex: 235,
      window: win,
    });
    expect(marks).toEqual([]);
  });

  it("con la edad al mando y cruce en OTRO mes, se rotula «Cruce»", () => {
    const marks = buildPhaseMarks({
      retirementTrigger: "target_age",
      liquidCrossingMonthIndex: 280,
      retirementMonthIndex: 235,
      window: win,
    });
    expect(marks.map((m) => [m.kind, m.month, m.label])).toEqual([
      ["crossing", 280, "Cruce"],
    ]);
  });

  it("con la edad al mando pero cruce en el MISMO mes, tampoco se rotula", () => {
    expect(
      buildPhaseMarks({
        retirementTrigger: "target_age",
        liquidCrossingMonthIndex: 235,
        retirementMonthIndex: 235,
        window: win,
      }),
    ).toEqual([]);
  });

  it("un cruce ANTERIOR a la jubilación también es una lectura válida", () => {
    const marks = buildPhaseMarks({
      retirementTrigger: "target_age",
      liquidCrossingMonthIndex: 180,
      retirementMonthIndex: 235,
      window: win,
    });
    expect(marks.map((m) => m.month)).toEqual([180]);
  });

  it("la pensión es una flecha propia y va ordenada por mes con el resto", () => {
    const marks = buildPhaseMarks({
      pensionStartMonthIndex: 300,
      retirementTrigger: "target_age",
      liquidCrossingMonthIndex: 280,
      retirementMonthIndex: 235,
      window: win,
    });
    expect(marks.map((m) => [m.kind, m.month])).toEqual([
      ["crossing", 280],
      ["pension", 300],
    ]);
  });

  it("descarta lo que cae fuera de la ventana visible (zoom)", () => {
    const marks = buildPhaseMarks({
      pensionStartMonthIndex: 300,
      retirementTrigger: "target_age",
      liquidCrossingMonthIndex: 280,
      retirementMonthIndex: 235,
      window: { startMonth: 0, endMonth: 290 },
    });
    expect(marks.map((m) => m.kind)).toEqual(["crossing"]);
  });

  it("en Hogar, un tick por miembro con su nombre y colores distintos", () => {
    const members = [
      { user_id: "u1", username: "Ana", retirement_month_index: 200 },
      { user_id: "u2", username: "Luis", retirement_month_index: 260 },
      { user_id: "u3", username: "Sin plan", retirement_month_index: null },
    ] as unknown as HouseholdMemberProjectionApi[];
    const marks = buildPhaseMarks({ members, window: win });
    expect(marks.map((m) => [m.kind, m.month, m.label])).toEqual([
      ["member", 200, "Ana"],
      ["member", 260, "Luis"],
    ]);
    expect(marks[0]!.color).not.toBe(marks[1]!.color);
    expect(marks.every((m) => m.color.startsWith("var(--"))).toBe(true);
  });
});
