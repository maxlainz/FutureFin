/**
 * Gráfica de apoyo de la pestaña «Movimientos» (cash-flow). Solo presentacional, sin fetch.
 *
 *  - `MonthlyCashflowBars`: cash-flow mes a mes desde `months[]` de `/v1/history/cashflow`
 *    (ingresos hacia arriba; gastos + ahorro hacia abajo; neto en el tooltip).
 *
 * Todos los colores vienen de tokens `var(--ff-*)` / `var(--cf-*)`.
 */

import type { CashflowMonthApi } from "../../api/types";
import { formatCurrencyNumber } from "../../lib/format";
import { monthLabelEs, monthShortLabelEs } from "../../lib/expenses";

/**
 * Cash-flow mes a mes: columna divergente por mes (ingresos hacia arriba, gastos + ahorro hacia
 * abajo). Escala compartida entre ambos lados para comparabilidad. `months[]` trae expense y
 * savings NEGATIVOS (signo real) e income positivo.
 */
export function MonthlyCashflowBars({
  months,
  currencyIso,
}: {
  months: CashflowMonthApi[];
  currencyIso: string;
}) {
  const cols = [...months]
    .sort((a, b) => a.month_index - b.month_index)
    .map((m) => {
      const income = Math.max(0, Number(m.income));
      const expenseMag = Math.abs(Number(m.expense));
      const savingsMag = Math.abs(Number(m.savings));
      return {
        month: m.date_ymd.slice(0, 7),
        income: Number.isFinite(income) ? income : 0,
        expenseMag: Number.isFinite(expenseMag) ? expenseMag : 0,
        savingsMag: Number.isFinite(savingsMag) ? savingsMag : 0,
        // Las dos cifras, porque dicen cosas distintas. La geometría de la barra ya codifica la
        // variación de caja (el ahorro se dibuja del lado de los gastos); lo que NO se puede leer
        // del dibujo es «gané más de lo que gasté», y ésa es la que el tooltip pone primero.
        cashDelta: Number(m.cash_delta),
        incomeMinusExpense: Number(m.income_minus_expense),
      };
    });
  const scale = cols.reduce(
    (acc, c) => Math.max(acc, c.income, c.expenseMag + c.savingsMag),
    0,
  );
  if (cols.length === 0 || !(scale > 0)) return null;

  const step = Math.max(1, Math.ceil(cols.length / 8));
  const half = (v: number) => `${Math.min(100, (v / scale) * 100)}%`;

  return (
    <div className="cf-chart bordered-top">
      <div className="cf-legend" aria-hidden>
        <span className="cf-legend-item">
          <span className="cf-swatch cf-swatch--income" /> Ingresos
        </span>
        <span className="cf-legend-item">
          <span className="cf-swatch cf-swatch--expense" /> Gastos
        </span>
        <span className="cf-legend-item">
          <span className="cf-swatch cf-swatch--savings" /> Ahorro
        </span>
      </div>
      <div className="cf-track" role="img" aria-label="Cash-flow mensual">
        {cols.map((c, i) => (
          <div
            className="cf-col"
            key={`${c.month}-${i}`}
            title={`${monthLabelEs(c.month)} · Ingresos ${formatCurrencyNumber(
              c.income,
              currencyIso,
            )} · Gastos ${formatCurrencyNumber(
              c.expenseMag,
              currencyIso,
            )} · Ahorro ${formatCurrencyNumber(
              c.savingsMag,
              currencyIso,
            )} · Ingresos − gastos ${formatCurrencyNumber(
              c.incomeMinusExpense,
              currencyIso,
            )} · Variación de caja ${formatCurrencyNumber(
              c.cashDelta,
              currencyIso,
            )}`}
          >
            <div className="cf-col-up">
              <div
                className="cf-bar cf-bar--income"
                style={{ height: half(c.income) }}
              />
            </div>
            <div className="cf-col-down">
              <div
                className="cf-bar cf-bar--expense"
                style={{ height: half(c.expenseMag) }}
              />
              <div
                className="cf-bar cf-bar--savings"
                style={{ height: half(c.savingsMag) }}
              />
            </div>
          </div>
        ))}
      </div>
      <div className="cf-xlabels" aria-hidden>
        {cols.map((c, i) => (
          <div className="cf-xlabel" key={`lbl-${c.month}-${i}`}>
            {i % step === 0 ? monthShortLabelEs(c.month) : ""}
          </div>
        ))}
      </div>
    </div>
  );
}
