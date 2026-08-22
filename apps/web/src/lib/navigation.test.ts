/**
 * Congela el mapeo slug ↔ id de las sub-pestañas de Ajustes.
 *
 * En 3.10.0 se reorganizaron (ocho → siete, agrupadas por lo que el usuario quiere hacer y no por
 * dónde vive el dato). Los slugs de antes siguen resolviendo: un enlace guardado que no se
 * reconoce acaba en la primera sub-pestaña sin avisar, que es peor que un 404.
 */
import { describe, expect, it } from "vitest";
import {
  SETTINGS_SUBTAB_LABEL,
  SETTINGS_SUBTAB_SLUG,
  settingsSubTabFromPathname,
  settingsSubTabPath,
  tabFromPathname,
} from "./navigation";

describe("settings sub-tabs", () => {
  it("la sub-pestaña General es la primera y tiene ruta propia", () => {
    expect(SETTINGS_SUBTAB_SLUG.general).toBe("general");
    expect(SETTINGS_SUBTAB_LABEL.general).toBe("General");
    expect(settingsSubTabPath("general")).toBe("/ajustes/general");
    expect(settingsSubTabFromPathname("/ajustes/general")).toBe("general");
  });

  it("todo el plan vive en una sola sub-pestaña", () => {
    expect(SETTINGS_SUBTAB_LABEL.plan).toBe("Plan");
    expect(settingsSubTabFromPathname("/ajustes/plan")).toBe("plan");
  });

  it("los slugs de antes de 3.10.0 siguen resolviendo", () => {
    // Sin esto, un enlace guardado caería en la primera sub-pestaña sin decir nada.
    expect(settingsSubTabFromPathname("/ajustes/acceso")).toBe("access");
    expect(settingsSubTabFromPathname("/ajustes/mcp")).toBe("integrations");
    expect(settingsSubTabFromPathname("/ajustes/calendario")).toBe("general");
    // «Proyección» y «Jubilación» eran mitades del mismo concepto: ahora las recoge «Plan».
    expect(settingsSubTabFromPathname("/ajustes/proyeccion")).toBe("plan");
    expect(settingsSubTabFromPathname("/ajustes/jubilacion")).toBe("plan");
  });

  it("slug desconocido → null (App cae a la sub-tab por defecto)", () => {
    expect(settingsSubTabFromPathname("/ajustes/nada")).toBeNull();
    expect(settingsSubTabFromPathname("/resumen")).toBeNull();
  });

  it("cualquier /ajustes/* resuelve a la pestaña settings", () => {
    expect(tabFromPathname("/ajustes/integraciones")).toBe("settings");
    expect(tabFromPathname("/ajustes")).toBe("settings");
  });

  it("todos los slugs son únicos y redondean por settingsSubTabPath", () => {
    const slugs = Object.values(SETTINGS_SUBTAB_SLUG);
    expect(new Set(slugs).size).toBe(slugs.length);
    for (const id of Object.keys(SETTINGS_SUBTAB_SLUG) as (keyof typeof SETTINGS_SUBTAB_SLUG)[]) {
      expect(settingsSubTabFromPathname(settingsSubTabPath(id))).toBe(id);
    }
  });
});
