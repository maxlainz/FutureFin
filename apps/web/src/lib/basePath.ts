/**
 * Prefijo de ruta cuando FutureFin se sirve detrás de un proxy inverso en un subpath
 * (Ingress de Home Assistant, `X-Forwarded-Prefix` genérico).
 *
 * El servidor recibe las peticiones YA SIN el prefijo — el proxy lo quita —, así que el
 * prefijo solo afecta a las URLs que resuelve el navegador: los `fetch` a la API y las
 * escrituras de `history.pushState`. El handler del `index.html` inyecta
 * `window.__FF_BASE__` cuando detecta el subpath; sin proxy no inyecta nada y aquí todo es
 * un passthrough carácter a carácter (contrato: sin `__FF_BASE__` la app se comporta
 * exactamente igual que antes de existir este módulo).
 *
 * El mismo shell inyecta los dos interruptores de acceso que dependen del despliegue:
 * `__FF_SSO__` (el proxy delantero ya autentica — Ingress del Supervisor) y `__FF_HA_LOGIN__`
 * (la instalación sabe hablar con un Home Assistant concreto y puede delegarle el login por
 * redirección). Son independientes: el primero es «ya sé quién eres», el segundo «ve a
 * preguntárselo a Home Assistant».
 *
 * Las funciones puras `*With(base, …)` son las que se testean; los wrappers de arriba solo
 * les pasan la constante leída del `window`.
 */

declare global {
  interface Window {
    __FF_BASE__?: string;
    __FF_SSO__?: boolean;
    __FF_HA_LOGIN__?: boolean;
  }
}

/**
 * Normaliza lo que venga en `window.__FF_BASE__`. Solo se acepta una ruta absoluta sin barra
 * final; cualquier otra cosa (vacío, "/", una URL absoluta, basura) degrada a "" — es decir,
 * al comportamiento sin prefijo, que siempre es el seguro.
 */
export function normalizeBase(raw: unknown): string {
  if (typeof raw !== "string") return "";
  const trimmed = raw.trim();
  if (!trimmed.startsWith("/")) return "";
  if (trimmed.startsWith("//")) return "";
  const withoutTrailing = trimmed.replace(/\/+$/, "");
  return withoutTrailing === "" ? "" : withoutTrailing;
}

/** Prefijo activo ("" si la app se sirve en la raíz). */
export const BASE_PATH: string =
  typeof window !== "undefined" ? normalizeBase(window.__FF_BASE__) : "";

/** ¿El proxy delantero autentica por su cuenta (SSO del supervisor)? */
export const SSO_AVAILABLE: boolean =
  typeof window !== "undefined" && window.__FF_SSO__ === true;

/**
 * ¿Esta instalación puede delegar el login en un Home Assistant concreto? El servidor solo
 * inyecta el flag cuando el add-on tiene configurada la URL de HA, así que el botón se pinta
 * mirando ESTO y nada más: no hay heurística de cliente que adivine si el flujo existe.
 */
export const HA_LOGIN_AVAILABLE: boolean =
  typeof window !== "undefined" && window.__FF_HA_LOGIN__ === true;

/**
 * Antepone `base` a una ruta absoluta de la app. IDEMPOTENTE: si la ruta ya viene prefijada
 * no se duplica — importa porque hay sitios (el `url` de una llamada, un path que vuelve de
 * `history`) que pueden pasar dos veces por aquí.
 */
export function apiUrlWith(base: string, path: string): string {
  if (base === "") return path;
  if (!path.startsWith("/")) return path;
  if (path === base || path.startsWith(`${base}/`)) return path;
  return `${base}${path}`;
}

/**
 * Quita `base` de un `window.location.pathname` para que el router vea la ruta canónica de
 * la app. La coincidencia exacta con el prefijo es la raíz.
 */
export function stripBaseWith(base: string, pathname: string): string {
  if (base === "") return pathname;
  if (pathname === base) return "/";
  if (pathname.startsWith(`${base}/`)) return pathname.slice(base.length);
  return pathname;
}

/** URL de una llamada a la API (destino de `fetch`). */
export function apiUrl(path: string): string {
  return apiUrlWith(BASE_PATH, path);
}

/**
 * URL de navegación de la app (destino de `pushState`/`replaceState` y de los `href` de la
 * navegación). Es **literalmente la misma función** que `apiUrl`: los dos nombres existen para
 * que el sitio de llamada diga qué está construyendo, pero comparten objeto a propósito — dos
 * cuerpos separados podrían divergir en silencio y dejar la mitad de la app fuera del prefijo.
 */
export const appUrl = apiUrl;

/** Ruta canónica de la app a partir de un `window.location.pathname`. */
export function stripBase(pathname: string): string {
  return stripBaseWith(BASE_PATH, pathname);
}

/**
 * URL que ARRANCA el login por Home Assistant. Es un destino de navegación completa (no un
 * `fetch`): el servidor responde con un 302 hacia HA y el navegador tiene que seguirlo.
 *
 * `next` es a dónde volver DENTRO de la app y viaja ya sin prefijo: el servidor no ve el
 * subpath del proxy (se lo quita el Ingress), así que guardar el prefijo lo duplicaría a la
 * vuelta. Se acepta un `next` que venga con prefijo o sin él — `stripBaseWith` es idempotente.
 */
export function haLoginHrefWith(base: string, next: string): string {
  const target = stripBaseWith(base, next);
  return `${apiUrlWith(base, "/v1/auth/ha/start")}?next=${encodeURIComponent(target)}`;
}

/** `haLoginHrefWith` atada al prefijo activo. */
export function haLoginHref(next: string): string {
  return haLoginHrefWith(BASE_PATH, next);
}
