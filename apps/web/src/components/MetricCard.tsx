import type { ReactNode } from "react";
import { METRIC_DASH } from "../lib/format";

export function MetricCard({
  label,
  value,
  suffix,
  parenthetical,
  action,
}: {
  label: string;
  value: string;
  suffix?: string;
  /** Detalle del mismo KPI entre paréntesis (`.metric-value-parenthetical`). */
  parenthetical?: string;
  /** Botón/icono opcional en la esquina superior derecha (p.ej. engranaje de config). */
  action?: ReactNode;
}) {
  return (
    <article className="metric-card">
      <div className="metric-card__header">
        <div className="metric-label">{label}</div>
        {action ? <div className="metric-card__action">{action}</div> : null}
      </div>
      <div className="metric-value-row">
        <span className="metric-value">{value}</span>
        {parenthetical != null && parenthetical !== "" ? (
          <span className="metric-value-parenthetical">
            ({parenthetical})
          </span>
        ) : null}
        {suffix && suffix !== METRIC_DASH ? (
          <span className="metric-suffix">{suffix}</span>
        ) : null}
      </div>
    </article>
  );
}
