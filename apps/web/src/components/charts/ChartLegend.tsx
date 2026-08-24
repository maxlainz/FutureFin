import { useId, useState, type CSSProperties } from "react";
import {
  applyLegendCollapse,
  DEFAULT_LEGEND_ASSET_CAP,
  type ChartLegendItem,
} from "../../lib/chart-legend";

/**
 * Leyenda compartida de charts (proyección grande y MiniProjection): HTML fuera del
 * <svg> — flex-wrap real en lugar de anchos estimados, y el toggle queda fuera de la
 * máquina de gestos pan/zoom del chart. Las entradas `structural` se ven siempre;
 * los `assets` se truncan a `collapsedCap` con un chip «+N más» expandible. Puramente
 * informativa: el único control es expandir/colapsar (efímero, se resetea al montar).
 */
export function ChartLegend({
  structural,
  assets = [],
  collapsedCap = DEFAULT_LEGEND_ASSET_CAP,
  size = "md",
  className,
  ariaLabel = "Leyenda del gráfico",
}: {
  structural: readonly ChartLegendItem[];
  /** Items de activo, ya ordenados y coloreados (ver lib/chart-legend.ts). */
  assets?: readonly ChartLegendItem[];
  collapsedCap?: number;
  size?: "sm" | "md";
  className?: string;
  ariaLabel?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const { visibleCount, hiddenCount } = applyLegendCollapse(
    assets.length,
    collapsedCap,
  );
  const shownAssets = expanded ? assets : assets.slice(0, visibleCount);
  const items = [...structural, ...shownAssets];
  return (
    <div
      className={`ff-chart-legend ff-chart-legend--${size}${className ? ` ${className}` : ""}`}
    >
      <ul id={listId} className="ff-chart-legend-items" aria-label={ariaLabel}>
        {items.map((item) => (
          <li
            key={item.key}
            className="ff-chart-legend-item"
            title={item.title ?? item.label}
            style={{ "--ff-legend-color": item.color } as CSSProperties}
          >
            <span
              aria-hidden="true"
              className={`ff-chart-legend-swatch ff-chart-legend-swatch--${item.swatch}`}
            />
            <span className="ff-chart-legend-label">{item.label}</span>
          </li>
        ))}
      </ul>
      {hiddenCount > 0 ? (
        <button
          type="button"
          className="ff-chart-legend-toggle"
          aria-expanded={expanded}
          aria-controls={listId}
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "Ver menos" : `+${hiddenCount} más`}
        </button>
      ) : null}
    </div>
  );
}
