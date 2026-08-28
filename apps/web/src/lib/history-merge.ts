/**
 * Fusión pura de la serie de proyección (futuro, `month_index ≥ 0`) con la serie histórica
 * (pasado, `month_index ≤ 0`) para dibujarlas en un único chart temporal.
 *
 * Cero React, cero fetch. El chart delega aquí toda la aritmética de empalme para que el caso
 * "sin histórico" sea **byte-idéntico** al render actual (identidad por referencia).
 */

import type {
  AssetSeriesApi,
  HistoryMarkerApi,
  HistorySeriesApi,
  ProjectionPointApi,
  ProjectionSeriesApi,
} from "../api/types";

export type MergedProjectionHistory = {
  /** Puntos fusionados: `[...pasado (k<0, asc), ...proyección (k≥0)]`. */
  pts: ProjectionPointApi[];
  /** `month_index` del primer punto (≤ 0). `0` cuando no hay histórico. */
  historyStartMonth: number;
  /** Nº de puntos históricos antepuestos = índice en `pts` donde empieza el mes 0.
   *  Sirve para re-mapear series paralelas a la proyección (p.ej. `fire_target_series`). */
  futureOffset: number;
  /** Series por activo, unión pasado∪futuro (padding 0 donde una serie no existe). */
  assetSeries: AssetSeriesApi[];
  /** Marcadores de snapshot (pass-through del histórico). */
  markers: HistoryMarkerApi[];
  /** Mínimo `net_worth` sobre `pts` (para decidir eje negativo). */
  minNetWorth: number;
  /**
   * `true` ⇒ el tramo PASADO de `pts` no es patrimonio neto, son **activos**: el servidor no
   * publicó `net_worth` porque el pasivo del scope no está fotografiado entero, y aquí se sustituye
   * por `assets_total` para no dejar el chart mudo. El futuro sigue siendo patrimonio neto (sale de
   * la proyección, que sí conoce la deuda viva), así que las dos mitades miden cosas distintas y
   * quien pinte esto **tiene que decirlo** — de lo contrario reproduce en píxeles el mismo error
   * que el `null` del servidor acaba de cerrar.
   *
   * `false` cuando no hay histórico fusionado (no hay tramo pasado que etiquetar).
   */
  pastIsAssetsOnly: boolean;
};

function minNetWorthOf(pts: ProjectionPointApi[]): number {
  let min = Infinity;
  for (const p of pts) {
    if (p.net_worth < min) min = p.net_worth;
  }
  return Number.isFinite(min) ? min : 0;
}

/**
 * Une proyección + histórico. Devuelve la **identidad** (referencias intactas → render idéntico
 * al actual) cuando no hay histórico usable: `history` es `null`, sin puntos, o su
 * `anchor_date_ymd` no coincide con el de la proyección (grids desalineadas → no se puede empalmar).
 *
 * En el caso fusionado: descarta puntos históricos con `month_index ≥ 0` (el vértice mes-0 lo
 * aporta la proyección), **cae a `assets_total` cuando el servidor no publica `net_worth`**
 * (marcándolo en `pastIsAssetsOnly`), ordena el pasado ascendente, y construye la unión de series por
 * `asset_id` rellenando con 0 las posiciones donde una serie no está presente. El nombre lo gana
 * la serie de futuro (activo vivo) en caso de colisión.
 */
export function mergeProjectionWithHistory(
  series: ProjectionSeriesApi,
  history: HistorySeriesApi | null,
): MergedProjectionHistory {
  // Resultado identidad (referencias intactas → render byte-idéntico al solo-futuro). Se devuelve
  // tanto cuando no hay histórico usable como cuando, tras filtrar, no sobrevive ningún punto PASADO.
  const identityResult = (): MergedProjectionHistory => ({
    pts: series.points,
    historyStartMonth: 0,
    futureOffset: 0,
    assetSeries: series.asset_series ?? [],
    markers: [],
    minNetWorth: minNetWorthOf(series.points),
    pastIsAssetsOnly: false,
  });

  const seriesAnchor = series.anchor_date_ymd;
  const identity =
    history == null ||
    history.points.length === 0 ||
    seriesAnchor == null ||
    history.anchor_date_ymd !== seriesAnchor;

  if (identity) {
    return identityResult();
  }

  // Índices de los puntos históricos a conservar (k < 0), ordenados ascendente por month_index.
  // Se mantiene el orden original para poder rebanar las series por activo (paralelas a points).
  const keptIdx: number[] = [];
  for (let i = 0; i < history.points.length; i++) {
    if (history.points[i].month_index < 0) keptIdx.push(i);
  }
  // Sin puntos PASADOS supervivientes (p.ej. histórico solo-hoy: un único punto k=0, que la
  // proyección ya aporta) → identidad. Empalmar aquí no dibujaría tramo pasado alguno y sí crearía
  // entradas de leyenda «solo-histórico» a cero (series fantasma). El corto-circuito clave sobre
  // los puntos supervivientes, no sobre `history.points.length` en bruto.
  if (keptIdx.length === 0) {
    return identityResult();
  }
  keptIdx.sort(
    (a, b) => history.points[a].month_index - history.points[b].month_index,
  );

  // Sin el pasivo fotografiado entero el servidor manda `net_worth: null` (no puede decir cuál es
  // el patrimonio neto). Se pinta `assets_total`, que sí es un dato real: la curva no desaparece ni
  // se rompe. Lo que NO puede pasar es que se llame «patrimonio neto» — de ahí `pastIsAssetsOnly`.
  const pastIsAssetsOnly = !history.liabilities_snapshotted;

  const histPts: ProjectionPointApi[] = keptIdx.map((i) => {
    const hp = history.points[i];
    return {
      month_index: hp.month_index,
      net_worth: hp.net_worth ?? hp.assets_total,
      // Centinela: el pasado no tiene "capital aportado"; nunca se renderiza en k<0.
      contributed_capital: 0,
    };
  });

  const pts: ProjectionPointApi[] = [...histPts, ...series.points];
  const futureOffset = histPts.length;
  const historyStartMonth = histPts.length > 0 ? histPts[0].month_index : 0;

  // ── Unión de series por activo, keyed por asset_id ──
  const pastLen = histPts.length;
  const futureLen = series.points.length;
  const futureSeries = series.asset_series ?? [];
  const historySeries = history.asset_series;

  // Valores históricos rebanados+ordenados a las mismas posiciones que histPts.
  const historyValsById = new Map<string, number[]>();
  for (const hs of historySeries) {
    historyValsById.set(
      hs.asset_id,
      keptIdx.map((i) => hs.values[i] ?? 0),
    );
  }

  const zerosPast = new Array<number>(pastLen).fill(0);
  const zerosFuture = new Array<number>(futureLen).fill(0);

  const assetSeries: AssetSeriesApi[] = [];
  const seen = new Set<string>();

  // Futuro primero: preserva el orden/color de los activos vivos y su nombre gana.
  for (const fs of futureSeries) {
    seen.add(fs.asset_id);
    const past = historyValsById.get(fs.asset_id) ?? zerosPast;
    assetSeries.push({
      asset_id: fs.asset_id,
      asset_name: fs.asset_name,
      values: [...past, ...fs.values],
    });
  }
  // Solo-histórico (activos borrados / backfill sin match vivo): padding 0 en el futuro.
  for (const hs of historySeries) {
    if (seen.has(hs.asset_id)) continue;
    seen.add(hs.asset_id);
    const past = historyValsById.get(hs.asset_id) ?? zerosPast;
    assetSeries.push({
      asset_id: hs.asset_id,
      asset_name: hs.asset_name,
      values: [...past, ...zerosFuture],
    });
  }

  return {
    pts,
    historyStartMonth,
    futureOffset,
    assetSeries,
    markers: history.markers,
    minNetWorth: minNetWorthOf(pts),
    pastIsAssetsOnly,
  };
}
