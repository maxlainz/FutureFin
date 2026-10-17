/**
 * Modelo PURO de la cabecera de resultados de Jubilación y de sus avisos (5.0.0, **modelo v2**:
 * «el éxito define la fecha», C1–C8 + §4 «Salidas y pantalla»).
 *
 * ## Qué cambió respecto de la primera 5.0.0, y por qué importa aquí
 *
 * Hasta el modelo v2 la cabecera giraba alrededor de un OBJETIVO: un capital estático que la línea
 * determinista cruzaba, con su base (perpetuidad / puente), su descuento y su colchón. El
 * diagnóstico del owner fue que ese objetivo decidía la fecha y el éxito solo la COMENTABA — «no
 * es tolerable planear y que luego no alcance». Así que el objetivo desapareció del contrato y con
 * él las tarjetas que lo servían (`target`, `coast_number`, `partial_gap`, `bridge`, `disposable`).
 *
 * Lo que queda son **tres tarjetas y ni una más**, y las tres contestan a la misma pregunta desde
 * ángulos distintos: cuánto necesitarías HOY, con qué seguridad aguanta el plan que tienes, y el
 * hito propio de tu estrategia. La correspondencia estrategia → tercera tarjeta es una tabla del
 * contrato (§4), no una preferencia de layout, así que vive aquí con un test que la fija y la
 * vista se limita a pintar lo que salga.
 *
 * ## Cuatro reglas que este módulo NO puede romper
 *
 *  1. **`null` no es cero.** Una `contribution_required_monthly` ausente significa «esta
 *     estrategia no responde a esa pregunta», y la tarjeta no se emite — nunca un 0 €, que se
 *     leería como «no necesitas ahorrar nada». Lo mismo con `contribution_underfunded`, donde
 *     `null` es «la pregunta no aplica» y `false` es «vas bien»: colapsarlos pinta de verde un
 *     plan que nadie ha evaluado.
 *  2. **«Capital necesario hoy» está SIEMPRE en euros de hoy** y el toggle «en dinero de hoy» del
 *     chart **no lo toca**. No es un olvido: la cifra es `needed_c(1)` (§2.7), una respuesta a
 *     «¿cuánto necesitaría si me jubilara ya?», y «ya» es hoy. Deflactarla otra vez la dividiría
 *     por un factor que ya está aplicado; multiplicarla por el nominal contestaría a una pregunta
 *     que nadie hace. La unidad la declara su `<?>` (`retirement.needed_capital`): desde
 *     2026-10-10 los subtítulos solo llevan VALOR o ESTADO, nunca la explicación de la cifra.
 *  3. **Un plan que no está no se rellena.** Con `plan_absent_reason` las tarjetas fijas enseñan
 *     la RAZÓN, no un guion mudo ni un número viejo: «falta tu fecha de nacimiento» y «el hogar no
 *     resuelve un plan común» son estados distintos y se dicen distinto. Y con el plan resuelto
 *     pero **sin fecha** (`retirement_date_basis: "not_reachable"`), la del ÉXITO tampoco se
 *     rellena: la serie publica ahí una cifra que mide otra cosa (A12, ver `buildRetirementTilesV2`).
 *  4. **Las unidades del contrato mandan.** `success_of_plan`/`success_wilson_low` son FRACCIONES
 *     `[0,1]`; `success_threshold_pct` es un PORCENTAJE entero; `success_sampling_error_pp` son
 *     PUNTOS PORCENTUALES y tiene formateador propio (`formatSamplingErrorPp`, en `risk-bands.ts`).
 *     Mezclarlos multiplica o divide por 100 lo que el usuario lee.
 *
 * Lo que este módulo NO hace: dibujar nada y resolver fechas por su cuenta. Recibe de la vista un
 * rotulador de meses y un resolutor de edades, porque los dos dependen del modo del eje, de la
 * fecha de nacimiento y de la zona horaria, y ninguna de las tres es asunto de este módulo.
 */

import type { ProjectionSeriesApi, RetirementStrategyApi } from "../api/types";
import { formatProjectionAxisYear, parseYmdComponents } from "./dates";
import { METRIC_DASH, formatCurrencyOrDash, parseDisplayDecimal } from "./format";
import { safeDateBoundsLine } from "./plan-sentence";
import { formatSuccessPercent, type PlanHelpTextId } from "./risk-bands";

/**
 * Los campos del bloque «plan» de la serie que la cabecera lee. Un `Pick` y no la respuesta entera
 * para que un test pueda escribir el caso mínimo sin inventarse una proyección completa.
 */
export type RetirementTileV2Series = Pick<
  ProjectionSeriesApi,
  | "strategy"
  | "retirement_date_basis"
  | "success_threshold_pct"
  | "safe_date_month_index"
  | "safe_date_date_ymd"
  | "safe_date_age"
  | "safe_date_at_100_month_index"
  | "safe_date_at_90_month_index"
  | "needed_capital_curve_state"
  | "success_of_plan"
  | "success_wilson_low"
  | "success_sampling_error_pp"
  | "paths_used"
  | "seed"
  | "needed_capital_today"
  | "needed_capital_absent_reason"
  | "contribution_required_monthly"
  | "contribution_required_search_ceiling"
  | "contribution_underfunded"
  | "coast_stop_month_index"
  | "partial_start_month_index"
  | "plan_absent_reason"
  | "fire_number_classic_today"
  | "warnings"
>;

/** Lo mínimo que necesitan los avisos: la estrategia no entra, los `warnings` mandan. */
export type RetirementNoticeSeries = Pick<
  ProjectionSeriesApi,
  "contribution_underfunded" | "warnings"
>;

export type RetirementTileTone = "default" | "danger";

/** Cuántas tarjetas caben en la cabecera de resultados. §4: **una frase de hito + 3 tarjetas**. */
export const RETIREMENT_TILES_V2_CAP = 3;

/** Una tarjeta de la cabecera: **una sola cifra** y, como mucho, una sublínea de VALOR («63 años»,
 *  «100 %: 2074 · 90 %: 2050») o de ESTADO («ya puedes jubilarte», el motivo de una ausencia).
 *  Lo que la cifra SIGNIFICA vive en su `<?>` (`helpId`), nunca en la sublínea (owner,
 *  2026-10-10: «pregunta–respuesta»). */
export type RetirementTileV2 = {
  /** Key de React y del test. Estable por tarjeta, nunca por posición. */
  key: string;
  label: string;
  value: string;
  /** Valor o estado corto. `undefined` = no hay nada que añadir, y entonces la vista reserva el
   *  slot igual (misma disciplina que el paréntesis de `MetricCard`). */
  subtitle?: string;
  tone: RetirementTileTone;
  helpId: PlanHelpTextId;
};

export type RetirementTilesV2Input = {
  series: RetirementTileV2Series | null | undefined;
  /** ISO de la divisa del hogar (`""` degrada a número sin símbolo, como el resto de la app). */
  currencyIso: string;
  /**
   * Rotulador de un mes de la rejilla → etiqueta del eje («2043», «a los 55»). Lo inyecta la
   * vista porque depende del modo del eje, de la fecha de nacimiento y de la zona horaria, y
   * ninguna de las tres es asunto de este módulo.
   */
  monthLabel: (monthIndex: number) => string;
  /**
   * Edad cumplida en un mes de la rejilla, o `null` sin fecha de nacimiento resuelta. Misma razón
   * que `monthLabel`: la aritmética civil vive en la vista. Ausente ⇒ los subtítulos van sin edad,
   * nunca con una inventada.
   */
  monthAge?: (monthIndex: number) => number | null;
  /** Edad objetivo del perfil, para nombrarla en el rojo. `null` ⇒ «tu edad objetivo». */
  targetRetirementAge: number | null;
};

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/**
 * Por qué el bloque «plan» no trae cifras, o `null` cuando sí las trae.
 *
 * Llevaba una rama `retirement_date_basis: "pending"` que devolvía «calculando…» y **el servidor
 * nunca emitió ese literal**: el nivel 1 del solve se resuelve en línea, dentro del permiso de la
 * serie, así que no hay ninguna ventana en la que la base esté a medias. Se retiró en A12 junto
 * con el literal del tipo. El único «calculando» real de esta pantalla es el nivel 2
 * (`needed_capital_curve_state: "computing"`), que sí llega en un GET posterior y no pasa por
 * aquí: no afecta a ninguna de las tres tarjetas.
 *
 * **`not_reachable` NO entra aquí**, y es deliberado: ahí el bloque «plan» sí viaja —hay
 * `needed_capital_today`, hay umbral, hay estrategia— y lo único que falta es la FECHA. Vaciar las
 * tres tarjetas por eso escondería la cifra que mejor contesta a «¿y cuánto me faltaría?». Lo que
 * ese caso rompe es solo la tarjeta del éxito, y se corrige ahí (ver `buildRetirementTilesV2`).
 *
 * `no_liquid_assets` tampoco: **`plan_absent_reason` no lo emite nunca** (sus tres constantes
 * viven en `apps/api/src/handlers/projection.rs`). El literal existe, pero en
 * `needed_capital_absent_reason`, que responde a otra pregunta.
 */
function planUnavailableReason(series: RetirementTileV2Series): string | null {
  switch (series.plan_absent_reason) {
    case "birth_date_missing":
      return "falta tu fecha de nacimiento";
    case "months_override":
      return "esta vista fija un horizonte propio";
    // `household_aggregate` y no `household_not_solved` (A12): el segundo es el valor fijo de
    // `HouseholdMemberProjectionApi.plan_state`, otro campo. Con el literal equivocado, la vista
    // del hogar caía al `default` y decía «no disponible».
    case "household_aggregate":
      return "el hogar no resuelve un plan común";
    case null:
    case undefined:
      return null;
    default:
      // Literal nuevo de un backend más moderno: se dice que falta, no se inventa el motivo.
      return "no disponible";
  }
}

/**
 * **El valor del tile cuando la necesidad cae por debajo de lo que el sorteo sabe medir.**
 *
 * `already_covered` es la única de las cuatro ausencias de `needed_capital_absent_reason` que es
 * una RESPUESTA y no un fallo de medición: ni dividiendo la cartera por 256 el plan incumple el
 * umbral, o sea que no hace falta capital adicional hoy. Enseñarlo con el mismo «—» que
 * «sin activos líquidos» tiraría la única de las cuatro que es una buena noticia, así que el tile
 * pone estas dos palabras donde iría la cifra — sin subtítulo: qué significa lo dice su `<?>`
 * (`retirement.needed_capital`).
 */
const ALREADY_COVERED_VALUE = "Ya cubierto";

/**
 * Por qué falta `needed_capital_today` cuando el resto del plan SÍ está resuelto — la pregunta que
 * `planUnavailableReason` de arriba responde para el bloque entero, aquí para esta CIFRA sola. Los
 * cuatro literales son los de `needed_capital.rs` (ver el doc de `needed_capital_absent_reason` en
 * `api/types.ts`); uno que esta función no reconoce cae al mismo «no disponible» genérico que el
 * resto de razones de ausencia de la app, nunca a un guion mudo.
 */
function neededCapitalAbsentReasonEs(
  reason: string | null | undefined,
): string {
  switch (reason) {
    case "no_liquid_assets":
      return "sin activos líquidos que escalar";
    case "threshold_unreachable":
      return "ningún capital alcanza tu umbral";
    case "month_beyond_horizon":
      return "la fecha cae fuera del horizonte";
    case "already_covered":
      return ALREADY_COVERED_VALUE.toLowerCase();
    default:
      return "no disponible";
  }
}

/** «55 años» a partir del resolutor inyectado; `null` sin fecha de nacimiento. */
function ageBit(
  monthIndex: number,
  monthAge: RetirementTilesV2Input["monthAge"],
): string | null {
  const age = monthAge?.(monthIndex);
  return finite(age) ? `${age} años` : null;
}

/** El ESTADO «ya» cuando el hito es hoy; `null` en otro caso (el «dentro de N años» era una
 *  explicación de la fecha que ya está en la cifra, y se retiró el 2026-10-10). */
function alreadyBit(monthIndex: number, already: string): string | null {
  return monthIndex <= 0 ? already : null;
}

function joinBits(bits: (string | null | undefined)[]): string | undefined {
  const kept = bits.filter((b): b is string => b != null && b !== "");
  return kept.length > 0 ? kept.join(" · ") : undefined;
}

/**
 * La tarjeta «Capital necesario hoy», **la misma en Jubilación y en Proyección** (2026-10-10:
 * Proyección tenía su propia sublínea explicativa, «en euros de hoy · para que aguanten N de cada
 * 100», y la tarjeta gemela de Jubilación ya no la llevaba). Una sola función para que las dos no
 * puedan volver a divergir: valor, y como sublínea solo el ESTADO de una ausencia.
 *
 * Con el plan resuelto pero sin esta CIFRA sola (`needed_capital_absent_reason`), la sublínea dice
 * por qué. Y con `already_covered` ni siquiera hay guion: el hueco no es una medición que falló,
 * es «no te hace falta nada más».
 */
export function neededCapitalTile(
  series: RetirementTileV2Series,
  currencyIso: string,
): RetirementTileV2 {
  const unavailable = planUnavailableReason(series);
  const alreadyCovered =
    !unavailable &&
    series.needed_capital_today == null &&
    series.needed_capital_absent_reason === "already_covered";
  return {
    key: "needed_capital",
    label: "Capital necesario hoy",
    value: unavailable
      ? METRIC_DASH
      : alreadyCovered
        ? ALREADY_COVERED_VALUE
        : formatCurrencyOrDash(series.needed_capital_today, currencyIso),
    subtitle:
      unavailable ??
      (alreadyCovered || series.needed_capital_today != null
        ? undefined
        : neededCapitalAbsentReasonEs(series.needed_capital_absent_reason)),
    tone: "default",
    helpId: "retirement.needed_capital",
  };
}

/**
 * La cabecera de resultados de Jubilación: **exactamente hasta 3 tarjetas, una cifra por tarjeta**.
 *
 * El orden es fijo y ES el contrato (§4):
 *
 * 1. **«Capital necesario hoy»** — la única cifra que todas las estrategias comparten. Es
 *    `needed_c(1)`: el líquido que, con tu mezcla de activos, sostiene el plan al umbral si te
 *    jubilaras ya. Siempre en euros de hoy (regla 2 de la cabecera del módulo).
 * 2. **«Éxito del plan»** — el KPI central del modelo v2, con el umbral y la precisión del sorteo
 *    en el subtítulo. Sin el umbral, el mismo 95,0 % es «justo lo que pedí» para uno y «cinco
 *    puntos de más» para otro.
 * 3. **La tercera, por estrategia**: `asap` → «Fecha válida»; `retire_at_age` → «Aportación
 *    mínima»; `coast` → «Mes coast»; `partial` → «Inicio de la jornada reducida».
 *
 * Las dos primeras se emiten SIEMPRE (con su razón cuando el plan no está). La tercera solo cuando
 * el servidor publica su cifra o el aviso que la sustituye: una tarjeta con guion diría «esto se
 * calcula y hoy falta el dato», y lo cierto es que la estrategia no hace esa pregunta.
 */
export function buildRetirementTilesV2(
  input: RetirementTilesV2Input,
): RetirementTileV2[] {
  const { series, currencyIso } = input;
  if (!series) return [];
  const unavailable = planUnavailableReason(series);
  const tiles: RetirementTileV2[] = [];

  // 1 · Capital necesario hoy — fija, primera, siempre en euros de HOY.
  tiles.push(neededCapitalTile(series, currencyIso));

  // 2 · Éxito del plan — fija, segunda.
  //
  // **Sin fecha válida no hay éxito que rotular** (A12). La serie SÍ publica un `success_of_plan`
  // con `retirement_date_basis: "not_reachable"`, pero mide otra cosa —la MEJOR observación del
  // solve, el mes que más cerca se quedó— y esta tarjeta promete «el éxito de TU plan». Copiarlo
  // ponía un porcentaje respetable («68,0 %») bajo el rótulo de un plan que no ocurre, y el
  // usuario no tiene forma de saber que ese número describe un mes que su plan no alcanza. El
  // valor que más cerca se queda sigue estando en la pantalla —la frase-hito lo cita como «lo más
  // cerca»— pero ahí va con su sujeto delante, que es lo que aquí no cabe.
  //
  // Sin color, además: **el semáforo del ÉXITO** de un plan sin fecha es la ausencia de semáforo
  // —no hay porcentaje que colorear—. Que el PLAN sí sea rojo lo dicen sus dos sitios: la
  // frase-hito de esta misma pantalla (`planSentence`, tono `danger`) y el estado de la tarjeta
  // del Resumen (`planStatusFromPlan`, `noValidDate`). Son dos juicios distintos sobre dos cosas
  // distintas, y por eso no se contradicen.
  const noValidDate = series.retirement_date_basis === "not_reachable";
  tiles.push({
    key: "success",
    label: "Éxito del plan",
    value: unavailable || noValidDate ? METRIC_DASH : formatSuccessPercent(series.success_of_plan),
    // El umbral está en el formulario de al lado y la precisión del sorteo en «Las cifras del
    // riesgo»: aquí solo el estado que cambia cómo se lee el guion.
    subtitle: unavailable ?? (noValidDate ? "ninguna fecha llega a tu umbral" : undefined),
    tone: "default",
    helpId: "retirement.success",
  });

  // 3 · La de la estrategia. Sin plan no hay tercera: repetir la misma razón por tercera vez no
  // añade nada, y un guion afirmaría que esa cifra se calcula y hoy falta.
  if (!unavailable) {
    const third = strategyTile(input, series);
    if (third) tiles.push(third);
  }

  return tiles.slice(0, RETIREMENT_TILES_V2_CAP);
}

/** La tercera tarjeta, la que cambia con la estrategia (§4). `null` = esta estrategia no publica
 *  su hito (y entonces la cabecera se queda en dos, que es honesto). */
function strategyTile(
  input: RetirementTilesV2Input,
  series: RetirementTileV2Series,
): RetirementTileV2 | null {
  const strategy: RetirementStrategyApi | null | undefined = series.strategy;
  switch (strategy) {
    case "asap":
      return safeDateTile(input, series);
    case "retire_at_age":
      return requiredContributionTile(input, series);
    case "coast":
      return coastTile(input, series);
    case "partial":
      return partialTile(input, series);
    default:
      // `null` en `view=household` (el agregado no tiene UNA estrategia) o un literal futuro.
      return null;
  }
}

/**
 * «Fecha válida» (`asap`): el primer mes en que, jubilándote ahí, ≥ umbral de los caminos no
 * vuelven a necesitar trabajar (§2.6, definición A).
 *
 * El valor es el AÑO —la misma unidad que la frase-hito de §4 («te jubilas en 2043 (a los 55)»)—
 * porque una fecha al día fingiría una precisión que un sorteo no tiene. El mes exacto sigue
 * viviendo en la marca vertical del chart, que es donde se puede leer sin prometer nada.
 *
 * `not_reachable` es «nunca», nunca un 0: ningún mes del horizonte cumple el umbral, y esa es una
 * respuesta, no un hueco.
 */
function safeDateTile(
  input: RetirementTilesV2Input,
  series: RetirementTileV2Series,
): RetirementTileV2 | null {
  if (series.retirement_date_basis === "not_reachable") {
    return {
      key: "safe_date",
      label: "Fecha válida",
      value: "Nunca",
      tone: "danger",
      helpId: "retirement.safe_date",
    };
  }

  const mi = series.safe_date_month_index;
  const civil = parseYmdComponents(series.safe_date_date_ymd);
  const value = civil
    ? formatProjectionAxisYear(civil)
    : finite(mi)
      ? input.monthLabel(mi)
      : null;
  if (value == null) return null;

  const age = series.safe_date_age;
  return {
    key: "safe_date",
    label: "Fecha válida",
    value,
    // VALOR: la edad y las dos fechas que acotan la del umbral («100 %: 2074 · 90 %: 2050»).
    subtitle: joinBits([
      finite(age) ? `${age} años` : null,
      finite(mi) ? alreadyBit(mi, "ya puedes jubilarte") : null,
      safeDateBoundsLine(series, input.monthLabel),
    ]),
    tone: "default",
    helpId: "retirement.safe_date",
  };
}

/**
 * «Aportación mínima» (`retire_at_age`): la menor aportación mensual extra que hace cumplir el
 * umbral en la edad que pediste (§3.2).
 *
 * Los tres estados que la cifra puede tener y que un importe pelado colapsaría:
 *
 * - **`contribution_underfunded`** — ni con el techo entero se llega. El importe SERÍA el techo, y
 *   pintarlo diría «ahorra esto y llegas», que es exactamente lo contrario. Va en rojo y con
 *   palabras.
 * - **cero** — «Ya llegas»: no hace falta aportar nada MÁS de lo que ya aportas. Un «0 €» aquí se
 *   lee como «no ahorres», que no es lo que dice el motor.
 * - **un importe** — con su denominador («de X €/mes de sobrante»), sin el cual no se sabe si es
 *   mucho o poco.
 */
function requiredContributionTile(
  input: RetirementTilesV2Input,
  series: RetirementTileV2Series,
): RetirementTileV2 | null {
  const money = (s: string | null | undefined) =>
    formatCurrencyOrDash(s, input.currencyIso);
  const ceiling = series.contribution_required_search_ceiling;
  // El denominador de la cifra, como VALOR: sin él no se sabe si es mucho o poco.
  const ceilingBit = ceiling != null ? `sobrante: ${money(ceiling)}/mes` : null;

  if (series.contribution_underfunded === true) {
    return {
      key: "required_contribution",
      label: "Aportación mínima",
      value: "Ni ahorrándolo todo",
      subtitle: ceilingBit ?? undefined,
      tone: "danger",
      helpId: "retirement.required_contribution",
    };
  }

  const amount = series.contribution_required_monthly;
  if (amount == null) return null;
  const n = parseDisplayDecimal(String(amount));
  if (n === 0) {
    return {
      key: "required_contribution",
      label: "Aportación mínima",
      value: "Ya llegas",
      tone: "default",
      helpId: "retirement.required_contribution",
    };
  }

  return {
    key: "required_contribution",
    label: "Aportación mínima",
    value: money(amount),
    subtitle: ceilingBit ?? undefined,
    tone: "default",
    helpId: "retirement.required_contribution",
  };
}

/**
 * «Mes coast» (`coast`): el primer mes en que puedes dejar de aportar y aun así llegar (modo A) o
 * el que fijaste tú (modo B).
 *
 * `coast_not_reachable` no es «no calculado»: es que no existe tal mes, ni aportando siempre. Se
 * dice con palabras porque un guion invitaría a esperar a que llegue el dato.
 */
function coastTile(
  input: RetirementTilesV2Input,
  series: RetirementTileV2Series,
): RetirementTileV2 | null {
  const warnings = new Set(series.warnings ?? []);
  if (warnings.has("coast_not_reachable")) {
    return {
      key: "coast_month",
      label: "Mes coast",
      value: "No puedes parar nunca",
      tone: "danger",
      helpId: "retirement.coast_month",
    };
  }

  const mi = series.coast_stop_month_index;
  if (!finite(mi)) return null;
  return {
    key: "coast_month",
    label: "Mes coast",
    value: input.monthLabel(mi),
    subtitle: joinBits([
      ageBit(mi, input.monthAge),
      alreadyBit(mi, "ya puedes dejar de aportar"),
    ]),
    tone: "default",
    helpId: "retirement.coast_month",
  };
}

/**
 * «Inicio de la jornada reducida» (`partial`, Barista FIRE): el mes en que arranca la fase, sea
 * porque lo pediste (modo A) o porque es el primero que el plan puede permitirse (modo B).
 *
 * Los dos fallos de solve de §3.4 dicen cosas distintas y por eso no comparten copy:
 * `partial_never_starts` es «la fase no empieza nunca» (y entonces no hay mes que enseñar);
 * `partial_never_fully_retires` es «empiezas, pero de ahí no sales» — la fase arranca y la
 * jubilación total nunca llega, que es peor y va en rojo con el mes puesto.
 */
function partialTile(
  input: RetirementTilesV2Input,
  series: RetirementTileV2Series,
): RetirementTileV2 | null {
  const warnings = new Set(series.warnings ?? []);
  if (warnings.has("partial_never_starts")) {
    return {
      key: "partial_start",
      label: "Inicio de la jornada reducida",
      value: "Nunca",
      tone: "danger",
      helpId: "retirement.partial_mode",
    };
  }

  const mi = series.partial_start_month_index;
  if (!finite(mi)) return null;
  const neverFull = warnings.has("partial_never_fully_retires");
  return {
    key: "partial_start",
    label: "Inicio de la jornada reducida",
    value: input.monthLabel(mi),
    subtitle: joinBits([
      ageBit(mi, input.monthAge),
      alreadyBit(mi, "ya puedes empezarla"),
      neverFull ? "nunca te jubilas del todo" : null,
    ]),
    tone: neverFull ? "danger" : "default",
    helpId: "retirement.partial_mode",
  };
}

/** Literales cerrados de `warnings[]` que esta vista sabe explicar, más el rojo que sale de un
 *  booleano. `birth_date_missing` NO está: lo cuenta la tarjeta «Capital necesario hoy» con su
 *  razón, y decirlo dos veces en la misma pantalla es ruido. */
export type RetirementNoticeCode =
  | "contribution_underfunded"
  | "coast_not_reachable"
  | "partial_never_starts"
  | "partial_never_fully_retires"
  | "pension_unpaid_during_partial"
  | "no_volatility_declared"
  | "target_retirement_age_missing";

export type RetirementNoticeTone = "danger" | "warn";

export type RetirementNotice = {
  code: RetirementNoticeCode;
  tone: RetirementNoticeTone;
  text: string;
};

/**
 * Los avisos de `warnings[]` traducidos, ya ordenados por precedencia: **primero lo que invalida
 * el plan (rojo), después lo que lo degrada o lo hace menos fiable**.
 *
 * Vive aparte de las tarjetas porque la vista los pinta en dos sitios: los que suben al panel de
 * resultado y los que bajan al «Detalle» (`retirementDetailRows`). Duplicar la traducción habría
 * dejado dos catálogos de copy para los mismos literales.
 *
 * `no_volatility_declared` merece su párrafo: NO es un fallo del plan, es un fallo del INDICADOR.
 * Sin ningún activo con σ el sorteo no dispersa, los 2.500 caminos son el mismo camino y el éxito
 * sale 0 % o 100 % por construcción. Ese 100 % es la lectura más cara de toda la pantalla, así que
 * el aviso dice explícitamente que no mide riesgo en vez de limitarse a describir la carencia.
 *
 * **Cada aviso es UNA línea con verbo** (2026-10-10); el porqué vive en el `<?>` de la cifra
 * afectada. Y `strategy_pension_bridge_migrated` **no se traduce**: el servidor lo sigue emitiendo
 * para los perfiles migrados desde «Puente hasta la pensión», pero un aviso de migración es código
 * de un solo uso y el owner lo retiró de la SPA — el cambio vive en el CHANGELOG.
 */
export function buildRetirementNotices(
  series: RetirementNoticeSeries | null | undefined,
  targetRetirementAge: number | null,
): RetirementNotice[] {
  const notices: RetirementNotice[] = [];
  if (!series) return notices;
  const warnings = new Set(series.warnings ?? []);

  if (series.contribution_underfunded === true) {
    notices.push({
      code: "contribution_underfunded",
      tone: "danger",
      text:
        targetRetirementAge != null
          ? `No llegas a los ${targetRetirementAge} ni invirtiendo todo tu sobrante.`
          : "No llegas a tu edad objetivo ni invirtiendo todo tu sobrante.",
    });
  }
  if (warnings.has("coast_not_reachable")) {
    notices.push({
      code: "coast_not_reachable",
      tone: "warn",
      text: "No hay mes coast: no puedes dejar de aportar y llegar.",
    });
  }
  if (warnings.has("partial_never_starts")) {
    notices.push({
      code: "partial_never_starts",
      tone: "warn",
      text: "La jornada reducida no empieza en ningún mes del horizonte.",
    });
  }
  if (warnings.has("partial_never_fully_retires")) {
    notices.push({
      code: "partial_never_fully_retires",
      tone: "warn",
      text: "Tras la jornada reducida nunca te jubilas del todo.",
    });
  }
  if (warnings.has("pension_unpaid_during_partial")) {
    notices.push({
      code: "pension_unpaid_during_partial",
      tone: "warn",
      text: "Tu pensión empieza en la jornada reducida: hasta el final solo cobras su fracción.",
    });
  }
  if (warnings.has("no_volatility_declared")) {
    notices.push({
      code: "no_volatility_declared",
      tone: "warn",
      text: "Sin volatilidad declarada, el éxito no mide riesgo.",
    });
  }
  if (warnings.has("target_retirement_age_missing")) {
    notices.push({
      code: "target_retirement_age_missing",
      tone: "warn",
      text: "Falta tu edad de jubilación: se simula como FIRE clásico.",
    });
  }

  return notices;
}

/** Una fila del «Detalle» plegado. `tone` solo lo llevan los avisos. */
export type RetirementDetailRow = {
  key: string;
  label: string;
  value: string;
  tone?: RetirementNoticeTone;
  /** Ayuda del catálogo, cuando la fila mide algo que su rótulo no puede explicar entero. */
  helpId?: PlanHelpTextId;
};

/**
 * Lo que la cabecera de 3 tarjetas ya no puede llevar, en el «Detalle» plegado.
 *
 * No es un cajón de sastre: son las lecturas de SEGUNDO orden —las que acotan o auditan una cifra
 * de arriba en vez de responder una pregunta propia— más los avisos.
 *
 * - **Número FIRE clásico**: tu gasto de jubilación anual ÷ el SWR del perfil (25× solo al 4 %)
 *   MÁS las cuotas de deuda que te quedan (el motor suma `debt_payments_remaining`, `target.rs`),
 *   sin restar la pensión. Sobrevive como
 *   LECTURA (S7) y como pin de `fire-parity.json`; ya no decide nada. Baja al detalle justamente
 *   porque durante media 5.0.0 fue el objetivo que disparaba la jubilación, y arriba invitaría a
 *   seguir leyéndolo así.
 * - **Fecha al 100 % / al 90 %**: las dos fechas que ACOTAN la del umbral configurado («al 100 %
 *   serían ocho años más»). `null` con el plan resuelto es «nunca», que es un resultado; con el
 *   plan sin resolver la fila no se pinta, porque ahí `null` sí sería «todavía no se sabe». En
 *   `asap` tampoco: ahí van bajo la tarjeta «Fecha válida».
 * - **Los avisos**, con su tono, en el orden de precedencia de `buildRetirementNotices`.
 */
export function retirementDetailRows(
  input: RetirementTilesV2Input,
): RetirementDetailRow[] {
  const { series, currencyIso, monthLabel, targetRetirementAge } = input;
  const rows: RetirementDetailRow[] = [];
  if (!series) return rows;
  const money = (s: string | null | undefined) => formatCurrencyOrDash(s, currencyIso);
  const planReady = planUnavailableReason(series) == null;

  if (series.fire_number_classic_today != null) {
    rows.push({
      key: "fire_number_classic",
      label: "Número FIRE clásico",
      value: money(series.fire_number_classic_today),
      helpId: "retirement.fire_number_classic",
    });
  }

  // Las dos cotas de la fecha. Solo con el plan resuelto: sin él, un «nunca» sería una afirmación
  // sobre un sorteo que no ha terminado. Y no en `asap`: ahí ya son la sublínea de la tarjeta
  // «Fecha válida» (2026-10-10), y dos veces la misma cifra en la misma pantalla es ruido.
  // Y tampoco mientras el nivel 2 esté en vuelo: el contrato publica `null` en las dos cotas
  // hasta que aterriza, y «nunca» durante esos segundos sería una mentira (2026-10-10).
  if (
    planReady &&
    series.strategy !== "asap" &&
    series.needed_capital_curve_state !== "computing"
  ) {
    const bound = (mi: number | null | undefined) =>
      finite(mi) ? monthLabel(mi) : "nunca";
    rows.push({
      key: "safe_date_100",
      label: "Fecha al 100 %",
      value: bound(series.safe_date_at_100_month_index),
      helpId: "retirement.safe_date",
    });
    rows.push({
      key: "safe_date_90",
      label: "Fecha al 90 %",
      value: bound(series.safe_date_at_90_month_index),
      helpId: "retirement.safe_date",
    });
  }

  // «Semilla y caminos» se retiró de aquí el 2026-10-10: duplicaba el pie de «Las cifras del
  // riesgo» (`riskFootnote`), que es donde se lee junto a la precisión del sorteo.

  for (const n of buildRetirementNotices(series, targetRetirementAge)) {
    rows.push({ key: `notice:${n.code}`, label: "Aviso", value: n.text, tone: n.tone });
  }

  return rows;
}
