/**
 * Helpers de chart de proyección: ticks del eje X (modo fechas / edades), geometría del SVG,
 * ticks Y "nice", subtítulo del horizonte, paleta de líneas por activo. Funciones puras.
 */

import type { InstallationAccess, ProjectionSeriesApi } from "../api/types";
import {
  addMonthsCivil,
  ageCompletedYearsCivil,
  formatProjectionAxisYear,
  formatProjectionHoverMonthYear,
  parseYmdComponents,
  todayYmdInTimeZone,
} from "./dates";
import { DISPLAY_NUMBER_LOCALE, METRIC_DASH } from "./format";

export const PROJECTION_FOCUS_STORAGE_KEY = "futurefin-projection-focus";
export const PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY =
  "futurefin-projection-inflation-adjusted";

/**
 * Paleta de las áreas/legend del chart de proyección. Las gráficas son la
 * única zona donde la app rompe el principio "base B/N + único acento":
 * para distinguir más de 2-3 series, el color es funcional. Sigue siendo
 * una paleta sobria, no decorativa.
 *
 * Cada entrada es una CSS var con variante claro/oscuro definida en
 * styles/theme.css. En oscuro las versiones son más claras para mantener
 * contraste sobre el fondo zinc-900.
 */
export const ASSET_LINE_COLORS = [
  "var(--proj-area-1)",
  "var(--proj-area-2)",
  "var(--proj-area-3)",
  "var(--proj-area-4)",
  "var(--proj-area-5)",
  "var(--proj-area-6)",
  "var(--proj-area-7)",
  "var(--proj-area-8)",
  "var(--proj-area-9)",
  "var(--proj-area-10)",
] as const;

export function normalizeAgeUiMode(raw: string | null | undefined): "dates" | "ages" {
  const t = (raw ?? "").trim().toLowerCase();
  return t === "ages" ? "ages" : "dates";
}

/** Prioriza `use_age_on_x_axis` del GET /v1/projection/series (fuente de verdad). */
export function resolveProjectionAxisAgeMode(
  series: ProjectionSeriesApi,
  installation: InstallationAccess | null,
): "dates" | "ages" {
  if (series.use_age_on_x_axis === true) {
    return "ages";
  }
  if (series.use_age_on_x_axis === false) {
    return "dates";
  }
  return normalizeAgeUiMode(
    series.show_age_mode ?? installation?.installation.show_age_mode,
  );
}

/**
 * Subtítulo bajo la gráfica de proyección: evita mostrar `horizon_years` como si fuera
 * una edad (es la duración en años de la vista). Preferimos edad objetivo + año de fin
 * de serie cuando procede.
 */
export function formatProjectionChartHorizonLine(series: ProjectionSeriesApi): string {
  const basis = series.horizon_basis;
  const spanYears = series.horizon_years;
  const anchorStr = series.anchor_date_ymd?.trim();
  const anchor = anchorStr ? parseYmdComponents(anchorStr) : null;
  const mc = series.months;

  const endCivil =
    anchor != null && mc >= 0
      ? addMonthsCivil(anchor.y, anchor.m, anchor.d, mc)
      : null;
  const endYearStr = endCivil ? formatProjectionAxisYear(endCivil) : null;

  switch (basis) {
    case "lifespan_90":
      if (endYearStr != null) {
        return `Horizonte 90 años · fin ${endYearStr}`;
      }
      return `Horizonte 90 años`;
    case "fallback_no_demographics":
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista · sin fecha de nacimiento`;
    case "months_override":
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista (meses explícitos)`;
    default:
      if (endYearStr != null) {
        return `${spanYears} años de vista · fin ${endYearStr}`;
      }
      return `${spanYears} años de vista`;
  }
}

/**
 * Deflactor multiplicativo del chart en el mes `monthIndex` (euros nominales → euros de hoy).
 * `annualPct ≤ 0` → 1 (sin ajuste). Keyed por `month_index` REAL, nunca por posición de array
 * (incidente v1.4.2). Ojo: `monthIndex` negativo (pasado) devuelve un factor > 1 — amplifica.
 */
export function deflationFactorAt(monthIndex: number, annualPct: number): number {
  return annualPct > 0
    ? 1 / Math.pow(1 + annualPct / 100, monthIndex / 12)
    : 1;
}

/**
 * Última POSICIÓN del array cuyo `month_index` no pasa de `maxMonth`.
 *
 * Existe porque con `density=hybrid` el servidor DIEZMA la serie (meses 0..12, 24, 36…): la
 * posición 13 es el mes 24, y `points.length` (~82) no es el número de meses. Todo lo que
 * recorte una ventana por un MES —`clampToMonth` del MiniProjection, el pie «hoy → fin» de
 * Jubilación— tiene que traducir mes → posición por aquí, nunca con un `Math.min(mes, len-1)`,
 * que en hybrid no recortaba nada.
 *
 * Devuelve `0` si el primer punto ya se pasa: siempre hay algo que pintar.
 */
export function lastPointIndexAtOrBeforeMonth(
  points: readonly { month_index: number }[],
  maxMonth: number,
): number {
  let last = 0;
  for (let i = 0; i < points.length; i++) {
    if (points[i]!.month_index > maxMonth) break;
    last = i;
  }
  return last;
}

export function buildProjectionMonthTickIndices(
  mc: number,
  maxTicks: number,
  startMonth = 0,
): number[] {
  // Con startMonth === 0 el comportamiento es idéntico al histórico (solo futuro).
  if (mc <= 0 && startMonth >= 0) {
    return [0];
  }
  const past = Math.min(0, startMonth);
  const span = Math.max(1, mc - past);
  const cap = Math.max(4, Math.min(maxTicks, 22));
  const roughStep = Math.ceil(span / Math.max(1, cap - 1));
  let step = roughStep;
  if (span > 36) {
    const yAligned = Math.max(12, Math.ceil(roughStep / 12) * 12);
    step = Math.min(12, yAligned);
  } else {
    const shortSteps = [1, 2, 3, 6, 12];
    step = shortSteps.find((s) => s >= roughStep) ?? roughStep;
  }
  const ticks: number[] = [0];
  for (let m = step; m < mc; m += step) {
    ticks.push(m);
  }
  if (mc > 0 && ticks[ticks.length - 1] !== mc) {
    ticks.push(mc);
  }
  // Cobertura del pasado (month_index negativos). El 0 ya está (divisor «Hoy»).
  for (let m = -step; m > startMonth; m -= step) {
    ticks.push(m);
  }
  if (startMonth < 0 && !ticks.includes(startMonth)) {
    ticks.push(startMonth);
  }
  return ticks.sort((a, b) => a - b);
}

export function buildProjectionTicksFirstMonthOfYear(
  anchor: { y: number; m: number; d: number },
  monthEnd: number,
  startMonth = 0,
): number[] {
  if (monthEnd < 1) return [];
  const end = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthEnd);
  // Extiende hacia atrás solo cuando hay pasado; con startMonth === 0 arranca en anchor.y (los
  // meses ≤ 0 quedan filtrados igual que antes → comportamiento idéntico).
  const startCivil =
    startMonth < 0
      ? addMonthsCivil(anchor.y, anchor.m, anchor.d, startMonth)
      : anchor;
  const out: number[] = [];
  for (let y = startCivil.y; y <= end.y; y++) {
    const mi = (y - anchor.y) * 12 + (1 - anchor.m);
    // Excluye el mes 0 (pertenece al divisor «Hoy») y lo que queda fuera del span.
    if (mi === 0 || mi < startMonth || mi > monthEnd) continue;
    out.push(mi);
  }
  return out;
}

export function buildProjectionTicksFirstMonthOfAge(
  anchor: { y: number; m: number; d: number },
  birth: { y: number; m: number; d: number },
  monthEnd: number,
  startMonth = 0,
): number[] {
  if (monthEnd < 1) return [];
  const lo = startMonth < 0 ? startMonth : 1;
  const out: number[] = [];
  for (let i = lo; i <= monthEnd; i++) {
    if (i === 0) continue; // el mes 0 pertenece al divisor «Hoy»
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, i);
    const prev = addMonthsCivil(anchor.y, anchor.m, anchor.d, i - 1);
    const age = ageCompletedYearsCivil(at, birth);
    const prevAge = ageCompletedYearsCivil(prev, birth);
    if (age !== prevAge) {
      out.push(i);
    }
  }
  return out;
}

export function projectionXTickLabel(
  monthIndex: number,
  monthCount: number,
  opts?: {
    ageUiMode: "dates" | "ages";
    birthDateIso?: string | null;
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
): string {
  const mc = Number(monthCount);
  const safeMc = Number.isFinite(mc) && mc >= 0 ? mc : 0;
  const relativeFallback =
    monthIndex === 0
      ? "Hoy"
      : safeMc <= 48
        ? `${monthIndex} m`
        : `${Math.round(monthIndex / 12)} a`;

  if (!opts) {
    return `Mes ${monthIndex}`;
  }

  const anchorStr =
    opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
      ? opts.anchorDateYmd.trim()
      : todayYmdInTimeZone(opts.calendarTz);
  const anchor = parseYmdComponents(anchorStr);

  if (opts.ageUiMode === "ages") {
    const birth = parseYmdComponents(opts.birthDateIso);
    if (!birth || !anchor) {
      return relativeFallback;
    }
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
    const age = ageCompletedYearsCivil(at, birth);
    return `${age} a`;
  }

  if (!anchor) {
    return `Mes ${monthIndex}`;
  }
  const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
  return formatProjectionAxisYear(at);
}

export function complementaryProjectionTickLabel(
  monthIndex: number,
  monthCount: number,
  primaryAgeMode: "dates" | "ages",
  opts: {
    birthDateIso?: string | null;
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
): string {
  const altMode = primaryAgeMode === "ages" ? "dates" : "ages";
  return projectionXTickLabel(monthIndex, monthCount, {
    ageUiMode: altMode,
    birthDateIso: opts.birthDateIso,
    anchorDateYmd: opts.anchorDateYmd,
    calendarTz: opts.calendarTz,
  });
}

export function projectionHoverTitle(
  monthIndex: number,
  ageUiMode: "dates" | "ages",
  userBirthDate: string | null,
  calendarTz: string,
  anchorDateYmd?: string | null,
): string {
  const anchorStr =
    anchorDateYmd != null && anchorDateYmd.trim() !== ""
      ? anchorDateYmd.trim()
      : todayYmdInTimeZone(calendarTz);
  const anchor = parseYmdComponents(anchorStr);
  if (!anchor) {
    return METRIC_DASH;
  }
  const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthIndex);
  const dateLabel = formatProjectionHoverMonthYear(at);
  const birth = parseYmdComponents(userBirthDate);
  const ageLabel = birth ? `${ageCompletedYearsCivil(at, birth)} años` : null;

  // En el hover mostramos siempre la unidad complementaria entre paréntesis:
  // en modo edad → fecha; en modo fecha → edad (si hay fecha de nacimiento).
  if (ageUiMode === "ages") {
    if (!ageLabel) return METRIC_DASH;
    return `${ageLabel} (${dateLabel})`;
  }
  return ageLabel ? `${dateLabel} (${ageLabel})` : dateLabel;
}

export function projectionXTicks(
  monthCount: number,
  opts?: {
    ageUiMode: "dates" | "ages";
    birthDateIso?: string | null;
    anchorDateYmd?: string | null;
    calendarTz: string;
  },
  density?: { plotWidthPx: number },
  startMonth = 0,
): { monthIndex: number; label: string }[] {
  const mcRaw = Number(monthCount);
  const mc = Number.isFinite(mcRaw) && mcRaw >= 0 ? mcRaw : 0;
  const pw = density?.plotWidthPx ?? 600;
  const maxTicks = projectionMaxXTicks(pw, opts?.ageUiMode ?? "dates");

  let ticks: number[] | null = null;

  if (opts) {
    const anchorStr =
      opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
        ? opts.anchorDateYmd.trim()
        : todayYmdInTimeZone(opts.calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (opts.ageUiMode === "dates" && anchor) {
      ticks = buildProjectionTicksFirstMonthOfYear(anchor, mc, startMonth);
    } else if (opts.ageUiMode === "ages" && anchor) {
      const birth = parseYmdComponents(opts.birthDateIso);
      if (birth) {
        ticks = buildProjectionTicksFirstMonthOfAge(anchor, birth, mc, startMonth);
      }
    }
  }

  if (!ticks) {
    ticks = buildProjectionMonthTickIndices(mc, maxTicks, startMonth);
  }

  return ticks.map((m) => ({
    monthIndex: m,
    label: projectionXTickLabel(m, mc, opts),
  }));
}

/**
 * Techo de etiquetas X que caben en un plot de `plotWidthPx`. En plots estrechos
 * (móvil, < 560) las etiquetas van rotadas y aun diezmadas se amontonan: se les
 * exige más aire por etiqueta → menos ticks.
 */
export function projectionMaxXTicks(
  plotWidthPx: number,
  ageUiMode: "dates" | "ages",
): number {
  const narrowPlot = plotWidthPx < 560;
  const minPx = ageUiMode === "dates" ? (narrowPlot ? 52 : 34) : narrowPlot ? 44 : 28;
  return Math.max(5, Math.min(18, Math.floor(Math.max(120, plotWidthPx) / minPx)));
}

/**
 * Diezmado por ancho de los ticks VISIBLES: los builders de año/edad emiten UN tick
 * por año (55 en un horizonte de 90) y en móvil salen amontonados e ilegibles. Debe
 * aplicarse DESPUÉS de recortar a la ventana visible (nunca sobre el horizonte
 * completo: la ventana filtraría los supervivientes y un zoom se quedaría sin
 * etiquetas). Se recorre desde el FINAL con paso fijo: el último tick visible
 * siempre queda etiquetado y todos los huecos son exactamente `step` (el resto de
 * la división cae en ticks iniciales, que simplemente pierden etiqueta).
 */
export function thinTicksFromEnd<T>(items: readonly T[], maxTicks: number): T[] {
  const cap = Math.max(1, Math.floor(maxTicks));
  if (items.length <= cap) return [...items];
  const step = Math.ceil(items.length / cap);
  const thinned: T[] = [];
  for (let i = items.length - 1; i >= 0; i -= step) {
    thinned.unshift(items[i]!);
  }
  return thinned;
}

/** Geometría del SVG en unidades de usuario: depende del ancho CSS real del contenedor.
 *  La leyenda ya NO participa: vive en HTML fuera del `<svg>` (`ChartLegend`), así que
 *  el margen superior es constante (solo la cabecera) sea cual sea el número de activos. */
export function buildProjectionChartLayout(
  containerCssWidth: number,
  containerCssHeight?: number,
  layoutOpts?: {
    /** Móvil: sin etiquetas del eje Y → el margen izquierdo se reduce a un borde
     *  mínimo y el plot gana todo ese ancho (estilo MiniProjection, pero navegable). */
    hideYAxisLabels?: boolean;
    /** Móvil: la 2.ª línea de meta («Dinero de hoy … · Δ regular …») no cabe en una
     *  sola y se parte en dos → la cabecera necesita una línea más de altura. */
    compactHeader?: boolean;
  },
) {
  const W = Math.max(300, Math.round(containerCssWidth));
  const aspect = 460 / 1040;
  let H = Math.round(W * aspect);
  if (
    containerCssHeight != null &&
    Number.isFinite(containerCssHeight) &&
    containerCssHeight > 0
  ) {
    H = Math.round(containerCssHeight);
  }
  H = Math.max(260, Math.min(H, 980));

  const narrow = W < 560;
  // Márgenes narrow recalibrados al hacer el viewBox == caja medida (antes el
  // `meet`-shrink centraba el dibujo y esas bandas laterales escondían que
  // «800 mil €» no cabía a la izquierda ni la última etiqueta X rotada a la derecha).
  const ml = layoutOpts?.hideYAxisLabels
    ? 16
    : narrow
      ? Math.round(50 + W * 0.035)
      : Math.round(68 + Math.min(36, (W - 560) * 0.045));
  const mr = narrow ? 22 : Math.round(26 + Math.min(30, (W - 560) * 0.022));
  const mb = narrow ? 32 : 38;

  const pw = W - ml - mr;

  const headlineBlockTopY = 34;
  // Cabecera (headline + 2 líneas de meta) + aire; compactHeader añade la 3.ª línea.
  // La etiqueta «Hoy» del divisor vive en la fila del eje X (no sobre el plot), así
  // que la cabecera no necesita aire extra para ella.
  const mt = headlineBlockTopY + (layoutOpts?.compactHeader ? 58 : 40) + 14;
  // Guarda: cuando la leyenda SVG inflaba `mt`, `ph` podía quedar casi nulo (o negativo)
  // sin nada que lo impidiera. El suelo garantiza un plot dibujable siempre.
  const ph = Math.max(80, H - mt - mb);

  return {
    W,
    H,
    ml,
    mr,
    mt,
    mb,
    pw,
    ph,
    narrow,
    headlineBlockTopY,
  };
}

export function niceYTicks(minV: number, maxV: number, tickCount: number): number[] {
  if (!Number.isFinite(minV) || !Number.isFinite(maxV)) return [0];
  if (minV === maxV) {
    const pad = Math.abs(minV) < 1 ? 1 : Math.abs(minV) * 0.05;
    return niceYTicks(minV - pad, maxV + pad, tickCount);
  }
  const span = maxV - minV;
  const rough = span / Math.max(2, tickCount - 1);
  const exp = Math.floor(Math.log10(rough));
  const frac = rough / 10 ** exp;
  const niceFrac = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 5 ? 5 : 10;
  const step = niceFrac * 10 ** exp;
  const lo = Math.floor(minV / step) * step;
  const hi = Math.ceil(maxV / step) * step;
  const out: number[] = [];
  for (let x = lo; x <= hi + step * 0.01; x += step) {
    out.push(Math.round(x / step) * step);
  }
  const dedup = [...new Set(out.map((v) => Number(v.toPrecision(12))))];
  return dedup.length > 8 ? dedup.filter((_, i) => i % 2 === 0) : dedup;
}

/**
 * «Ya alcanzado» / «5 meses» / «1 año» / «1 año y 5 meses» — nunca redondea a años sueltos: con
 * `Math.round(months/12)` el mes 17 salía «1 años» (ni exacto ni bien pluralizado) y el mes 5, que
 * es "ya casi", salía «0 años». `months <= 0` es el caso del clamp «ya jubilado» (#132): `0` es un
 * mes válido (el cruce es HOY), no la ausencia de cruce — ese caso ya se filtra antes de llamar.
 */
export function formatYearsEsFromMonths(months: number): string {
  if (months <= 0) return "Ya alcanzado";
  const fmt = (n: number) =>
    new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    }).format(n);
  if (months < 12) {
    return `${fmt(months)} ${months === 1 ? "mes" : "meses"}`;
  }
  const years = Math.floor(months / 12);
  const remMonths = months - years * 12;
  const yearsLabel = `${fmt(years)} ${years === 1 ? "año" : "años"}`;
  return remMonths === 0
    ? yearsLabel
    : `${yearsLabel} y ${fmt(remMonths)} ${remMonths === 1 ? "mes" : "meses"}`;
}
