/**
 * Aviso de alta de Jubilación (D33): se descarta una vez y no vuelve. El sesgo del módulo es
 * «ante la duda, enséñalo», así que lo que hay que fijar es que SOLO el valor que escribimos
 * cuenta como descartado — un `localStorage` con basura no puede silenciar el aviso para siempre.
 */

import { describe, expect, it } from "vitest";
import {
  RETIREMENT_INTRO_DISMISSED_STORAGE_KEY,
  isRetirementIntroDismissed,
  readRetirementIntroDismissed,
  persistRetirementIntroDismissed,
} from "./retirement-intro";

describe("isRetirementIntroDismissed", () => {
  it("solo «1» cuenta como descartado", () => {
    expect(isRetirementIntroDismissed("1")).toBe(true);
    expect(isRetirementIntroDismissed(" 1 ")).toBe(true);
  });

  it("ausencia o basura ⇒ se vuelve a enseñar", () => {
    expect(isRetirementIntroDismissed(null)).toBe(false);
    expect(isRetirementIntroDismissed(undefined)).toBe(false);
    expect(isRetirementIntroDismissed("")).toBe(false);
    expect(isRetirementIntroDismissed("0")).toBe(false);
    expect(isRetirementIntroDismissed("true")).toBe(false);
  });

  it("la clave sigue el espacio de nombres de la SPA", () => {
    expect(RETIREMENT_INTRO_DISMISSED_STORAGE_KEY).toBe(
      "futurefin-retirement-intro-dismissed",
    );
  });
});

describe("lectura y escritura sin `window`", () => {
  // Vitest corre en `node`: no hay `window`. Es justo el contrato que hay que fijar — las dos
  // funciones tienen que degradar sin lanzar (SSR, iframe con almacenamiento bloqueado).
  it("no lanzan y degradan a «enséñalo»", () => {
    expect(typeof globalThis.window).toBe("undefined");
    expect(() => persistRetirementIntroDismissed()).not.toThrow();
    expect(readRetirementIntroDismissed()).toBe(false);
  });
});
