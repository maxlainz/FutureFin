/**
 * Preparación de las líneas finas por miembro del chart del Hogar (D32).
 *
 * Lo que estos tests protegen, por orden de gravedad:
 *  1. **Que nada se calcule sobre posiciones de array.** Las series de miembro llegan decimadas
 *     igual que `points[]` (`density=hybrid`: la posición 13 es el mes 24). El valor de un mes se
 *     busca por MES, con la convención «último vértice servido en ese mes o antes» — la del API
 *     (`jubilacion_series_position`) —, no por índice.
 *  2. **Que el horizonte propio TERMINA la línea.** El agregado corre al horizonte común, así que
 *     un miembro con menos años declarados tiene serie de sobra; dibujarla entera enseñaría el
 *     patrimonio de una vida que esa persona no declaró.
 *  3. **Que el color empareja línea, tick de la tira y entrada de leyenda.** Si las tres no salen
 *     de `householdMemberColor`, la leyenda acaba nombrando la curva de otro — un chart del hogar
 *     que atribuye mal el dinero.
 *  4. Que la deflactación se aplica mes a mes con el `month_index` real (un factor único, o el
 *     factor de la posición, desvía la curva justo donde más se mira).
 */

import { describe, expect, it } from "vitest";
import type { HouseholdMemberProjectionApi } from "../api/types";
import { buildHouseholdMemberLines, memberValueAtMonth } from "./member-lines";
import { householdMemberColor } from "./chart-legend";
import { buildHouseholdMemberLegendItems, buildPhaseMarks } from "./phase-strip";
import { deflationFactorAt } from "./projection-chart";

const NOMINAL = () => 1;

/** Serie decimada como la sirve `density=hybrid`: mensual hasta el 12, luego anual. */
function hybridSeries(
  monthlyValueAt: (month: number) => number,
  lastMonth: number,
): { month_index: number; net_worth: number; net_worth_liquid: number }[] {
  const months: number[] = [];
  for (let m = 0; m <= Math.min(12, lastMonth); m++) months.push(m);
  for (let m = 24; m <= lastMonth; m += 12) months.push(m);
  if (months[months.length - 1] !== lastMonth) months.push(lastMonth);
  return months.map((m) => ({
    month_index: m,
    net_worth: monthlyValueAt(m),
    net_worth_liquid: monthlyValueAt(m) / 2,
  }));
}

function member(
  over: Partial<HouseholdMemberProjectionApi> & { user_id: string; username: string },
): HouseholdMemberProjectionApi {
  return {
    strategy: "asap",
    jubilacion_month_index: null,
    jubilacion_age: null,
    liquid_crossing_month_index: null,
    retirement_month_index: null,
    coast_fire_month_index: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    assets_depleted_month_index: null,
    warnings: [],
    ...over,
  } as HouseholdMemberProjectionApi;
}

const ANA = member({
  user_id: "u-ana",
  username: "Ana",
  horizon_months: 601,
  series: hybridSeries((m) => 100_000 + m * 1_000, 600),
});
const LUIS = member({
  user_id: "u-luis",
  username: "Luis",
  horizon_months: 601,
  series: hybridSeries((m) => 50_000 + m * 500, 600),
});

describe("buildHouseholdMemberLines", () => {
  it("sin miembros (vista «Yo») devuelve un array vacío, no una línea de ceros", () => {
    expect(buildHouseholdMemberLines(undefined, NOMINAL)).toEqual([]);
    expect(buildHouseholdMemberLines([], NOMINAL)).toEqual([]);
  });

  it("conserva los month_index servidos y los importes nominales tal cual", () => {
    const [ana] = buildHouseholdMemberLines([ANA], NOMINAL);
    expect(ana!.key).toBe("member-u-ana");
    expect(ana!.label).toBe("Ana");
    // Decimada: 0..12 mensual y de ahí anual — 13 + 49 vértices, no 601.
    expect(ana!.points.length).toBe(ANA.series!.length);
    expect(ana!.points.slice(0, 3).map((p) => [p.month_index, p.value])).toEqual([
      [0, 100_000],
      [1, 101_000],
      [2, 102_000],
    ]);
    // El vértice después del 12 es el mes 24, no el 13: la rejilla es la del servidor.
    expect(ana!.points[13]!.month_index).toBe(24);
  });

  it("deflacta cada vértice con SU mes, no con un factor único ni con su posición", () => {
    const pct = 3;
    const [ana] = buildHouseholdMemberLines([ANA], (m) => deflationFactorAt(m, pct));
    const p24 = ana!.points.find((p) => p.month_index === 24)!;
    expect(p24.value).toBeCloseTo(124_000 * deflationFactorAt(24, pct), 6);
    // El mes 0 nunca se mueve: el deflactor vale 1 ahí.
    expect(ana!.points[0]!.value).toBe(100_000);
    // Y la posición 13 NO se deflacta como el mes 13 (que es lo que haría un índice de array).
    expect(p24.value).not.toBeCloseTo(124_000 * deflationFactorAt(13, pct), 6);
  });

  it("recorta la línea al horizonte PROPIO del miembro (nunca extrapola)", () => {
    const corto = member({
      user_id: "u-corto",
      username: "Corto",
      // Vive declarados 361 meses: su último mes propio es el 360.
      horizon_months: 361,
      series: hybridSeries((m) => 1_000 + m, 600),
    });
    const [line] = buildHouseholdMemberLines([corto], NOMINAL);
    expect(line!.points[line!.points.length - 1]!.month_index).toBe(360);
    expect(line!.points.some((p) => p.month_index > 360)).toBe(false);
    // Y no inventa un vértice final en el horizonte del hogar.
    expect(line!.points.some((p) => p.month_index === 600)).toBe(false);
  });

  it("sin horizon_months (backend antiguo) dibuja la serie entera", () => {
    const sinHorizonte = member({
      user_id: "u-x",
      username: "X",
      series: hybridSeries((m) => m, 600),
    });
    const [line] = buildHouseholdMemberLines([sinHorizonte], NOMINAL);
    expect(line!.points[line!.points.length - 1]!.month_index).toBe(600);
  });

  it("un miembro sin serie servida da una línea vacía, no una línea en cero", () => {
    const sinSerie = member({ user_id: "u-y", username: "Y", horizon_months: 601 });
    const [line] = buildHouseholdMemberLines([sinSerie], NOMINAL);
    expect(line!.points).toEqual([]);
    expect(line!.label).toBe("Y");
  });

  it("descarta vértices con importe o mes no finito en vez de pintarlos como 0", () => {
    const sucio = member({
      user_id: "u-z",
      username: "Z",
      horizon_months: 601,
      series: [
        { month_index: 0, net_worth: 10, net_worth_liquid: 5 },
        {
          month_index: 1,
          net_worth: Number.NaN,
          net_worth_liquid: 0,
        },
        { month_index: 2, net_worth: 30, net_worth_liquid: 15 },
      ],
    });
    const [line] = buildHouseholdMemberLines([sucio], NOMINAL);
    expect(line!.points.map((p) => p.month_index)).toEqual([0, 2]);
  });

  it("ordena por mes aunque el servidor mandara los puntos desordenados", () => {
    const desordenado = member({
      user_id: "u-d",
      username: "D",
      horizon_months: 601,
      series: [
        { month_index: 24, net_worth: 3, net_worth_liquid: 1 },
        { month_index: 0, net_worth: 1, net_worth_liquid: 1 },
        { month_index: 12, net_worth: 2, net_worth_liquid: 1 },
      ],
    });
    const [line] = buildHouseholdMemberLines([desordenado], NOMINAL);
    expect(line!.points.map((p) => p.month_index)).toEqual([0, 12, 24]);
  });
});

describe("emparejamiento de color línea ↔ tick ↔ leyenda", () => {
  it("las tres superficies dan el MISMO color por posición en members[]", () => {
    const members = [
      { ...ANA, retirement_month_index: 200 },
      { ...LUIS, retirement_month_index: 260 },
    ];
    const lines = buildHouseholdMemberLines(members, NOMINAL);
    const legend = buildHouseholdMemberLegendItems(members);
    const marks = buildPhaseMarks({
      members,
      window: { startMonth: 0, endMonth: 600 },
    }).filter((m) => m.kind === "member");

    expect(lines.map((l) => l.color)).toEqual([
      householdMemberColor(0),
      householdMemberColor(1),
    ]);
    expect(legend.map((l) => l.color)).toEqual(lines.map((l) => l.color));
    // Las marcas se ordenan por mes; se emparejan por key, no por posición.
    for (const line of lines) {
      expect(marks.find((m) => m.key === line.key)!.color).toBe(line.color);
      expect(legend.find((l) => l.key === line.key)!.color).toBe(line.color);
    }
  });

  it("la leyenda del miembro dibuja una LÍNEA: es lo que hay pintado de él", () => {
    expect(buildHouseholdMemberLegendItems([ANA])[0]!.swatch).toBe("line");
  });

  it("los colores son tokens, nunca hex", () => {
    for (const line of buildHouseholdMemberLines([ANA, LUIS], NOMINAL)) {
      expect(line.color).toMatch(/^var\(--/);
    }
  });
});

describe("memberValueAtMonth", () => {
  const [ana] = buildHouseholdMemberLines([ANA], NOMINAL);

  it("en un mes con vértice propio devuelve su valor", () => {
    expect(memberValueAtMonth(ana!, 24)).toBe(124_000);
  });

  it("en un mes SIN vértice devuelve el último servido antes, no el siguiente ni un 0", () => {
    // Hybrid: hay puntos en 24 y 36. El mes 30 se lee con el del 24.
    expect(memberValueAtMonth(ana!, 30)).toBe(124_000);
    expect(memberValueAtMonth(ana!, 35)).toBe(124_000);
    expect(memberValueAtMonth(ana!, 36)).toBe(136_000);
  });

  it("antes del primer vértice (el tramo histórico del chart) devuelve null", () => {
    expect(memberValueAtMonth(ana!, -12)).toBeNull();
  });

  it("pasado el último vértice devuelve null: la línea no se extrapola", () => {
    const last = ana!.points[ana!.points.length - 1]!;
    expect(memberValueAtMonth(ana!, last.month_index)).toBe(last.value);
    expect(memberValueAtMonth(ana!, last.month_index + 1)).toBeNull();
  });

  it("una línea vacía devuelve null (ausencia, nunca cero)", () => {
    expect(memberValueAtMonth({ points: [] }, 12)).toBeNull();
  });

  it("un miembro cuya línea ya terminó desaparece del tooltip, no repite su último importe", () => {
    const corto = member({
      user_id: "u-corto",
      username: "Corto",
      horizon_months: 121,
      series: hybridSeries((m) => 1_000 + m, 600),
    });
    const [line] = buildHouseholdMemberLines([corto], NOMINAL);
    expect(memberValueAtMonth(line!, 120)).toBe(1_120);
    // El chart no lo pinta más allá del mes 120; el tooltip tampoco lo cuenta ahí.
    expect(memberValueAtMonth(line!, 480)).toBeNull();
  });
});
