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
  ProjectionBandPointApi,
  ProjectionBandsApi,
  SuccessVerdictApi,
} from "../api/types";
import {
  DISPLAY_NUMBER_LOCALE,
  METRIC_DASH,
  formatCurrencyOrDash,
  formatFractionAsPercent,
  formatPercentDisplay,
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

/** Un RECUENTO (caminos sorteados), con separador de millar español. No es dinero ni
 *  porcentaje: no pasa por los formateadores de importe. */
function formatCount(n: number): string {
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    maximumFractionDigits: 0,
  }).format(n);
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
 * `success_probability` → «87,0 %» (V1 de la tercera vuelta de UX, 5.0.0).
 *
 * Antes esta función devolvía la ORACIÓN entera («87 de cada 100 escenarios se jubilan y no
 * agotan el capital») como valor del tile. La frase era correcta y la tipografía del valor
 * (`.metric-value`, mono, `tabular-nums`) es para «87,0 %», no para once palabras: el owner la
 * leyó como «demasiado texto para caber en una caja» (F2). La condición no se pierde — baja al
 * subtítulo (`successParenthetical`), que es el slot que SÍ envuelve.
 *
 * **Se pasa por `scenariosPerHundred` y no por `formatFractionAsPercent`** a propósito: ahí
 * viven los dos topes anti-mentira (línea 214). `formatFractionAsPercent("0.9999")` imprimiría
 * «100,0 %» sobre un plan que falla en uno de cada diez mil, que es exactamente la mentira
 * silenciosa que esta app existe para no contar — y desde V7 el verde es EXCLUSIVO del 100 %,
 * así que un redondeo optimista pintaría de verde un plan que el servidor da por ámbar.
 *
 * Un decimal, como todo porcentaje de la casa (`design-system.md` §Formato de cifras).
 */
export function formatSuccessPercent(
  probability: string | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  if (n == null) return METRIC_DASH;
  return formatPercentDisplay(n);
}

/** «4 de cada 100» — la misma cifra sin sujeto, para las filas que ya lo llevan en el rótulo. */
export function formatScenariosPerHundred(
  probability: string | null | undefined,
): string {
  const n = scenariosPerHundred(probability);
  return n == null ? METRIC_DASH : `${n} de cada 100`;
}

/**
 * El SUBTÍTULO del tile de éxito: qué mide ese «87,0 %».
 *
 * **Sin «umbral» (V7)**: el corte ya no es del usuario —verde solo con cero caminos fallidos,
 * ámbar hasta el 90 %, rojo por debajo—, así que un denominador configurable que ya no existe
 * solo podía confundir. Lo que queda es el sujeto de la cifra, que sin él vuelve a quedar
 * abierto: un «87,0 %» pelado no dice de qué.
 *
 * En VERDE la frase genérica sería una perífrasis de «no falla ninguno», así que se sustituye
 * por el recuento exacto —«0 de 500 escenarios agotan el capital»— que es la lectura que hace
 * auditable el verde: con 500 caminos, un solo fallo ya es ámbar, y decir cuántos se sortearon
 * declara la precisión del 100 %. `paths` solo viaja con las bandas (el bloque `plan` del
 * Resumen no lo publica): sin él se cae a la frase genérica en vez de inventar un denominador.
 *
 * `undefined` ⟺ no hay cifra que subtitular.
 */
export function successParenthetical(
  probability: string | null | undefined,
  paths?: number | null,
): string | undefined {
  const n = scenariosPerHundred(probability);
  if (n == null) return undefined;
  if (n >= 100 && finite(paths) && paths > 0) {
    return `0 de ${formatCount(paths)} escenarios agotan el capital`;
  }
  return "de los escenarios no agotan el capital";
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
 * Copy de cada razón por la que el colchón NO se simuló (5.0.0, pase de correcciones §E,
 * ampliado por V6).
 *
 * `not_requested` **no está a propósito**: desde V6 ese literal ya no puede llegar con un perfil
 * vivo (sin colchón explícito el servidor lo deriva del tope de la regla), y si llegara diría
 * «no falta nada» — convertir el estado normal en una carencia es justo lo que no queremos.
 *
 * Las demás sí se enseñan, y cada una dice qué habría que TOCAR para tenerlo: son razones
 * accionables, no diagnósticos. Un literal desconocido (backend más nuevo) no pinta nada:
 * inventar la razón es peor que no darla.
 */
const BUFFER_INACTIVE_REASON_ES: Record<string, string> = {
  no_volatility: "sin volatilidad declarada, no hay de qué protegerse",
  no_safe_liquid_asset: "no tienes un activo líquido sin volatilidad donde vivir",
  no_capped_rule:
    "ninguna regla de ahorro con tope («hasta X €») apunta a tu líquido sin volatilidad",
  cap_is_zero: "el tope de tu regla de ahorro es 0 €",
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

  // ── Ruina total: la cifra que se queda sin sitio al retirar la tabla por edad ────────────
  //
  // La tabla «agotar a los 65/70/…» desaparece con el degradado de la banda (V5): el color YA
  // dice la probabilidad edad a edad y con más resolución. Lo que el color no dice es el TOTAL,
  // porque su última parada es el borde derecho del plot y ahí no hay etiqueta. Sin esta fila,
  // el peor número del panel —cuántos escenarios se quedaron sin capital EN ALGÚN MOMENTO— se
  // habría perdido en el rediseño, y con él la única lectura acumulada del sorteo.
  //
  // Se toma el ÚLTIMO punto de la rejilla porque la serie es acumulada por contrato (solo puede
  // crecer) y el último mes es el final del horizonte. `null` no se pinta: inventar un 0 % ahí
  // sería declarar un plan infalible a partir de un dato que no llegó.
  const depletionPoints = Array.isArray(b.depletion_probability_by_age)
    ? b.depletion_probability_by_age.filter((p) => finite(p.month_index))
    : [];
  const depletionLast = depletionPoints[depletionPoints.length - 1];
  if (depletionLast != null && depletionLast.probability != null) {
    rows.push({
      key: "depletion_total",
      label: "Escenarios que agotan el capital en algún momento",
      value: formatFractionAsPercent(depletionLast.probability),
      detail:
        "acumulado hasta el final del horizonte: es el mismo sorteo que colorea la banda del gráfico",
      helpId: "retirement.depletion_by_age",
    });
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
  }
  // El caso INACTIVO ya no es una fila de detalle: desde V6 el colchón se deriva y su procedencia
  // (o su ausencia y por qué) es información de primer orden, así que vive en la línea del bloque
  // «Riesgo» (`cashBufferLine`). Aquí se queda solo lo que el sorteo MIDIÓ, que es de segundo
  // orden por definición: cuántas veces hubo que rellenarlo y cuánto se movió.

  return rows;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// La línea informativa del colchón de caja (V6)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/**
 * Lo que el bloque «Riesgo» dice del colchón de caja, ahora que la SPA ya no lo PREGUNTA (V6).
 *
 * El owner cerró la decisión así: «lo lógico sería derivar el colchón de caja de las reglas de
 * ahorro (hasta X en activos)». El input desapareció del formulario y el servidor resuelve el
 * colchón desde el tope de la regla que apunta a tu líquido sin volatilidad. Un valor derivado se
 * ROTULA como derivado (regla 5 de `design-system.md` §Reglas para añadir UI nueva): sin decir de
 * dónde sale, un colchón que aparece solo se lee como un ajuste que alguien hizo, y el usuario no
 * sabría dónde cambiarlo.
 *
 * Las cuatro formas de la línea, y por qué cada una dice lo que dice:
 *
 *  - **`allocation_cap`** — el caso normal. Dice el IMPORTE (que es lo que el motor mantiene, en
 *    nominal, P2), de qué regla sale, su equivalente informativo en meses, y **el precio**: el
 *    colchón cuesta puntos de éxito en este modelo (hallazgo P4). Callar el coste convertiría una
 *    derivación automática en una promesa de seguridad que los números no sostienen.
 *  - **`explicit`** — alguien lo fijó por API o MCP y manda sobre la derivación. La línea lo dice
 *    y ofrece SOLTARLO (`PATCH {"cash_buffer_months": null}`): sin esa salida, un override puesto
 *    desde fuera sería irreversible desde la pantalla, que es el mismo fallo que ya obligó a
 *    añadir «Volver a la derivada» en la base del objetivo.
 *  - **`none`** — no hay colchón, y la razón dice qué habría que tocar para tenerlo.
 *  - **Sin `buffer_source`** (backend anterior a V6) — solo se enseña la razón cuando el colchón
 *    NO se simuló; con colchón activo, la fila de «Detalle del cálculo» ya lo cuenta y una línea
 *    más solo repetiría.
 *
 * `null` ⟺ no hay nada que decir (sin bandas, o un backend viejo con el colchón funcionando).
 */
export type CashBufferLine = {
  text: string;
  /** `true` ⟺ ofrecer «volver al tope de tu regla» — un `PATCH` del colchón a `null`. */
  canResetToDerived: boolean;
  /** `true` ⟺ ofrecer el salto a las reglas de ahorro, que es donde se cambia de verdad. */
  linksToAllocationRules: boolean;
};

export function cashBufferLine(
  bands: ProjectionBandsApi | null | undefined,
  currencyIso: string,
): CashBufferLine | null {
  if (!bands) return null;
  const reason = BUFFER_INACTIVE_REASON_ES[bands.buffer_inactive_reason ?? ""];
  const months = bands.buffer_months_effective;

  switch (bands.buffer_source) {
    case "allocation_cap": {
      const amount = formatCurrencyOrDash(bands.buffer_target_amount, currencyIso);
      const rule = bands.buffer_source_asset_name
        ? ` — el tope de tu regla de ahorro para «${bands.buffer_source_asset_name}»`
        : " — el tope de tu regla de ahorro";
      const equiv = finite(months) ? ` (≈ ${months} meses de tu gasto de hoy)` : "";
      return {
        text:
          `Colchón de caja: ${amount}${rule}${equiv}. Se mantiene en efectivo durante la ` +
          "jubilación y se rellena vendiendo inversiones; en este modelo cuesta unos puntos de " +
          "éxito.",
        canResetToDerived: false,
        linksToAllocationRules: true,
      };
    }
    case "explicit":
      return {
        text: finite(months)
          ? `Colchón de caja: ${months} meses, fijados por API. De serie sale del tope de tu regla de ahorro.`
          : "Colchón de caja fijado por API. De serie sale del tope de tu regla de ahorro.",
        canResetToDerived: true,
        linksToAllocationRules: false,
      };
    case "none":
      return reason == null
        ? null
        : {
            text: `Sin colchón de caja: ${reason}.`,
            canResetToDerived: false,
            linksToAllocationRules: false,
          };
    default:
      // Backend anterior a V6: solo hay algo que decir si el colchón NO corrió.
      if (bands.buffer_active || reason == null) return null;
      return {
        text: `Sin colchón de caja: ${reason}.`,
        canResetToDerived: false,
        linksToAllocationRules: false,
      };
  }
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
  /** Slot del paréntesis: el SUJETO de la cifra («de los escenarios no agotan el capital»).
   *  Hasta V7 aquí iba el umbral; ese ajuste ya no existe y el corte es fijo. */
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
 * Cero aritmética: la probabilidad y el veredicto vienen del MISMO sorteo que dibuja la sección
 * «Riesgo» de Jubilación (el servidor los sirve de su cache de bandas). Recalcular aquí el
 * semáforo con otra muestra enseñaría dos éxitos del mismo plan en la misma pantalla.
 */
export function summarySuccessTile(
  plan:
    | {
        success_probability?: string | null;
        success_verdict?: SuccessVerdictApi | string | null;
        success_absent_reason?: string | null;
        absent_reason?: string | null;
        never_retired_probability?: string | null;
      }
    | null
    | undefined,
): SuccessTileModel | null {
  if (!plan) return null;
  if (plan.success_probability == null) {
    const reason = plan.success_absent_reason ?? plan.absent_reason ?? null;
    // Sin probabilidad NI razón el backend está publicando un hueco mudo: es exactamente el
    // caso en que no hay nada honesto que decir, así que la tarjeta no se pinta.
    if (reason == null) return null;
    return {
      value: METRIC_DASH,
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
    value: formatSuccessPercent(plan.success_probability),
    // Sin `paths` en el bloque `plan` del Resumen, el subtítulo del verde cae a la frase
    // genérica: el recuento exacto solo se puede afirmar donde viaja el tamaño de la muestra.
    parenthetical: successParenthetical(plan.success_probability),
    detail:
      neverRetired != null && neverRetired > 0
        ? `${formatScenariosPerHundred(plan.never_retired_probability)} no llegan a jubilarse`
        : undefined,
    tone: tone === "danger" ? "danger" : tone === "warn" ? "warn" : "default",
  };
}
