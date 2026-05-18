/**
 * Preferencia de tema (Auto/Claro/Oscuro) + aplicación al `<html data-theme>`.
 *
 * - `auto` (default): sigue `prefers-color-scheme`.
 * - Persiste en localStorage.
 * - Aplicar al cargar la app y cada vez que cambie la preferencia.
 */

export type ThemePref = "auto" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "ff-theme-pref";

export function loadThemePref(): ThemePref {
  if (typeof window === "undefined") return "auto";
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "auto") return raw;
  } catch {
    // Acceso denegado a localStorage (modo privado, sandboxes, etc.).
  }
  return "auto";
}

export function saveThemePref(pref: ThemePref): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    // ignore
  }
}

export function resolveTheme(pref: ThemePref): ResolvedTheme {
  if (pref === "light" || pref === "dark") return pref;
  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  ) {
    return "dark";
  }
  return "light";
}

export function applyTheme(pref: ThemePref): void {
  if (typeof document === "undefined") return;
  const t = resolveTheme(pref);
  document.documentElement.dataset.theme = t;
}

/**
 * Cuando `pref === "auto"` queremos reflejar cambios en vivo del sistema.
 * Llamar desde un useEffect y limpiar con el returned unsubscribe.
 */
export function subscribeSystemThemeChanges(cb: () => void): () => void {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return () => {};
  }
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = () => cb();
  // Safari < 14 usa addListener; modern usa addEventListener.
  if (typeof mql.addEventListener === "function") {
    mql.addEventListener("change", handler);
    return () => mql.removeEventListener("change", handler);
  }
  mql.addListener(handler);
  return () => mql.removeListener(handler);
}
