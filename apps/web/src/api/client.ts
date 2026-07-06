/**
 * Cliente HTTP único para todas las llamadas a la API. Centraliza:
 *  - `credentials: "include"` para que el navegador envíe la cookie `ff_session`.
 *  - Extracción del mensaje de error del cuerpo JSON (`{ message: "..." }`) cuando la API
 *    responde con un código no exitoso.
 *  - `Content-Type: application/json` en métodos con body.
 *
 * Los wrappers `apiGet/Post/Patch/Delete` lanzan `Error` con el mensaje del backend cuando
 * la respuesta no es 2xx, devolviendo el JSON parseado en el caso bueno.
 */

export const defaultFetchInit: RequestInit = {
  credentials: "include",
};

/** Lee `{message}` del cuerpo JSON; si no es JSON o no hay mensaje, devuelve `HTTP <status>`. */
export async function errorMessageFromResponse(res: Response): Promise<string> {
  const ct = res.headers.get("content-type") ?? "";
  if (ct.includes("application/json")) {
    try {
      const body = (await res.json()) as { message?: string };
      if (typeof body.message === "string" && body.message.length > 0) {
        return body.message;
      }
    } catch {
      /* ignore */
    }
  }
  return `HTTP ${res.status}`;
}

async function ensureOk(res: Response): Promise<Response> {
  if (!res.ok && res.status !== 204) {
    throw new Error(await errorMessageFromResponse(res));
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
  const res = await fetch(url, defaultFetchInit);
  await ensureOk(res);
  const data = await parseJsonIfPresent<T>(res);
  if (data === null) throw new Error(`Expected JSON body from GET ${url}`);
  return data;
}

export async function apiPost<T>(url: string, body: unknown): Promise<T | null> {
  const res = await fetch(url, {
    ...defaultFetchInit,
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiPatch<T>(url: string, body: unknown): Promise<T | null> {
  const res = await fetch(url, {
    ...defaultFetchInit,
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiPut<T>(url: string, body: unknown): Promise<T | null> {
  const res = await fetch(url, {
    ...defaultFetchInit,
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  await ensureOk(res);
  return parseJsonIfPresent<T>(res);
}

export async function apiDelete(url: string): Promise<void> {
  const res = await fetch(url, {
    ...defaultFetchInit,
    method: "DELETE",
  });
  await ensureOk(res);
}
