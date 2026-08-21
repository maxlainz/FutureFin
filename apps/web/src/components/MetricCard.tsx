import type { ReactNode } from "react";
import { METRIC_DASH } from "../lib/format";

export type MetricCardTone = "default" | "hero" | "accent" | "accent-2";

export function MetricCard({
  label,
  value,
  suffix,
  parenthetical,
  trend,
  detail,
  action,
  tone = "default",
}: {
  label: string;
  value: string;
  suffix?: string;
  /** Detalle del mismo KPI entre paréntesis (`.metric-value-parenthetical`). */
  parenthetical?: string;
  /**
   * Contenido de tendencia (flecha + delta) que ocupa el MISMO slot reservado que el paréntesis
   * (baseline intacta). Si se pasa, tiene prioridad sobre `parenthetical`.
   */
  trend?: ReactNode;
  /**
   * SEGUNDO slot reservado, bajo el de trend/paréntesis. Existe porque una tarjeta puede
   * necesitar las dos cosas a la vez (p.ej. «▼ −55 € vs plan» + «24,4 % de tus ingresos») y
   * `trend` gana el primero. Igual que aquel, se renderiza SIEMPRE — con `&nbsp;` cuando está
   * vacío — para que la baseline siga alineada entre KPIs de la misma fila.
   */
  detail?: string;
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
  const showDetail = detail != null && detail !== "";
  const hasTrend = trend != null && trend !== false;
  const slotFilled = hasTrend || showParen;
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
        aria-hidden={slotFilled ? undefined : true}
      >
        {hasTrend ? trend : showParen ? `(${parenthetical})` : " "}
      </div>
      {/* Segundo slot, también SIEMPRE presente: una tarjeta puede necesitar trend y detalle
          a la vez, y `trend` gana el primero. Reservarlo mantiene la baseline por fila. */}
      <div className="metric-value-detail" aria-hidden={showDetail ? undefined : true}>
        {showDetail ? `(${detail})` : " "}
      </div>
    </article>
  );
}
