import { describe, expect, it } from "vitest";
import type {
  HistorySeriesApi,
  ProjectionPointApi,
  ProjectionSeriesApi,
} from "../api/types";
import { mergeProjectionWithHistory } from "./history-merge";

const ANCHOR = "2026-07-06";

function makeSeries(
  overrides: Partial<ProjectionSeriesApi> = {},
): ProjectionSeriesApi {
  const points: ProjectionPointApi[] = [
    { month_index: 0, net_worth: 1005, contributed_capital: 0 },
    { month_index: 1, net_worth: 1100, contributed_capital: 50 },
  ];
  return {
    points,
    milestones: [],
    months: 1,
    horizon_years: 1,
    horizon_basis: "months_override",
    starting_net_worth: "1005",
    monthly_delta_assumption: "95",
    model_note: "test",
    anchor_date_ymd: ANCHOR,
    asset_series: [
      { asset_id: "A", asset_name: "Alpha", values: [600, 660] },
      { asset_id: "C", asset_name: "Cee", values: [400, 440] },
    ],
    ...overrides,
  };
}

/** Histórico con puntos deliberadamente desordenados (-1, -2, 0) para probar el orden. */
function makeHistory(
  overrides: Partial<HistorySeriesApi> = {},
): HistorySeriesApi {
  return {
    anchor_date_ymd: ANCHOR,
    anchor_month_first_ymd: "2026-07-01",
    view: "household",
    window_months: 1200,
    window_truncated: false,
    liabilities_snapshotted: true,
    points: [
      { month_index: -1, net_worth: 900, assets_total: 900, liabilities_total: 0 },
      { month_index: -2, net_worth: 800, assets_total: 800, liabilities_total: 0 },
      { month_index: 0, net_worth: 1000, assets_total: 1000, liabilities_total: 0 },
    ],
    // Paralelas a points, en el MISMO orden desordenado.
    asset_series: [
      { asset_id: "A", asset_name: "Alpha old", values: [500, 450, 600] },
      { asset_id: "B", asset_name: "Beta old", values: [300, 250, 0] },
    ],
    markers: [
      {
        date_ymd: "2026-05-15",
        month_index: -2,
        month_fraction: -1.5484,
        kind: "asset",
        source: "capture",
        owner_user_id: "u1",
        total: 800,
      },
    ],
    ...overrides,
  };
}

describe("mergeProjectionWithHistory — identidad por referencia", () => {
  it("history null → pts y assetSeries por referencia, sin marcadores", () => {
    const series = makeSeries();
    const out = mergeProjectionWithHistory(series, null);
    expect(out.pts).toBe(series.points);
    expect(out.assetSeries).toBe(series.asset_series);
    expect(out.markers).toEqual([]);
    expect(out.historyStartMonth).toBe(0);
    expect(out.futureOffset).toBe(0);
    expect(out.minNetWorth).toBe(1005);
  });

  it("history con points vacíos → identidad", () => {
    const series = makeSeries();
    const out = mergeProjectionWithHistory(
      series,
      makeHistory({ points: [], asset_series: [], markers: [] }),
    );
    expect(out.pts).toBe(series.points);
    expect(out.futureOffset).toBe(0);
  });

  it("anchor distinto → identidad (grids desalineadas)", () => {
    const series = makeSeries();
    const out = mergeProjectionWithHistory(
      series,
      makeHistory({ anchor_date_ymd: "2026-01-01" }),
    );
    expect(out.pts).toBe(series.points);
    expect(out.markers).toEqual([]);
  });

  it("series sin anchor_date_ymd → identidad aunque el histórico traiga datos", () => {
    const series = makeSeries({ anchor_date_ymd: undefined });
    const out = mergeProjectionWithHistory(series, makeHistory());
    expect(out.pts).toBe(series.points);
  });

  it("asset_series ausente en la serie → identidad devuelve [] (no undefined)", () => {
    const series = makeSeries({ asset_series: undefined });
    const out = mergeProjectionWithHistory(series, null);
    expect(out.assetSeries).toEqual([]);
  });

  it("today_only_history_is_identity — histórico solo-hoy (único punto k=0) → identidad por referencia", () => {
    const series = makeSeries();
    // history.points.length === 1 (pasa el guard superior), pero ningún punto k<0 sobrevive al
    // filtro → identidad: no hay tramo pasado que dibujar y una serie solo-histórico sería fantasma.
    const out = mergeProjectionWithHistory(
      series,
      makeHistory({
        points: [
          { month_index: 0, net_worth: 1000, assets_total: 1000, liabilities_total: 0 },
        ],
        asset_series: [{ asset_id: "B", asset_name: "Beta old", values: [1000] }],
        markers: [],
      }),
    );
    expect(out.pts).toBe(series.points);
    expect(out.assetSeries).toBe(series.asset_series);
    expect(out.markers).toEqual([]);
    expect(out.historyStartMonth).toBe(0);
    expect(out.futureOffset).toBe(0);
  });
});

describe("mergeProjectionWithHistory — fusión", () => {
  it("ordena el pasado ascendente y calcula futureOffset / historyStartMonth", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    expect(out.pts.map((p) => p.month_index)).toEqual([-2, -1, 0, 1]);
    expect(out.futureOffset).toBe(2);
    expect(out.historyStartMonth).toBe(-2);
  });

  it("descarta los puntos históricos con month_index >= 0 (el mes 0 lo aporta la proyección)", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    const zeroPts = out.pts.filter((p) => p.month_index === 0);
    expect(zeroPts).toHaveLength(1);
    // El histórico traía net_worth 1000 en el mes 0; la proyección trae 1005 → gana la proyección.
    expect(zeroPts[0].net_worth).toBe(1005);
  });

  it("los puntos históricos llevan contributed_capital = 0 (centinela)", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    expect(out.pts[0].contributed_capital).toBe(0);
    expect(out.pts[1].contributed_capital).toBe(0);
  });

  it("unión de series por asset_id con padding en ambas direcciones", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    const byId = new Map(out.assetSeries.map((s) => [s.asset_id, s]));

    // A: presente en pasado y futuro → nombre del futuro gana.
    const a = byId.get("A")!;
    expect(a.asset_name).toBe("Alpha");
    expect(a.values).toEqual([450, 500, 600, 660]);

    // C: solo futuro → pasado rellenado con 0.
    const c = byId.get("C")!;
    expect(c.values).toEqual([0, 0, 400, 440]);

    // B: solo histórico → futuro rellenado con 0, conserva su nombre.
    const b = byId.get("B")!;
    expect(b.asset_name).toBe("Beta old");
    expect(b.values).toEqual([250, 300, 0, 0]);

    // Toda serie fusionada es paralela a pts.
    for (const s of out.assetSeries) {
      expect(s.values).toHaveLength(out.pts.length);
    }
  });

  it("minNetWorth = mínimo net_worth sobre pts combinados", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    expect(out.minNetWorth).toBe(800);
  });

  it("pasa los marcadores tal cual", () => {
    const history = makeHistory();
    const out = mergeProjectionWithHistory(makeSeries(), history);
    expect(out.markers).toBe(history.markers);
  });

  it("con net_worth publicado, pastIsAssetsOnly = false", () => {
    const out = mergeProjectionWithHistory(makeSeries(), makeHistory());
    expect(out.pastIsAssetsOnly).toBe(false);
  });
});

/**
 * Sin el pasivo fotografiado entero el servidor manda `net_worth: null` en toda la serie
 * (`liabilities_snapshotted: false`). El chart no puede quedarse mudo: pinta `assets_total` y
 * marca `pastIsAssetsOnly` para que quien lo dibuje lo etiquete como activos, no como patrimonio.
 */
describe("mergeProjectionWithHistory — histórico sin patrimonio neto", () => {
  const historyWithoutNetWorth = () =>
    makeHistory({
      liabilities_snapshotted: false,
      points: [
        {
          month_index: -1,
          net_worth: null,
          assets_total: 900,
          liabilities_total: 0,
        },
        {
          month_index: -2,
          net_worth: null,
          assets_total: 800,
          liabilities_total: 0,
        },
        {
          month_index: 0,
          net_worth: null,
          assets_total: 1000,
          liabilities_total: 0,
        },
      ],
    });

  it("cae a assets_total: la curva del pasado no desaparece ni se rompe", () => {
    const out = mergeProjectionWithHistory(
      makeSeries(),
      historyWithoutNetWorth(),
    );
    expect(out.pts.map((p) => p.month_index)).toEqual([-2, -1, 0, 1]);
    // Mismos valores que traía assets_total, en el mismo orden ascendente.
    expect(out.pts.slice(0, 2).map((p) => p.net_worth)).toEqual([800, 900]);
    // Nada de NaN/null colándose al eje: minNetWorth sigue siendo un número usable.
    expect(out.minNetWorth).toBe(800);
  });

  it("marca pastIsAssetsOnly para que el chart lo etiquete como activos", () => {
    const out = mergeProjectionWithHistory(
      makeSeries(),
      historyWithoutNetWorth(),
    );
    expect(out.pastIsAssetsOnly).toBe(true);
  });

  it("sin histórico fusionado, pastIsAssetsOnly = false (no hay tramo pasado que etiquetar)", () => {
    // Identidad por `history: null` y por histórico solo-hoy: en ninguno de los dos casos se
    // dibuja pasado, así que la etiqueta no aplica aunque el flag del servidor sea false.
    expect(mergeProjectionWithHistory(makeSeries(), null).pastIsAssetsOnly).toBe(
      false,
    );
    const onlyToday = makeHistory({
      liabilities_snapshotted: false,
      points: [
        {
          month_index: 0,
          net_worth: null,
          assets_total: 1000,
          liabilities_total: 0,
        },
      ],
      asset_series: [{ asset_id: "B", asset_name: "Beta old", values: [1000] }],
      markers: [],
    });
    expect(
      mergeProjectionWithHistory(makeSeries(), onlyToday).pastIsAssetsOnly,
    ).toBe(false);
  });
});
