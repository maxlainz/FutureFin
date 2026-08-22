/**
 * Estado vacío reutilizable: **qué es este bloque, qué puede hacer aquí el usuario y el botón
 * que lleva directo a hacerlo**. Sustituye a los banners telegráficos («Activos · Ajustes →
 * Categorías») y a los «Sin datos.» sueltos de las vistas del ledger. Modelo: el panel «Sin
 * movimientos» de `GastosView`.
 *
 * Es la otra mitad de la **política de ceros** unificada (ver `App.css` §Empty states): un
 * bloque con datos enseña sus ceros; un bloque sin datos se sustituye ENTERO por uno de estos.
 *
 * Reglas de la casa: copy en español tuteando, una o dos frases, sin exclamaciones. La acción
 * apunta al formulario concreto (abrir el modal), nunca a otra pestaña — salvo cuando la única
 * salida real está en Ajustes (no quedan categorías), y entonces se dice con todas las letras.
 */
export function EmptyState({
  title,
  description,
  actionLabel,
  onAction,
  actionDisabled,
  secondaryLabel,
  onSecondary,
  embedded = false,
}: {
  /** Titular corto del bloque vacío («Sin activos»). */
  title: string;
  /** Una o dos frases: qué es esta vista y qué gana el usuario rellenándola. */
  description: string;
  /** Botón primario. Sin `actionLabel` + `onAction` el estado vacío es solo texto (solo lectura). */
  actionLabel?: string;
  onAction?: () => void;
  actionDisabled?: boolean;
  /** Acción alternativa opcional («Añadir gasto» junto a «Añadir ingreso»). */
  secondaryLabel?: string;
  onSecondary?: () => void;
  /**
   * `true` cuando el estado vacío ya vive DENTRO de un `.panel` (columna del Presupuesto, lista
   * de Próximos, bloques del Resumen): no dibuja su propio panel, solo el separador superior.
   */
  embedded?: boolean;
}) {
  const hasPrimary = actionLabel != null && onAction != null;
  const hasSecondary = secondaryLabel != null && onSecondary != null;
  const body = (
    <>
      <p className="muted empty-state-text">{description}</p>
      {hasPrimary || hasSecondary ? (
        <div className="empty-state-actions">
          {hasPrimary ? (
            <button
              type="button"
              className="btn primary"
              onClick={onAction}
              disabled={actionDisabled}
            >
              {actionLabel}
            </button>
          ) : null}
          {hasSecondary ? (
            <button type="button" className="btn ghost" onClick={onSecondary}>
              {secondaryLabel}
            </button>
          ) : null}
        </div>
      ) : null}
    </>
  );

  if (embedded) {
    return (
      <div className="empty-state empty-state--embedded bordered-top">
        <p className="empty-state-title">{title}</p>
        {body}
      </div>
    );
  }

  return (
    <section className="panel empty-state">
      <h3 className="panel-title">{title}</h3>
      <div className="empty-state-body bordered-top">{body}</div>
    </section>
  );
}
