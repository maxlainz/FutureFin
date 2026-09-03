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
} from "../api/types";
import type { HelpTextId } from "./helpTexts";
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
 */
export function buildRetirementTiles(
  input: RetirementTilesInput,
): RetirementTilesModel {
  const { series, currencyIso, monthLabel, targetRetirementAge } = input;
  const tiles: RetirementTileModel[] = [];
  const notices: RetirementNotice[] = [];
  if (!series) return { tiles, notices };

  const strategy = series.strategy ?? null;
  const warnings = new Set(series.warnings ?? []);
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

  // ── Avisos ───────────────────────────────────────────────────────────────────────────────
  //
  // Precedencia: primero lo que invalida el plan (rojo), después lo que lo degrada o lo hace más
  // conservador. `birth_date_missing` no está aquí a propósito (banner de alta, D33).
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
  if (warnings.has("target_retirement_age_missing")) {
    notices.push({
      code: "target_retirement_age_missing",
      tone: "warn",
      text:
        "Falta tu edad de jubilación objetivo: mientras tanto el plan se simula como «Cuanto antes».",
    });
  }

  return { tiles, notices };
}

/** Rótulo de la base del objetivo para el subtítulo del tile «Patrimonio objetivo». */
export const TARGET_BASIS_TILE_LABEL: Record<"perpetuity" | "bridge_to_pension", string> = {
  perpetuity: "base: renta perpetua",
  bridge_to_pension: "base: puente hasta la pensión",
};
