/**
 * Congela el contrato del prefijo de subpath. Lo que más importa aquí es el caso SIN prefijo:
 * con `base === ""` todas las funciones son la identidad, que es lo que garantiza que la app
 * servida en la raíz (dev, compose) se comporte exactamente igual que antes de existir el
 * módulo. Lo segundo, la idempotencia: una ruta ya prefijada no se prefija dos veces.
 */
import { describe, expect, it } from "vitest";
import {
  apiUrlWith,
  normalizeBase,
  stripBaseWith,
  apiUrl,
  appUrl,
  stripBase,
  BASE_PATH,
  SSO_AVAILABLE,
} from "./basePath";

const HA = "/api/hassio_ingress/abc123";

describe("normalizeBase", () => {
  it("acepta una ruta absoluta y le quita la barra final", () => {
    expect(normalizeBase(HA)).toBe(HA);
    expect(normalizeBase(`${HA}/`)).toBe(HA);
    expect(normalizeBase(`${HA}///`)).toBe(HA);
    expect(normalizeBase("  /futurefin  ")).toBe("/futurefin");
  });

  it("degrada a «sin prefijo» cualquier valor que no sea una ruta absoluta", () => {
    expect(normalizeBase(undefined)).toBe("");
    expect(normalizeBase(null)).toBe("");
    expect(normalizeBase(42)).toBe("");
    expect(normalizeBase("")).toBe("");
    expect(normalizeBase("/")).toBe("");
    expect(normalizeBase("futurefin")).toBe("");
    expect(normalizeBase("https://evil.example/x")).toBe("");
    expect(normalizeBase("//evil.example/x")).toBe("");
  });
});

describe("apiUrlWith", () => {
  it("sin prefijo devuelve la ruta intacta", () => {
    expect(apiUrlWith("", "/v1/summary")).toBe("/v1/summary");
    expect(apiUrlWith("", "/v1/assets?view=mine")).toBe("/v1/assets?view=mine");
  });

  it("antepone el prefijo a una ruta absoluta", () => {
    expect(apiUrlWith(HA, "/v1/summary")).toBe(`${HA}/v1/summary`);
    expect(apiUrlWith(HA, "/")).toBe(`${HA}/`);
  });

  it("es idempotente: no duplica un prefijo ya presente", () => {
    expect(apiUrlWith(HA, `${HA}/v1/summary`)).toBe(`${HA}/v1/summary`);
    expect(apiUrlWith(HA, apiUrlWith(HA, "/v1/summary"))).toBe(
      `${HA}/v1/summary`,
    );
    expect(apiUrlWith(HA, HA)).toBe(HA);
  });

  it("no toca una ruta relativa", () => {
    expect(apiUrlWith(HA, "v1/summary")).toBe("v1/summary");
  });
});

describe("stripBaseWith", () => {
  it("sin prefijo devuelve el pathname intacto", () => {
    expect(stripBaseWith("", "/resumen")).toBe("/resumen");
    expect(stripBaseWith("", "/")).toBe("/");
  });

  it("quita el prefijo y trata la coincidencia exacta como la raíz", () => {
    expect(stripBaseWith(HA, `${HA}/resumen`)).toBe("/resumen");
    expect(stripBaseWith(HA, `${HA}/`)).toBe("/");
    expect(stripBaseWith(HA, HA)).toBe("/");
  });

  it("deja pasar un pathname que no empieza por el prefijo", () => {
    expect(stripBaseWith(HA, "/resumen")).toBe("/resumen");
    expect(stripBaseWith(HA, `${HA}extra/resumen`)).toBe(`${HA}extra/resumen`);
  });

  it("compone con apiUrlWith: escribir y volver a leer es la identidad", () => {
    expect(stripBaseWith(HA, apiUrlWith(HA, "/ajustes/general"))).toBe(
      "/ajustes/general",
    );
  });
});

describe("wrappers sobre el window del test", () => {
  it("sin `__FF_BASE__` inyectado, todo es passthrough", () => {
    expect(BASE_PATH).toBe("");
    expect(SSO_AVAILABLE).toBe(false);
    expect(apiUrl("/v1/summary")).toBe("/v1/summary");
    expect(appUrl("/resumen")).toBe("/resumen");
    expect(stripBase("/resumen")).toBe("/resumen");
  });
});
