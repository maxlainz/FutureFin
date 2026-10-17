/**
 * Duraciones en meses → texto español (5.0.0 U1a, rediseño UX de Jubilación).
 *
 * Existe porque `formatYearsEsFromMonths` (`lib/projection-chart.ts`) NO es un formateador de
 * duración: es el rótulo de una CUENTA ATRÁS desde hoy, y por eso trata el `0` como «Ya
 * alcanzado». Esa frase es correcta para «dentro de cuánto te jubilas» y falsa para un TRAMO
 * («¿cuánto dura el puente?»): un puente de cero meses no está «ya alcanzado», simplemente no
 * existe. Mezclar los dos usos es justo el bug S8 —la tarjeta de puente medía meses desde hoy en
 * vez del tramo jubilación→pensión—, así que las dos lecturas viven en funciones distintas.
 *
 * La regla de redacción es la misma que ya usa la app: nunca «12 años y 0 meses».
 */

import { DISPLAY_NUMBER_LOCALE } from "./format";

function int(n: number): string {
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(n);
}

/**
 * Un TRAMO de `months` meses en español: «5 meses», «1 año», «12 años», «1 año y 5 meses».
 *
 * - Por debajo de 12 meses se cuenta en meses.
 * - A partir de 12 se cuenta en años, y los meses sobrantes solo aparecen si los hay: nunca
 *   «12 años y 0 meses».
 * - `0` y cualquier valor negativo son «0 meses» — un tramo vacío es un tramo, no un hito
 *   alcanzado. Un no-finito devuelve «0 meses» por la misma razón: aquí no hay nada que estimar.
 */
export function formatMonthSpanEs(months: number): string {
  if (!Number.isFinite(months) || months <= 0) return "0 meses";
  const total = Math.round(months);
  if (total < 12) return `${int(total)} ${total === 1 ? "mes" : "meses"}`;
  const years = Math.floor(total / 12);
  const rem = total - years * 12;
  const yearsLabel = `${int(years)} ${years === 1 ? "año" : "años"}`;
  if (rem === 0) return yearsLabel;
  return `${yearsLabel} y ${int(rem)} ${rem === 1 ? "mes" : "meses"}`;
}
