/**
 * Primer test de `lib/navigation.ts` — nació con la sub-tab «MCP» (issue #3): congela el mapeo
 * slug ↔ id de las sub-tabs de Ajustes, incluido que el slug histórico `acceso` sobrevive al
 * renombrado del label a «Usuarios» (URLs guardadas).
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
  it("mcp tiene slug y label propios", () => {
    expect(SETTINGS_SUBTAB_SLUG.mcp).toBe("mcp");
    expect(SETTINGS_SUBTAB_LABEL.mcp).toBe("MCP");
    expect(settingsSubTabPath("mcp")).toBe("/ajustes/mcp");
    expect(settingsSubTabFromPathname("/ajustes/mcp")).toBe("mcp");
  });

  it("access se renombró a «Usuarios» pero conserva el slug histórico", () => {
    expect(SETTINGS_SUBTAB_LABEL.access).toBe("Usuarios");
    expect(SETTINGS_SUBTAB_SLUG.access).toBe("acceso");
    expect(settingsSubTabFromPathname("/ajustes/acceso")).toBe("access");
  });

  it("slug desconocido → null (App cae a la sub-tab por defecto)", () => {
    expect(settingsSubTabFromPathname("/ajustes/nada")).toBeNull();
    expect(settingsSubTabFromPathname("/resumen")).toBeNull();
  });

  it("cualquier /ajustes/* resuelve a la pestaña settings", () => {
    expect(tabFromPathname("/ajustes/mcp")).toBe("settings");
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
