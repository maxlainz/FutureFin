/**
 * Cliente HTTP único para todas las llamadas a la API. Centraliza:
 *  - `credentials: "include"` para que el navegador envíe la cookie `ff_session`.
 *  - Traducción del error al español (`{ code, message }` → catálogo `lib/errorMessages`).
 *  - `Content-Type: application/json` en métodos con body.
 *  - El prefijo de subpath: **toda URL de API pasa por `apiUrl`** (`lib/basePath`), que la deja
 *    intacta salvo que la app se sirva detrás de un proxy en un subpath.
 *
 * Los wrappers `apiGet/Post/Patch/Delete` lanzan `ApiRequestError` cuando la respuesta no es 2xx,
 * devolviendo el JSON parseado en el caso bueno.
 *
 * IMPORTANTE: `ApiRequestError.message` ya viene EN ESPAÑOL. Todo el código que hace
 * `setError(e instanceof Error ? e.message : "…")` —hay ~50 sitios— muestra español sin
 * cambiar una línea. El texto técnico en inglés del backend queda en `.detail`, para
 * enseñarlo plegado o mandarlo a la consola, nunca como frase principal.
 */

import { apiUrl } from "../lib/basePath";
import { messageForError } from "../lib/errorMessages";

export const defaultFetchInit: RequestInit = {
  credentials: "include",
};

/**
 * Qué hacer cuando el servidor responde 401 a CUALQUIER petición. Lo registra `App.tsx` al
 * montar (→ `setUser(null)`), y por eso vive aquí y no en un catch: hasta 4.0.0 el único sitio
 * que miraba el 401 era `refreshSession`, que solo corre al arrancar la app. Si la sesión
 * caducaba con la pestaña abierta, cada acción devolvía «Tu sesión ha caducado» en un banner y
 * el usuario seguía viendo su patrimonio en pantalla, sin forma de volver al login salvo
 * recargar. Todo error de API nace en `apiErrorFromResponse`, así que este es el único punto por
 * el que pasan los 401 de las ~50 llamadas de la app.
 */
let unauthorizedHandler: (() => void) | null = null;

export function setUnauthorizedHandler(fn: (() => void) | null): void {
  unauthorizedHandler = fn;
}

/**
 * Error de una llamada a la API. `message` está en español y es lo que se enseña; `detail`
 * guarda el texto técnico del backend (inglés) y `code` el código estable con el que se tradujo.
 */
export class ApiRequestError extends Error {
  readonly code: string | null;
  readonly status: number;
  /** Mensaje técnico del backend, en inglés. Para plegar bajo «Detalles técnicos», no para leer. */
  readonly detail: string | null;

  constructor(message: string, opts: { code: string | null; status: number; detail: string | null }) {
    super(message);
    this.name = "ApiRequestError";
    this.code = opts.code;
    this.status = opts.status;
    this.detail = opts.detail;
  }
}

/**
 * `fetch` que nunca deja escapar el error crudo del navegador. Sin este envoltorio, con el API
 * caído el `TypeError` de la plataforma llegaba tal cual a la UI y el usuario leía
 * «Failed to fetch» — en inglés y sin decirle qué hacer, justo en el escenario en que más
 * necesita saber que el problema no es su dato sino el servidor.
 */
export async function apiFetch(url: string, init?: RequestInit): Promise<Response> {
  try {
    return await fetch(apiUrl(url), init);
  } catch (e: unknown) {
    const detail = e instanceof Error ? e.message : String(e);
    console.debug(`[api] network_error ${url}: ${detail}`);
    // `status: 0` = no hubo respuesta. Importa: el handler de 401 no debe dispararse aquí, un
    // corte de red no es una sesión caducada.
    return Promise.reject(
      new ApiRequestError(messageForError("network_error", null), {
        code: "network_error",
        status: 0,
        detail,
      }),
    );
  }
}

/** Construye el error traducido de una respuesta no-2xx. */
export async function apiErrorFromResponse(res: Response): Promise<ApiRequestError> {
  let code: string | null = null;
  let detail: string | null = null;
  const ct = res.headers.get("content-type") ?? "";
  if (ct.includes("application/json")) {
    try {
      const body = (await res.json()) as { code?: string; message?: string };
      if (typeof body.code === "string" && body.code.length > 0) code = body.code;
      if (typeof body.message === "string" && body.message.length > 0) detail = body.message;
    } catch {
      /* cuerpo ilegible: nos quedamos con el status */
    }
  }
  // El detalle técnico no se pierde aunque nadie lo pinte: sin esto, depurar un 400 desde el
  // navegador obligaría a abrir la pestaña de red y repetir la petición.
  if (detail) console.debug(`[api] ${res.status} ${code ?? "sin código"}: ${detail}`);
  if (res.status === 401) unauthorizedHandler?.();
  return new ApiRequestError(messageForError(code, res.status), {
    code,
    status: res.status,
    detail,
  });
}

/**
 * Compat: devuelve solo la frase en español. Se conserva porque hay llamadas que solo quieren
 * el texto; si necesitas el código o el detalle, usa `apiErrorFromResponse`.
 */
export async function errorMessageFromResponse(res: Response): Promise<string> {
  return (await apiErrorFromResponse(res)).message;
}

async function ensureOk(res: Response): Promise<Response> {
  if (!res.ok && res.status !== 204) {
    throw await apiErrorFromResponse(res);
  }
  return res;
}

async function parseJsonIfPresent<T>(res: Response): Promise<T | null> {
  if (res.status === 204) return null;
  const ct = res.headers.get("content-type") ?? "";
  if (!ct.includes("application/json")) return null;
  return (await res.json()) as T;
}

export async function apiGet<T>(url: string): Promise<T> {
  const res = await apiFetch(url, defaultFetchInit);
  await ensureOk(res);
  const data = await parseJsonIfPresent<T>(res);
  if (data === null) {
    // Respuesta 2xx sin JSON: o el proxy devolvió HTML, o la ruta no existe en el backend.
    throw new ApiRequestError("La respuesta del servidor no es válida. Comprueba que la API está en marcha.", {
      code: "empty_json_body",
      status: res.status,
      detail: `Expected JSON body from GET ${url}`,
    });
  }
  return data;
}

export async function apiPost<T>(url: string, body: unknown): Promise<T | null> {
  const res = await apiFetch(url, {
    ...defaultFetchInit,
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiPatch<T>(url: string, body: unknown): Promise<T | null> {
  const res = await apiFetch(url, {
    ...defaultFetchInit,
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiPut<T>(url: string, body: unknown): Promise<T | null> {
  const res = await apiFetch(url, {
    ...defaultFetchInit,
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiDelete(url: string): Promise<void> {
  const res = await apiFetch(url, {
    ...defaultFetchInit,
    method: "DELETE",
  });
  await ensureOk(res);
}

/** DELETE que SÍ devuelve cuerpo (p. ej. `/v1/transactions/{id}/reconcile` → las dos patas).
 *  `null` cuando la respuesta es 204 o no es JSON, igual que el resto de wrappers. */
export async function apiDeleteJson<T>(url: string): Promise<T | null> {
  const res = await apiFetch(url, {
    ...defaultFetchInit,
    method: "DELETE",
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}
