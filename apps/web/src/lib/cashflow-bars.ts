/**
 * Geometría del gráfico de cash-flow mensual (`MonthlyCashflowBars`). Puro: sin React, sin fetch,
 * sin CSS — solo la aritmética de las columnas, para poder fijarla en tests.
 *
 * ## Por qué existe (4.15.0)
 *
 * Hasta 4.14.x la columna de abajo apilaba `gastos + inversión` bajo unos ingresos que no la
 * cubrían: la clase `savings` no es un gasto, es dinero que YA está dentro de «ingresos − gastos»
 * y que además se movió a un producto de inversión. Apilarlos sumaba dos veces la misma salida y
 * las dos mitades de la columna dejaban de cuadrar por alturas — un mes de ahorro grande parecía
 * un mes de descontrol.
 *
 * La regla nueva: la mitad de abajo dibuja **a dónde fue cada euro que entró**. Con `I` ingresos,
 * `E` gastos (magnitud) y `A = I − E`:
 *
 * ```
 *   arriba   →  Ingresos I
 *   abajo    →  Gastos min(E, I)          (sólido, color gasto)
 *               Déficit max(0, E − I)     (RAYADO, color gasto)   — gastaste más de lo que entró
 *               Invertido min(S, max(A,0))(sólido, color inversión)
 *               En cuenta max(A,0) − Sv   (sólido, tinte claro de inversión)
 *               De reservas max(0, S − max(A,0)) (RAYADO, color inversión) — invertiste más de
 *                                          lo que ahorraste: la diferencia salió de antes
 * ```
 *
 * Las dos consecuencias que hacen que el dibujo se pueda leer sin leyenda:
 *
 * 1. **Con `A ≥ 0`, la parte SÓLIDA de abajo mide exactamente `I`.** Los gastos y el ahorro (ya
 *    repartido en invertido + en cuenta) agotan el ingreso, así que las dos mitades son espejo.
 * 2. **Lo rayado es siempre dinero que NO salió del ingreso de ese mes** — o porque no llegaba
 *    (déficit) o porque venía de reservas anteriores. Es la única parte que puede pasarse de `I`.
 *
 * `S = max(0, −savings)` y no `|savings|`: un mes de DESINVERSIÓN neta (`savings > 0`, se sacó
 * dinero del fondo) no dibuja segmento — dibujarlo del lado de las salidas diría lo contrario de
 * lo que pasó. La cifra sigue viva en el tooltip.
 */

import type { CashflowMonthApi } from "../api/types";

/** Una columna del gráfico: las magnitudes del mes y los cinco segmentos ya resueltos (todos ≥ 0). */
export type CashflowColumn = {
  /** `YYYY-MM`. */
  month: string;
  /** `I`: ingresos del mes, saneados a ≥ 0. */
  income: number;
  /** `E`: gasto del mes en MAGNITUD (la API lo sirve negativo). */
  expense: number;
  /** `S`: inversión NETA del mes en magnitud; 0 en un mes de desinversión neta. */
  savings: number;
  /** `A`: ingresos − gastos del mes (`income_minus_expense`). Puede ser negativo. */
  net: number;
  /** `expense + income + savings`: variación de caja. Solo para el tooltip. */
  cashDelta: number;
  /** Gasto cubierto por el ingreso del mes: `min(E, I)`. Sólido. */
  expenseCovered: number;
  /** Gasto que el ingreso no cubrió: `max(0, E − I)`. Rayado. */
  deficit: number;
  /** Parte del ahorro que se invirtió: `min(S, max(A, 0))`. Sólido. */
  invested: number;
  /** Parte del ahorro que se quedó en cuenta: `max(A, 0) − invertido`. Sólido. */
  cash: number;
  /** Inversión por encima del ahorro del mes: `max(0, S − max(A, 0))`. Rayado. */
  fromReserves: number;
  /** Altura total de la mitad de abajo: `E + max(A, 0) + fromReserves`. */
  downTotal: number;
};

/** Número finito o 0. Los importes llegan como string decimal desde la API. */
function num(raw: string | number | null | undefined): number {
  const n = Number(raw);
  return Number.isFinite(n) ? n : 0;
}

/**
 * Construye las columnas y la escala compartida por las dos mitades.
 *
 * `scale` es el máximo, sobre todos los meses, de `max(I, downTotal)`: la misma referencia para
 * arriba y abajo, que es lo que hace comparables las dos mitades de una columna y las columnas
 * entre sí. Con `scale === 0` (todos los meses a cero) el caller no debe dibujar nada.
 *
 * Los meses se ordenan por `month_index` ascendente; la entrada no se muta.
 */
export function buildCashflowColumns(months: CashflowMonthApi[]): {
  cols: CashflowColumn[];
  scale: number;
} {
  const cols = [...months]
    .sort((a, b) => a.month_index - b.month_index)
    .map((m): CashflowColumn => {
      const income = Math.max(0, num(m.income));
      const expense = Math.abs(num(m.expense));
      // `max(0, −savings)`, NO `abs`: un mes de desinversión neta no dibuja segmento.
      const savings = Math.max(0, -num(m.savings));
      const net = num(m.income_minus_expense);
      const saved = Math.max(0, net);

      const expenseCovered = Math.min(expense, income);
      const deficit = Math.max(0, expense - income);
      const invested = Math.min(savings, saved);
      const cash = saved - invested;
      const fromReserves = Math.max(0, savings - saved);

      return {
        month: m.date_ymd.slice(0, 7),
        income,
        expense,
        savings,
        net,
        cashDelta: num(m.cash_delta),
        expenseCovered,
        deficit,
        invested,
        cash,
        fromReserves,
        downTotal: expense + saved + fromReserves,
      };
    });

  const scale = cols.reduce((acc, c) => Math.max(acc, c.income, c.downTotal), 0);
  return { cols, scale };
}

/** ¿Algún mes de la serie tiene déficit (gastó más de lo que ingresó)? Gobierna la leyenda. */
export function hasDeficit(cols: CashflowColumn[]): boolean {
  return cols.some((c) => c.deficit > 0);
}

/** ¿Algún mes invirtió por encima de su ahorro (tirando de reservas)? Gobierna la leyenda. */
export function hasFromReserves(cols: CashflowColumn[]): boolean {
  return cols.some((c) => c.fromReserves > 0);
}
