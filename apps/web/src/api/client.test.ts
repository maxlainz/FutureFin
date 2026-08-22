import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  apiDelete,
  apiGet,
  apiPatch,
  apiPost,
  apiErrorFromResponse,
  ApiRequestError,
  defaultFetchInit,
} from "./client";

const originalFetch = globalThis.fetch;

beforeEach(() => {
  globalThis.fetch = vi.fn();
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

function mockResponse(opts: {
  status?: number;
  body?: unknown;
  contentType?: string;
}): Response {
  const status = opts.status ?? 200;
  const ct = opts.contentType ?? (opts.body !== undefined ? "application/json" : "");
  const headers = new Headers(ct ? { "content-type": ct } : {});
  // El constructor de Response prohíbe body en respuestas 204/205/304.
  if (status === 204 || status === 205 || status === 304) {
    return new Response(null, { status, headers });
  }
  const bodyText = opts.body === undefined
    ? ""
    : typeof opts.body === "string"
      ? opts.body
      : JSON.stringify(opts.body);
  return new Response(bodyText, { status, headers });
}

describe("defaultFetchInit", () => {
  it("always includes credentials", () => {
    expect(defaultFetchInit.credentials).toBe("include");
  });
});

describe("apiErrorFromResponse", () => {
  it("traduce el código estable al español y guarda el técnico en detail", async () => {
    const res = mockResponse({
      status: 409,
      body: { code: "username_taken", message: "that username is already registered" },
    });
    const err = await apiErrorFromResponse(res);
    expect(err).toBeInstanceOf(ApiRequestError);
    expect(err.message).toBe("Ese nombre de usuario ya está registrado. Elige otro.");
    expect(err.code).toBe("username_taken");
    expect(err.status).toBe(409);
    expect(err.detail).toBe("that username is already registered");
  });

  it("con un código desconocido cae al status, nunca al mensaje inglés", async () => {
    const res = mockResponse({
      status: 403,
      body: { code: "codigo_que_no_existe_en_el_catalogo", message: "forbidden" },
    });
    const err = await apiErrorFromResponse(res);
    expect(err.message).toBe("No tienes permisos para hacer esto.");
    expect(err.message).not.toContain("forbidden");
    // El técnico sigue disponible para depurar.
    expect(err.detail).toBe("forbidden");
  });

  it("sin cuerpo JSON usa solo el código de estado", async () => {
    const res = mockResponse({ status: 502, body: "bad gateway", contentType: "text/plain" });
    const err = await apiErrorFromResponse(res);
    expect(err.message).toBe(
      "No se ha podido contactar con el servidor. Comprueba que sigue en marcha.",
    );
    expect(err.code).toBeNull();
    expect(err.detail).toBeNull();
  });

  it("un status sin frase propia cae al genérico", async () => {
    const res = mockResponse({ status: 418, body: { message: "teapot" } });
    const err = await apiErrorFromResponse(res);
    expect(err.message).toBe("Algo ha fallado. Inténtalo de nuevo en unos segundos.");
  });
});

describe("apiGet", () => {
  it("sends credentials: include and parses JSON", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(mockResponse({ body: { foo: "bar" } }));
    const data = await apiGet<{ foo: string }>("/v1/test");
    expect(data).toEqual({ foo: "bar" });
    expect(fetchMock).toHaveBeenCalledWith("/v1/test", { credentials: "include" });
  });
  it("lanza el mensaje traducido en un 4xx", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(
      mockResponse({ status: 401, body: { code: "unauthorized", message: "authentication required" } }),
    );
    await expect(apiGet("/v1/secret")).rejects.toThrow(
      "Tu sesión ha caducado. Vuelve a iniciar sesión.",
    );
  });
});

describe("apiPost + apiPatch + apiDelete", () => {
  it("POST serializes body and sets Content-Type", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(mockResponse({ status: 201, body: { id: "abc" } }));
    const data = await apiPost<{ id: string }>("/v1/things", { name: "x" });
    expect(data).toEqual({ id: "abc" });
    expect(fetchMock).toHaveBeenCalledWith("/v1/things", {
      credentials: "include",
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "x" }),
    });
  });
  it("PATCH returns null on 204", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(mockResponse({ status: 204 }));
    const data = await apiPatch<{ id: string }>("/v1/things/1", { name: "y" });
    expect(data).toBeNull();
  });
  it("DELETE returns void on 204", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(mockResponse({ status: 204 }));
    await expect(apiDelete("/v1/things/1")).resolves.toBeUndefined();
  });
  it("PATCH throws on conflict", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(
      mockResponse({ status: 409, body: { code: "conflict", message: "resource conflict" } }),
    );
    await expect(apiPatch("/v1/things/1", {})).rejects.toThrow("Ese valor ya existe. Elige otro.");
  });
});
