/**
 * **La FRASE-HITO del plan** (5.0.0, modelo v2 «el éxito define la fecha», C1-C8 de #207).
 *
 * El resultado de una simulación de jubilación no es un número: es un hito con fecha, edad y
 * probabilidad, y con la estrategia mandando sobre qué significa cada uno. Por eso vive aquí y
 * no en una vista: la misma oración la pintan la cabecera de Jubilación, la tarjeta del Resumen
 * y —en tercera persona— cada miembro del hogar, y tres copias divergen al primer cambio de
 * estrategia.
 *
 * ## Qué cambió con el modelo v2
 *
 * La versión anterior narraba un CRUCE («te jubilas cuando tu patrimonio cubre el objetivo»).
 * En v2 no hay objetivo: la fecha la fija el ÉXITO (`retirement_date_basis`), y la frase tiene
 * que decir **con cuántos escenarios de cada cien** aguanta el plan, porque esa es la cifra que
 * decidió la fecha. Una frase que solo dijera «te jubilas en 2043» ocultaría justo el dato que
 * la hace verdad o mentira.
 *
 * Reglas que este módulo NO puede romper:
 *
 *  1. **Todos los `*_month_index` viven en la MISMA rejilla** (mes 0 = hoy) y jamás son
 *     posiciones de array. Nada se calcula contando puntos de `points[]`, que con
 *     `density=hybrid` no son meses.
 *  2. **Nunca se rotula una edad que el motor no leyó** (bug B5). Las edades salen de
 *     `safe_date_age` / `jubilacion_age` —las que el servidor calculó con la fecha de
 *     nacimiento— o del resolutor `ageAt` que inyecta la vista con el MISMO calendario del eje.
 *     La edad GUARDADA del perfil (`targetRetirementAge`) solo aparece como **lo que pediste**,
 *     nunca emparejada con un mes que el motor resolvió sin ella. Y sin fecha de nacimiento no
 *     hay plan: el servidor lo dice con `plan_absent_reason: "birth_date_missing"` (C5) y la
 *     frase lo repite en vez de inventar una edad.
 *  3. **Un índice `null` no es un cero.** Cada estado tiene su frase: «nunca» para una fecha al
 *     100 % inalcanzable, la razón de `plan_absent_reason` cuando no hay bloque, y la frase de
 *     `not_reachable` con lo más cerca que se llegó. Ninguna se rellena con un guion.
 *  4. **El umbral no se re-juzga aquí.** El tono aplica la MISMA regla C3 que el servidor
 *     (`success_wilson_low ≥ umbral/100`, y con umbral 100 cero fallos) leyendo los campos que
 *     él publica — no es un segundo semáforo con otra muestra.
 *  5. **El rotulador de meses lo inyecta la vista** (fechas o edades, con su zona horaria). Este
 *     módulo no resuelve calendarios.
 */

import type {
  HouseholdMemberProjectionApi,
  ProjectionSeriesApi,
  RetirementStrategyApi,
} from "../api/types";
import type { CoastModeApi } from "../api/types";
import { formatCurrencyAmount } from "./format";
import { RETIREMENT_STRATEGY_LABEL } from "./retirementProfile";
import { scenariosPerHundred } from "./risk-bands";

/** Tono de la frase — el mismo vocabulario de estado que `plan-card.ts` y el design system
 *  (`.retirement-sentence--ok|warn|danger`). `ok` ES el estado de ÉXITO: el plan cumple su
 *  umbral. `warn` = pendiente, o por debajo del umbral pero con margen para llegar. `danger` =
 *  no hay fecha, no se llega ni ahorrándolo todo, o falta la fecha de nacimiento. */
export type PlanSentenceTone = "ok" | "warn" | "danger";

/** El segundo hito que la estrategia añade a la jubilación, cuando lo tiene. */
export type PlanSecondaryKind = "coast" | "partial" | "pension";

/** Qué fijó la fecha del plan (eco de `retirement_date_basis`). */
export type PlanDateBasis = NonNullable<ProjectionSeriesApi["retirement_date_basis"]>;

/** Por qué el bloque «plan» entero no viaja (eco de `plan_absent_reason`). */
export type PlanAbsentReason = NonNullable<ProjectionSeriesApi["plan_absent_reason"]>;

/**
 * Las piezas con las que se armó la frase, publicadas para que la vista pueda reusarlas (un
 * subtítulo, un `aria-label`, la tira de fases) **sin volver a calcularlas**.
 */
export type PlanSentenceParts = {
  strategy: RetirementStrategyApi | null;
  /** Qué fijó la fecha. `null` en un backend que no publica el bloque «plan». */
  basis: PlanDateBasis | null;
  /** Por qué no hay bloque «plan». `null` ⟺ lo hay. */
  absentReason: PlanAbsentReason | null;
  /** Mes EFECTIVO de jubilación en la rejilla (0 = hoy). `null` = no la hay. */
  retirementMonthIndex: number | null;
  retirementLabel: string | null;
  /** Años cumplidos en la jubilación, **tal y como los publicó el servidor**. Nunca la edad
   *  guardada del perfil (B5). */
  retirementAge: number | null;
  /** Meses de HOY a la jubilación. `null` sin jubilación; `0` = ya. */
  monthsToRetirement: number | null;
  /** FRACCIÓN [0,1] del éxito en la fecha del plan, tal cual llega. */
  successOfPlan: number | null;
  /** Esa misma fracción como «N de cada 100», con los topes anti-mentira. */
  successOutOfHundred: number | null;
  successThresholdPct: number | null;
  /** ¿Cumple el umbral con la regla C3? `null` = no hay con qué juzgarlo. */
  meetsThreshold: boolean | null;
  secondaryKind: PlanSecondaryKind | null;
  secondaryMonthIndex: number | null;
  secondaryLabel: string | null;
  /** **S8**: `pension_start_month_index − jubilación`. `null` si falta cualquiera de los dos;
   *  `0` o negativo = la pensión ya está en marcha al jubilarse. */
  bridgeMonths: number | null;
  /** €/mes que harían falta para llegar a la edad pedida (`contribution_required_monthly`). */
  contributionRequiredMonthly: string | null;
  /** `contribution_underfunded`: ni con todo el sobrante se llega. **`null` = la pregunta no
   *  aplica a esta estrategia**, nunca `false` para decir «no aplica». */
  underfunded: boolean | null;
};

export type PlanSentence = {
  text: string;
  tone: PlanSentenceTone;
  parts: PlanSentenceParts;
};

/**
 * Cómo rotula el eje la vista. En `ages` el `monthLabel` ya devuelve una edad, así que el
 * paréntesis «(a los N)» sobraría y se omite: decir la edad dos veces la hace ilegible.
 */
export type PlanSentenceAgeMode = "dates" | "ages";

/** Los campos de la serie que deciden la frase. Un `Pick` para que un test escriba el caso
 *  mínimo sin inventarse una proyección entera. */
export type PlanSentenceSeries = Pick<
  ProjectionSeriesApi,
  | "strategy"
  | "jubilacion_month_index"
  | "jubilacion_age"
  | "partial_retirement_month_index"
  | "pension_start_month_index"
  | "retirement_date_basis"
  | "success_threshold_pct"
  | "safe_date_month_index"
  | "safe_date_age"
  | "safe_date_at_100_month_index"
  | "safe_date_at_90_month_index"
  | "success_of_plan"
  | "success_wilson_low"
  | "contribution_required_monthly"
  | "contribution_underfunded"
  | "coast_stop_month_index"
  | "partial_start_month_index"
  | "success_by_retirement_year"
  | "plan_absent_reason"
  | "horizon_lifespan_age"
  | "warnings"
>;

export type PlanSentenceInput = {
  series: PlanSentenceSeries | null | undefined;
  /** Edad objetivo GUARDADA del perfil. Solo se usa para decir **lo que pediste** (la edad de
   *  `retire_at_age` / `coast` modo A), jamás para rotular un mes (B5). */
  targetRetirementAge: number | null;
  monthLabel: (monthIndex: number) => string;
  /**
   * Edad cumplida en un mes de la rejilla, resuelta por la VISTA con el mismo calendario que el
   * eje (`anchor_date_ymd` + fecha de nacimiento). Solo se consulta donde el servidor **no**
   * publica edad: el mes coast y el de media jornada. Sin resolutor, el paréntesis de edad
   * simplemente no se pinta — nunca se estima restando años.
   */
  ageAt?: (monthIndex: number) => number | null;
  /** ISO de la divisa de la instalación: la frase de `retire_at_age` lleva un importe. */
  currencyIso: string;
  /** Default `dates`. */
  ageMode?: PlanSentenceAgeMode;
  /** Modo de coast del perfil (borrador incluido). Sin él se deduce de `retirement_date_basis`:
   *  modo A fija la edad de jubilación (`target_age`), modo B la deja salir del sorteo. */
  coastMode?: CoastModeApi | null;
};

const EMPTY_PARTS: PlanSentenceParts = {
  strategy: null,
  basis: null,
  absentReason: null,
  retirementMonthIndex: null,
  retirementLabel: null,
  retirementAge: null,
  monthsToRetirement: null,
  successOfPlan: null,
  successOutOfHundred: null,
  successThresholdPct: null,
  meetsThreshold: null,
  secondaryKind: null,
  secondaryMonthIndex: null,
  secondaryLabel: null,
  bridgeMonths: null,
  contributionRequiredMonthly: null,
  underfunded: null,
};

function idx(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/**
 * ¿Cumple el plan su umbral? **La regla es la del servidor (C3), no una segunda opinión**: con
 * umbral < 100 manda el límite inferior del intervalo de Wilson, y con umbral = 100 hace falta
 * cero fallos de N (éxito exactamente 1). `null` cuando falta con qué juzgarlo.
 */
function meetsThresholdOf(s: PlanSentenceSeries): boolean | null {
  const u = idx(s.success_threshold_pct);
  if (u == null) return null;
  if (u >= 100) {
    const p = idx(s.success_of_plan);
    return p == null ? null : p >= 1;
  }
  const low = idx(s.success_wilson_low);
  if (low == null) return null;
  return low >= u / 100;
}

/** Copy de cada razón por la que el bloque «plan» no viaja. Las tres son situaciones DISTINTAS y
 *  se dicen distintas: un guion mudo las haría indistinguibles.
 *
 *  Eran cuatro hasta A12: la de `no_liquid_assets` se retiró porque **`plan_absent_reason` no
 *  emite ese literal** (sus tres constantes viven en `apps/api/src/handlers/projection.rs`) — el
 *  que sí lo emite es `needed_capital_absent_reason`, que dice por qué falta una CIFRA y no por
 *  qué falta el plan. La frase estaba escrita y no se podía leer nunca. */
const ABSENT_ES: Record<PlanAbsentReason, { text: string; tone: PlanSentenceTone }> = {
  birth_date_missing: {
    text:
      "Falta tu fecha de nacimiento para situar la pensión y el horizonte: sin ella no hay fecha válida.",
    tone: "danger",
  },
  months_override: {
    text: "Esta simulación usa un horizonte forzado, así que no resuelve tu fecha válida.",
    tone: "warn",
  },
  // **El literal del hogar es `household_aggregate`** (A12). La tabla decía
  // `household_not_solved`, que es el valor fijo de OTRO campo
  // (`HouseholdMemberProjectionApi.plan_state`): en `view=household` esta frase no se leía nunca y
  // la vista caía al genérico «Tu plan no tiene fecha válida», que además dice otra cosa — el
  // hogar no es que no llegue, es que no tiene UN plan.
  household_aggregate: {
    text: "El hogar no resuelve una fecha: mira la de cada persona en su vista «Yo».",
    tone: "warn",
  },
};

/**
 * La oración del plan, una por estrategia (modelo v2, §4 del documento del modelo).
 *
 * | Estrategia | Frase |
 * |---|---|
 * | `asap` | «Con tu plan te jubilas en 2043 (a los 55): aguantan 95 de cada 100 escenarios. Al 100 % sería 2051; al 90 %, 2040.» |
 * | `retire_at_age` | «A los 55, como pediste: aguantan 82 de cada 100 escenarios (tu umbral es 95). Para llegar harían falta 300 € más al mes.» |
 * | `coast` A | «Puedes dejar de aportar en 2031 (a los 41) y jubilarte a los 55 con 95 de cada 100.» |
 * | `coast` B | «Dejando de aportar a los 41, te jubilas en 2047 (a los 57) con 95 de cada 100.» |
 * | `partial` | «Puedes pasar a jornada reducida en 2031 (a los 41) y jubilarte del todo en 2045 (a los 55) con 95 de cada 100.» |
 *
 * Y dos estados que ganan a la estrategia, en este orden: **el bloque «plan» ausente**
 * (`plan_absent_reason`, con `birth_date_missing` a la cabeza) y **`not_reachable`** (ningún mes
 * del horizonte cumple el umbral). Eran tres hasta A12; el tercero, `pending`, describía un
 * literal que el servidor nunca emitió.
 *
 * Una `strategy` nula (el agregado del hogar, o un backend viejo) usa la lectura de `asap`.
 */
export function planSentence(input: PlanSentenceInput): PlanSentence {
  const s = input.series;
  if (!s) {
    return { text: "Sin plan que mostrar", tone: "warn", parts: { ...EMPTY_PARTS } };
  }
  const label = input.monthLabel;
  const ages = input.ageMode === "ages";
  const strategy = s.strategy ?? null;
  const currency = input.currencyIso;
  const ageAt = (mi: number | null): number | null =>
    mi == null || input.ageAt == null ? null : idx(input.ageAt(mi));

  const basis = s.retirement_date_basis ?? null;
  const absentReason = s.plan_absent_reason ?? null;
  const warned = new Set<string>(s.warnings ?? []);

  const safeMi = idx(s.safe_date_month_index);
  const jubMi = idx(s.jubilacion_month_index);
  // El mes EFECTIVO y su edad SIEMPRE del mismo par: emparejar `jubilacion_month_index` con
  // `safe_date_age` diría la edad de otra fecha en las estrategias por edad, donde no coinciden.
  const retMi = jubMi ?? safeMi;
  const retAge = jubMi != null ? idx(s.jubilacion_age) : idx(s.safe_date_age);

  const coastMi = idx(s.coast_stop_month_index);
  const partialMi = idx(s.partial_start_month_index) ?? idx(s.partial_retirement_month_index);
  const pensionMi = idx(s.pension_start_month_index);
  const underfunded = s.contribution_underfunded ?? null;
  const successOfPlan = idx(s.success_of_plan);
  // «N de cada 100» lo cuenta `risk-bands.ts` y NO se reimplementa aquí: los dos topes
  // anti-mentira (nunca 100 sin un 1 exacto, nunca 0 con éxito positivo) tienen que ser los
  // mismos que los del tile de «Riesgo», o la frase y el KPI dirían cifras distintas del mismo
  // sorteo.
  const outOfHundred = scenariosPerHundred(successOfPlan);
  const threshold = idx(s.success_threshold_pct);
  const meets = meetsThresholdOf(s);
  const requestedAge = retAge ?? input.targetRetirementAge ?? null;

  const parts: PlanSentenceParts = {
    strategy,
    basis,
    absentReason,
    retirementMonthIndex: retMi,
    retirementLabel: retMi == null ? null : label(retMi),
    retirementAge: retAge,
    monthsToRetirement: retMi == null ? null : Math.max(0, retMi),
    successOfPlan,
    successOutOfHundred: outOfHundred,
    successThresholdPct: threshold,
    meetsThreshold: meets,
    secondaryKind:
      strategy === "coast"
        ? "coast"
        : strategy === "partial"
          ? "partial"
          : pensionMi != null
            ? "pension"
            : null,
    secondaryMonthIndex: null,
    secondaryLabel: null,
    // S8: el puente es el TRAMO entre jubilación y pensión, no el plazo desde hoy.
    bridgeMonths: retMi != null && pensionMi != null ? pensionMi - retMi : null,
    contributionRequiredMonthly: s.contribution_required_monthly ?? null,
    underfunded,
  };
  if (parts.secondaryKind === "coast") parts.secondaryMonthIndex = coastMi;
  if (parts.secondaryKind === "partial") parts.secondaryMonthIndex = partialMi;
  if (parts.secondaryKind === "pension") parts.secondaryMonthIndex = pensionMi;
  if (parts.secondaryMonthIndex != null) {
    parts.secondaryLabel = label(parts.secondaryMonthIndex);
  }

  const done = (text: string, tone: PlanSentenceTone): PlanSentence => ({ text, tone, parts });

  /** «2043 (a los 55)» — sin edad publicada, solo la fecha; en modo edades el rótulo ya la lleva. */
  const when = (mi: number, age: number | null): string =>
    age != null && !ages ? `${label(mi)} (a los ${age})` : label(mi);

  /** « con 95 de cada 100» — vacío cuando no hay sorteo que citar. */
  const withScenarios =
    outOfHundred == null ? "" : ` con ${outOfHundred} de cada 100`;

  /** El tono base de una frase que SÍ tiene fecha: rojo si no se llega ni ahorrándolo todo,
   *  ámbar si el plan no cumple su umbral, verde si lo cumple (o no hay con qué juzgarlo). */
  const dateTone: PlanSentenceTone =
    underfunded === true ? "danger" : meets === false ? "warn" : "ok";

  // ── Estados que ganan a la estrategia ──────────────────────────────────────────────────────
  if (absentReason != null) {
    const copy = ABSENT_ES[absentReason];
    if (copy) return done(copy.text, copy.tone);
    return done("Tu plan no tiene fecha válida.", "warn");
  }
  // La rama `basis === "pending"` («Calculando tu fecha…») se retiró en A12: **el servidor nunca
  // emitió ese literal**. El nivel 1 del solve se resuelve en línea, dentro del permiso de la
  // serie, así que cuando esta respuesta llega la base ya está decidida. El único cálculo que sí
  // llega tarde es el nivel 2 (`needed_capital_curve_state`), y no cambia ni una palabra de esta
  // frase — solo la curva del chart.
  if (basis === "not_reachable") {
    const u = threshold ?? 95;
    const horizon = idx(s.horizon_lifespan_age);
    const head = horizon != null
      ? `Con tu plan no hay ninguna fecha en la que aguanten ${u} de cada 100 escenarios hasta los ${horizon} años.`
      : `Con tu plan no hay ninguna fecha en la que aguanten ${u} de cada 100 escenarios hasta el final de tu horizonte.`;
    const best = bestRetirementYear(s.success_by_retirement_year);
    if (best == null) return done(head, "danger");
    const bestN = scenariosPerHundred(best.success);
    if (bestN == null) return done(head, "danger");
    return done(`${head} Lo más cerca: ${label(best.month_index)} con ${bestN} de cada 100.`, "danger");
  }

  switch (strategy) {
    // ── «Jubilarme a una edad»: la edad manda y el sorteo la juzga ────────────────────────────
    case "retire_at_age": {
      if (requestedAge == null) {
        return done("Falta tu edad de jubilación objetivo.", "warn");
      }
      const head =
        outOfHundred == null
          ? `A los ${requestedAge}, como pediste.`
          : threshold == null
            ? `A los ${requestedAge}, como pediste: aguantan ${outOfHundred} de cada 100 escenarios.`
            : `A los ${requestedAge}, como pediste: aguantan ${outOfHundred} de cada 100 escenarios (tu umbral es ${threshold}).`;

      if (underfunded === true) {
        return done(
          `${head} Ni ahorrando todo tu sobrante llegas a los ${requestedAge}.`,
          "danger",
        );
      }
      if (meets === false) {
        const extra = s.contribution_required_monthly;
        if (extra != null) {
          return done(
            `${head} Para llegar harían falta ${formatCurrencyAmount(extra, currency)} más al mes.`,
            "warn",
          );
        }
        return done(head, "warn");
      }
      // Ya llega: la fecha válida al lado dice cuánto margen hay («podrías incluso a los 47»).
      if (safeMi != null && jubMi != null && safeMi < jubMi) {
        const safeAge = idx(s.safe_date_age);
        return done(
          safeAge != null
            ? `${head} Ya llegas: podrías incluso a los ${safeAge}.`
            : `${head} Ya llegas: podrías incluso en ${label(safeMi)}.`,
          "ok",
        );
      }
      return done(`${head} Ya llegas.`, "ok");
    }

    // ── «Coast FIRE»: el hito es el mes en que dejas de aportar ───────────────────────────────
    case "coast": {
      if (warned.has("coast_not_reachable")) {
        return done(
          requestedAge != null
            ? `Ni aportando hasta el final llegas a los ${requestedAge} con tu umbral.`
            : "Ni aportando hasta el final llegas a tu edad objetivo con tu umbral.",
          "danger",
        );
      }
      if (coastMi == null) {
        return done("Todavía no hay ningún mes en el que puedas dejar de aportar.", "warn");
      }
      const mode =
        input.coastMode ?? (basis === "target_age" ? "fixed_retirement_age" : "fixed_stop_age");
      const coastAge = ageAt(coastMi);

      if (mode === "fixed_stop_age") {
        // Modo B: la edad de parada la elegiste tú; la fecha de jubilación sale del sorteo.
        const stop =
          coastAge != null && !ages ? `a los ${coastAge}` : `en ${label(coastMi)}`;
        if (retMi == null) {
          return done(
            `Dejando de aportar ${stop}, tu plan no alcanza ninguna fecha válida.`,
            "danger",
          );
        }
        return done(
          `Dejando de aportar ${stop}, te jubilas en ${when(retMi, retAge)}${withScenarios}.`,
          dateTone,
        );
      }
      // Modo A: tú fijas la edad de jubilación y el solve resuelve cuándo puedes parar.
      const tail =
        requestedAge != null
          ? `jubilarte a los ${requestedAge}`
          : retMi != null
            ? `jubilarte en ${label(retMi)}`
            : "jubilarte igual";
      if (coastMi <= 0) {
        return done(`Ya puedes dejar de aportar y ${tail}${withScenarios}.`, dateTone);
      }
      return done(
        `Puedes dejar de aportar en ${when(coastMi, coastAge)} y ${tail}${withScenarios}.`,
        dateTone,
      );
    }

    // ── «Jornada reducida» (Barista FIRE): dos hitos en una frase ─────────────────────────────
    case "partial": {
      if (warned.has("partial_never_starts")) {
        return done(
          "Tu plan no puede permitirse la jornada reducida en ningún mes del horizonte.",
          "danger",
        );
      }
      if (warned.has("partial_never_fully_retires")) {
        return done(
          partialMi != null
            ? `Pasas a jornada reducida en ${when(partialMi, ageAt(partialMi))}, pero no llegas a jubilarte del todo dentro del horizonte.`
            : "Empiezas la jornada reducida, pero no llegas a jubilarte del todo dentro del horizonte.",
          "danger",
        );
      }
      if (partialMi == null) {
        if (retMi == null) return done("Tu plan no alcanza ninguna fecha válida.", "danger");
        return done(
          `Sin fase de jornada reducida: te jubilas en ${when(retMi, retAge)}${withScenarios}.`,
          dateTone,
        );
      }
      if (retMi == null) {
        return done(
          `Pasas a jornada reducida en ${when(partialMi, ageAt(partialMi))}, pero tu plan no alcanza ninguna fecha válida.`,
          "danger",
        );
      }
      return done(
        `Puedes pasar a jornada reducida en ${when(partialMi, ageAt(partialMi))} y jubilarte del todo en ${when(retMi, retAge)}${withScenarios}.`,
        dateTone,
      );
    }

    // ── `asap` y el plan sin estrategia declarada: manda la fecha válida ──────────────────────
    default: {
      if (retMi == null) return done("Tu plan no alcanza ninguna fecha válida.", "danger");
      const head =
        retMi <= 0
          ? outOfHundred == null
            ? "Con tu plan ya puedes jubilarte."
            : `Con tu plan ya puedes jubilarte: aguantan ${outOfHundred} de cada 100 escenarios.`
          : outOfHundred == null
            ? `Con tu plan te jubilas en ${when(retMi, retAge)}.`
            : `Con tu plan te jubilas en ${when(retMi, retAge)}: aguantan ${outOfHundred} de cada 100 escenarios.`;
      const alt = alternativeDates(s, label);
      return done(alt == null ? head : `${head} ${alt}`, dateTone);
    }
  }
}

/** El punto de mayor éxito de la tira «éxito por año de jubilación» — lo más cerca que estuvo un
 *  plan que no llega. `null` mientras el nivel 2 no lo publica (nunca un 0 inventado). */
function bestRetirementYear(
  points: ProjectionSeriesApi["success_by_retirement_year"],
): { month_index: number; success: number } | null {
  if (points == null || points.length === 0) return null;
  let best: { month_index: number; success: number } | null = null;
  for (const p of points) {
    const mi = idx(p?.month_index);
    const su = idx(p?.success);
    if (mi == null || su == null) continue;
    if (best == null || su > best.success) best = { month_index: mi, success: su };
  }
  return best;
}

/**
 * «Al 100 % sería 2051; al 90 %, 2040.» — las dos fechas que acotan la del umbral configurado.
 *
 * Un índice `null` es **«nunca»** y así se dice (el contrato lo define como «no se alcanza ni al
 * final del horizonte», no como «todavía no calculado»). Sin ninguna de las dos, no hay segunda
 * oración: media frase sobre una alternativa que no existe no aporta nada.
 */
function alternativeDates(
  s: PlanSentenceSeries,
  label: (monthIndex: number) => string,
): string | null {
  const has100 = "safe_date_at_100_month_index" in s;
  const has90 = "safe_date_at_90_month_index" in s;
  if (!has100 && !has90) return null;
  const mi100 = idx(s.safe_date_at_100_month_index);
  const mi90 = idx(s.safe_date_at_90_month_index);
  const first = has100
    ? mi100 != null
      ? `Al 100 % sería ${label(mi100)}`
      : "Al 100 %, nunca"
    : null;
  const second = has90
    ? mi90 != null
      ? `al 90 %, ${label(mi90)}`
      : "al 90 %, nunca"
    : null;
  if (first == null && second == null) return null;
  if (first == null) {
    // Sin la del 100 %, la del 90 % abre la oración y hay que capitalizarla.
    return `Al 90 %, ${mi90 != null ? label(mi90) : "nunca"}.`;
  }
  return second == null ? `${first}.` : `${first}; ${second}.`;
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
// La frase por MIEMBRO del hogar (U10 + bug B7)
// ═════════════════════════════════════════════════════════════════════════════════════════════

/** Los campos de un miembro del hogar que la frase en tercera persona necesita. B7: la frase
 *  anterior leía tres campos y se callaba el estado que el servidor ya publicaba.
 *
 *  **NO lleva `coast_fire_month_index` ni `underfunded`**: el hogar no resuelve el plan de nadie
 *  (D9) y `HouseholdMemberProjection` (`apps/api/src/handlers/projection.rs`) nunca los publicó —
 *  esta frase los leyó igual hasta que la revisión de W10 encontró el hueco. Si cualquiera de los
 *  dos vuelve a este objeto en el JSON de un cliente viejo, `memberPlanSentence` no lo mira. */
export type MemberPlanSentenceMember = Pick<
  HouseholdMemberProjectionApi,
  | "username"
  | "strategy"
  | "jubilacion_month_index"
  | "jubilacion_age"
  | "partial_retirement_month_index"
  | "warnings"
  | "plan_state"
>;

export type MemberPlanSentence = {
  text: string;
  tone: PlanSentenceTone;
};

/**
 * Sufijos de estado, con PRECEDENCIA: primero lo que invalida el plan de esa persona, después lo
 * que le falta. El orden ES la regla y por eso está en una lista y no en tres `if` sueltos.
 */
const MEMBER_WARNING_SUFFIX: Array<{
  warning: string;
  text: string;
  tone: PlanSentenceTone;
}> = [
  { warning: "birth_date_missing", text: "falta su fecha de nacimiento", tone: "danger" },
  { warning: "coast_not_reachable", text: "no llega ni aportando siempre", tone: "danger" },
  {
    warning: "partial_never_starts",
    text: "no puede permitirse la jornada reducida",
    tone: "danger",
  },
  {
    warning: "partial_never_fully_retires",
    text: "no llega a jubilarse del todo",
    tone: "danger",
  },
  {
    warning: "target_retirement_age_missing",
    text: "falta su edad de jubilación",
    tone: "warn",
  },
];

/**
 * La frase de UN miembro del hogar, en **tercera persona** (U10) y **con estado** (bug B7).
 *
 * ## Por qué lleva estado, y por qué no lleva fecha propia
 *
 * El agregado del hogar **no resuelve el plan de nadie** (D9, modelo v2): cada fila llega con
 * `plan_state: "household_not_solved"` y sin éxito, sin fecha válida y sin capital necesario. Lo
 * que sí llega es lo determinista: si su estrategia impone una edad, el motor la simuló con ese
 * mes forzado y `jubilacion_month_index` existe; si su fecha la fijaría el sorteo, **no hay
 * ninguna** — y decir «no cruza el objetivo en el horizonte» sería mentir sobre un plan que
 * nadie ha resuelto. Por eso la frase dice que la fecha válida se resuelve en su propia vista
 * Jubilación, en vez de rotular un mes que nadie calculó para ella.
 *
 * ## Qué dice, en orden
 *
 * 1. **Su estrategia**, con el mismo rótulo de producto que ve en su propio formulario
 *    (`RETIREMENT_STRATEGY_LABEL`, `retirementProfile.ts`) — sin él, dos miembros con estrategias
 *    distintas leían la misma frase genérica y no había forma de saber cuál llega por edad y
 *    cuál por umbral.
 * 2. **Su fecha por edad**, si el motor la fijó (`jubilacion_month_index` con `jubilacion_age`) —
 *    nunca una edad inventada (B5): sin `jubilacion_age` se rotula el MES, y sin
 *    `jubilacion_month_index` se manda a su vista Jubilación en vez de un guion o un
 *    «calculando» que nadie va a resolver aquí.
 * 3. **Su jornada reducida**, si la tiene (`partial_retirement_month_index`) — es un hecho
 *    determinista suyo, no una cifra del sorteo, así que viaja aunque no haya fecha efectiva.
 *
 * Los avisos que la fila publica (`birth_date_missing`, `coast_not_reachable`…) se cuelgan como
 * sufijo con su tono: antes un miembro al que le faltaba un dato se leía exactamente igual que
 * uno que llega, y la tarjeta propia sí lo pintaba de rojo.
 */
export function memberPlanSentence(
  member: MemberPlanSentenceMember,
  monthLabel: (monthIndex: number) => string,
): MemberPlanSentence {
  const name = String(member.username ?? "").trim() || "Esta persona";
  const strategyLabel =
    member.strategy != null ? (RETIREMENT_STRATEGY_LABEL[member.strategy] ?? null) : null;
  const mi = idx(member.jubilacion_month_index);
  const age = idx(member.jubilacion_age);
  const partialMi = idx(member.partial_retirement_month_index);
  const warned = new Set<string>(member.warnings ?? []);

  let tone: PlanSentenceTone = "ok";
  let suffix: string | null = null;
  for (const entry of MEMBER_WARNING_SUFFIX) {
    if (warned.has(entry.warning)) {
      tone = entry.tone;
      suffix = entry.text;
      break;
    }
  }

  const extras: string[] = [];
  if (partialMi != null) {
    extras.push(`hace jornada reducida desde ${monthLabel(partialMi)}`);
  }
  const extraTail = extras.length === 0 ? "" : ` (${extras.join(" y ")})`;

  const head = strategyLabel != null ? `${name}: ${strategyLabel}` : name;

  let dateBit: string;
  if (mi == null) {
    // Sin mes efectivo la fecha la fijaría el sorteo, y el hogar no lo corre (D9): su fecha
    // válida se resuelve en SU vista, nunca aquí.
    dateBit = "fecha válida: en su vista Jubilación";
  } else if (mi <= 0) {
    dateBit = "ya puede jubilarse";
  } else if (age != null) {
    dateBit = `a los ${age}, ${monthLabel(mi)}`;
  } else {
    // Sin `jubilacion_age` se rotula el MES, nunca una edad inventada (B5).
    dateBit = monthLabel(mi);
  }

  const body = `${head} — ${dateBit}${extraTail}`;
  return { text: `${body}${suffix == null ? "" : ` — ${suffix}`}.`, tone };
}
