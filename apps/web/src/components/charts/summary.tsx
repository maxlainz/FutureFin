/**
 * Donut + breakdown table del resumen. Activos en familia fría, pasivos en
 * cálida. Las gráficas son la única zona donde la app usa varios colores
 * funcionales (el resto sigue la regla "B/N + único acento periwinkle").
 */

import { useMemo } from "react";
import {
  breakdownPercentOfTotal,
  formatBreakdownPct,
  formatCurrencyAmount,
  parseDisplayDecimal,
} from "../../lib/format";
import { useIsMobile } from "../../lib/responsive";

type ChartScope = "asset" | "liability";

const SUMMARY_CHART_PALETTE_ASSET: readonly [number, number, number][] = [
  [205, 52, 37],
  [218, 46, 42],
  [192, 42, 47],
  [178, 48, 44],
  [240, 36, 46],
  [160, 36, 40],
  [200, 52, 50],
  [222, 40, 38],
];

const SUMMARY_CHART_PALETTE_LIABILITY: readonly [number, number, number][] = [
  [14, 64, 41],
  [32, 58, 44],
  [354, 58, 41],
  [22, 62, 39],
  [8, 66, 40],
  [38, 54, 46],
  [26, 60, 43],
  [320, 50, 42],
];

function summaryChartSliceParts(
  scope: ChartScope,
  sliceIndex: number,
): { h: number; s: number; l: number } {
  const pal =
    scope === "asset"
      ? SUMMARY_CHART_PALETTE_ASSET
      : SUMMARY_CHART_PALETTE_LIABILITY;
  const [h, s, l] = pal[sliceIndex % pal.length]!;
  return { h, s, l };
}

function summaryChartSliceColor(scope: ChartScope, sliceIndex: number): string {
  const { h, s, l } = summaryChartSliceParts(scope, sliceIndex);
  return `hsl(${h} ${s}% ${l}%)`;
}

function summaryBreakdownBarGradient(scope: ChartScope, sliceIndex: number): string {
  const { h, s, l } = summaryChartSliceParts(scope, sliceIndex);
  const lo = `hsl(${h} ${s}% ${l}%)`;
  const hi = `hsl(${h} ${Math.max(s - 8, 36)}% ${Math.min(l + 12, 56)}%)`;
  return `linear-gradient(90deg, ${lo}, ${hi})`;
}

function summaryDonutGradient(
  rows: { total: string }[],
  totalWhole: string,
  scope: ChartScope,
): string | null {
  const tw = parseDisplayDecimal(totalWhole) ?? 0;
  if (tw <= 0 || rows.length === 0) {
    return null;
  }
  let accPct = 0;
  const stops: string[] = [];
  rows.forEach((r, rowIndex) => {
    const v = parseDisplayDecimal(r.total) ?? 0;
    if (v <= 0) {
      return;
    }
    const pct = Math.min(100 - accPct, (v / tw) * 100);
    const c = summaryChartSliceColor(scope, rowIndex);
    const start = accPct;
    accPct += pct;
    stops.push(`${c} ${start}% ${accPct}%`);
  });
  if (stops.length === 0) {
    return null;
  }
  return `conic-gradient(${stops.join(", ")})`;
}

export function SummaryDonutChart({
  title,
  rows,
  totalWhole,
  currencyIso,
  chartScope,
}: {
  title: string;
  rows: { key: string; label: string; total: string }[];
  totalWhole: string;
  currencyIso: string;
  chartScope: ChartScope;
}) {
  const filtered = useMemo(
    () => rows.filter((r) => (parseDisplayDecimal(r.total) ?? 0) > 0),
    [rows],
  );
  const g = useMemo(
    () => summaryDonutGradient(rows, totalWhole, chartScope),
    [rows, totalWhole, chartScope],
  );
  if (!g || filtered.length === 0) {
    return (
      <div className="summary-donut-card">
        <h4 className="donut-card-title">{title}</h4>
        <p className="muted tight">Sin datos.</p>
      </div>
    );
  }
  return (
    <div className="summary-donut-card">
      <h4 className="donut-card-title">{title}</h4>
      <div className="summary-donut-inner">
        <div
          className="summary-donut-ring"
          style={{ background: g }}
          role="img"
          aria-label={title}
        />
        <ul className="summary-donut-legend">
          {rows.map((r, rowIndex) => {
            if ((parseDisplayDecimal(r.total) ?? 0) <= 0) {
              return null;
            }
            const sw = summaryChartSliceColor(chartScope, rowIndex);
            return (
              <li key={r.key}>
                <span
                  className="summary-donut-legend-swatch"
                  style={{ background: sw }}
                  aria-hidden
                />
                <span className="summary-donut-legend-label">{r.label}</span>
                <span className="summary-donut-legend-val">
                  {formatCurrencyAmount(r.total, currencyIso)}
                </span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}

export function SummaryBreakdownBlock({
  title,
  rows,
  totalWhole,
  currencyIso,
  labelColumn,
  chartScope,
}: {
  title: string;
  rows: { key: string; label: string; total: string }[];
  totalWhole: string;
  currencyIso: string;
  labelColumn: string;
  chartScope: ChartScope;
}) {
  const isMobile = useIsMobile();
  if (rows.length === 0) {
    return (
      <div className="breakdown-block">
        <h4 className="panel-title">{title}</h4>
        <p className="muted tight">Sin datos.</p>
      </div>
    );
  }
  return (
    <div className="breakdown-block">
      <h4 className="panel-title">{title}</h4>
      <div className="breakdown-table-wrap bordered-top">
        <table className="breakdown-table">
          <thead>
            <tr>
              <th>{labelColumn}</th>
              <th className="num">Importe</th>
              <th className="num">%</th>
              {isMobile ? null : <th className="breakdown-bar-col" aria-hidden />}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, idx) => {
              const pct = breakdownPercentOfTotal(row.total, totalWhole);
              return (
                <tr key={row.key}>
                  <td>{row.label}</td>
                  <td className="num">
                    {formatCurrencyAmount(row.total, currencyIso)}
                  </td>
                  <td className="num muted">
                    {formatBreakdownPct(row.total, totalWhole)}
                  </td>
                  {isMobile ? null : (
                    <td className="breakdown-bar-cell">
                      <div className="breakdown-bar-track">
                        <div
                          className="breakdown-bar-fill"
                          style={{
                            width: pct !== null ? `${pct}%` : "0%",
                            background: summaryBreakdownBarGradient(chartScope, idx),
                          }}
                        />
                      </div>
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
