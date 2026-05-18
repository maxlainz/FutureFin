/**
 * Mapeo pestaña ↔ ruta canónica. Compartido por App.tsx y vistas (RetirementView linkea a
 * Ajustes → Jubilación; SettingsView usa los slugs de sub-tab).
 */

export type TabId =
  | "summary"
  | "assets"
  | "liabilities"
  | "budget"
  | "upcoming"
  | "projection"
  | "retirement"
  | "settings";

export type SettingsSubTabId =
  | "access"
  | "calendar"
  | "projection"
  | "retirement"
  | "categories"
  | "data";

export const TABS: { id: TabId; label: string }[] = [
  { id: "summary", label: "Resumen" },
  { id: "assets", label: "Activos" },
  { id: "liabilities", label: "Pasivos" },
  { id: "budget", label: "Presupuesto" },
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
  upcoming: "/proximos",
  projection: "/proyeccion",
  retirement: "/jubilacion",
  settings: "/ajustes",
};

export const SETTINGS_SUBTAB_SLUG: Record<SettingsSubTabId, string> = {
  access: "acceso",
  calendar: "calendario",
  projection: "proyeccion",
  retirement: "jubilacion",
  categories: "categorias",
  data: "datos",
};

export const SETTINGS_SUBTAB_LABEL: Record<SettingsSubTabId, string> = {
  access: "Acceso",
  calendar: "Calendario",
  projection: "Proyección",
  retirement: "Jubilación",
  categories: "Categorías",
  data: "Datos y sistema",
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
  return null;
}

export function settingsSubTabPath(id: SettingsSubTabId): string {
  return `${TAB_PATH.settings}/${SETTINGS_SUBTAB_SLUG[id]}`;
}
