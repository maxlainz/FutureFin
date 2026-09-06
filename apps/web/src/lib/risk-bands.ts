/**
 * Modelo PURO de la sección «Riesgo» de Jubilación (5.0.0, modelo v2 §2.4/§2.5 y §4 «Riesgo»):
 * el abanico de percentiles, el KPI de éxito y las lecturas que lo hacen auditable.
 *
 * Aquí no se dibuja nada y no se recalcula NADA del modelo: el veredicto, la probabilidad, el
 * intervalo de Wilson y las medianas las decide el servidor (`GET /v1/projection/bands`) y este
 * módulo se limita a alinearlas, deflactarlas y traducirlas a copy. Que sea un módulo aparte con
 * test es deliberado — lo que aquí puede romperse en silencio son cuatro cosas, y ninguna se ve
 * mirando la pantalla:
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
 *  4. **El SUJETO de la probabilidad.** En el modelo v2 el éxito ya no es «no agotar el capital»:
 *     un camino falla si la cartera no cubre un mes (F1), si la tasa inicial del mes de
 *     jubilación supera el tope (F2) o si el permitido de una regla por saldo se queda por debajo
 *     de la necesidad ordinaria (F3). El verbo de la casa para las tres es **«aguantar»**, y la
 *     copy lo lleva dentro, no en el popover.
 *
 * Y dos reglas de lectura que la copy no puede olvidar:
 *
 * - **La mediana no es un camino.** El p50 de cada mes se calcula ordenando los valores de ESE
 *   mes, así que la curva p50 no corresponde a ninguna simulación real y no cumple ninguna
 *   identidad contable. Lo dice el `model_note` del servidor y lo dice la ayuda `retirement.bands`.
 * - **«pp» no es «%».** El semiancho de Wilson viaja en PUNTOS PORCENTUALES: «±1,2 pp» sobre un
 *   95,0 % es «entre 93,8 % y 96,2 %», no «±1,2 % de 95». Por eso tiene formateador propio
 *   (`formatSamplingErrorPp`) y no comparte el de los porcentajes.
 */

import type {
  ProjectionBandPointApi,
  ProjectionBandsApi,
  SummaryPlanApi,
  SuccessVerdictApi,
} from "../api/types";
import {
  DISPLAY_NUMBER_LOCALE,
  METRIC_DASH,
  formatFractionAsPercent,
  formatPercentDisplay,
  parseDisplayDecimal,
} from "./format";
import type { HelpTextId } from "./helpTexts";
import { lastPointIndexAtOrBeforeMonth } from "./projection-chart";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Los ids de ayuda de las filas y las tarjetas del plan
// ─────────────────────────────────────────────────────────────────────────────────────────────

/**
 * Id de ayuda de una fila o de una tarjeta del plan: **exactamente los del catálogo**, sin
 * excepciones.
 *
 * Durante W6 fue `HelpTextId` más una lista de ids que el modelo v2 estrenaba y que `helpTexts.ts`
 * todavía no tenía, para que los módulos puros del rediseño compilaran antes que el catálogo. W8
 * escribió esos textos y la lista se borró: el alias sobrevive solo como nombre de dominio —lo
 * importan `retirement-tiles.ts` y las filas de riesgo—, y **no puede volver a ensancharse**. Un
 * alias más ancho que el catálogo es un permiso abierto para inventarse ids que el popover
 * enseñaría vacíos; si hace falta un id nuevo, el sitio donde se añade es el catálogo.
 */
export type PlanHelpTextId = HelpTextId;

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

/** Un RECUENTO (caminos sorteados), con separador de millar español. No es dinero ni
 *  porcentaje: no pasa por los formateadores de importe. */
function formatCount(n: number): string {
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    maximumFractionDigits: 0,
  }).format(n);
}

/** Fracción `[0,1]` que viaja como `number` (excepción chart-only) o como Decimal-string: las dos
 *  formas conviven en este contrato y ninguna de las dos puede caer a 0 por accidente. */
function fractionOf(v: string | number | null | undefined): number | null {
  if (v == null) return null;
  const n = typeof v === "number" ? v : parseDisplayDecimal(String(v));
  return n != null && Number.isFinite(n) ? n : null;
}

/**
 * Banda + línea determinista → todo lo que el SVG necesita, en unidades de MES.
 *
 * @deprecated 5.0.0 U1b — **sin consumidor de UI**. El rediseño funde los dos gráficos de
 * Jubilación en uno (U5): la banda entra ahora en `MiniProjection` como una lista de
 * `{month, p10, p90}` en euros NOMINALES, y la deflactación de las tres series (patrimonio,
 * capital necesario y banda) la aplica el chart una sola vez, que es lo que garantiza que el
 * abanico contenga a la línea. `RiskFanChart.tsx` se retiró con él. Se conserva esta función —con
 * su test— porque es donde vive la alineación de dos rejillas distintas por MES: si alguna vez
 * vuelve a hacer falta un abanico con su mediana y su determinista, la aritmética no hay que
 * volver a derivarla. Si al leer esto sigue sin consumidores, bórrala.
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
 * Veredicto del SERVIDOR → tono de la app. Es una traducción, no una decisión: el umbral del
 * perfil y el intervalo de Wilson los aplica `projection_bands.rs` (C3), y recalcularlos aquí
 * abriría la puerta a que el tile y el fan chart discreparan sobre el mismo plan.
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
 * Fracción → «de cada 100», con los DOS topes que impiden la mentira por redondeo: mientras la
 * probabilidad no sea 1 EXACTA el redondeo no llega a 100, y mientras no sea 0 exacta no baja a
 * 0. `0.999` es un plan que falla en uno de cada mil y «100 de cada 100» lo diría infalible.
 *
 * `null` ⟺ no hay cifra que contar (ni la hay que inventar).
 */
export function scenariosPerHundred(
  probability: string | number | null | undefined,
): number | null {
  const p = fractionOf(probability);
  if (p == null) return null;
  let n = Math.round(p * 100);
  if (n >= 100 && p < 1) n = 99;
  if (n <= 0 && p > 0) n = 1;
  return n;
}

/**
 * `success_of_plan` → «87,0 %» (V1 de la tercera vuelta de UX, 5.0.0).
 *
 * Antes esta función devolvía la ORACIÓN entera como valor del tile. La frase era correcta y la
 * tipografía del valor (`.metric-value`, mono, `tabular-nums`) es para «87,0 %», no para once
 * palabras: el owner la leyó como «demasiado texto para caber en una caja» (F2). La condición no
 * se pierde — baja al subtítulo (`successParenthetical`), que es el slot que SÍ envuelve.
 *
 * **Se pasa por `scenariosPerHundred` y no por `formatFractionAsPercent`** a propósito: ahí
 * viven los dos topes anti-mentira. `formatFractionAsPercent("0.9999")` imprimiría «100,0 %»
 * sobre un plan que falla en uno de cada diez mil, que es exactamente la mentira silenciosa que
 * esta app existe para no contar. **Efecto lateral asumido**: el porcentaje queda cuantizado a
 * unidades de «de cada 100», así que un `0,952` se imprime «95,0 %» y no «95,2 %». Es el precio
 * de los topes, y la precisión real del sorteo la declara `formatSamplingErrorPp` justo al lado
 * (±1,2 pp hace irrelevante la segunda cifra).
 *
 * Un decimal, como todo porcentaje de la casa (`design-system.md` §Formato de cifras).
 */
export function formatSuccessPercent(
  probability: string | number | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  if (n == null) return METRIC_DASH;
  return formatPercentDisplay(n);
}

/** «4 de cada 100» — la misma cifra sin sujeto, para las filas que ya lo llevan en el rótulo. */
export function formatScenariosPerHundred(
  probability: string | number | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  return n == null ? METRIC_DASH : `${n} de cada 100`;
}

/**
 * Semiancho del intervalo de Wilson → «±1,2 pp».
 *
 * **`pp` no es `%` y por eso esto no es `formatPercentAmount`.** Un «±1,2 %» sobre un 95,0 % se
 * leería como «±1,2 % DE 95», es decir ±1,14 puntos; lo que el servidor publica son PUNTOS
 * PORCENTUALES: el intervalo va de 93,8 a 96,2. La distinción es la misma que la regla de oro de
 * las unidades del contrato (`_pct` vs `_ratio`), y se rompe igual de silenciosamente.
 *
 * Un decimal, como todo porcentaje de la casa, y el sufijo lo pone la función — **nunca se
 * concatena a mano** (misma disciplina que `formatPercentAmount`, que también trae el suyo).
 *
 * `null` ⟺ el sorteo no publicó precisión: guion, no un «±0,0 pp» que afirmaría una muestra
 * infinita.
 */
export function formatSamplingErrorPp(pp: string | number | null | undefined): string {
  const n = fractionOf(pp);
  if (n == null) return METRIC_DASH;
  const abs = Math.abs(n);
  const digits = new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(abs);
  return `±${digits} pp`;
}

/**
 * Cota de la REGLA DE TRES, en porcentaje, para el caso «0 fallos de N» (§2.5, S5).
 *
 * Con cero fallos observados el estimador puntual es 100 % y no hay intervalo que calcular: lo
 * que la estadística sí da es una cota superior aproximada del riesgo real, `3/N`. Con 2.500
 * caminos son 0,12 %.
 *
 * **Deliberadamente hasta DOS decimales**, y es la única cifra de la app que se sale del decimal
 * único: `3/N` con los N que este contrato admite (500–5.000) vive siempre por debajo del 1 %, y
 * un decimal imprimiría «0,1 %» donde el número es 0,12 y «0,0 %» en cuanto N pase de 6.000 — un
 * riesgo cero que es justo lo que esta frase existe para negar. El mínimo sigue siendo un decimal,
 * así que la forma habitual («0,6 %») no cambia.
 */
function formatRuleOfThreePercent(paths: number): string {
  const pct = (3 / paths) * 100;
  const digits = new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 2,
  }).format(pct);
  return `${digits} %`;
}

/**
 * El SUBTÍTULO del tile de éxito: qué mide ese «87,0 %» y **contra qué listón** (C3).
 *
 * El umbral vuelve al subtítulo —donde V7 lo había quitado— porque en el modelo v2 vuelve a ser
 * del usuario, y además es lo que DEFINE la fecha: sin él, el mismo 95,0 % es «justo lo que pedí»
 * para uno y «cinco puntos de más» para otro, y la tarjeta no distingue los dos casos.
 *
 * En el 100 % la frase genérica sería una perífrasis de «no falla ninguno», así que se sustituye
 * por el recuento exacto MÁS la cota de la regla de tres: «0 de 2.500 escenarios fallan · el
 * riesgo real puede llegar al 0,12 %». Ese segundo trozo no es un adorno estadístico — es lo
 * único que impide leer un 100 % muestral como una certeza, y el owner cerró el umbral 100 % con
 * esa condición explícita (§2.5). `paths` solo viaja con las bandas (el bloque `plan` del Resumen
 * no lo publica): sin él se cae a la frase genérica en vez de inventar un denominador.
 *
 * `undefined` ⟺ no hay cifra que subtitular.
 */
export function successParenthetical(
  probability: string | number | null | undefined,
  thresholdPct?: number | null,
  paths?: number | null,
): string | undefined {
  const n = scenariosPerHundred(probability);
  if (n == null) return undefined;
  const bits =
    n >= 100 && finite(paths) && paths > 0
      ? [
          `0 de ${formatCount(paths)} escenarios fallan`,
          `el riesgo real puede llegar al ${formatRuleOfThreePercent(paths)}`,
        ]
      : ["de los escenarios aguantan"];
  // El umbral va SIEMPRE que se conozca, también en el 100 %: es el listón que fija la fecha, y
  // saber si el usuario pidió 100 o 95 cambia por completo cómo se lee un plan que aguanta todo.
  if (finite(thresholdPct)) bits.push(`umbral ${formatPercentDisplay(thresholdPct)}`);
  return bits.join(" · ");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Lecturas de segundo orden del panel de riesgo
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type RiskExtraRow = {
  key: string;
  label: string;
  value: string;
  /** Segunda línea, cuando el número necesita su base para no leerse mal. */
  detail?: string;
  /** Ayuda del catálogo, cuando la fila mide algo que su rótulo no puede explicar entero. La
   *  pinta la vista junto al rótulo; el escáner de `helpTexts.test.ts` ve esta forma de objeto. */
  helpId?: PlanHelpTextId;
};

export type RiskExtraRowsInput = {
  bands: ProjectionBandsApi | null | undefined;
};

/**
 * Los tres modos de fallo de §2.4, en el ORDEN FIJO de `failures_by_kind`. Cada rótulo dice lo
 * que le pasó al camino, no la sigla: «F2» no significa nada para quien mira la pantalla, y
 * «tasa inicial por encima del tope» sí — es además la única de las tres que ocurre en un solo
 * mes (el de la jubilación) y la que más sorprende, porque el dinero sigue ahí.
 */
const FAILURE_KIND_LABELS: readonly [string, string, string] = [
  "Se quedan sin dinero",
  "Tasa inicial por encima del tope",
  "La regla no cubre el gasto",
];

/**
 * Las filas EXTRA del panel de riesgo: lo que hace AUDITABLE el «Éxito del plan» de arriba.
 *
 * Cada una responde una pregunta que el número grande deja abierta: por qué fallan los que
 * fallan, cuánto se apretó el cinturón en los que aguantan, cuántos acaban fallando en algún
 * momento del horizonte, y qué garantiza la muestra.
 *
 * Ninguna se esconde por la regla de retirada. Hasta el pase de correcciones de 5.0.0 las dos
 * filas de cobertura se ocultaban con `fixed_real` porque medían solo el recorte de la REGLA —y
 * esa regla no tiene techo, así que sus cifras eran 0 y 1 por construcción. Desde §F miden también
 * **el gasto que la cartera no pudo financiar**, así que con `fixed_real` son perfectamente
 * informativas: la regla no recorta nunca, pero el dinero se puede acabar igual (un fixture medido
 * pasó de `1,0` a `0,0865` al dejar de ignorar el descubierto). Esconderlas ahí era esconder el
 * caso en que la cobertura tiene una sola causa y es la peor.
 *
 * ## La excepción: sin éxito que auditar no hay auditoría (A12)
 *
 * Cuando la respuesta trae `success_absent_reason`, el escenario sorteado **no lleva mes de
 * jubilación** y el motor solo clasifica fallos estando jubilado. Las cifras que alimentan estas
 * filas siguen viajando —el servidor no las esconde, porque describen la trayectoria del
 * patrimonio sin jubilarse— pero valen todas 0 o casi: `failures_by_kind` es `[0, 0, 0]`,
 * `failure_probability_by_age` trae una sola fila valiendo 0 y las dos de cobertura no tienen
 * meses jubilados que medir.
 *
 * Pintadas como siempre, esas filas dicen «cero fallos, cobertura entera, 0 % de escenarios
 * fallan»: **un plan impecable que no existe**, que es exactamente la lectura contraria a la
 * verdadera. No son cifras equivocadas, son cifras de otra pregunta. Así que en ese caso NO se
 * emite ninguna: se emite **una sola fila** que dice que no hay éxito que auditar y por qué. Sin
 * porcentaje —no hay ninguno honesto que poner— y sin adjetivos: «no hay fecha» no es una alarma
 * (nada se ha roto) ni una tranquilidad (nada aguanta), y la fila que lo cuenta tampoco.
 */
export function buildRiskExtraRows(input: RiskExtraRowsInput): RiskExtraRow[] {
  const b = input.bands;
  if (!b) return [];

  // Ver §«La excepción» del doc: los ceros de abajo son de un plan sin jubilación, no de un plan
  // seguro. La fila pasa por `successAbsentReasonEs` —la MISMA tabla que subtitula el KPI del
  // Resumen— para que las dos superficies no expliquen la misma ausencia con dos frases distintas.
  if (b.success_absent_reason != null) {
    return [
      {
        key: "success_absent",
        label: "Sin éxito que auditar",
        value: METRIC_DASH,
        detail: successAbsentReasonEs(b.success_absent_reason),
        helpId: "retirement.success",
      },
    ];
  }

  const rows: RiskExtraRow[] = [];

  // ── Por qué falla el que falla (§2.4) ────────────────────────────────────────────────────
  //
  // Las TRES filas o ninguna. Si no ha fallado nadie, tres «0 de 2.500» son ruido: el número
  // grande ya cuenta la historia entera. Si ha fallado alguien, un cero SÍ es una lectura («por
  // esto no falló ninguno») y además es lo que hace que las tres casillas cuadren con el total:
  // esconder las vacías dejaría un desglose que no suma.
  const kinds = Array.isArray(b.failures_by_kind) ? b.failures_by_kind : null;
  if (kinds && kinds.length === 3 && kinds.some((n) => finite(n) && n > 0)) {
    const total = finite(b.paths) && b.paths > 0 ? b.paths : null;
    kinds.forEach((count, i) => {
      if (!finite(count)) return;
      rows.push({
        key: `failure_kind_${i + 1}`,
        label: FAILURE_KIND_LABELS[i]!,
        value:
          total == null
            ? formatCount(count)
            : `${formatCount(count)} de ${formatCount(total)}`,
        helpId: i === 0 ? "retirement.success" : undefined,
      });
    });
  }

  // ── Cuánto se apretó el cinturón el que aguantó ──────────────────────────────────────────
  rows.push({
    key: "months_below_need",
    label: "Meses por debajo del gasto (mediana)",
    value: `${b.months_below_need_p50}`,
    detail: "meses jubilados en que gastaste menos de lo que necesitabas",
    helpId: "retirement.coverage",
  });
  rows.push({
    key: "withdrawal_to_need",
    label: "Parte del gasto que la regla cubrió (mediana)",
    value: formatFractionAsPercent(b.withdrawal_to_need_ratio_p50),
    detail:
      "por el techo de la regla y por lo que la cartera no dio; 100 % = el gasto entero",
    helpId: "retirement.coverage",
  });

  // ── El total acumulado, que el color de la banda no puede rotular ────────────────────────
  //
  // La tabla «agotar a los 65/70/…» desapareció con el degradado (V5): el color YA dice la
  // probabilidad edad a edad y con más resolución. Lo que el color no dice es el TOTAL, porque su
  // última parada es el borde derecho del plot y ahí no hay etiqueta.
  //
  // Se toma el ÚLTIMO punto de la rejilla porque la serie es acumulada por contrato (solo puede
  // crecer) y el último mes es el final del horizonte. `null` no se pinta: inventar un 0 % ahí
  // sería declarar un plan infalible a partir de un dato que no llegó.
  const failurePoints = Array.isArray(b.failure_probability_by_age)
    ? b.failure_probability_by_age.filter((p) => finite(p.month_index))
    : [];
  const failureLast = failurePoints[failurePoints.length - 1];
  if (failureLast != null && failureLast.probability != null) {
    rows.push({
      key: "failure_total",
      label: "Escenarios que fallan en algún momento",
      value: formatFractionAsPercent(String(failureLast.probability)),
      detail:
        "acumulado hasta el final del horizonte: es el mismo sorteo que colorea la banda del gráfico",
      helpId: "retirement.failure_by_age",
    });
  }

  // ── Qué garantiza la MUESTRA (C3) ────────────────────────────────────────────────────────
  //
  // El límite inferior de Wilson es la magnitud que de verdad decide el umbral, y es la única
  // que no se mueve con la semilla. Enseñar solo el estimador puntual invita a leer un 95,0 %
  // como un hecho cuando con 2.500 caminos puede ser un 93,8 %.
  const wilson = fractionOf(b.success_wilson_low);
  if (wilson != null) {
    rows.push({
      key: "success_wilson_low",
      label: "Con 95 % de confianza, al menos",
      value: formatFractionAsPercent(String(wilson)),
      detail:
        "límite inferior del intervalo sobre el éxito: es lo que se compara con tu umbral, no la cifra de arriba",
      helpId: "retirement.success_threshold",
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
 * la lectura más cara posible de este chart — y en el modelo v2 el precio sube: sin dispersión el
 * éxito sale 0 % o 100 % y deja de medir riesgo (aviso `no_volatility_declared`, C5).
 */
export function showsNoVolatilityNotice(
  bands: ProjectionBandsApi | null | undefined,
): boolean {
  return bands != null && bands.any_volatility_declared === false;
}

/**
 * `true` ⟺ el fallo por edad de estas bandas **puede teñir la banda del chart**. Es el permiso,
 * no el degradado: las paradas las calcula `riskGradientStops` (`lib/risk-gradient.ts`) y la
 * ventana la pone la vista.
 *
 * Dos vetos, y los dos existen porque su caso pinta la banda de VERDE ENTERO —el color que nadie
 * cuestiona— sobre un sorteo que no midió riesgo:
 *
 * - **sin volatilidad declarada**: las tres bandas SON la línea determinista, y un abanico de
 *   ancho cero teñido de verde dice «ningún escenario falla» sobre escenarios que no se
 *   dispersaron;
 * - **sin éxito que medir** (`success_absent_reason`, A12): el escenario sorteado no lleva mes de
 *   jubilación, el motor solo clasifica fallos estando jubilado y `failure_probability_by_age`
 *   llega en ceros. Ese cero es «sin jubilación no hay fallo que contar», no «riesgo cero».
 *
 * Vive aquí y no en la vista **para poder probarse**: es una decisión de contrato (qué autoriza a
 * pintar un juicio de riesgo), no una condición de layout, y escrita en un `useMemo` no había
 * forma de fijarla con un test.
 */
export function showsRiskGradient(
  bands: ProjectionBandsApi | null | undefined,
): boolean {
  if (bands == null) return false;
  if (bands.any_volatility_declared === false) return false;
  return bands.success_absent_reason == null;
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
// El KPI «Éxito del plan» del Resumen (D27, modelo v2)
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type SuccessTileModel = {
  value: string;
  /** Slot del paréntesis: el SUJETO de la cifra y el umbral contra el que se evaluó. */
  parenthetical?: string;
  /** Segundo slot: por qué no hay cifra, cuando no la hay. */
  detail?: string;
  /** `default` en verde: «va bien» es el estado normal y no lleva piel propia. */
  tone: "default" | "warn" | "danger";
};

/** Copy de cada razón por la que el éxito no existe. Son situaciones DISTINTAS y se dicen
 *  distintas: el hogar no tiene un plan, la proyección no se pudo calcular, el sorteo falló con el
 *  plan intacto, falta la fecha de nacimiento (sin edad no hay contra qué resolver nada, C5) y
 *  —desde A12— el plan existe pero **ninguna fecha del horizonte llega al umbral**. Un `—` mudo
 *  las haría indistinguibles.
 *
 *  `not_reachable` es la única de la lista que NO describe una carencia de datos: hay plan, hay
 *  sorteo y hay respuesta, y la respuesta es que no existe un mes en el que jubilarse cumpliendo
 *  el listón. Por eso la frase habla de la FECHA que falta y no de un cálculo que faltó.
 *
 *  `no_liquid_assets` se retiró de esta tabla en A12: nunca fue un literal de `absent_reason` ni
 *  de `success_absent_reason` —vive en `needed_capital_absent_reason`, que dice por qué falta una
 *  cifra, no por qué falta el éxito— y traducirlo aquí prometía una frase que ningún backend
 *  podía disparar. */
const SUCCESS_ABSENT_ES: Record<string, string> = {
  household_aggregate: "solo en tu vista «Yo»",
  household_not_solved: "solo en tu vista «Yo»",
  projection_unavailable: "no se pudo calcular tu proyección",
  bands_unavailable: "no se pudieron sortear los escenarios",
  birth_date_missing: "falta tu fecha de nacimiento",
  months_override: "esta vista fija un horizonte propio",
  not_reachable: "no hay ninguna fecha que llegue a tu umbral, así que no hay éxito que medir",
};

/**
 * La frase de una razón de ausencia del éxito — **la única traducción de la app**, para que el
 * tile del Resumen, las filas de Jubilación y la nota de precisión del sorteo no expliquen la
 * misma ausencia de tres maneras.
 *
 * Un literal que esta tabla no conoce cae a «no disponible» A PROPÓSITO: un backend más moderno
 * puede estrenar una razón, y lo honesto entonces es decir que falta, no inventarle una frase que
 * describiría una situación que nadie ha comprobado.
 */
export function successAbsentReasonEs(reason: string | null | undefined): string {
  return (reason != null ? SUCCESS_ABSENT_ES[reason] : undefined) ?? "no disponible";
}

/**
 * `summary.plan` → la tarjeta «Éxito del plan», o `null` cuando el backend no publica el bloque
 * y por tanto no hay nada que enseñar — ni siquiera un guion, que se leería como «tu plan no tiene
 * éxito medible» en vez de «esta versión no lo mide».
 *
 * Cero aritmética: la probabilidad, el umbral y el veredicto vienen del MISMO sorteo (la misma
 * cache de plan) que dibuja la sección «Riesgo» de Jubilación. Recalcular aquí el semáforo con
 * otra muestra enseñaría dos éxitos del mismo plan en la misma pantalla.
 *
 * Los TRES estados de `plan_state` se dicen distinto, y esa es la mitad del trabajo de esta
 * función:
 *
 * - **`pending`** — «calculando…». El nivel 1 del solve sigue en marcha (típicamente el primer GET
 *   tras una mutación). NO es un guion: un guion dice «no hay», y aquí lo que hay es una espera.
 * - **`absent`** — el guion CON su razón.
 * - **`ready`** — la cifra. Y si aun así falta el éxito (`success_absent_reason`), su razón: «no
 *   sabemos tu probabilidad» ≠ «no sabemos tu plan».
 */
export function summarySuccessTile(
  plan: Partial<SummaryPlanApi> | null | undefined,
): SuccessTileModel | null {
  if (!plan) return null;

  if (plan.plan_state === "pending") {
    return { value: METRIC_DASH, detail: "calculando…", tone: "default" };
  }

  if (plan.success_of_plan == null) {
    const reason = plan.success_absent_reason ?? plan.absent_reason ?? null;
    // Sin cifra NI razón el backend está publicando un hueco mudo: es exactamente el caso en que
    // no hay nada honesto que decir, así que la tarjeta no se pinta.
    if (reason == null) return null;
    return {
      value: METRIC_DASH,
      detail: successAbsentReasonEs(reason),
      tone: "default",
    };
  }

  const tone = successVerdictTone(plan.success_verdict);
  return {
    value: formatSuccessPercent(plan.success_of_plan),
    // Sin `paths` en el bloque `plan` del Resumen, el subtítulo del 100 % cae a la frase genérica:
    // el recuento exacto y la cota de la regla de tres solo se pueden afirmar donde viaja el
    // tamaño de la muestra.
    parenthetical: successParenthetical(
      plan.success_of_plan,
      plan.success_threshold_pct,
    ),
    tone: tone === "danger" ? "danger" : tone === "warn" ? "warn" : "default",
  };
}
