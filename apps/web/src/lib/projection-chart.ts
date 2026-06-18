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

export function buildProjectionMonthTickIndices(
  mc: number,
  maxTicks: number,
): number[] {
  if (mc <= 0) {
    return [0];
  }
  const cap = Math.max(4, Math.min(maxTicks, 22));
  const roughStep = Math.ceil(mc / Math.max(1, cap - 1));
  let step = roughStep;
  if (mc > 36) {
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
  if (ticks[ticks.length - 1] !== mc) {
    ticks.push(mc);
  }
  return ticks;
}

export function buildProjectionTicksFirstMonthOfYear(
  anchor: { y: number; m: number; d: number },
  monthEnd: number,
): number[] {
  if (monthEnd < 1) return [];
  const end = addMonthsCivil(anchor.y, anchor.m, anchor.d, monthEnd);
  const out: number[] = [];
  for (let y = anchor.y + 1; y <= end.y; y++) {
    const mi = (y - anchor.y) * 12 + (1 - anchor.m);
    if (mi < 1 || mi > monthEnd) continue;
    out.push(mi);
  }
  return out;
}

export function buildProjectionTicksFirstMonthOfAge(
  anchor: { y: number; m: number; d: number },
  birth: { y: number; m: number; d: number },
  monthEnd: number,
): number[] {
  if (monthEnd < 1) return [];
  const out: number[] = [];
  let prevAge = ageCompletedYearsCivil(anchor, birth);
  for (let i = 1; i <= monthEnd; i++) {
    const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, i);
    const age = ageCompletedYearsCivil(at, birth);
    if (age !== prevAge) {
      out.push(i);
      prevAge = age;
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
): { monthIndex: number; label: string }[] {
  const mcRaw = Number(monthCount);
  const mc = Number.isFinite(mcRaw) && mcRaw >= 0 ? mcRaw : 0;
  const pw = density?.plotWidthPx ?? 600;
  const minPx = opts?.ageUiMode === "dates" ? 34 : 28;
  const maxTicks = Math.max(5, Math.min(18, Math.floor(Math.max(120, pw) / minPx)));

  let ticks: number[] | null = null;

  if (opts) {
    const anchorStr =
      opts.anchorDateYmd != null && opts.anchorDateYmd.trim() !== ""
        ? opts.anchorDateYmd.trim()
        : todayYmdInTimeZone(opts.calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (opts.ageUiMode === "dates" && anchor) {
      ticks = buildProjectionTicksFirstMonthOfYear(anchor, mc);
    } else if (opts.ageUiMode === "ages" && anchor) {
      const birth = parseYmdComponents(opts.birthDateIso);
      if (birth) {
        ticks = buildProjectionTicksFirstMonthOfAge(anchor, birth, mc);
      }
    }
  }

  if (!ticks) {
    ticks = buildProjectionMonthTickIndices(mc, maxTicks);
  }

  return ticks.map((m) => ({
    monthIndex: m,
    label: projectionXTickLabel(m, mc, opts),
  }));
}

/** Geometría del SVG en unidades de usuario: depende del ancho CSS real del contenedor. */
export function buildProjectionChartLayout(
  containerCssWidth: number,
  containerCssHeight: number | undefined,
  legendLabels: string[],
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
  const ml = narrow
    ? Math.round(42 + W * 0.035)
    : Math.round(68 + Math.min(36, (W - 560) * 0.045));
  const mr = narrow ? 12 : Math.round(26 + Math.min(30, (W - 560) * 0.022));
  const mb = narrow ? 32 : 38;

  const pw = W - ml - mr;

  const legendRowHeight = 22;
  const legendItemGap = 16;
  // Estimación generosa del ancho del carácter (12px medium) — antes 6.5
  // subestimaba y la leyenda acababa solapándose.
  const legendCharPx = 7.6;
  const legendSwatchPx = 24 + 8;
  const legendRightAnchor = ml + pw;
  const legendBudgetWidth = Math.max(180, Math.round(pw * 0.66));
  const itemWidths = legendLabels.map(
    (label) => legendSwatchPx + label.length * legendCharPx,
  );
  const rows: number[][] = [];
  {
    let currentRow: number[] = [];
    let currentRowWidth = 0;
    for (let i = 0; i < legendLabels.length; i++) {
      const itemW = itemWidths[i];
      const widthIfAdded =
        currentRow.length === 0
          ? itemW
          : currentRowWidth + legendItemGap + itemW;
      if (currentRow.length > 0 && widthIfAdded > legendBudgetWidth) {
        rows.push(currentRow);
        currentRow = [i];
        currentRowWidth = itemW;
      } else {
        currentRow.push(i);
        currentRowWidth = widthIfAdded;
      }
    }
    if (currentRow.length > 0) rows.push(currentRow);
  }
  const rowsNeeded = Math.max(1, rows.length);
  const placements: Array<{ x: number; y: number }> = new Array(
    legendLabels.length,
  );
  rows.forEach((row, rowIdx) => {
    const rowTotal = row.reduce(
      (sum, itemIdx, idx) =>
        sum + itemWidths[itemIdx] + (idx > 0 ? legendItemGap : 0),
      0,
    );
    let cursor = legendRightAnchor - rowTotal;
    for (const itemIdx of row) {
      placements[itemIdx] = { x: cursor, y: rowIdx * legendRowHeight };
      cursor += itemWidths[itemIdx] + legendItemGap;
    }
  });

  const legendBlockHeight = rowsNeeded * legendRowHeight;
  const legendVerticalPad = 18;
  const headlineBlockTopY = 34;
  const headlineBlockBottom = headlineBlockTopY + 40 + 14;
  const mt = Math.max(
    legendBlockHeight + legendVerticalPad * 2,
    headlineBlockBottom,
  );
  const legendY = Math.round((mt - legendBlockHeight) / 2);
  const ph = H - mt - mb;

  const legend = { x: 0, y: legendY };

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
    legend,
    legendPlacements: placements,
    legendRows: rowsNeeded,
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

export function formatYearsEsFromMonths(months: number): string {
  const y = Math.round(months / 12);
  return `${new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(y)} años`;
}
