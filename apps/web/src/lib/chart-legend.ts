/**
 * Modelo puro de la leyenda de charts (ChartLegend): qué entradas hay, en qué orden,
 * con qué color y cuántas se ven colapsadas. Sin React — testeable en node (Vitest).
 *
 * Propiedad de color que sale gratis: el chart PINTA los activos por peak ascendente
 * y el color se fija por esa posición de pintado (`colorIndex`). La leyenda los lista
 * por peak DESCENDENTE, así que los primeros `cap` items visibles colapsados tienen
 * índices de pintado consecutivos → colores distintos entre sí mientras `cap ≤ 10`.
 * Con N > 10 la paleta se recicla globalmente (módulo 10), decisión aceptada.
 */

import { ASSET_LINE_COLORS } from "./projection-chart";
import { normalizeSearchText } from "./expenses";

export type ChartLegendSwatch = "line" | "dashed" | "area";

export type ChartLegendItem = {
  /** Key de React (para activos: el asset_id). */
  key: string;
  /** Copy es-ES ya resuelto, incluido el sufijo de owner cuando aplica. */
  label: string;
  /** SIEMPRE un token (`var(--proj-*)` / `var(--ff-*)`), nunca hex. */
  color: string;
  swatch: ChartLegendSwatch;
  /** Texto completo para `title=` si la etiqueta se trunca; default = label. */
  title?: string;
};

/**
 * Color de un miembro del hogar por su POSICIÓN en `members[]` (5.0.0, D32).
 *
 * Única definición del emparejamiento: la línea fina del chart, el tick de la tira de fases y la
 * entrada de leyenda de la misma persona salen todas de aquí. Mientras cada una calculaba su
 * color por su cuenta, bastaba con que una ordenara distinto para que el nombre de la leyenda
 * señalara la curva de otro — y un chart del hogar que atribuye el patrimonio a quien no es
 * exactamente el error que la vista agregada existe para no cometer.
 *
 * Reusa la paleta de activos (`ASSET_LINE_COLORS`, tokens `var(--proj-asset-*)`): un hogar no
 * tiene diez miembros, así que el reciclado módulo 10 es teórico.
 */
export function householdMemberColor(index: number): string {
  const n = ASSET_LINE_COLORS.length;
  const i = Number.isFinite(index) ? Math.max(0, Math.trunc(index)) : 0;
  return ASSET_LINE_COLORS[i % n]!;
}

/** Cap de activos visibles colapsados cuando no hay ancho medido (mini charts). */
export const DEFAULT_LEGEND_ASSET_CAP = 4;
/** Activos listados uno a uno en el tooltip antes de agregar en «Otros». */
export const TOOLTIP_ASSET_LIMIT = 5;

/**
 * Entradas estructurales del chart de proyección, siempre visibles y en este orden.
 *
 * `historyIsAssetsOnly` cambia la etiqueta del tramo pasado de «Histórico» a «Activos
 * (histórico)». No es cosmética: cuando el pasivo no está fotografiado entero, el servidor no
 * publica patrimonio neto histórico y el chart pinta activos. Ese tramo YA se dibuja con su propio
 * color, así que basta con que la leyenda diga qué es — llamarlo «Histórico» junto a un
 * «Patrimonio neto» invita a leer las dos mitades como la misma magnitud, que es exactamente el
 * error que se está corrigiendo.
 */
export function buildStructuralLegendItems(opts: {
  hasFire: boolean;
  hasHistory: boolean;
  historyIsAssetsOnly?: boolean;
  /** #136-5a: `false` = modo euros de hoy, donde la línea «aportado» está retirada (su cifra
   *  correcta no es computable desde la serie servida). Default `true` (nominal). */
  hasContributed?: boolean;
}): ChartLegendItem[] {
  const items: ChartLegendItem[] = [
    { key: "nw", label: "Patrimonio neto", color: "var(--proj-nw)", swatch: "line" },
  ];
  if (opts.hasContributed !== false) {
    items.push({
      key: "cc",
      label: "Capital aportado",
      color: "var(--proj-cc)",
      swatch: "dashed",
    });
  }
  if (opts.hasFire) {
    items.push({
      key: "fire",
      label: "Objetivo FIRE",
      color: "var(--proj-fire)",
      swatch: "dashed",
    });
  }
  if (opts.hasHistory) {
    items.push({
      key: "hist",
      label: opts.historyIsAssetsOnly ? "Activos (histórico)" : "Histórico",
      color: "var(--proj-nw-past)",
      swatch: "line",
      title: opts.historyIsAssetsOnly
        ? "Sin snapshots de pasivo no hay patrimonio neto histórico: el tramo pasado son tus activos."
        : undefined,
    });
  }
  return items;
}

/**
 * Reordena los activos para la leyenda de MAYOR a menor peak (los grandes primero:
 * son los que el ojo busca), conservando `colorIndex` = posición en el orden de
 * PINTADO recibido — que es la que fija el color del área. Empates por nombre asc.
 */
export function legendOrderByPeakDesc<T extends { name: string; peak: number }>(
  painted: readonly T[],
): { item: T; colorIndex: number }[] {
  return painted
    .map((item, colorIndex) => ({ item, colorIndex }))
    .sort((a, b) => {
      if (a.item.peak !== b.item.peak) return b.item.peak - a.item.peak;
      return a.item.name.localeCompare(b.item.name);
    });
}

/**
 * Items de leyenda de activo, ya ordenados. Color = `ASSET_LINE_COLORS[colorIndex % 10]`.
 *
 * Sufijo de owner («Cuenta corriente · Max») cuando el nombre está duplicado. El mapa
 * distingue tres estados por id:
 * - `string` → activo actual con owner resoluble.
 * - `null` → activo actual SIN owner resoluble (`owner_user_id` NULL/compartido, o su
 *   usuario ya no es miembro).
 * - ausente → serie solo-histórica (viene de snapshots, no existe en `/v1/assets`):
 *   nunca se sufija y NUNCA veta — el histórico multiplica series homónimas y si
 *   vetara, ningún grupo con pasado llevaría sufijo jamás.
 *
 * Regla todo-o-nada entre los activos ACTUALES del grupo: si uno de ellos no tiene
 * owner resoluble, el grupo entero queda sin sufijo (media etiqueta sufijada sugeriría
 * que la otra «no es de nadie»). Grupo de tamaño 1 → nunca sufijo. Mapa vacío/null →
 * sin sufijos (la leyenda funciona igual sin resolver owners).
 */
export function buildAssetLegendItems(
  entries: readonly { id: string; name: string; colorIndex: number }[],
  ownerNameByAssetId?: Readonly<Record<string, string | null | undefined>> | null,
): ChartLegendItem[] {
  const owners = ownerNameByAssetId ?? null;
  const groupSize = new Map<string, number>();
  for (const e of entries) {
    const k = normalizeSearchText(e.name).trim();
    groupSize.set(k, (groupSize.get(k) ?? 0) + 1);
  }
  const groupSuffixable = new Map<string, boolean>();
  if (owners) {
    for (const e of entries) {
      const k = normalizeSearchText(e.name).trim();
      if ((groupSize.get(k) ?? 0) < 2) continue;
      if (!(e.id in owners)) continue; // solo-histórica: ni sufija ni veta
      const owner = owners[e.id];
      const resolvable = typeof owner === "string" && owner.trim() !== "";
      groupSuffixable.set(k, (groupSuffixable.get(k) ?? true) && resolvable);
    }
  }
  return entries.map((e) => {
    const k = normalizeSearchText(e.name).trim();
    const owner = owners?.[e.id];
    const label =
      groupSuffixable.get(k) === true && typeof owner === "string" && owner
        ? `${e.name} · ${owner}`
        : e.name;
    return {
      key: e.id,
      label,
      color: ASSET_LINE_COLORS[e.colorIndex % ASSET_LINE_COLORS.length]!,
      swatch: "area" as const,
    };
  });
}

/**
 * Mapa asset_id → nombre de owner, uniendo GET /v1/assets con GET /v1/installation/members.
 * TODO activo actual tiene entrada; `null` = existe pero su owner no es resoluble (la
 * distinción ausente-vs-null es la que usa `buildAssetLegendItems` para no dejar que las
 * series solo-históricas veten el sufijo).
 */
export function assetOwnerNameById(
  assets: readonly { id: string; owner_user_id?: string | null }[],
  members: readonly { user_id: string; username: string }[],
): Record<string, string | null> {
  const nameByUser = new Map(members.map((m) => [m.user_id, m.username] as const));
  const out: Record<string, string | null> = {};
  for (const a of assets) {
    const owner = a.owner_user_id ? nameByUser.get(a.owner_user_id) : undefined;
    out[a.id] = owner ?? null;
  }
  return out;
}

/** Activos visibles con la leyenda colapsada, por ancho del contenedor (breakpoints canónicos). */
export function collapsedAssetLegendCap(containerWidthPx: number): number {
  if (containerWidthPx <= 640) return 3;
  if (containerWidthPx <= 720) return 4;
  return 6;
}

/**
 * Regla del colapso. `total ≤ cap+1` → todo visible: un chip «+1 más» ocuparía lo
 * mismo que el único item que esconde.
 */
export function applyLegendCollapse(
  total: number,
  cap: number,
): { visibleCount: number; hiddenCount: number } {
  const c = Math.max(1, Math.floor(cap));
  if (total <= c + 1) {
    return { visibleCount: Math.max(0, total), hiddenCount: 0 };
  }
  return { visibleCount: c, hiddenCount: total - c };
}

/**
 * Top-N de activos para el tooltip, por |valor| desc en el mes hovered, más el
 * agregado del resto. Descarta 0/NaN/undefined — importante: history-merge rellena
 * con ceros el futuro de los activos solo-históricos y no deben listarse.
 * `hiddenTotal` suma valores CRUDOS (no absolutos).
 */
export function topAssetTooltipRows(
  rows: readonly { id: string; label: string; value: number | null | undefined }[],
  limit: number = TOOLTIP_ASSET_LIMIT,
): {
  shown: { id: string; label: string; value: number }[];
  hiddenCount: number;
  hiddenTotal: number;
} {
  const usable: { id: string; label: string; value: number }[] = [];
  for (const r of rows) {
    const v = r.value;
    if (typeof v !== "number" || !Number.isFinite(v) || v === 0) continue;
    usable.push({ id: r.id, label: r.label, value: v });
  }
  usable.sort((a, b) => Math.abs(b.value) - Math.abs(a.value));
  const shown = usable.slice(0, Math.max(0, Math.floor(limit)));
  const hidden = usable.slice(shown.length);
  return {
    shown,
    hiddenCount: hidden.length,
    hiddenTotal: hidden.reduce((s, r) => s + r.value, 0),
  };
}
