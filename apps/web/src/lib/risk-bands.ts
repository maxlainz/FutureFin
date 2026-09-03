/**
 * Modelo PURO de la sección «Riesgo» de Jubilación (5.0.0, D22/D25/D28 y §B.5 del plan de #207):
 * el abanico de percentiles, el semáforo de éxito, la tabla de agotamiento por edad y las
 * lecturas que solo existen en algunas estrategias.
 *
 * Aquí no se dibuja nada y no se recalcula NADA del modelo: el veredicto, la probabilidad y las
 * medianas las decide el servidor (`GET /v1/projection/bands`) y este módulo se limita a
 * alinearlas, deflactarlas y traducirlas a copy. Que sea un módulo aparte con test es
 * deliberado — lo que aquí puede romperse en silencio son cuatro cosas, y ninguna se ve mirando
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
 *  4. **El SUJETO de la probabilidad.** Desde el pase de correcciones de 5.0.0 (§F/§G) el éxito
 *     exige jubilarse Y no agotar, y la cobertura cuenta también el gasto que la cartera no
 *     pudo financiar. Las dos son cifras cuyo nombre corto —«éxito», «recorte»— sobrevivió a su
 *     definición: la copy tiene que llevar la condición dentro, no en el popover.
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
import type { HelpTextId } from "./helpTexts";
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
 * @deprecated 5.0.0 U1b — **sin consumidor de UI**. El rediseño funde los dos gráficos de
 * Jubilación en uno (U5): la banda entra ahora en `MiniProjection` como una lista de
 * `{month, p10, p90}` en euros NOMINALES, y la deflactación de las tres series (patrimonio,
 * objetivo y banda) la aplica el chart una sola vez, que es lo que garantiza que el abanico
 * contenga a la línea. `RiskFanChart.tsx` se retiró con él. Se conserva esta función —con su
 * test— porque es donde vive la alineación de dos rejillas distintas por MES: si alguna vez
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
 * Fracción → «de cada 100», con los DOS topes que impiden la mentira por redondeo: mientras la
 * probabilidad no sea 1 EXACTA el redondeo no llega a 100, y mientras no sea 0 exacta no baja a
 * 0. `0.999` es un plan que falla en uno de cada mil y «100 de cada 100» lo diría infalible.
 *
 * `null` ⟺ no hay cifra que contar (ni la hay que inventar).
 */
export function scenariosPerHundred(
  probability: string | null | undefined,
): number | null {
  const p = probability == null ? null : parseDisplayDecimal(String(probability));
  if (p == null || !Number.isFinite(p)) return null;
  let n = Math.round(p * 100);
  if (n >= 100 && p < 1) n = 99;
  if (n <= 0 && p > 0) n = 1;
  return n;
}

/**
 * `success_probability` → «87 de cada 100 escenarios se jubilan y no agotan el capital».
 *
 * La frase dice las DOS condiciones porque desde el pase de correcciones de 5.0.0 el éxito son
 * dos y no una (§G): el camino tiene que **jubilarse dentro del horizonte** —o jubilarlo la
 * edad— **y** no agotar la cartera. Con la definición vieja («no se agota») un plan que no
 * llegaba a jubilar a nadie puntuaba altísimo por no gastar nunca, y el número más caro de la
 * app decía lo contrario de lo que pasaba. Un rótulo corto —«87 de cada 100 escenarios»— vuelve
 * a dejar esa lectura abierta, así que la condición viaja EN la cifra, no en el popover.
 */
export function formatSuccessScenarios(
  probability: string | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  if (n == null) return METRIC_DASH;
  return `${n} de cada 100 escenarios se jubilan y no agotan el capital`;
}

/** «4 de cada 100» — la misma cifra sin sujeto, para las filas que ya lo llevan en el rótulo. */
export function formatScenariosPerHundred(
  probability: string | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  return n == null ? METRIC_DASH : `${n} de cada 100`;
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
 *
 * **La última fila es el HORIZONTE, no una edad más** (5.0.0, pase de correcciones §H): la
 * rejilla avanza de cinco en cinco años desde la jubilación y ahora cierra siempre en el último
 * mes del plan, así que esa celda es la RUINA TOTAL — todos los escenarios que se quedaron sin
 * capital en algún momento. Rotularla «a los 87» la haría parecer un peldaño más de la escalera
 * y el usuario leería el peor número de la tabla como si le faltara horizonte por delante.
 *
 * Se reconoce por su MES (`month_index + 1 >= horizonMonths`, porque la rejilla es 0-based y
 * `months` es un recuento), nunca por ser la última posición: con un backend anterior al pase la
 * última fila **no** es el horizonte, y la comprobación falla cerrada dejando su rótulo por edad
 * en vez de inventar una ruina total que ese backend no calculó.
 */
export function buildDepletionRows(
  points: readonly DepletionProbabilityPointApi[] | null | undefined,
  horizonMonths?: number | null,
): DepletionRow[] {
  if (!Array.isArray(points)) return [];
  const rows = points.filter((p) => finite(p.month_index));
  const lastMonth = rows.length > 0 ? rows[rows.length - 1]!.month_index : null;
  const horizonMonth =
    finite(horizonMonths) && lastMonth != null && lastMonth + 1 >= horizonMonths
      ? lastMonth
      : null;
  return rows.map((p) => ({
    key: `dep-${p.month_index}`,
    label:
      p.month_index === horizonMonth
        ? "al final del horizonte"
        : finite(p.age)
          ? `a los ${p.age}`
          : `mes ${p.month_index}`,
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
  /** Ayuda del catálogo, cuando la fila mide algo que su rótulo no puede explicar entero. La
   *  pinta la vista junto al rótulo; el escáner de `helpTexts.test.ts` ve esta forma de objeto. */
  helpId?: HelpTextId;
};

export type RiskExtraRowsInput = {
  bands: ProjectionBandsApi | null | undefined;
  /** ISO de la divisa (`""` degrada a número sin símbolo, como el resto de la app). */
  currencyIso: string;
  /** Rotulador de un mes de la rejilla → etiqueta del eje («2043», «a los 55»). Lo inyecta la
   *  vista: depende del modo del eje, de la fecha de nacimiento y de la zona horaria. */
  monthLabel: (monthIndex: number) => string;
};

/**
 * Copy de cada razón por la que el colchón NO se simuló (5.0.0, pase de correcciones §E).
 *
 * `not_requested` **no está a propósito**: no hay colchón configurado, no falta nada, y una fila
 * diciéndolo convertiría el estado normal en una carencia. Las otras dos sí se enseñan porque en
 * las dos el usuario SÍ pidió un colchón y no lo tuvo — callarlo dejaría un ajuste guardado que
 * la simulación ignora en silencio, que es la peor combinación posible.
 *
 * Un literal desconocido (backend más nuevo) tampoco pinta fila: inventar la razón es peor que
 * no darla.
 */
const BUFFER_INACTIVE_REASON_ES: Record<string, string> = {
  no_volatility: "sin volatilidad declarada",
  no_safe_liquid_asset: "sin un activo líquido sin volatilidad donde guardarlo",
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

  // ── Las dos mitades del éxito nuevo (§G del pase de correcciones) ────────────────────────
  //
  // Van las PRIMERAS y solo cuando hay algo que contar: si ningún camino se queda sin jubilar,
  // el éxito del tile ya es la historia entera y estas dos filas serían un 0 y una repetición.
  // Cuando sí los hay, la de arriba dice cuántos y la de abajo devuelve la lectura que el
  // usuario creía estar leyendo antes del pase — el éxito ENTRE los que llegan a jubilarse.
  const neverRetired = scenariosPerHundred(b.never_retired_probability);
  if (neverRetired != null && neverRetired > 0) {
    rows.push({
      key: "never_retired",
      label: "No llegan a jubilarse en el horizonte",
      value: formatScenariosPerHundred(b.never_retired_probability),
      detail:
        "escenarios en los que el plan nunca te jubila: no cuentan como éxito aunque el dinero siga entero",
    });
    // `null` = no hay denominador (nadie se jubila), y entonces no hay condicional que enseñar.
    // Un «—» aquí se leería como dato perdido en vez de como pregunta sin sujeto.
    if (b.success_given_retired != null) {
      rows.push({
        key: "success_given_retired",
        label: "Éxito entre los que se jubilan",
        value: formatFractionAsPercent(b.success_given_retired),
        detail: "de los escenarios que sí te jubilan, los que además no agotan el capital",
      });
    }
  }

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

  // ── Las dos lecturas de la COBERTURA, en TODAS las reglas ────────────────────────────────
  //
  // Hasta el pase de correcciones estas dos filas se escondían con `fixed_real` porque medían
  // solo el recorte de la REGLA, y esa regla no tiene techo: sus cifras eran 0 y 1 por
  // construcción. Desde §F miden algo distinto — incluyen también **el gasto que la cartera no
  // pudo financiar** —, así que con `fixed_real` son perfectamente informativas: la regla no
  // recorta nunca, pero el dinero se puede acabar igual. Esconderlas ahí era esconder justo el
  // caso en que la cobertura tiene una sola causa y es la peor (un fixture medido pasó de `1,0`
  // a `0,0865` al dejar de ignorar el descubierto). Por eso ya no hay condición de regla, y por
  // eso el parámetro de la entrada que la gobernaba desapareció con ella: uno que ya no decide
  // nada es una invitación a volver a esconderlas.
  rows.push({
    key: "months_below_need",
    label: "Meses por debajo del gasto (mediana)",
    value: `${b.months_below_need_p50}`,
    detail: "meses jubilados en que gastaste menos de lo que necesitabas",
    helpId: "retirement.coverage",
  });
  rows.push({
    key: "withdrawal_to_need",
    label: "Retirada / gasto (mediana)",
    value: formatFractionAsPercent(b.withdrawal_to_need_ratio_p50),
    detail:
      "qué parte de la necesidad se pagó de verdad, por el techo de la regla y por lo que la cartera no dio; 100 % = entera",
  });

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
      helpId: "retirement.cash_buffer",
    });
  } else {
    // Colchón pedido y NO simulado: se dice por qué. Es un ajuste guardado que la simulación
    // ignora, y sin esta fila el usuario cree que está protegido por algo que no corrió.
    const reason = BUFFER_INACTIVE_REASON_ES[b.buffer_inactive_reason ?? ""];
    if (reason != null) {
      rows.push({
        key: "buffer",
        label: "Colchón de caja",
        value: "No simulado",
        detail: reason,
        helpId: "retirement.cash_buffer",
      });
    }
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
        never_retired_probability?: string | null;
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
  // Subtítulo solo cuando hay escenarios sin jubilar: es la mitad que el número grande ya no
  // cuenta sola desde §G, y la que explica un éxito bajo que no se debe a agotar el capital
  // sino a no llegar nunca. Con cero, la tarjeta no gana nada diciendo «0 de cada 100».
  const neverRetired = scenariosPerHundred(plan.never_retired_probability);
  return {
    value: formatSuccessScenarios(plan.success_probability),
    parenthetical: threshold,
    detail:
      neverRetired != null && neverRetired > 0
        ? `${formatScenariosPerHundred(plan.never_retired_probability)} no llegan a jubilarse`
        : undefined,
    tone: tone === "danger" ? "danger" : tone === "warn" ? "warn" : "default",
  };
}
