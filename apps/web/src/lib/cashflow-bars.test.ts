/**
 * Geometría del cash-flow mensual. Lo que se fija aquí NO es «el dibujo sale bonito»: son las
 * cuatro invariantes que hacen que las dos mitades de una columna se puedan comparar a ojo, y
 * que 4.14.x rompía apilando gastos + inversión bajo unos ingresos que no los cubrían.
 */

import { describe, expect, it } from "vitest";
import type { CashflowMonthApi } from "../api/types";
import {
  buildCashflowColumns,
  hasDeficit,
  hasFromReserves,
  type CashflowColumn,
} from "./cashflow-bars";

/** Un mes con la convención de signos de la API: income ≥ 0, expense ≤ 0, savings ≤ 0. */
function month(
  index: number,
  income: number,
  expense: number,
  savings: number,
): CashflowMonthApi {
  const ym = `2026-${String(index + 1).padStart(2, "0")}`;
  return {
    month_index: index,
    date_ymd: `${ym}-01`,
    income: String(income),
    expense: String(expense),
    savings: String(savings),
    cash_delta: String(income + expense + savings),
    income_minus_expense: String(income + expense),
  };
}

/** Los cinco segmentos de la mitad de abajo, en orden de pintado. */
function downSegments(c: CashflowColumn): number[] {
  return [c.expenseCovered, c.deficit, c.invested, c.cash, c.fromReserves];
}

describe("buildCashflowColumns", () => {
  it("mes normal: los gastos y el ahorro agotan el ingreso, y no hay tramas", () => {
    // 3.000 entran, 2.000 se gastan, 800 se invierten → 200 se quedan en cuenta.
    const { cols, scale } = buildCashflowColumns([month(0, 3000, -2000, -800)]);
    const c = cols[0];
    expect(c.month).toBe("2026-01");
    expect(c.income).toBe(3000);
    expect(c.expense).toBe(2000);
    expect(c.savings).toBe(800);
    expect(c.net).toBe(1000);
    expect(downSegments(c)).toEqual([2000, 0, 800, 200, 0]);
    // Con A ≥ 0 la parte SÓLIDA hacia abajo mide exactamente los ingresos: las dos mitades son
    // espejo, que es justo lo que el gráfico de 4.14.x no cumplía.
    expect(c.expenseCovered + c.invested + c.cash).toBe(c.income);
    expect(c.downTotal).toBe(3000);
    expect(scale).toBe(3000);
  });

  it("déficit: lo que el ingreso no cubre sale rayado y desborda la mitad de abajo", () => {
    // 1.000 entran, 1.500 se gastan → 500 de déficit; sin ahorro que repartir.
    const { cols, scale } = buildCashflowColumns([month(0, 1000, -1500, 0)]);
    const c = cols[0];
    expect(c.net).toBe(-500);
    expect(downSegments(c)).toEqual([1000, 500, 0, 0, 0]);
    // H_down = E + max(A,0) + «de reservas»
    expect(c.downTotal).toBe(1500 + 0 + 0);
    // La escala la marca el lado grande: el gasto, no el ingreso.
    expect(scale).toBe(1500);
    expect(hasDeficit(cols)).toBe(true);
    expect(hasFromReserves(cols)).toBe(false);
  });

  it("de reservas: invertir más de lo ahorrado no infla «en cuenta», se declara aparte", () => {
    // 2.000 entran, 1.800 se gastan (ahorro 200) y aun así se invierten 500: 300 vienen de antes.
    const { cols } = buildCashflowColumns([month(0, 2000, -1800, -500)]);
    const c = cols[0];
    expect(c.net).toBe(200);
    expect(downSegments(c)).toEqual([1800, 0, 200, 0, 300]);
    // Lo sólido sigue midiendo el ingreso; lo que se pasa es exactamente lo rayado.
    expect(c.expenseCovered + c.invested + c.cash).toBe(c.income);
    expect(c.downTotal).toBe(1800 + 200 + 300);
    expect(hasFromReserves(cols)).toBe(true);
    expect(hasDeficit(cols)).toBe(false);
  });

  it("desinversión neta (savings > 0) no dibuja segmento: S = max(0, −savings), no |savings|", () => {
    // Se sacaron 400 del fondo: es una ENTRADA de caja, no una salida. El ahorro del mes
    // (1.000) se queda entero «en cuenta».
    const { cols } = buildCashflowColumns([month(0, 3000, -2000, 400)]);
    const c = cols[0];
    expect(c.savings).toBe(0);
    expect(downSegments(c)).toEqual([2000, 0, 0, 1000, 0]);
    // La cifra no se pierde: sigue viva en la variación de caja que lee el tooltip.
    expect(c.cashDelta).toBe(1400);
  });

  it("ahorro exactamente igual a lo invertido: no quedan restos en cuenta ni en reservas", () => {
    const { cols } = buildCashflowColumns([month(0, 2000, -1500, -500)]);
    expect(downSegments(cols[0])).toEqual([1500, 0, 500, 0, 0]);
  });

  it("la escala es el máximo de las dos mitades sobre TODOS los meses", () => {
    const { cols, scale } = buildCashflowColumns([
      month(0, 3000, -2000, -800), // arriba 3000, abajo 3000
      month(1, 1000, -1500, 0), //    arriba 1000, abajo 1500
      month(2, 2000, -1800, -900), //  arriba 2000, abajo 1800+200+700 = 2700
    ]);
    expect(cols).toHaveLength(3);
    expect(scale).toBe(
      Math.max(...cols.map((c) => Math.max(c.income, c.downTotal))),
    );
    expect(scale).toBe(3000);
  });

  it("ningún segmento es negativo, en ninguna combinación de signos", () => {
    const meses = [
      month(0, 3000, -2000, -800),
      month(1, 1000, -1500, 0),
      month(2, 2000, -1800, -900),
      month(3, 0, 0, 0),
      month(4, 0, -500, -100),
      month(5, 1200, 0, -1500),
      month(6, 900, -300, 250),
    ];
    for (const c of buildCashflowColumns(meses).cols) {
      for (const seg of downSegments(c)) {
        expect(seg, `segmento negativo en ${c.month}`).toBeGreaterThanOrEqual(0);
      }
      expect(c.income).toBeGreaterThanOrEqual(0);
      expect(c.downTotal).toBeCloseTo(
        c.expense + Math.max(c.net, 0) + c.fromReserves,
        9,
      );
      // La suma de los cinco tramos ES la altura declarada: si divergieran, la barra dibujada
      // no mediría lo que el tooltip afirma.
      expect(downSegments(c).reduce((a, b) => a + b, 0)).toBeCloseTo(c.downTotal, 9);
    }
  });

  it("ordena por month_index y no muta la entrada", () => {
    const input = [month(2, 100, -10, 0), month(0, 100, -10, 0), month(1, 100, -10, 0)];
    const snapshot = input.map((m) => m.month_index);
    const { cols } = buildCashflowColumns(input);
    expect(cols.map((c) => c.month)).toEqual(["2026-01", "2026-02", "2026-03"]);
    expect(input.map((m) => m.month_index)).toEqual(snapshot);
  });

  it("una serie toda a cero deja la escala en 0 (el caller no dibuja nada)", () => {
    const { cols, scale } = buildCashflowColumns([month(0, 0, 0, 0)]);
    expect(cols).toHaveLength(1);
    expect(scale).toBe(0);
  });

  it("un importe ilegible cuenta como 0 en vez de propagar NaN a las alturas", () => {
    const roto: CashflowMonthApi = {
      ...month(0, 1000, -400, -100),
      expense: "no-es-un-numero",
    };
    const c = buildCashflowColumns([roto]).cols[0];
    expect(c.expense).toBe(0);
    expect(Number.isFinite(c.downTotal)).toBe(true);
  });
});
