/**
 * Preparación PURA de las «líneas finas por miembro» del chart de Proyección en vista Hogar
 * (5.0.0, D32 / §G del plan de #207).
 *
 * El agregado del hogar publica UNA curva —la Σ de N simulaciones independientes— y, al lado,
 * `members[].series`: la serie de cada persona a la MISMA densidad y con los MISMOS `month_index`
 * que `points[]`. Este módulo convierte esas series en lo que el SVG necesita para pintarlas:
 * puntos en euros ya deflactados, recortados al horizonte propio de cada miembro y con el color
 * que empareja línea, tick de la tira de fases y entrada de leyenda.
 *
 * Tres reglas que no son negociables, y por las que esto vive aquí y no dentro del componente:
 *
 *  1. **Todo se razona en MESES (`month_index`), jamás en posiciones de array.** Las series de
 *     miembro vienen decimadas igual que `points[]` (`density=hybrid`: meses 0..12, luego
 *     anuales), así que la posición 13 es el mes 24. El chart traduce mes → píxel con su `xScale`,
 *     que también toma meses, y el tooltip busca «el último punto servido en el mes o antes».
 *  2. **La misma deflactación que el resto del chart**, mes a mes con el `month_index` REAL del
 *     punto. El servidor no publica una variante «en euros de hoy» por miembro (sí lo hace para la
 *     Σ, `points[].net_worth_real`), así que aquí se aplica el deflactor de TypeScript
 *     —`deflationFactorAt`, el mismo que ya deflacta las áreas de activo— y la divergencia con el
 *     `deflator_at_month_index` del servidor es la ya pineada por `deflator-parity.json`. El
 *     factor entra como parámetro: quien dibuja ya lo tiene resuelto y no puede haber dos.
 *  3. **Horizonte propio: se TERMINA, no se extrapola.** El agregado corre al horizonte común
 *     `max(horizontes)`, así que un miembro con `horizon_lifespan_age` menor tiene serie más allá
 *     de los años que declaró vivir. Su línea acaba en su último mes propio; alargarla dibujaría
 *     un patrimonio de una vida que esa persona no ha declarado.
 *
 * Lo que este módulo NO hace: dibujar `net_worth_liquid`. D32 pide UNA línea fina por miembro; dos
 * por persona convierten el chart del hogar en una maraña y el líquido solo tiene sentido contra
 * el objetivo de esa persona, que el agregado no publica.
 */

import type { HouseholdMemberProjectionApi } from "../api/types";
import { householdMemberColor } from "./chart-legend";
import { lastPointIndexAtOrBeforeMonth } from "./projection-chart";

/** Un vértice de la línea de un miembro. `month_index` (y no `x`) para que el punto se pueda
 *  pasar tal cual a `lastPointIndexAtOrBeforeMonth`, que es la única forma correcta de traducir
 *  un mes a una posición en una serie decimada. */
export type MemberLinePoint = {
  /** Mes de la rejilla, el mismo que `points[].month_index`. */
  month_index: number;
  /** Patrimonio neto de ese mes, YA deflactado con el factor recibido. */
  value: number;
};

/** Una línea fina lista para pintar, con su identidad y su color. */
export type MemberLine = {
  /** Key de React y de la leyenda: el mismo `member-${user_id}` que usan la tira y la leyenda. */
  key: string;
  userId: string;
  /** Nombre del miembro, tal y como lo rotula la leyenda. */
  label: string;
  /** Token `var(--proj-*)`, nunca hex. Emparejado con su tick y su entrada de leyenda. */
  color: string;
  /** Vértices en orden de mes ascendente. Puede quedar vacío (miembro sin serie servida). */
  points: MemberLinePoint[];
};

/**
 * `members[]` → una línea por miembro, deflactada y recortada a su horizonte propio.
 *
 * `deflator(monthIndex)` es el factor por el que se multiplica el importe nominal de ese mes;
 * en modo nominal el chart pasa una función que devuelve 1 y los valores salen intactos.
 *
 * Descarta en silencio lo que no se puede dibujar (miembro sin `series`, punto con importe o mes
 * no finito) en vez de pintar un 0: un vértice inventado en el suelo del eje se lee como «este
 * mes no tenía nada», que es una afirmación sobre el dinero de alguien.
 */
export function buildHouseholdMemberLines(
  members: readonly HouseholdMemberProjectionApi[] | null | undefined,
  deflator: (monthIndex: number) => number,
): MemberLine[] {
  return (members ?? []).map((m, idx) => {
    const horizon =
      typeof m.horizon_months === "number" &&
      Number.isFinite(m.horizon_months) &&
      m.horizon_months > 0
        ? m.horizon_months
        : null;
    const points: MemberLinePoint[] = [];
    for (const p of m.series ?? []) {
      if (p == null) continue;
      const mi = p.month_index;
      const v = p.net_worth;
      if (!Number.isFinite(mi) || !Number.isFinite(v)) continue;
      // `months` es un CONTADOR y los `month_index` van 0..months−1 (`density_month_indices`),
      // así que el último mes propio del miembro es `horizon_months − 1`.
      if (horizon !== null && mi > horizon - 1) continue;
      points.push({ month_index: mi, value: v * deflator(mi) });
    }
    points.sort((a, b) => a.month_index - b.month_index);
    return {
      key: `member-${m.user_id}`,
      userId: m.user_id,
      label: m.username,
      color: householdMemberColor(idx),
      points,
    };
  });
}

/**
 * Valor de la línea en el mes `month`: el **último vértice servido en ese mes o antes**, y `null`
 * donde la línea no existe. Es lo que lee el tooltip, y por construcción coincide con lo que hay
 * dibujado bajo el cursor.
 *
 * La convención del «último en ese mes o antes» es la misma que `jubilacion_series_position` del
 * API y la única que funciona con una serie decimada: al pasar por el mes 30, la serie hybrid
 * tiene vértices en 24 y 36; el valor del segmento que se está atravesando es el del 24, nunca el
 * del 36 —que sería el patrimonio de medio año DESPUÉS— ni un 0.
 *
 * Hay `null` en los dos extremos, y por la misma razón: **la línea no se extrapola**.
 *  - Antes del primer vértice — el tramo histórico del chart, con `month_index` negativo, donde
 *    ninguna persona tiene serie.
 *  - Después del último — un miembro con horizonte propio más corto, cuya curva ya ha terminado.
 *    Repetir ahí su último importe diría «esto es lo que tiene en ese mes» sobre un año que esa
 *    persona no declaró vivir.
 */
export function memberValueAtMonth(
  line: Pick<MemberLine, "points">,
  month: number,
): number | null {
  const pts = line.points;
  if (pts.length === 0 || !Number.isFinite(month)) return null;
  if (pts[0]!.month_index > month) return null;
  if (pts[pts.length - 1]!.month_index < month) return null;
  const idx = lastPointIndexAtOrBeforeMonth(pts, month);
  return pts[idx]?.value ?? null;
}
