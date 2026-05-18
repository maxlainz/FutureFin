import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  apiDelete,
  apiGet,
  apiPatch,
  apiPost,
  defaultFetchInit,
  errorMessageFromResponse,
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

describe("errorMessageFromResponse", () => {
  it("reads {message} from JSON body", async () => {
    const res = mockResponse({ status: 400, body: { message: "amount must be > 0" } });
    expect(await errorMessageFromResponse(res)).toBe("amount must be > 0");
  });
  it("falls back to HTTP status when not JSON", async () => {
    const res = mockResponse({ status: 500, body: "boom", contentType: "text/plain" });
    expect(await errorMessageFromResponse(res)).toBe("HTTP 500");
  });
  it("falls back to HTTP status when JSON has no message", async () => {
    const res = mockResponse({ status: 422, body: { error: "validation" } });
    expect(await errorMessageFromResponse(res)).toBe("HTTP 422");
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
  it("throws with body message on 4xx", async () => {
    const fetchMock = vi.mocked(globalThis.fetch);
    fetchMock.mockResolvedValueOnce(
      mockResponse({ status: 401, body: { message: "authentication required" } }),
    );
    await expect(apiGet("/v1/secret")).rejects.toThrow("authentication required");
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
      mockResponse({ status: 409, body: { message: "resource conflict" } }),
    );
    await expect(apiPatch("/v1/things/1", {})).rejects.toThrow("resource conflict");
  });
});
