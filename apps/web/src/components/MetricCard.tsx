import type { ReactNode } from "react";
import { METRIC_DASH } from "../lib/format";
import { HelpPopover } from "./HelpPopover";
import { HELP_TEXTS, type HelpTextId } from "../lib/helpTexts";

export type MetricCardTone = "default" | "hero" | "accent" | "accent-2" | "danger";

export function MetricCard({
  label,
  value,
  suffix,
  parenthetical,
  trend,
  detail,
  helpId,
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
  /**
   * Id del catálogo de descripciones. Pinta el interrogante en el slot `action` — esquina
   * superior derecha, FUERA del flujo del label, así que no puede desalinear la baseline ni
   * competir con un `trend` por el slot reservado.
   */
  helpId?: HelpTextId;
  /** Botón/icono opcional en la esquina superior derecha (p.ej. engranaje de config). */
  action?: ReactNode;
  /** Variante visual: hero (acento más marcado), accent / accent-2 (tinte suave), danger (el
   *  rojo de D17: el plan está completo y NO llega — mismo tinte que `.error-banner` y que la
   *  tarjeta de plan del Resumen, un solo vocabulario de «esto va mal» en toda la app). */
  tone?: MetricCardTone;
}) {
  const toneClass =
    tone === "hero"
      ? " metric-card--hero"
      : tone === "accent"
        ? " metric-card--accent"
        : tone === "accent-2"
          ? " metric-card--accent-2"
          : tone === "danger"
            ? " metric-card--danger"
            : "";
  const showParen = parenthetical != null && parenthetical !== "";
  const showDetail = detail != null && detail !== "";
  const hasTrend = trend != null && trend !== false;
  const slotFilled = hasTrend || showParen;
  return (
    <article className={`metric-card${toneClass}`}>
      <div className="metric-card__header">
        <div className="metric-label">{label}</div>
        {action ?? helpId ? (
          <div className="metric-card__action">
            {action}
            {helpId ? (
              <HelpPopover
                title={HELP_TEXTS[helpId].title}
                body={HELP_TEXTS[helpId].body}
              />
            ) : null}
          </div>
        ) : null}
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
