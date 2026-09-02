/**
 * Gráfica de apoyo de la pestaña «Movimientos» (cash-flow). Solo presentacional, sin fetch.
 *
 *  - `MonthlyCashflowBars`: cash-flow mes a mes desde `months[]` de `/v1/history/cashflow`
 *    (ingresos hacia arriba; a dónde fue cada euro que entró, hacia abajo).
 *
 * Toda la aritmética vive en [`lib/cashflow-bars.ts`](../../lib/cashflow-bars.ts) —incluida la
 * razón por la que la mitad de abajo ya no apila «gastos + inversión»—; aquí solo queda el
 * render. Todos los colores vienen de tokens `var(--ff-*)` / `var(--cf-*)`.
 */

import type { CashflowMonthApi } from "../../api/types";
import { formatCurrencyNumber } from "../../lib/format";
import {
  buildCashflowColumns,
  hasDeficit,
  hasFromReserves,
  type CashflowColumn,
} from "../../lib/cashflow-bars";
import { monthLabelEs, monthShortLabelEs } from "../../lib/expenses";

/**
 * Texto del tooltip de una columna. Déficit y «de reservas» se OMITEN cuando valen cero: son las
 * dos anomalías del mes, y enseñarlas siempre a cero las convierte en ruido que se deja de leer.
 */
function columnTitle(c: CashflowColumn, currencyIso: string): string {
  const money = (n: number) => formatCurrencyNumber(n, currencyIso);
  const parts = [
    monthLabelEs(c.month),
    `Ingresos ${money(c.income)}`,
    `Gastos ${money(c.expense)}`,
    `Ahorro ${money(c.net)} (invertido ${money(c.invested)} · en cuenta ${money(c.cash)})`,
  ];
  if (c.deficit > 0) parts.push(`Déficit ${money(c.deficit)}`);
  if (c.fromReserves > 0) parts.push(`De reservas ${money(c.fromReserves)}`);
  parts.push(`Variación de caja ${money(c.cashDelta)}`);
  return parts.join(" · ");
}

/**
 * Cash-flow mes a mes: columna divergente por mes. Arriba, los ingresos. Abajo, a dónde fue el
 * dinero: gastos, lo invertido y lo que quedó en cuenta — más los dos segmentos RAYADOS que
 * marcan lo que no salió del ingreso del mes (déficit y «de reservas»). Escala compartida entre
 * ambos lados, así que con un mes sin déficit las dos mitades miden lo mismo.
 */
export function MonthlyCashflowBars({
  months,
  currencyIso,
}: {
  months: CashflowMonthApi[];
  currencyIso: string;
}) {
  const { cols, scale } = buildCashflowColumns(months);
  if (cols.length === 0 || !(scale > 0)) return null;

  const step = Math.max(1, Math.ceil(cols.length / 8));
  const half = (v: number) => `${Math.min(100, (v / scale) * 100)}%`;
  // Las dos entradas de anomalía solo entran en la leyenda cuando algún mes las dibuja: una
  // leyenda con seis entradas fijas obliga a buscar en el gráfico dos tramas que no están.
  const showDeficit = hasDeficit(cols);
  const showReserves = hasFromReserves(cols);

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
          <span className="cf-swatch cf-swatch--savings" /> Invertido
        </span>
        <span className="cf-legend-item">
          <span className="cf-swatch cf-swatch--savings-cash" /> En cuenta
        </span>
        {showDeficit ? (
          <span className="cf-legend-item">
            <span className="cf-swatch cf-swatch--expense cf-swatch--hatched" /> Déficit
          </span>
        ) : null}
        {showReserves ? (
          <span className="cf-legend-item">
            <span className="cf-swatch cf-swatch--savings cf-swatch--hatched" /> De
            reservas
          </span>
        ) : null}
      </div>
      <div className="cf-track" role="img" aria-label="Cash-flow mensual">
        {cols.map((c, i) => (
          <div
            className="cf-col"
            key={`${c.month}-${i}`}
            title={columnTitle(c, currencyIso)}
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
                style={{ height: half(c.expenseCovered) }}
              />
              <div
                className="cf-bar cf-bar--expense cf-bar--hatched"
                style={{ height: half(c.deficit) }}
              />
              <div
                className="cf-bar cf-bar--savings"
                style={{ height: half(c.invested) }}
              />
              <div
                className="cf-bar cf-bar--savings-cash"
                style={{ height: half(c.cash) }}
              />
              <div
                className="cf-bar cf-bar--savings cf-bar--hatched"
                style={{ height: half(c.fromReserves) }}
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
