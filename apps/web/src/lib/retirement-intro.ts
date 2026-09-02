/**
 * Aviso de alta de la pestaña Jubilación (D33 de 5.0.0): se enseña UNA vez por navegador y se
 * descarta para siempre.
 *
 * La persistencia vive aquí y no en el componente por dos razones: el `localStorage` puede
 * lanzar (Safari en privado, cookies de terceros bloqueadas en un iframe — el add-on de Home
 * Assistant sirve la SPA dentro del Ingress), y la decisión «qué cuenta como descartado» es una
 * función pura que sí se puede fijar en un test.
 *
 * Sesgo elegido: ante un almacenamiento ilegible el aviso **se enseña**. Es un aviso de una
 * línea con su botón de descarte; verlo otra vez cuesta un clic, no verlo nunca deja al usuario
 * sin saber que la estrategia es suya y se elige.
 */

/** Clave de `localStorage`; mismo espacio de nombres que el resto de preferencias de la SPA. */
export const RETIREMENT_INTRO_DISMISSED_STORAGE_KEY =
  "futurefin-retirement-intro-dismissed";

/** Único valor que cuenta como «descartado». Cualquier otra cosa (basura, `null`) es «enséñalo». */
const DISMISSED_VALUE = "1";

/** Puro: traduce lo leído del almacenamiento a la decisión de enseñar o no. */
export function isRetirementIntroDismissed(
  stored: string | null | undefined,
): boolean {
  return stored?.trim() === DISMISSED_VALUE;
}

/** Lectura tolerante a un `localStorage` que lanza o no existe (SSR, iframe restringido). */
export function readRetirementIntroDismissed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return isRetirementIntroDismissed(
      window.localStorage.getItem(RETIREMENT_INTRO_DISMISSED_STORAGE_KEY),
    );
  } catch {
    return false;
  }
}

/** Escritura igual de tolerante: si no se puede persistir, el aviso volverá en la próxima visita. */
export function persistRetirementIntroDismissed(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      RETIREMENT_INTRO_DISMISSED_STORAGE_KEY,
      DISMISSED_VALUE,
    );
  } catch {
    /* almacenamiento bloqueado: el estado en memoria ya oculta el aviso en esta sesión */
  }
}
