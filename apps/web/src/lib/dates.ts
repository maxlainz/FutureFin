/**
 * Aritmética civil de fechas (sin zona horaria) e interpretación de `YYYY-MM-DD`. Mantenida
 * paralela al motor Rust (`crates/engine`) para que las previsualizaciones del cliente cuadren
 * con el servidor sin pedir al backend.
 */

import { DISPLAY_NUMBER_LOCALE } from "./format";

/** Fallback civil date cuando una IANA TZ es inválida. */
export function utcTodayYmd(): string {
  const d = new Date();
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** Fecha civil hoy en una IANA TZ (coincide con el cálculo del servidor). */
export function todayYmdInTimeZone(tz: string): string {
  const trimmed = tz.trim() || "UTC";
  try {
    const fmt = new Intl.DateTimeFormat("en-CA", {
      timeZone: trimmed,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });
    const parts = fmt.formatToParts(new Date());
    const y = parts.find((p) => p.type === "year")?.value;
    const m = parts.find((p) => p.type === "month")?.value;
    const d = parts.find((p) => p.type === "day")?.value;
    if (y && m && d) return `${y}-${m}-${d}`;
  } catch {
    /* unknown time zone */
  }
  return utcTodayYmd();
}

export function parseYmdUtc(ymd: string): Date {
  const [ys, ms, ds] = ymd.split("-").map((x) => Number(x));
  return new Date(Date.UTC(ys, ms - 1, ds));
}

/**
 * Suma `n` meses civiles al ancla, recortando el día al mes de destino — SIN encadenar: el
 * mes 7 desde el 31-08 vuelve a caer en el 31-03 aunque el 6.º cayera en el 28-02. Espejo de
 * `payment_interval_count` en `apps/api/src/handlers/liabilities.rs` (#123).
 */
export function addMonthsFromAnchorUtc(anchor: Date, n: number): Date {
  const y = anchor.getUTCFullYear();
  const m = anchor.getUTCMonth() + n;
  const day = anchor.getUTCDate();
  const dim = new Date(Date.UTC(y, m + 1, 0)).getUTCDate();
  const out = new Date(Date.UTC(y, m, 1));
  out.setUTCDate(Math.min(day, dim));
  return out;
}

/**
 * Cuenta los intervalos de pago entre `startYmd` y `endYmd` (ambos inclusive). Mensual usa
 * vencimientos `ancla + n meses` — cada uno recalculado desde el ancla (#123): encadenar
 * `addOneMonthUtc` degradaba el día 29-31 al pasar por un mes corto y contaba una cuota de
 * más (13 donde el recibo real gira 12). Semanal usa `ceil(días/7)`. Devuelve `null` si
 * fechas inválidas, fin < inicio, o más de 1.200 iteraciones (seguro contra infinite loops).
 */
export function paymentIntervalCountUtc(
  freq: "monthly" | "weekly",
  startYmd: string,
  endYmd: string,
): number | null {
  const start = parseYmdUtc(startYmd);
  const end = parseYmdUtc(endYmd);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
    return null;
  }
  if (end.getTime() < start.getTime()) return null;
  if (freq === "monthly") {
    let n = 0;
    while (addMonthsFromAnchorUtc(start, n).getTime() <= end.getTime()) {
      n += 1;
      if (n > 1200) return null;
    }
    return n;
  }
  const days = Math.floor((end.getTime() - start.getTime()) / 86400000) + 1;
  const di = Math.max(1, days);
  return Math.ceil(di / 7);
}

/** Parsea `YYYY-MM-DD` en componentes; null si falta o es inválido. */
export function parseYmdComponents(
  ymd: string | null | undefined,
): { y: number; m: number; d: number } | null {
  if (!ymd || typeof ymd !== "string") return null;
  const t = ymd.trim();
  const mm = /^(\d{4})-(\d{2})-(\d{2})/.exec(t);
  if (!mm) return null;
  const y = Number(mm[1]);
  const m = Number(mm[2]);
  const d = Number(mm[3]);
  if (
    !Number.isFinite(y) ||
    !Number.isFinite(m) ||
    !Number.isFinite(d) ||
    m < 1 ||
    m > 12 ||
    d < 1 ||
    d > 31
  ) {
    return null;
  }
  return { y, m, d };
}

/** `YYYY-MM-DD` → `DD/MM/YYYY`; devuelve el original si no parsea. */
export function formatDateDmy(ymd: string): string {
  const c = parseYmdComponents(ymd);
  if (!c) return ymd;
  return `${String(c.d).padStart(2, "0")}/${String(c.m).padStart(2, "0")}/${c.y}`;
}

/** `YYYY-MM-DD` → `DD/MM` (sin año, para celdas compactas en móvil); original si no parsea. */
export function formatDateDm(ymd: string): string {
  const c = parseYmdComponents(ymd);
  if (!c) return ymd;
  return `${String(c.d).padStart(2, "0")}/${String(c.m).padStart(2, "0")}`;
}

/** Número de días del mes civil `(y, m)` con `m` 1-based. */
export function civilDaysInMonth(y: number, m: number): number {
  return new Date(y, m, 0).getDate();
}

/** Suma `deltaMonths` meses civiles a `(y, m, d)`, truncando el día si supera el mes destino. */
export function addMonthsCivil(
  y: number,
  m: number,
  d: number,
  deltaMonths: number,
): { y: number; m: number; d: number } {
  const total = y * 12 + (m - 1) + deltaMonths;
  const ny = Math.floor(total / 12);
  const nm = (total % 12) + 1;
  const dim = civilDaysInMonth(ny, nm);
  const nd = Math.min(d, dim);
  return { y: ny, m: nm, d: nd };
}

/** Edad en años cumplidos (misma definición que `age_completed_years` en la API). */
export function ageCompletedYearsCivil(
  today: { y: number; m: number; d: number },
  birth: { y: number; m: number; d: number },
): number {
  const tb = birth.y * 10000 + birth.m * 100 + birth.d;
  const tt = today.y * 10000 + today.m * 100 + today.d;
  if (birth.y > today.y) return 0;
  if (tb > tt) return 0;
  let years = today.y - birth.y;
  if (
    today.m < birth.m ||
    (today.m === birth.m && today.d < birth.d)
  ) {
    years -= 1;
  }
  return years;
}

/** Año civil para etiquetas del eje X (modo fechas). */
export function formatProjectionAxisYear(civil: { y: number; m: number; d: number }): string {
  const dt = new Date(civil.y, civil.m - 1, civil.d);
  return new Intl.DateTimeFormat(DISPLAY_NUMBER_LOCALE, {
    year: "numeric",
  }).format(dt);
}

/** Tooltip mes/año civil (`MM/YYYY`). */
export function formatProjectionHoverMonthYear(civil: { y: number; m: number; d: number }): string {
  const mm = String(civil.m).padStart(2, "0");
  return `${mm}/${civil.y}`;
}
