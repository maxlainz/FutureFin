/**
 * Modelo PURO de las tarjetas de estrategia y de los avisos de la vista Jubilación
 * (5.0.0, D16/D17/D31 y tabla §C del plan de #207).
 *
 * Cada estrategia contesta preguntas DISTINTAS y por eso enseña tarjetas distintas: «¿cuánto
 * tengo que ahorrar para llegar a los 55?» no existe en «Cuanto antes», y «¿cuál es mi número
 * coast?» solo existe en Coast FIRE. La correspondencia estrategia → tarjetas es una tabla del
 * contrato (§C), no una preferencia de layout, así que vive aquí con un test que la fija y la
 * vista se limita a pintar `MetricCard` con lo que salga.
 *
 * Tres reglas que este módulo NO puede romper:
 *
 *  1. **`null` no es cero.** Un `required_contribution_monthly` ausente significa «esta
 *     estrategia no responde a esa pregunta», y se pinta con guion — nunca con un 0 €, que se
 *     leería como «no necesitas ahorrar nada». Lo mismo con `underfunded`, donde `null` es «la
 *     pregunta no aplica» y `false` es «vas bien»: colapsarlos pinta de verde un plan que nadie
 *     ha evaluado.
 *  2. **El margen tiene DOS bases** y la copia lo dice (§B.7): con una edad objetivo es «lo que
 *     te sobra del máximo sobrante una vez cubierto el plan»; con Coast FIRE es «todo tu
 *     sobrante, pero solo desde el mes coast» — y antes de ese mes vale 0 de verdad, no por
 *     falta de dato. Publicar los dos con la misma frase haría que el mismo número significara
 *     dos cosas.
 *  3. **Las unidades del contrato mandan.** `bridge_effective_withdrawal_pct` y
 *     `bridge_discount_annual_pct` son PORCENTAJES (`6.5` = 6,5 %); `pension_coverage_ratio` es
 *     una FRACCIÓN (`0.6` = 60 %). Formatearlos con el helper equivocado multiplica o divide por
 *     100 lo que el usuario lee.
 *
 * Lo que este módulo NO hace: dibujar nada, resolver fechas (recibe un rotulador de meses de la
 * vista, que es quien sabe si el eje va en fechas o en edades) ni tocar la serie
 * `disposable_capital` — D31 la deja fuera del chart y fuera de aquí: el margen es tile.
 */

import type {
  ProjectionSeriesApi,
  RetirementStrategyApi,
  TargetBasisApi,
} from "../api/types";
import type { HelpTextId } from "./helpTexts";
import { formatMonthSpanEs } from "./duration";
import {
  formatCurrencyOrDash,
  formatFractionAsPercent,
  formatPercentAmount,
} from "./format";
import { formatYearsEsFromMonths } from "./projection-chart";

/** Los campos de la respuesta que deciden las tarjetas. Un `Pick` y no la respuesta entera para
 *  que un test pueda escribir el caso mínimo sin inventarse una proyección completa. */
export type RetirementTileSeries = Pick<
  ProjectionSeriesApi,
  | "strategy"
  | "required_contribution_monthly"
  | "required_contribution_search_ceiling"
  | "underfunded"
  | "disposable_monthly"
  | "disposable_capital_at_retirement"
  | "disposable_capital_today"
  | "coast_fire_month_index"
  | "coast_number"
  | "partial_gap_target"
  | "partial_phase_capital_growing"
  | "pension_start_month_index"
  | "bridge_effective_withdrawal_pct"
  | "pension_coverage_ratio"
  | "bridge_discount_annual_pct"
  | "warnings"
>;

export type RetirementTileTone = "default" | "danger";

/** Una tarjeta lista para `MetricCard`: mismos slots y mismos nombres, sin traducción intermedia. */
export type RetirementTileModel = {
  /** Key de React y del test. Estable por tarjeta, no por posición. */
  key: string;
  label: string;
  helpId: HelpTextId;
  value: string;
  /** Primer slot bajo la cifra (`MetricCard` lo pinta entre paréntesis). */
  parenthetical?: string;
  /** Segundo slot, bajo el anterior. */
  detail?: string;
  tone: RetirementTileTone;
};

/** Literales cerrados de `warnings[]` que esta vista sabe explicar. `birth_date_missing` NO está:
 *  lo cuenta el banner de alta (D33), y decirlo dos veces en la misma pantalla es ruido. */
export type RetirementNoticeCode =
  | "retire_at_age_underfunded"
  | "coast_not_reachable"
  | "partial_phase_capital_shrinking"
  | "bridge_discount_no_liquid_assets"
  | "bridge_discount_clamped"
  | "target_retirement_age_missing";

export type RetirementNoticeTone = "danger" | "warn";

export type RetirementNotice = {
  code: RetirementNoticeCode;
  tone: RetirementNoticeTone;
  text: string;
};

export type RetirementTilesInput = {
  series: RetirementTileSeries | null | undefined;
  /** ISO de la divisa del hogar (`""` degrada a número sin símbolo, como el resto de la app). */
  currencyIso: string;
  /**
   * Rotulador de un mes de la rejilla → etiqueta del eje («2043», «a los 55»). Lo inyecta la
   * vista porque depende del modo del eje, de la fecha de nacimiento y de la zona horaria, y
   * ninguna de las tres es asunto de este módulo.
   */
  monthLabel: (monthIndex: number) => string;
  /** Edad objetivo del perfil, para nombrarla en el rojo. `null` ⇒ «tu edad objetivo». */
  targetRetirementAge: number | null;
};

export type RetirementTilesModel = {
  tiles: RetirementTileModel[];
  /** Avisos ya ordenados: primero lo que invalida el plan, después lo que le falta. */
  notices: RetirementNotice[];
};

/**
 * Estrategias que resuelven una aportación necesaria contra una edad (§B.7). `partial` solo
 * cuando el servidor de verdad ha resuelto —el perfil puede no tener edad total y entonces el
 * campo viene `null` y la tarjeta se cae sola.
 */
function hasAgeSolve(strategy: RetirementStrategyApi | null | undefined): boolean {
  return strategy === "retire_at_age" || strategy === "partial";
}

/**
 * §C: qué tarjetas enseña cada estrategia, con las cifras del servidor tal cual.
 *
 * Ninguna tarjeta se inventa por «coherencia visual»: una fila de tarjetas con guiones dice
 * «esto se calcula y hoy no hay dato», y eso es falso cuando la estrategia simplemente no hace
 * esa pregunta. Por eso la lista es variable y no un hueco fijo.
 *
 * @deprecated — retirar en U1b/U2. La cabecera de resultados del rediseño (U7) es **una frase de
 * hito + como mucho 3 tarjetas**, y esta versión emite hasta cinco con dos slots de subtítulo por
 * tarjeta. Su sustituta es `buildRetirementTilesV2` (+ `retirementDetailRows` para lo que baja al
 * «Detalle»). Se conserva mientras `RetirementView.tsx` siga consumiéndola.
 */
export function buildRetirementTiles(
  input: RetirementTilesInput,
): RetirementTilesModel {
  const { series, currencyIso, monthLabel, targetRetirementAge } = input;
  const tiles: RetirementTileModel[] = [];
  const notices: RetirementNotice[] = [];
  if (!series) return { tiles, notices };

  const strategy = series.strategy ?? null;
  const money = (s: string | null | undefined) => formatCurrencyOrDash(s, currencyIso);

  // ── «Ahorro necesario» (retire_at_age / partial con edad) ────────────────────────────────
  //
  // El rojo de D17 vive en la tarjeta además de en el banner: quien mira la cifra tiene que ver
  // ahí mismo que no es «lo que hay que ahorrar» sino «todo lo que hay, y aun así no llega».
  if (hasAgeSolve(strategy) && series.required_contribution_monthly != null) {
    const underfunded = series.underfunded === true;
    const ceiling = series.required_contribution_search_ceiling;
    tiles.push({
      key: "required_contribution",
      label: "Ahorro necesario",
      helpId: "retirement.required_contribution",
      value: money(series.required_contribution_monthly),
      parenthetical:
        ceiling != null ? `de ${money(ceiling)}/mes de sobrante` : undefined,
      detail: underfunded ? "es TODO tu sobrante y no basta" : undefined,
      tone: underfunded ? "danger" : "default",
    });
  }

  // ── «Mes coast» y «Número coast» (solo coast) ────────────────────────────────────────────
  if (strategy === "coast") {
    const coastMi = series.coast_fire_month_index;
    const reachable = typeof coastMi === "number" && Number.isFinite(coastMi);
    tiles.push({
      key: "coast_month",
      label: "Mes coast",
      helpId: "retirement.coast_month",
      value: reachable ? monthLabel(coastMi as number) : "No alcanzable",
      parenthetical: reachable
        ? (coastMi as number) <= 0
          ? "ya puedes dejar de aportar"
          : `dentro de ${formatYearsEsFromMonths(coastMi as number)}`
        : "ni aportando siempre llegas",
      tone: "default",
    });
    tiles.push({
      key: "coast_number",
      label: "Número coast",
      helpId: "retirement.coast_number",
      value: money(series.coast_number),
      parenthetical: reachable ? "líquido al entrar en el mes coast" : undefined,
      tone: "default",
    });
  }

  // ── «Margen disponible» (D16/D31) — dos bases, dos copias ────────────────────────────────
  if (
    (hasAgeSolve(strategy) || strategy === "coast") &&
    series.disposable_monthly != null
  ) {
    const capAtRet = series.disposable_capital_at_retirement;
    const capToday = series.disposable_capital_today;
    const coastMi = series.coast_fire_month_index;
    const coastPending =
      strategy === "coast" &&
      (typeof coastMi !== "number" || !Number.isFinite(coastMi) || coastMi > 0);
    tiles.push({
      key: "disposable",
      label: "Margen disponible",
      helpId: "retirement.disposable",
      value: money(series.disposable_monthly),
      parenthetical: coastPending
        ? typeof coastMi === "number" && Number.isFinite(coastMi)
          ? "al mes hasta el mes coast; desde él, todo tu sobrante"
          : "al mes: sin mes coast no hay sobrante que liberar"
        : capAtRet != null
          ? `al mes · ${money(capAtRet)} acumulados al jubilarte`
          : "al mes",
      detail:
        !coastPending && capToday != null
          ? `${money(capToday)} en dinero de hoy`
          : undefined,
      tone: "default",
    });
  }

  // ── «Hueco de media jornada» (solo partial) ──────────────────────────────────────────────
  if (strategy === "partial" && series.partial_gap_target != null) {
    // `partial_phase_capital_growing` es `true` / `false` / `null`, y los tres dicen cosas
    // distintas: creció, menguó, y «no hubo fase parcial que medir». El `null` no pinta línea.
    const growing = series.partial_phase_capital_growing;
    tiles.push({
      key: "partial_gap",
      label: "Hueco de media jornada",
      helpId: "retirement.partial_gap",
      value: money(series.partial_gap_target),
      parenthetical: "capital que cubriría ese hueco a perpetuidad",
      detail:
        growing === true
          ? "el capital sigue creciendo en media jornada"
          : growing === false
            ? "el capital DECRECE en media jornada"
            : undefined,
      tone: growing === false ? "danger" : "default",
    });
  }

  // ── «Puente» (pension_bridge, o cualquier estrategia con pensión con fecha) ──────────────
  //
  // La condición es la PENSIÓN, no la estrategia: quien declara una pensión con fecha tiene un
  // puente que cubrir aunque se jubile por cruce, y esconderle la tasa efectiva sería esconder
  // justo lo que la perpetuidad disimula.
  const pensionMi = series.pension_start_month_index;
  if (typeof pensionMi === "number" && Number.isFinite(pensionMi)) {
    const eff = series.bridge_effective_withdrawal_pct;
    const cov = series.pension_coverage_ratio;
    const disc = series.bridge_discount_annual_pct;
    const parts: string[] = [];
    if (cov != null) parts.push(`la pensión cubre el ${formatFractionAsPercent(cov)} del gasto`);
    if (disc != null) parts.push(`descontado al ${formatPercentAmount(disc)} anual`);
    tiles.push({
      key: "bridge",
      label: "Puente hasta la pensión",
      helpId: "retirement.bridge",
      value: pensionMi <= 0 ? "Ya la cobras" : formatYearsEsFromMonths(pensionMi),
      parenthetical:
        eff != null
          ? `retiras el ${formatPercentAmount(eff)} del capital al año durante el puente`
          : undefined,
      detail: parts.length > 0 ? parts.join(" · ") : undefined,
      tone: "default",
    });
  }

  return { tiles, notices: buildRetirementNotices(series, targetRetirementAge) };
}

/**
 * Los avisos de `warnings[]` traducidos, ya ordenados por precedencia: **primero lo que invalida
 * el plan (rojo), después lo que lo degrada o lo hace más conservador**.
 *
 * Vive aparte de las tarjetas porque las dos generaciones de cabecera los necesitan igual: la v1
 * los pinta bajo su rejilla y la v2 los baja al «Detalle» (`retirementDetailRows`). Duplicar la
 * traducción habría dejado dos catálogos de copy para los mismos seis literales.
 *
 * `birth_date_missing` NO está a propósito: lo cuenta el banner de alta (D33), y decirlo dos
 * veces en la misma pantalla es ruido.
 */
export function buildRetirementNotices(
  series: RetirementTileSeries | null | undefined,
  targetRetirementAge: number | null,
): RetirementNotice[] {
  const notices: RetirementNotice[] = [];
  if (!series) return notices;
  const warnings = new Set(series.warnings ?? []);

  if (warnings.has("retire_at_age_underfunded") || series.underfunded === true) {
    notices.push({
      code: "retire_at_age_underfunded",
      tone: "danger",
      text:
        targetRetirementAge != null
          ? `Con tu ahorro actual no llegas a los ${targetRetirementAge} años. Te jubilarás igual —manda la edad— pero por debajo de tu objetivo.`
          : "Con tu ahorro actual no llegas a tu edad objetivo. Te jubilarás igual —manda la edad— pero por debajo de tu objetivo.",
    });
  }
  if (warnings.has("coast_not_reachable")) {
    notices.push({
      code: "coast_not_reachable",
      tone: "warn",
      text:
        "No hay mes coast: ni aportando todos los meses llegas al objetivo en tu edad. La línea discontinua del chart es lo mejor que da tu plan.",
    });
  }
  if (warnings.has("partial_phase_capital_shrinking")) {
    notices.push({
      code: "partial_phase_capital_shrinking",
      tone: "warn",
      text:
        "Durante la media jornada tu capital DECRECE: el ingreso parcial no cubre el gasto y la diferencia sale de la cartera.",
    });
  }
  if (warnings.has("bridge_discount_no_liquid_assets")) {
    notices.push({
      code: "bridge_discount_no_liquid_assets",
      tone: "warn",
      text:
        "Sin activos líquidos declarados no hay rentabilidad con la que descontar el puente: se calcula sin descuento, y el objetivo sale más alto.",
    });
  }
  // Hermano del anterior y distinto caso (5.0.0, pase de correcciones §H): SÍ hay líquidos, pero
  // su rentabilidad esperada es negativa. Descontar el puente a una tasa negativa lo encarecería
  // exponencialmente —y a horizontes largos se sale de lo calculable—, así que el motor la sube
  // a 0 y lo dice. El aviso existe porque el objetivo resultante NO es el que la configuración
  // del usuario describe, y sin la frase esa diferencia sería invisible.
  if (warnings.has("bridge_discount_clamped")) {
    notices.push({
      code: "bridge_discount_clamped",
      tone: "warn",
      text: "La rentabilidad esperada de tus líquidos es negativa: el puente se descuenta al 0 %.",
    });
  }
  if (warnings.has("target_retirement_age_missing")) {
    notices.push({
      code: "target_retirement_age_missing",
      tone: "warn",
      text:
        "Falta tu edad de jubilación objetivo: mientras tanto el plan se simula como «Cuanto antes».",
    });
  }

  return notices;
}

/** Rótulo de la base del objetivo para el subtítulo del tile «Patrimonio objetivo». */
export const TARGET_BASIS_TILE_LABEL: Record<"perpetuity" | "bridge_to_pension", string> = {
  perpetuity: "base: renta perpetua",
  bridge_to_pension: "base: puente hasta la pensión",
};

// ═════════════════════════════════════════════════════════════════════════════════════════════
// V2 — la cabecera de resultados del rediseño (U7)
// ═════════════════════════════════════════════════════════════════════════════════════════════

/** Cuántas tarjetas caben en la cabecera de resultados. U7: **una frase de hito + como mucho 3**. */
export const RETIREMENT_TILES_V2_CAP = 3;

/** Una tarjeta de la cabecera V2: **una sola cifra** y un subtítulo COMPLETO (U7 prohíbe
 *  truncarlo — el subtítulo es donde vive la base de la cifra, y media base es peor que ninguna). */
export type RetirementTileV2 = {
  /** Key de React y del test. Estable por tarjeta, nunca por posición. */
  key: string;
  label: string;
  value: string;
  /** Texto completo, puede ser largo. `undefined` = no hay nada que añadir, y entonces la vista
   *  reserva el slot igual (misma disciplina que el paréntesis de `MetricCard`). */
  subtitle?: string;
  tone: RetirementTileTone;
  helpId: HelpTextId;
};

/** Los campos de la serie que la cabecera V2 lee. Añade a los de V1 el objetivo, el mes de
 *  jubilación y el cruce puro: la tarjeta «Objetivo» y el puente de S8 los necesitan. */
export type RetirementTileV2Series = RetirementTileSeries &
  Pick<
    ProjectionSeriesApi,
    | "jubilacion_month_index"
    | "jubilacion_age"
    | "jubilacion_target_net_worth"
    | "jubilacion_target_net_worth_nominal"
    | "liquid_crossing_month_index"
  >;

export type RetirementTilesV2Input = {
  series: RetirementTileV2Series | null | undefined;
  currencyIso: string;
  /** Mes de la rejilla → etiqueta del eje. Lo inyecta la vista (fechas o edades). */
  monthLabel: (monthIndex: number) => string;
  /** Edad objetivo GUARDADA; respalda a `jubilacion_age` cuando no hay fecha de nacimiento. */
  targetRetirementAge: number | null;
  /** Base EFECTIVA del objetivo (R6). `null` ⇒ la tarjeta «Objetivo» va sin subtítulo. */
  targetBasis: TargetBasisApi | null;
  /** Edad de inicio de la pensión declarada. `null` ⇒ el puente se rotula sin edades. */
  pensionStartAge: number | null;
};

/**
 * La cabecera de resultados de Jubilación (U7): **como mucho 3 tarjetas, una cifra por tarjeta**.
 *
 * ## La regla de prioridad, que es lo que hay que acertar
 *
 * Los candidatos se construyen SIEMPRE en este orden, y **el orden ES la prioridad**:
 *
 * 1. **«Objetivo (euros de hoy)»** — primera y nunca se cae. Es la única cifra que todas las
 *    estrategias comparten y contra la que se leen las demás. (La versión NOMINAL «al cruce» ya
 *    no comparte tarjeta con ella: baja a `retirementDetailRows`, porque dos importes del mismo
 *    nombre en la misma tarjeta era la confusión que el catálogo de métricas documenta.)
 * 2. **Las de la estrategia**: `retire_at_age`/`partial` con solve ⇒ «Ahorro necesario» y
 *    «Margen disponible»; `coast` ⇒ «Mes coast» y «Número coast»; `partial` ⇒ «Hueco de media
 *    jornada».
 * 3. **El puente, siempre el último.** Existe con CUALQUIER estrategia que declare una pensión
 *    con fecha, pero es la lectura más contextual de las tres, así que es la primera en caerse.
 *
 * Al pasarse del tope se trunca **por el final**. Consecuencias que el test fija, porque son
 * decisiones y no accidentes:
 *
 * - `partial` con solve de edad enseña objetivo + ahorro + margen, y **pierde el hueco y el
 *   puente**: sin saber cuánto hay que ahorrar, el hueco de la fase parcial no se puede
 *   interpretar.
 * - `coast` nunca enseña margen (sus dos tarjetas propias ocupan los dos huecos), a diferencia
 *   de la V1. El margen de coast sigue publicándose por el servidor y se lee en el Resumen.
 * - `asap` enseña objetivo (+ puente si hay pensión): es la estrategia que menos preguntas hace.
 *
 * `null` sigue sin ser cero: una tarjeta cuya cifra el servidor no publica **no se emite**.
 */
export function buildRetirementTilesV2(
  input: RetirementTilesV2Input,
): RetirementTileV2[] {
  const { series, currencyIso, monthLabel } = input;
  if (!series) return [];
  const money = (s: string | null | undefined) => formatCurrencyOrDash(s, currencyIso);
  const strategy = series.strategy ?? null;
  const tiles: RetirementTileV2[] = [];

  // 1 · Objetivo — siempre primera, nunca se cae.
  tiles.push({
    key: "target",
    label: "Objetivo (euros de hoy)",
    value: money(series.jubilacion_target_net_worth),
    subtitle:
      input.targetBasis != null ? TARGET_BASIS_TILE_LABEL[input.targetBasis] : undefined,
    tone: "default",
    helpId: "retirement.target",
  });

  // 2 · Las de la estrategia.
  if (hasAgeSolve(strategy) && series.required_contribution_monthly != null) {
    const underfunded = series.underfunded === true;
    const ceiling = series.required_contribution_search_ceiling;
    const bits: string[] = [];
    if (ceiling != null) bits.push(`de ${money(ceiling)}/mes de sobrante`);
    if (underfunded) bits.push("es TODO tu sobrante y no basta");
    tiles.push({
      key: "required_contribution",
      label: "Ahorro necesario",
      value: money(series.required_contribution_monthly),
      subtitle: bits.length > 0 ? bits.join(" · ") : undefined,
      tone: underfunded ? "danger" : "default",
      helpId: "retirement.required_contribution",
    });
    if (series.disposable_monthly != null) {
      const bitsM: string[] = ["al mes"];
      if (series.disposable_capital_at_retirement != null) {
        bitsM.push(`${money(series.disposable_capital_at_retirement)} acumulados al jubilarte`);
      }
      tiles.push({
        key: "disposable",
        label: "Margen disponible",
        value: money(series.disposable_monthly),
        subtitle: bitsM.join(" · "),
        tone: "default",
        helpId: "retirement.disposable",
      });
    }
  }

  if (strategy === "coast") {
    const coastMi = series.coast_fire_month_index;
    const reachable = typeof coastMi === "number" && Number.isFinite(coastMi);
    tiles.push({
      key: "coast_month",
      label: "Mes coast",
      value: reachable ? monthLabel(coastMi as number) : "No alcanzable",
      subtitle: reachable
        ? (coastMi as number) <= 0
          ? "ya puedes dejar de aportar"
          : `dentro de ${formatMonthSpanEs(coastMi as number)}`
        : "ni aportando todos los meses llegas al objetivo en tu edad",
      tone: "default",
      helpId: "retirement.coast_month",
    });
    tiles.push({
      key: "coast_number",
      label: "Número coast",
      value: money(series.coast_number),
      subtitle: reachable ? "líquido al entrar en el mes coast" : undefined,
      tone: "default",
      helpId: "retirement.coast_number",
    });
  }

  if (strategy === "partial" && series.partial_gap_target != null) {
    // `partial_phase_capital_growing` tiene TRES valores y los tres dicen cosas distintas:
    // creció, menguó, y «no hubo fase parcial que medir». El `null` no añade línea.
    const growing = series.partial_phase_capital_growing;
    const bits = ["capital que cubriría ese hueco a perpetuidad"];
    if (growing === true) bits.push("el capital sigue creciendo en media jornada");
    if (growing === false) bits.push("el capital DECRECE en media jornada");
    tiles.push({
      key: "partial_gap",
      label: "Hueco de media jornada",
      value: money(series.partial_gap_target),
      subtitle: bits.join(" · "),
      tone: growing === false ? "danger" : "default",
      helpId: "retirement.partial_gap",
    });
  }

  // 3 · El puente, siempre el último candidato.
  const bridge = bridgeTile(input);
  if (bridge) tiles.push(bridge);

  return tiles.slice(0, RETIREMENT_TILES_V2_CAP);
}

/**
 * La tarjeta de puente, con la corrección **S8**.
 *
 * El bug que corrige: la V1 rotulaba el puente con `formatYearsEsFromMonths(pension_start)`, es
 * decir **meses desde HOY hasta la pensión** — que incluye los años que faltan para jubilarse. Un
 * puente real de 12 años se leía como 22, y el número era perfectamente plausible.
 *
 * La longitud del puente es el TRAMO `pension_start_month_index − jubilacion_month_index`, los
 * dos en la misma rejilla (mes 0 = hoy). Sin mes de jubilación **no hay puente que medir** y la
 * tarjeta no se emite: un puente necesita sus dos extremos, y publicar solo la fecha de la
 * pensión invita otra vez a contar desde hoy.
 *
 * Rótulo y subtítulo, tal y como los pide U7: label «Puente 60→72», valor «12 años», subtítulo
 * «retiras el 8,7 % del capital al año · la pensión cubre el 96 % del gasto». La tasa de
 * descuento del puente NO entra aquí — es un supuesto, no un resultado, y vive en el «Detalle».
 */
function bridgeTile(input: RetirementTilesV2Input): RetirementTileV2 | null {
  const s = input.series;
  if (!s) return null;
  const pensionMi = s.pension_start_month_index;
  const retMi = s.jubilacion_month_index;
  if (typeof pensionMi !== "number" || !Number.isFinite(pensionMi)) return null;
  if (typeof retMi !== "number" || !Number.isFinite(retMi)) return null;

  const months = pensionMi - retMi;
  const fromAge = s.jubilacion_age ?? input.targetRetirementAge ?? null;
  const toAge = input.pensionStartAge;
  const label =
    fromAge != null && toAge != null
      ? `Puente ${fromAge}→${toAge}`
      : "Puente hasta la pensión";

  const bits: string[] = [];
  if (s.bridge_effective_withdrawal_pct != null) {
    bits.push(
      `retiras el ${formatPercentAmount(s.bridge_effective_withdrawal_pct)} del capital al año`,
    );
  }
  if (s.pension_coverage_ratio != null) {
    bits.push(`la pensión cubre el ${formatFractionAsPercent(s.pension_coverage_ratio)} del gasto`);
  }

  return {
    key: "bridge",
    label,
    value: months > 0 ? formatMonthSpanEs(months) : "Sin puente",
    subtitle:
      months > 0
        ? bits.length > 0
          ? bits.join(" · ")
          : undefined
        : ["cobras la pensión desde el primer mes de jubilación", ...bits].join(" · "),
    tone: "default",
    helpId: "retirement.bridge",
  };
}

/** Una fila del «Detalle» plegado. `tone` solo lo llevan los avisos. */
export type RetirementDetailRow = {
  key: string;
  label: string;
  value: string;
  tone?: RetirementNoticeTone;
};

/**
 * Lo que la cabecera de 3 tarjetas ya no puede llevar, en el «Detalle» plegado (U7).
 *
 * No es un cajón de sastre: son las lecturas de SEGUNDO orden —las que matizan una cifra de
 * arriba en vez de responder una pregunta propia— más los avisos. Que estén plegadas no las hace
 * opcionales; que estén **fuera de la cabecera** es lo que permite que la cabecera se lea de un
 * vistazo.
 *
 * - **Objetivo al cruce (nominal)**: el mismo objetivo en euros del mes del cruce. Difiere del de
 *   arriba en más de 2× a décadas vista, y compartir tarjeta con él es el enredo que el catálogo
 *   de métricas documenta en `retirement.target`.
 * - **Cruce del objetivo**: solo cuando cae en un mes DISTINTO del de la jubilación efectiva —
 *   con `asap` coinciden y repetirlo diría que son dos hechos.
 * - **Margen acumulado en dinero de hoy**, **descuento del puente**, **cobertura de la pensión**.
 * - **Los avisos**, con su tono, en el orden de precedencia de `buildRetirementNotices`.
 */
export function retirementDetailRows(
  input: RetirementTilesV2Input,
): RetirementDetailRow[] {
  const { series, currencyIso, monthLabel, targetRetirementAge } = input;
  const rows: RetirementDetailRow[] = [];
  if (!series) return rows;
  const money = (s: string | null | undefined) => formatCurrencyOrDash(s, currencyIso);

  if (series.jubilacion_target_net_worth_nominal != null) {
    rows.push({
      key: "target_nominal",
      label: "Objetivo al cruce (euros de ese mes)",
      value: money(series.jubilacion_target_net_worth_nominal),
    });
  }

  const crossing = series.liquid_crossing_month_index;
  if (
    typeof crossing === "number" &&
    Number.isFinite(crossing) &&
    crossing !== series.jubilacion_month_index
  ) {
    rows.push({
      key: "liquid_crossing",
      label: "Cruce del objetivo",
      value: monthLabel(crossing),
    });
  }

  if (series.disposable_capital_today != null) {
    rows.push({
      key: "disposable_today",
      label: "Margen acumulado en dinero de hoy",
      value: money(series.disposable_capital_today),
    });
  }

  if (series.bridge_discount_annual_pct != null) {
    rows.push({
      key: "bridge_discount",
      label: "Descuento del puente",
      value: formatPercentAmount(series.bridge_discount_annual_pct),
    });
  }

  if (series.pension_coverage_ratio != null) {
    rows.push({
      key: "pension_coverage",
      label: "Cobertura de la pensión",
      value: formatFractionAsPercent(series.pension_coverage_ratio),
    });
  }

  for (const n of buildRetirementNotices(series, targetRetirementAge)) {
    rows.push({ key: `notice:${n.code}`, label: "Aviso", value: n.text, tone: n.tone });
  }

  return rows;
}
