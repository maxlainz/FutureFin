/**
 * Mapeo pestaña ↔ ruta canónica. Compartido por App.tsx y vistas (RetirementView linkea a
 * Ajustes → Jubilación; SettingsView usa los slugs de sub-tab).
 */

export type TabId =
  | "summary"
  | "assets"
  | "liabilities"
  | "budget"
  | "expenses"
  | "upcoming"
  | "projection"
  | "retirement"
  | "settings";

/**
 * Sub-pestañas de Ajustes, reorganizadas en 3.10.0.
 *
 * Antes eran ocho y estaban partidas por *dónde vive el dato* en vez de por *qué quiere hacer el
 * usuario*: la sub-pestaña «Jubilación» contenía SOLO los tramos de IRPF mientras el SWR y el
 * objetivo FIRE vivían en la PESTAÑA «Jubilación» — dos cosas con el mismo nombre y mitades del
 * mismo concepto. «Proyección» mezclaba un supuesto económico (inflación), una preferencia de
 * visualización (modo edad) y el modo del motor bajo una sola cabecera. Y el propietario
 * aterrizaba en «Usuarios» → «Nadie pendiente», mientras el resto aterrizaba en «MCP», la página
 * más técnica de la app.
 *
 * Ahora: todo el plan en un sitio, lo técnico agrupado en «Integraciones», y la primera pantalla
 * es la que casi todo el mundo viene a tocar.
 */
export type SettingsSubTabId =
  | "general"
  | "plan"
  | "categories"
  | "history"
  | "access"
  | "integrations"
  | "data";

export const TABS: { id: TabId; label: string }[] = [
  { id: "summary", label: "Resumen" },
  { id: "assets", label: "Activos" },
  { id: "liabilities", label: "Pasivos" },
  { id: "budget", label: "Presupuesto" },
  { id: "expenses", label: "Movimientos" },
  { id: "upcoming", label: "Próximos" },
  { id: "retirement", label: "Jubilación" },
  { id: "projection", label: "Proyección" },
  { id: "settings", label: "Ajustes" },
];

export const TAB_PATH: Record<TabId, string> = {
  summary: "/resumen",
  assets: "/activos",
  liabilities: "/pasivos",
  budget: "/presupuesto",
  expenses: "/movimientos",
  upcoming: "/proximos",
  projection: "/proyeccion",
  retirement: "/jubilacion",
  settings: "/ajustes",
};

export const SETTINGS_SUBTAB_SLUG: Record<SettingsSubTabId, string> = {
  general: "general",
  plan: "plan",
  categories: "categorias",
  history: "historico",
  access: "usuarios",
  integrations: "integraciones",
  data: "datos",
};

/**
 * Slugs de antes de la reorganización de 3.10.0 → sub-pestaña que los recoge ahora. Existe para
 * que un enlace guardado no acabe silenciosamente en la primera pestaña, que es lo que hace el
 * canonicalizador con una ruta que no reconoce.
 */
const LEGACY_SUBTAB_SLUGS: Record<string, SettingsSubTabId> = {
  acceso: "access",
  mcp: "integrations",
  calendario: "general",
  proyeccion: "plan",
  jubilacion: "plan",
};

export const SETTINGS_SUBTAB_LABEL: Record<SettingsSubTabId, string> = {
  general: "General",
  plan: "Plan",
  categories: "Categorías",
  history: "Histórico",
  access: "Usuarios",
  integrations: "Integraciones",
  data: "Copias de seguridad",
};

export function normalizeAppPath(pathname: string): string {
  const p = pathname.replace(/\/+$/, "") || "/";
  return p;
}

export function tabFromPathname(pathname: string): TabId | null {
  const p = normalizeAppPath(pathname);
  if (p === TAB_PATH.settings || p.startsWith(`${TAB_PATH.settings}/`)) {
    return "settings";
  }
  // Alias de lectura del renombrado «Gastos» → «Movimientos»: la ruta canónica es
  // /movimientos (la escribe la navegación), pero /gastos sigue resolviendo a la pestaña.
  if (p === "/gastos") return "expenses";
  const ids = Object.keys(TAB_PATH) as TabId[];
  for (const id of ids) {
    if (TAB_PATH[id] === p) return id;
  }
  return null;
}

export function settingsSubTabFromPathname(pathname: string): SettingsSubTabId | null {
  const p = normalizeAppPath(pathname);
  const prefix = `${TAB_PATH.settings}/`;
  if (!p.startsWith(prefix)) return null;
  const slug = p.slice(prefix.length);
  const entries = Object.entries(SETTINGS_SUBTAB_SLUG) as [
    SettingsSubTabId,
    string,
  ][];
  for (const [id, s] of entries) {
    if (s === slug) return id;
  }
  return LEGACY_SUBTAB_SLUGS[slug] ?? null;
}

export function settingsSubTabPath(id: SettingsSubTabId): string {
  return `${TAB_PATH.settings}/${SETTINGS_SUBTAB_SLUG[id]}`;
}
