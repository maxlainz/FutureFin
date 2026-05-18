import type { ReactNode } from "react";
import { METRIC_DASH } from "../lib/format";

export type MetricCardTone = "default" | "hero" | "accent" | "accent-2";

export function MetricCard({
  label,
  value,
  suffix,
  parenthetical,
  action,
  tone = "default",
}: {
  label: string;
  value: string;
  suffix?: string;
  /** Detalle del mismo KPI entre paréntesis (`.metric-value-parenthetical`). */
  parenthetical?: string;
  /** Botón/icono opcional en la esquina superior derecha (p.ej. engranaje de config). */
  action?: ReactNode;
  /** Variante visual: hero (acento más marcado), accent / accent-2 (tinte suave). */
  tone?: MetricCardTone;
}) {
  const toneClass =
    tone === "hero"
      ? " metric-card--hero"
      : tone === "accent"
        ? " metric-card--accent"
        : tone === "accent-2"
          ? " metric-card--accent-2"
          : "";
  const showParen = parenthetical != null && parenthetical !== "";
  return (
    <article className={`metric-card${toneClass}`}>
      <div className="metric-card__header">
        <div className="metric-label">{label}</div>
        {action ? <div className="metric-card__action">{action}</div> : null}
      </div>
      <div className="metric-value-row">
        <span className="metric-value">{value}</span>
        {suffix && suffix !== METRIC_DASH ? (
          <span className="metric-suffix">{suffix}</span>
        ) : null}
      </div>
      {/* Reservar siempre el slot del paréntesis para alinear KPIs en fila. */}
      <div
        className="metric-value-parenthetical"
        aria-hidden={showParen ? undefined : true}
      >
        {showParen ? `(${parenthetical})` : " "}
      </div>
    </article>
  );
}
