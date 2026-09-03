/**
 * Modelo PURO de la sección «Riesgo» de Jubilación (5.0.0, D22/D25/D28 y §B.5 del plan de #207):
 * el abanico de percentiles, el semáforo de éxito, la tabla de agotamiento por edad y las
 * lecturas que solo existen en algunas estrategias.
 *
 * Aquí no se dibuja nada y no se recalcula NADA del modelo: el veredicto, la probabilidad y las
 * medianas las decide el servidor (`GET /v1/projection/bands`) y este módulo se limita a
 * alinearlas, deflactarlas y traducirlas a copy. Que sea un módulo aparte con test es
 * deliberado — lo que aquí puede romperse en silencio son tres cosas, y ninguna se ve mirando
 * la pantalla:
 *
 *  1. **La alineación por `month_index`.** La banda viaja SIEMPRE a densidad `hybrid`; la serie
 *     determinista que el chart ya tiene cargada puede ser `monthly` (la segunda fase del
 *     two-phase de `App.tsx`). Las dos rejillas NO son la misma, así que emparejarlas por
 *     posición de array desplaza el abanico décadas — y el resultado sigue pareciendo un chart
 *     correcto. Todo se dibuja por MES; la posición no se usa jamás como si fuera un mes.
 *  2. **La deflactación.** «En dinero de hoy» tiene que aplicar el MISMO factor por mes a las
 *     tres bandas y a la línea determinista; deflactar solo una las separa y el abanico deja de
 *     contener a la línea.
 *  3. **El redondeo de la probabilidad.** «100 de cada 100 escenarios» con un plan que falla en
 *     alguno es exactamente la mentira silenciosa que esta app existe para no contar: el
 *     redondeo se topa a 99 mientras la probabilidad no sea 1 exacta (y a 1 mientras no sea 0).
 *
 * Y una regla de lectura que la copy no puede olvidar: **la mediana no es un camino**. El p50 de
 * cada mes se calcula ordenando los valores de ESE mes, así que la curva p50 no corresponde a
 * ninguna simulación real y no cumple ninguna identidad contable. Lo dice el `model_note` del
 * servidor y lo dice la ayuda `retirement.bands`.
 */

import type {
  DepletionProbabilityPointApi,
  ProjectionBandPointApi,
  ProjectionBandsApi,
  SuccessVerdictApi,
} from "../api/types";
import {
  METRIC_DASH,
  formatCurrencyOrDash,
  formatFractionAsPercent,
  parseDisplayDecimal,
} from "./format";
import { lastPointIndexAtOrBeforeMonth } from "./projection-chart";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Abanico
// ─────────────────────────────────────────────────────────────────────────────────────────────

/** Un punto de la banda, ya deflactado. `month` es un MES de la rejilla, nunca una posición. */
export type RiskFanBandPoint = {
  month: number;
  p10: number;
  p50: number;
  p90: number;
};

/** Un punto de la línea determinista, en su PROPIA rejilla (puede ser más densa que la banda). */
export type RiskFanLinePoint = { month: number; value: number };

export type RiskFanModel = {
  band: RiskFanBandPoint[];
  deterministic: RiskFanLinePoint[];
  /** Primer y último MES dibujables — la intersección de las dos rejillas. */
  monthStart: number;
  monthEnd: number;
  /** Rango de valores de TODO lo que se pinta (banda + línea), para escalar el eje Y. */
  valueMin: number;
  valueMax: number;
  /** Mes de la jubilación efectiva, si cae dentro de la ventana. `null` = no se marca. */
  retirementMonth: number | null;
};

export type RiskFanInput = {
  bandPoints: readonly ProjectionBandPointApi[];
  /** `points[]` de `/v1/projection/series` — a la densidad que el cliente tenga cargada. */
  seriesPoints: readonly { month_index: number; net_worth: number }[];
  /** Factor por el que se multiplica el importe NOMINAL de ese mes; `() => 1` en modo nominal.
   *  Es el mismo que deflacta el chart grande, y se aplica a las CUATRO series por igual. */
  deflator: (monthIndex: number) => number;
  /** `jubilacion_month_index` de la serie. `null`/fuera de ventana ⇒ sin marcador. */
  retirementMonthIndex?: number | null;
};

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/**
 * Banda + línea determinista → todo lo que el SVG necesita, en unidades de MES.
 *
 * Devuelve `null` cuando no hay nada dibujable (menos de dos puntos de banda, o ninguna
 * intersección con la serie): media banda sin línea, o una línea sin banda, se leerían como un
 * abanico degenerado en vez de como «no hay dato», que es lo que son.
 *
 * La ventana es la de la BANDA (`monthStart`/`monthEnd` de `bandPoints`) y la línea determinista
 * se recorta a ella con `lastPointIndexAtOrBeforeMonth`, nunca con un `slice` por longitud: con
 * dos densidades distintas ese `slice` corta por el mes equivocado sin avisar.
 */
export function buildRiskFan(input: RiskFanInput): RiskFanModel | null {
  const raw = input.bandPoints.filter((p) => finite(p.month_index));
  if (raw.length < 2) return null;

  const band: RiskFanBandPoint[] = [];
  for (const p of raw) {
    if (!finite(p.net_worth_p10) || !finite(p.net_worth_p50) || !finite(p.net_worth_p90)) {
      continue;
    }
    const f = input.deflator(p.month_index);
    band.push({
      month: p.month_index,
      p10: p.net_worth_p10 * f,
      p50: p.net_worth_p50 * f,
      p90: p.net_worth_p90 * f,
    });
  }
  if (band.length < 2) return null;
  band.sort((a, b) => a.month - b.month);
  const monthStart = band[0]!.month;
  const monthEnd = band[band.length - 1]!.month;

  // La determinista se recorta a la ventana de la banda POR MES. Los dos extremos importan: el
  // primer punto futuro de la serie es el mes 0 igual que el de la banda, y el último tiene que
  // ser el mismo mes o el abanico terminaría a la derecha de la línea (o al revés).
  const sortedSeries = input.seriesPoints
    .filter((p) => finite(p.month_index) && finite(p.net_worth))
    .slice()
    .sort((a, b) => a.month_index - b.month_index);
  const deterministic: RiskFanLinePoint[] = [];
  if (sortedSeries.length > 0) {
    const lastPos = lastPointIndexAtOrBeforeMonth(sortedSeries, monthEnd);
    for (let i = 0; i <= lastPos && i < sortedSeries.length; i++) {
      const p = sortedSeries[i]!;
      // `lastPointIndexAtOrBeforeMonth` devuelve 0 cuando el PRIMER punto ya se pasa (siempre hay
      // algo que pintar, por contrato): sin este segundo guard ese punto entraría fuera de la
      // ventana y la línea empezaría a la derecha del abanico.
      if (p.month_index < monthStart || p.month_index > monthEnd) continue;
      deterministic.push({
        month: p.month_index,
        value: p.net_worth * input.deflator(p.month_index),
      });
    }
  }

  let valueMin = Number.POSITIVE_INFINITY;
  let valueMax = Number.NEGATIVE_INFINITY;
  for (const b of band) {
    valueMin = Math.min(valueMin, b.p10, b.p50, b.p90);
    valueMax = Math.max(valueMax, b.p10, b.p50, b.p90);
  }
  for (const d of deterministic) {
    valueMin = Math.min(valueMin, d.value);
    valueMax = Math.max(valueMax, d.value);
  }

  const rm = input.retirementMonthIndex;
  const retirementMonth =
    finite(rm) && rm >= monthStart && rm <= monthEnd ? rm : null;

  return {
    band,
    deterministic,
    monthStart,
    monthEnd,
    valueMin,
    valueMax,
    retirementMonth,
  };
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Semáforo de éxito
// ─────────────────────────────────────────────────────────────────────────────────────────────

/** Los tres tonos que la app ya habla (`MetricCard`, `.plan-card`): verde no tiene piel propia
 *  —«en plan» es el estado normal—, ámbar y rojo sí. */
export type RiskTone = "ok" | "warn" | "danger";

/**
 * Veredicto del SERVIDOR → tono de la app. Es una traducción, no una decisión: el umbral y los
 * 10 puntos de margen de D28 los aplica `projection_bands.rs`, y recalcularlos aquí abriría la
 * puerta a que el tile y el fan chart discreparan sobre el mismo plan.
 *
 * Un literal desconocido cae a `ok` **sin piel**: un veredicto futuro no debe pintar de rojo
 * algo que nadie ha evaluado.
 */
export function successVerdictTone(
  verdict: SuccessVerdictApi | string | null | undefined,
): RiskTone {
  if (verdict === "red") return "danger";
  if (verdict === "amber") return "warn";
  return "ok";
}

/**
 * `success_probability` → «87 de cada 100 escenarios».
 *
 * Dos topes deliberados: mientras la probabilidad no sea 1 EXACTA el redondeo no llega a 100, y
 * mientras no sea 0 exacta no baja a 0. `0.999` es un plan que falla en uno de cada mil y
 * «100 de cada 100» lo diría infalible.
 */
export function formatSuccessScenarios(
  probability: string | null | undefined,
): string {
  const p = probability == null ? null : parseDisplayDecimal(String(probability));
  if (p == null || !Number.isFinite(p)) return METRIC_DASH;
  let n = Math.round(p * 100);
  if (n >= 100 && p < 1) n = 99;
  if (n <= 0 && p > 0) n = 1;
  return `${n} de cada 100 escenarios`;
}

/** «umbral 95 %» — el denominador del semáforo. Sin él, «87 de cada 100» no se puede auditar. */
export function formatSuccessThreshold(
  thresholdPct: number | null | undefined,
): string | undefined {
  if (!finite(thresholdPct)) return undefined;
  return `umbral ${thresholdPct} %`;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Tabla de agotamiento por edad
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type DepletionRow = {
  /** Clave estable de React: el mes, que es único en la tabla. */
  key: string;
  /** «a los 75» con fecha de nacimiento; «mes 240» sin ella — la cifra existe igual. */
  label: string;
  /** Ya formateada («12,0 %») o `—` si el servidor no publicó probabilidad. */
  value: string;
};

/**
 * `depletion_probability_by_age[]` → filas listas para pintar.
 *
 * Las filas SIN edad no se esconden: la probabilidad es real aunque no se pueda rotular con una
 * edad, y ocultarlas dejaría la tabla vacía a quien no ha puesto su fecha de nacimiento — que es
 * justo quien más necesita el aviso.
 */
export function buildDepletionRows(
  points: readonly DepletionProbabilityPointApi[] | null | undefined,
): DepletionRow[] {
  if (!Array.isArray(points)) return [];
  return points
    .filter((p) => finite(p.month_index))
    .map((p) => ({
      key: `dep-${p.month_index}`,
      label: finite(p.age) ? `a los ${p.age}` : `mes ${p.month_index}`,
      value: formatFractionAsPercent(p.probability),
    }));
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Lecturas que solo existen en algunas estrategias
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type RiskExtraRow = {
  key: string;
  label: string;
  value: string;
  /** Segunda línea, cuando el número necesita su base para no leerse mal. */
  detail?: string;
};

export type RiskExtraRowsInput = {
  bands: ProjectionBandsApi | null | undefined;
  /** ISO de la divisa (`""` degrada a número sin símbolo, como el resto de la app). */
  currencyIso: string;
  /** Rotulador de un mes de la rejilla → etiqueta del eje («2043», «a los 55»). Lo inyecta la
   *  vista: depende del modo del eje, de la fecha de nacimiento y de la zona horaria. */
  monthLabel: (monthIndex: number) => string;
  /** `withdrawal_rule.kind` GUARDADO. Con `fixed_real` las dos filas de recorte no se pintan:
   *  esa regla no tiene techo y sus dos cifras son 0 y 1 por construcción — publicarlas
   *  sugeriría que se midió algo. */
  withdrawalRuleKind: string | null | undefined;
};

/**
 * Las filas EXTRA del panel de riesgo, cada una condicionada a que su pregunta exista.
 *
 * `retirement_month_index_percentiles` y `underfunded_probability` son EXCLUYENTES por contrato
 * (el trigger dice cuál toca), así que nunca aparecen las dos: no hay que elegir aquí, basta con
 * respetar los `null` del servidor.
 */
export function buildRiskExtraRows(input: RiskExtraRowsInput): RiskExtraRow[] {
  const b = input.bands;
  if (!b) return [];
  const rows: RiskExtraRow[] = [];
  const money = (s: string | null | undefined) =>
    formatCurrencyOrDash(s, input.currencyIso);

  // ── «Jubilación probable» (solo trigger por cruce) ──────────────────────────────────────
  const pct = b.retirement_month_index_percentiles;
  if (pct) {
    const at = (m: number | null) =>
      finite(m) ? input.monthLabel(m) : "no se jubila";
    rows.push({
      key: "retirement_percentiles",
      label: "Jubilación probable",
      value: at(pct.p50),
      detail: `${at(pct.p10)} en el 10 % de mercados mejores · ${at(pct.p90)} en el 10 % peores`,
    });
  }

  // ── «Probabilidad de no llegar a la edad» (solo trigger por edad, D17 probabilístico) ────
  if (b.underfunded_probability != null) {
    rows.push({
      key: "underfunded_probability",
      label: "Probabilidad de no llegar a la edad",
      value: formatFractionAsPercent(b.underfunded_probability),
      detail: "escenarios que alcanzan tu edad objetivo por debajo del objetivo",
    });
  }

  // ── Las dos lecturas del RECORTE, que NO es fracaso (D24) ────────────────────────────────
  if (input.withdrawalRuleKind != null && input.withdrawalRuleKind !== "fixed_real") {
    rows.push({
      key: "months_below_need",
      label: "Meses con recorte (mediana)",
      value: `${b.months_below_need_p50}`,
      detail: "meses jubilados en que la regla retiró menos de lo que necesitabas",
    });
    rows.push({
      key: "withdrawal_to_need",
      label: "Retirada / gasto (mediana)",
      value: formatFractionAsPercent(b.withdrawal_to_need_ratio_p50),
      detail: "qué parte de la necesidad cubrió la regla; 100 % = la cubrió entera",
    });
  }

  // ── Colchón (P4). El importe es la mediana de un TOTAL MOVIDO, no un saldo ───────────────
  if (b.buffer_active) {
    const refills = b.buffer_refills_p50;
    rows.push({
      key: "buffer",
      label: "Colchón de caja",
      value: finite(refills) ? `${refills} meses con relleno` : METRIC_DASH,
      detail:
        b.buffer_refill_net_total_p50 != null
          ? `${money(b.buffer_refill_net_total_p50)} movidos al colchón (mediana entre escenarios, no un saldo)`
          : undefined,
    });
  }

  return rows;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Avisos y pie
// ─────────────────────────────────────────────────────────────────────────────────────────────

/**
 * `true` ⟺ hay bandas y **ningún activo declara volatilidad**: las tres coinciden con la línea
 * determinista y hay que decirlo. Un abanico plano sin este aviso se lee como certeza, que es
 * la lectura más cara posible de este chart.
 */
export function showsNoVolatilityNotice(
  bands: ProjectionBandsApi | null | undefined,
): boolean {
  return bands != null && bands.any_volatility_declared === false;
}

/**
 * Pie del panel: coste, tamaño de la muestra y semilla. No es adorno — sin los caminos, la
 * probabilidad no tiene precisión declarada; sin la semilla, no se puede reproducir el sorteo.
 * `computed_in_ms: 0` es un HIT de cache y se dice así, en vez de fingir «0 ms de cálculo».
 */
export function riskFootnote(bands: ProjectionBandsApi): string {
  const time =
    bands.computed_in_ms > 0
      ? `Calculado en ${bands.computed_in_ms} ms`
      : "Resultado en cache";
  return `${time} · ${bands.paths} caminos · semilla ${bands.seed}`;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// El KPI «Éxito del plan» del Resumen (D28)
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type SuccessTileModel = {
  value: string;
  /** Slot del paréntesis: el umbral, que es el denominador del semáforo. */
  parenthetical?: string;
  /** Segundo slot: por qué no hay cifra, cuando no la hay. */
  detail?: string;
  /** `default` en verde: «va bien» es el estado normal y no lleva piel propia. */
  tone: "default" | "warn" | "danger";
};

/** Copy de cada razón por la que el éxito no existe. Las tres son situaciones DISTINTAS y se
 *  dicen distintas: el hogar no tiene un plan, la proyección no se pudo calcular, y el sorteo
 *  falló con el plan intacto. Un `—` mudo las haría indistinguibles. */
const SUCCESS_ABSENT_ES: Record<string, string> = {
  household_aggregate: "solo en tu vista «Yo»",
  projection_unavailable: "no se pudo calcular tu proyección",
  bands_unavailable: "no se pudieron sortear los escenarios",
};

/**
 * `summary.plan` → la tarjeta «Éxito del plan», o `null` cuando el backend no publica el bloque
 * (anterior a WP6b) y por tanto no hay nada que enseñar — ni siquiera un guion, que se leería
 * como «tu plan no tiene éxito medible» en vez de «esta versión no lo mide».
 *
 * Cero aritmética: la probabilidad, el umbral y el veredicto vienen del MISMO sorteo que dibuja
 * la sección «Riesgo» de Jubilación (el servidor los sirve de su cache de bandas). Recalcular
 * aquí el semáforo con otra muestra enseñaría dos éxitos del mismo plan en la misma pantalla.
 */
export function summarySuccessTile(
  plan:
    | {
        success_probability?: string | null;
        success_threshold_pct?: number | null;
        success_verdict?: SuccessVerdictApi | string | null;
        success_absent_reason?: string | null;
        absent_reason?: string | null;
      }
    | null
    | undefined,
): SuccessTileModel | null {
  if (!plan) return null;
  const threshold = formatSuccessThreshold(plan.success_threshold_pct);
  if (plan.success_probability == null) {
    const reason = plan.success_absent_reason ?? plan.absent_reason ?? null;
    // Sin probabilidad NI razón el backend está publicando un hueco mudo: es exactamente el
    // caso en que no hay nada honesto que decir, así que la tarjeta no se pinta.
    if (reason == null) return null;
    return {
      value: METRIC_DASH,
      parenthetical: threshold,
      detail: SUCCESS_ABSENT_ES[reason] ?? "no disponible",
      tone: "default",
    };
  }
  const tone = successVerdictTone(plan.success_verdict);
  return {
    value: formatSuccessScenarios(plan.success_probability),
    parenthetical: threshold,
    tone: tone === "danger" ? "danger" : tone === "warn" ? "warn" : "default",
  };
}
