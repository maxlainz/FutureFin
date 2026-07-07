/**
 * Detección de viewport móvil (ancho phone) para render condicional de columnas
 * de tabla. Espejo del patrón matchMedia de `lib/theme.ts`: lectura síncrona
 * inicial + suscripción con fallback `addListener` para Safari < 14.
 *
 * `MOBILE_MAX_WIDTH = 640` es el breakpoint canónico `bp:mobile 640` de App.css
 * (densidad phone). Mantener ambos en sincronía.
 */

import { useEffect, useState } from "react";

/** Breakpoint canónico phone (bp:mobile 640). Ver cabecera de App.css. */
export const MOBILE_MAX_WIDTH = 640;

/** Helper puro: ¿este ancho (px) es móvil? `<= MOBILE_MAX_WIDTH`. Testeable en node. */
export function isMobileWidth(width: number): boolean {
  return width <= MOBILE_MAX_WIDTH;
}

/**
 * Hook: `true` cuando el viewport está en el tramo phone (`<= 640px`). SSR-safe
 * (devuelve `false` sin `window`). Lee de forma síncrona en el primer render y
 * se suscribe a cambios de la media query (mismo `addEventListener`/`addListener`
 * con fallback que `subscribeSystemThemeChanges`).
 */
export function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState<boolean>(() => {
    if (
      typeof window === "undefined" ||
      typeof window.matchMedia !== "function"
    ) {
      return false;
    }
    return window.matchMedia(`(max-width: ${MOBILE_MAX_WIDTH}px)`).matches;
  });

  useEffect(() => {
    if (
      typeof window === "undefined" ||
      typeof window.matchMedia !== "function"
    ) {
      return;
    }
    const mql = window.matchMedia(`(max-width: ${MOBILE_MAX_WIDTH}px)`);
    const handler = () => setIsMobile(mql.matches);
    // Sincroniza por si el ancho cambió entre el render inicial y el effect.
    handler();
    // Safari < 14 usa addListener; modern usa addEventListener.
    if (typeof mql.addEventListener === "function") {
      mql.addEventListener("change", handler);
      return () => mql.removeEventListener("change", handler);
    }
    mql.addListener(handler);
    return () => mql.removeListener(handler);
  }, []);

  return isMobile;
}
