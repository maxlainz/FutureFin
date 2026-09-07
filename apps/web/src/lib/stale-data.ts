/**
 * Datos que se REVALIDAN sin desaparecer, y datos que llegan tarde (5.0.0, W13).
 *
 * Dos piezas puras, las dos nacidas del mismo informe del owner sobre Jubilación (2026-09-07):
 * «durante el cálculo la GUI parpadea constantemente… si recargara in situ sin mover nada aún
 * sería tolerable: actualizar la data, no descargar y cargar nada nuevo».
 *
 * ## 1 · `nextLastGood` — stale-while-revalidate de verdad
 *
 * El patrón que había en `RetirementView` era el contrario: **una sola señal booleana de carga
 * apagaba el contenido entero**. `retirementMetricsReady` valía
 * `!projectionBusy && !retirementBusy && projectionSeries != null`, así que en cuanto un PATCH
 * del perfil disparaba el refetch (y el autosave dispara uno por cada campo que se toca) la
 * frase-hito volvía a «Calculando tu plan…», las tres tarjetas desaparecían del DOM —llevándose
 * su altura— y el chart se quedaba sin marcas, sin curva y sin tira de éxito. Todo eso con la
 * respuesta anterior todavía en memoria, intacta y correcta.
 *
 * La latch resuelve exactamente eso: **la última respuesta buena se sigue pintando entera
 * mientras llega la siguiente**, y lo único que cambia mientras tanto es un booleano
 * (`refreshing`) que la vista traduce a un indicador discreto. Reglas:
 *
 *  - `incoming != null` ⇒ manda el dato nuevo. Es el caso normal y no hay nada que conservar.
 *  - `incoming == null` **con carga en vuelo** ⇒ se conserva lo último bueno y se marca
 *    `refreshing`. Aquí es donde antes se vaciaba la pantalla.
 *  - `incoming == null` **sin carga en vuelo** ⇒ se suelta. Un `null` con el loader ya apagado
 *    es un error o un scope sin datos, y seguir enseñando cifras de otro momento sería mentir:
 *    la latch no es una cache, es una ventana de tolerancia mientras dura el fetch.
 *
 * Devuelve `prev` **por identidad** cuando nada cambia: la consumen `useMemo`s que dependen del
 * objeto, y una copia nueva por render los invalidaría todos sin que ningún dato se moviera.
 *
 * Es IDEMPOTENTE (`f(f(p,i,b),i,b) === f(p,i,b)` en los cuatro casos), que es lo que la hace
 * segura de aplicar sobre un `ref` durante el render — incluido el doble render de StrictMode.
 *
 * ## 2 · El sondeo del NIVEL 2 (`needed_capital_curve_state`)
 *
 * La curva de capital necesario por edad se calcula en segundo plano: la respuesta se declara
 * `computing` y la curva llega en un GET POSTERIOR (`handlers/projection.rs`,
 * `CURVE_STATE_COMPUTING`). Nadie pedía ese GET: sin un cambio de pestaña o una mutación, la
 * vista se quedaba con «Calculando el capital necesario por edad…» **para siempre** y el chart
 * sin su línea auxiliar. El sondeo lo cierra, con tres cotas para que no se convierta en el
 * bucle que el owner describía:
 *
 *  - empieza en 2 s y crece ×1,5 hasta un techo de 15 s (`curvePollDelayMs`);
 *  - se rinde a los `CURVE_POLL_MAX_ATTEMPTS` intentos (~2 min de reloj en total);
 *  - **solo mientras el estado sea `computing`**: `ready` y `unavailable` son finales y no se
 *    vuelve a pedir (`shouldPollNeededCurve`).
 *
 * El refetch que dispara es SILENCIOSO (no toca los flags de carga): con la latch de arriba, la
 * pantalla no se entera más que por el indicador discreto.
 */

/** Lo último bueno + si hay una revalidación en vuelo. */
export type LastGood<T> = { readonly value: T | null; readonly refreshing: boolean };

/** Estado inicial de una latch: sin dato y sin carga declarada. */
export const NO_LAST_GOOD: LastGood<never> = { value: null, refreshing: false };

/**
 * Siguiente estado de la latch. `busy` es «hay una petición en vuelo para este dato», no «la
 * pantalla está ocupada»: si se le pasa un booleano global, cualquier carga ajena marcaría
 * `refreshing` sobre un dato que nadie está revalidando.
 */
export function nextLastGood<T>(
  prev: LastGood<T>,
  incoming: T | null | undefined,
  busy: boolean,
): LastGood<T> {
  if (incoming != null) {
    return prev.value === incoming && prev.refreshing === busy
      ? prev
      : { value: incoming, refreshing: busy };
  }
  if (busy && prev.value != null) {
    return prev.refreshing ? prev : { value: prev.value, refreshing: true };
  }
  return prev.value === null && prev.refreshing === busy
    ? prev
    : { value: null, refreshing: busy };
}

/** Primer intervalo del sondeo del nivel 2. */
export const CURVE_POLL_BASE_DELAY_MS = 2_000;
/** Techo del backoff: a partir de aquí el sondeo deja de espaciarse. */
export const CURVE_POLL_MAX_DELAY_MS = 15_000;
/** Intentos antes de rendirse (~2 min con el backoff de arriba). */
export const CURVE_POLL_MAX_ATTEMPTS = 12;

/** Espera antes del intento `attempt` (0 = el primero): 2 s ×1,5 por intento, tope 15 s. */
export function curvePollDelayMs(attempt: number): number {
  const n = Math.max(0, Math.floor(attempt));
  return Math.min(
    CURVE_POLL_MAX_DELAY_MS,
    Math.round(CURVE_POLL_BASE_DELAY_MS * 1.5 ** n),
  );
}

/**
 * ¿Toca pedir otra vez la serie? Solo con el nivel 2 declarado `computing`, con la pestaña que
 * lo dibuja activa y sin haber agotado los intentos. `ready` y `unavailable` son estados
 * FINALES: seguir pidiendo con `unavailable` es un bucle contra una respuesta que no va a
 * cambiar.
 */
export function shouldPollNeededCurve(args: {
  state: string | null | undefined;
  attempts: number;
  active: boolean;
}): boolean {
  if (!args.active) return false;
  if (args.state !== "computing") return false;
  return args.attempts < CURVE_POLL_MAX_ATTEMPTS;
}
