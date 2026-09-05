/**
 * Geometría PURA de la tira de fases del chart de Proyección (5.0.0, D29 / §G del plan de #207).
 *
 * La tira que va bajo el eje X dice EN QUÉ FASE está cada tramo del horizonte («Trabajo», «Media
 * jornada», «Jubilado») y marca los dos hitos que no son fases: el inicio de la pensión y —solo
 * cuando la edad manda y no coinciden— el cruce del objetivo. Aquí vive el modelo; el SVG solo
 * pinta lo que estas funciones devuelven, igual que `cashflow-bars.ts` con las barras de
 * Movimientos.
 *
 * **Todo se razona en MESES (`month_index`), jamás en posiciones de array.** El servidor decima la
 * serie con `density=hybrid` (meses 0..12, luego anuales, más el último del horizonte), así que la
 * posición 13 de `points` es el mes 24: una transición en el mes 271 no tiene punto propio y
 * cualquier aritmética sobre índices la colocaría años fuera de sitio. La consecuencia comprobable
 * —y la que fija `phase-strip.test.ts`— es que estas funciones dan EXACTAMENTE el mismo resultado
 * con la serie mensual y con la decimada: no reciben `points` porque no los necesitan. El chart
 * traduce mes → píxel con su `xScale`, que también toma meses.
 *
 * Ver `.claude/frontend-structure.md` §«Índice de array ≠ mes».
 */

import type {
  HouseholdMemberProjectionApi,
  PhaseTransitionApi,
  ProjectionPhaseApi,
  RetirementTriggerApi,
} from "../api/types";
import { householdMemberColor, type ChartLegendItem } from "./chart-legend";

/** Ventana visible del chart, en meses de la rejilla (`startMonth` puede ser negativo: el chart se
 *  extiende al pasado con el histórico, donde no hay fases que pintar). */
export type MonthWindow = { startMonth: number; endMonth: number };

/** Copy es-ES de cada fase. `short` es la variante de móvil (≤640px), donde la etiqueta larga no
 *  cabe en un segmento de pocos píxeles. */
export const PHASE_LABELS: Record<
  ProjectionPhaseApi,
  { long: string; short: string }
> = {
  accumulating: { long: "Trabajo", short: "Trab." },
  partial: { long: "Media jornada", short: "½ jorn." },
  retired: { long: "Jubilado", short: "Jub." },
};

const PHASE_ORDER: Record<ProjectionPhaseApi, number> = {
  accumulating: 0,
  partial: 1,
  retired: 2,
};

function isKnownPhase(raw: string): raw is ProjectionPhaseApi {
  return raw === "accumulating" || raw === "partial" || raw === "retired";
}

/** Un tramo de la tira, ya recortado a la ventana visible. Todo en meses. */
export type PhaseSegment = {
  phase: ProjectionPhaseApi;
  /** Primer mes VISIBLE del tramo (recortado por la ventana). */
  startMonth: number;
  /** Último mes VISIBLE del tramo, **inclusive**. */
  endMonth: number;
  /** Mes en que la fase empieza de verdad, sin recortar — el borde que se rotula. Igual a
   *  `startMonth` salvo que la ventana empiece dentro del tramo. */
  transitionMonth: number;
  label: string;
  shortLabel: string;
};

/**
 * `phase_transitions` → tramos de la tira, en meses.
 *
 * Contrato del servidor: las fases son **monótonas**, siempre arrancan con `accumulating` en el
 * mes 0 y la que no ocurre no aparece. Esta función no confía en ello para no romperse con una
 * respuesta rara: ordena, descarta literales desconocidos, colapsa repeticiones de la misma fase y,
 * cuando dos transiciones caen en el mismo mes, se queda con la MÁS AVANZADA (jubilarse el mismo
 * mes en que empieza la media jornada es jubilarse: la fase posterior gana; un tramo de cero meses
 * no se pinta).
 *
 * Cada tramo llega hasta el mes anterior a la siguiente transición; el último, hasta el final de
 * la ventana. Los tramos que caen enteros fuera de la ventana no salen.
 */
export function buildPhaseSegments(
  transitions: readonly PhaseTransitionApi[] | null | undefined,
  window: MonthWindow,
): PhaseSegment[] {
  if (!transitions || transitions.length === 0) return [];
  if (!Number.isFinite(window.startMonth) || !Number.isFinite(window.endMonth)) {
    return [];
  }
  if (window.endMonth < window.startMonth) return [];

  const clean = transitions
    .filter(
      (t): t is PhaseTransitionApi =>
        t != null &&
        isKnownPhase(String(t.phase)) &&
        Number.isFinite(t.month_index),
    )
    .map((t) => ({
      phase: t.phase,
      month: Math.max(0, Math.trunc(t.month_index)),
    }))
    .sort((a, b) =>
      a.month !== b.month
        ? a.month - b.month
        : PHASE_ORDER[a.phase] - PHASE_ORDER[b.phase],
    );
  if (clean.length === 0) return [];

  // Mismo mes ⇒ gana la fase más avanzada (ya ordenadas, así que la última);
  // fase repetida ⇒ se colapsa en el tramo anterior.
  const collapsed: { phase: ProjectionPhaseApi; month: number }[] = [];
  for (const t of clean) {
    const prev = collapsed[collapsed.length - 1];
    if (prev && prev.month === t.month) {
      collapsed[collapsed.length - 1] = t;
      continue;
    }
    if (prev && prev.phase === t.phase) continue;
    collapsed.push(t);
  }

  const out: PhaseSegment[] = [];
  for (let i = 0; i < collapsed.length; i++) {
    const t = collapsed[i]!;
    const next = collapsed[i + 1];
    const rawEnd = next ? next.month - 1 : window.endMonth;
    const startMonth = Math.max(t.month, window.startMonth);
    const endMonth = Math.min(rawEnd, window.endMonth);
    if (endMonth < startMonth) continue;
    const labels = PHASE_LABELS[t.phase];
    out.push({
      phase: t.phase,
      startMonth,
      endMonth,
      transitionMonth: t.month,
      label: labels.long,
      shortLabel: labels.short,
    });
  }
  return out;
}

/** La fase vigente en un mes dado, o `null` si ese mes queda fuera de los tramos (el pasado, por
 *  ejemplo). Lo usa el tooltip para decidir si un punto es «después de la jubilación». */
export function phaseAtMonth(
  segments: readonly PhaseSegment[],
  month: number,
): ProjectionPhaseApi | null {
  for (const s of segments) {
    if (month >= s.startMonth && month <= s.endMonth) return s.phase;
  }
  return null;
}

/** Marcas de la tira que no son fases. `member` solo aparece en la vista Hogar. */
export type PhaseMarkKind = "pension" | "crossing" | "member";

export type PhaseMark = {
  key: string;
  kind: PhaseMarkKind;
  /** Mes de la rejilla (`month_index`), nunca una posición de array. */
  month: number;
  label: string;
  /** Variante corta para móvil; igual al label cuando ya es corto. */
  shortLabel: string;
  /** Token de color (`var(--proj-*)`), nunca hex. */
  color: string;
};

/**
 * Las marcas de la tira, ordenadas por mes.
 *
 * - **Pensión** (`pension_start_month_index`): flecha con el rótulo «Pensión».
 * - **Cruce** (`liquid_crossing_month_index`): SOLO cuando la edad manda
 *   (`retirement_trigger === "target_age"`) **y** el cruce cae en un mes distinto del de la
 *   jubilación efectiva. Con `asap` los dos son el mismo mes por construcción, y rotular dos veces
 *   el mismo instante sugiere dos hechos donde hay uno. Es una LECTURA («el capital habría bastado
 *   aquí»), nunca un marcador vertical: los verticales son solo de la jubilación efectiva.
 * - **Miembros** (Hogar, D32): el mes de jubilación de cada miembro con su nombre, en el color que
 *   `householdMemberColor` reparte por posición — el MISMO de su línea fina y de su entrada de
 *   leyenda. La curva gruesa es la Σ y no se jubila: estas marcas son lo que dice de quién es cada
 *   hito.
 */
export function buildPhaseMarks(input: {
  pensionStartMonthIndex?: number | null;
  liquidCrossingMonthIndex?: number | null;
  retirementTrigger?: RetirementTriggerApi | null;
  retirementMonthIndex?: number | null;
  members?: readonly HouseholdMemberProjectionApi[] | null;
  window: MonthWindow;
}): PhaseMark[] {
  const { window } = input;
  if (!Number.isFinite(window.startMonth) || !Number.isFinite(window.endMonth)) {
    return [];
  }
  const visible = (m: number | null | undefined): m is number =>
    m != null &&
    Number.isFinite(m) &&
    m >= window.startMonth &&
    m <= window.endMonth;

  const marks: PhaseMark[] = [];

  if (visible(input.pensionStartMonthIndex)) {
    marks.push({
      key: "pension",
      kind: "pension",
      month: input.pensionStartMonthIndex,
      label: "Pensión",
      shortLabel: "Pensión",
      color: "var(--proj-meta)",
    });
  }

  if (
    input.retirementTrigger === "target_age" &&
    visible(input.liquidCrossingMonthIndex) &&
    input.liquidCrossingMonthIndex !== input.retirementMonthIndex
  ) {
    marks.push({
      key: "crossing",
      kind: "crossing",
      month: input.liquidCrossingMonthIndex,
      label: "Cruce",
      shortLabel: "Cruce",
      color: "var(--proj-fire)",
    });
  }

  (input.members ?? []).forEach((m, idx) => {
    if (!visible(m.retirement_month_index)) return;
    marks.push({
      key: `member-${m.user_id}`,
      kind: "member",
      month: m.retirement_month_index,
      label: m.username,
      shortLabel: m.username,
      color: householdMemberColor(idx),
    });
  });

  return marks.sort((a, b) => a.month - b.month);
}

/**
 * Entradas de leyenda de los miembros del hogar (D32): mismo modelo `ChartLegendItem` que el resto
 * del chart, mismo orden y mismos colores (`householdMemberColor`) que sus marcas en la tira y que
 * su línea fina, para que nombre, tick y curva se emparejen de un vistazo.
 *
 * `swatch: "line"` desde WP5-2/WP7-3b: cada miembro tiene ya una polyline fina propia sobre la Σ
 * en grueso (`lib/member-lines.ts`), así que la muestra dibuja lo que hay pintado. Mientras el
 * agregado no traía `members[].series`, lo único de cada persona era su tick y el swatch era
 * `dashed` — la muestra prometía una línea que no existía.
 */
export function buildHouseholdMemberLegendItems(
  members: readonly HouseholdMemberProjectionApi[] | null | undefined,
): ChartLegendItem[] {
  return (members ?? []).map((m, idx) => ({
    key: `member-${m.user_id}`,
    label: m.username,
    color: householdMemberColor(idx),
    swatch: "line" as const,
    title: `Patrimonio de ${m.username} · su jubilación, marcada en la tira de fases`,
  }));
}
