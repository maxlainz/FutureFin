/**
 * Aritmética PURA de los gestos táctiles del chart de proyección (pan / pinch).
 *
 * Es un ESPEJO EXACTO de la matemática de `onWheel` en `ProjectionNetWorthChart`:
 * mismos clamps (`clampWindowStart`), mismo `minSpan = min(months, 12)` y mismo
 * `domainMonths = months − historyStartMonth` con borde izquierdo en el pasado
 * histórico. `onWheel` NO consume estos helpers (queda byte-idéntico); solo la
 * máquina de gestos táctil los usa. El test `chart-gestures.test.ts` prueba la
 * equivalencia reproduciendo inline la aritmética del wheel.
 *
 * Toda la ventana se expresa en MESES REALES (no índices de array), igual que
 * `viewWindow` en el chart: `startMonth` puede ser negativo (pasado histórico).
 */

/** Dominio panorámico de la ventana: horizonte futuro + borde histórico. */
export interface ChartDomain {
  /** Horizonte futuro en meses (`series.months`). */
  totalMonths: number;
  /** Borde izquierdo del dominio (`historyStartMonth`, ≤ 0). */
  historyStartMonth: number;
}

/** Span total paneable = totalMonths − historyStartMonth. Espejo de `domainMonths`. */
export function domainMonthsOf(d: ChartDomain): number {
  return d.totalMonths - d.historyStartMonth;
}

/** Span mínimo visible = min(totalMonths, 12). Espejo de `minSpan` / `minMonthSpan`. */
export function minSpanOf(d: ChartDomain): number {
  return Math.min(d.totalMonths, 12);
}

/**
 * Clampa un `startMonth` propuesto al rango válido para un `span` dado.
 * Espejo EXACTO de `onWheel`:
 *   maxStart  = max(historyStartMonth, totalMonths − span)
 *   result    = max(historyStartMonth, min(maxStart, rawStart))
 */
export function clampWindowStart(
  rawStart: number,
  span: number,
  d: ChartDomain,
): number {
  const maxStart = Math.max(d.historyStartMonth, d.totalMonths - span);
  return Math.max(d.historyStartMonth, Math.min(maxStart, rawStart));
}

/**
 * Pan por manipulación directa 1:1. `dxPx` = desplazamiento horizontal del dedo
 * desde el inicio del pan (clientX_actual − clientX_inicio). Dedo a la DERECHA
 * (`dxPx > 0`) → la vista retrocede en el tiempo → `startMonth` DECRECE (aparece
 * el pasado). `monthsPerPx` se captura una sola vez al comprometer el pan
 * (= (span−1) / anchoPlotClientPx). El `span` es constante durante el pan.
 * El clamp es idéntico al del wheel.
 */
export function panWindow(
  startSnapshot: number,
  span: number,
  dxPx: number,
  monthsPerPx: number,
  d: ChartDomain,
): number {
  const deltaMonths = -dxPx * monthsPerPx;
  return clampWindowStart(Math.round(startSnapshot + deltaMonths), span, d);
}

/**
 * Pinch/zoom anclado. `anchorMonth` = mes real bajo el punto medio de los dos
 * dedos al iniciar el pinch (constante durante el gesto). `scale` = distancia
 * actual / distancia inicial entre dedos: `>1` acerca (span decrece), `<1` aleja
 * (span crece). El `startSnapshot`/`spanSnapshot` son la ventana al iniciar el
 * pinch.
 *
 * Espejo EXACTO del bloque de zoom de `onWheel` (con `scale = 1/zoomFactor`):
 *   nextSpan   = clamp(round(spanSnapshot / scale), minSpan, domainMonths)
 *   ratio      = (anchorMonth − startSnapshot) / max(1, spanSnapshot − 1)
 *   nextStart  = clampWindowStart(round(anchorMonth − ratio·(nextSpan − 1)), nextSpan)
 */
export function pinchWindow(
  startSnapshot: number,
  spanSnapshot: number,
  anchorMonth: number,
  scale: number,
  d: ChartDomain,
): { startMonth: number; monthSpan: number } {
  const minSpan = minSpanOf(d);
  const domainMonths = domainMonthsOf(d);
  const safeScale = scale > 0 ? scale : 1;
  const nextSpanRaw = Math.round(spanSnapshot / safeScale);
  const nextSpan = Math.max(minSpan, Math.min(domainMonths, nextSpanRaw));
  const ratio =
    (anchorMonth - startSnapshot) / Math.max(1, spanSnapshot - 1);
  const nextStartRaw = Math.round(anchorMonth - ratio * (nextSpan - 1));
  const nextStart = clampWindowStart(nextStartRaw, nextSpan, d);
  return { startMonth: nextStart, monthSpan: nextSpan };
}
