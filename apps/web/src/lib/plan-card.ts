/**
 * Modelo PURO de la tarjeta «Tu plan» del Resumen (5.0.0, D27 / §G del plan de #207) y de las
 * tarjetas por miembro de la vista Hogar (D32).
 *
 * La tarjeta contesta tres cosas y nada más: con qué ESTRATEGIA simulas, cuál es tu PRÓXIMO HITO
 * (fecha y edad de la jubilación efectiva) y en qué ESTADO estás. Aquí vive la traducción de esos
 * tres campos del servidor a copy; la vista solo pinta.
 *
 * Por qué es un módulo aparte y no lógica dentro de `SummaryView`: el estado sale de un array de
 * literales cerrados (`warnings[]`) con precedencia entre ellos, y esa precedencia es exactamente
 * el tipo de regla que se rompe en silencio al añadir el cuarto aviso. Con la regla aquí, un test
 * la fija (`plan-card.test.ts`).
 */

import type {
  ProjectionSeriesApi,
  RetirementStrategyApi,
  SummaryPlanApi,
} from "../api/types";
import {
  addMonthsCivil,
  ageCompletedYearsCivil,
  formatDateDmy,
  parseYmdComponents,
} from "./dates";
import {
  planSentence,
  type PlanSentenceAgeMode,
  type PlanSentenceParts,
  type PlanSentenceSeries,
} from "./plan-sentence";
import { RETIREMENT_STRATEGY_LABEL } from "./retirementProfile";
import { summarySuccessTile } from "./risk-bands";

/**
 * Avisos que el plan sabe leer. Literales cerrados del contrato (`warnings[]` de
 * `GET /v1/projection/series` y de `members[]`).
 *
 * `retire_at_age_underfunded` lo emite el servidor desde WP5-2b (`solve.rs`, §B.7:
 * `underfunded = c > sobrante`) y viaja además como el booleano `underfunded` del plan y de la
 * serie. Las dos vías dicen lo mismo y `planStatusFromPlan` mira las dos: el objeto `plan` del
 * Resumen NO trae `warnings[]`, así que sin el booleano la tarjeta se quedaría verde con un plan
 * que no llega.
 */
export type PlanWarning =
  | "birth_date_missing"
  | "target_retirement_age_missing"
  | "retire_at_age_underfunded";

export type PlanStatusTone = "ok" | "warn" | "danger";

export type PlanStatus = {
  /** El aviso que ganó la precedencia, o `null` cuando no hay ninguno («En plan»). */
  warning: PlanWarning | null;
  tone: PlanStatusTone;
  label: string;
  /** Adónde lleva el enlace de arreglo, cuando hay algo que arreglar. */
  action: { label: string; target: "account" | "retirement" } | null;
};

/**
 * Precedencia deliberada: **primero lo que invalida el plan, después lo que le falta**.
 *
 * - `retire_at_age_underfunded` gana siempre: el plan está completo y NO llega (D17 — la edad
 *   manda y el aviso es rojo grande). Es un resultado, no un hueco de configuración.
 * - `birth_date_missing` y `target_retirement_age_missing` son datos que faltan; con ellos la
 *   estrategia por edad **degradó a «Cuanto antes»**, así que el hito que se ve al lado es el de
 *   otra simulación — decirlo es el objetivo de la línea.
 * - Sin avisos: «En plan».
 *
 * Los literales que no conoce se ignoran (un aviso nuevo del servidor nunca deja la tarjeta sin
 * estado; a lo sumo dice «En plan» hasta que alguien lo traduzca aquí).
 */
export function planStatusFromWarnings(
  warnings: readonly string[] | null | undefined,
): PlanStatus {
  const set = new Set(warnings ?? []);
  if (set.has("retire_at_age_underfunded")) {
    return {
      warning: "retire_at_age_underfunded",
      tone: "danger",
      label: "Con tu ahorro actual no llegas a tu edad objetivo",
      action: { label: "Revisar tu plan", target: "retirement" },
    };
  }
  if (set.has("birth_date_missing")) {
    return {
      warning: "birth_date_missing",
      tone: "warn",
      label: "Falta tu fecha de nacimiento",
      action: { label: "Tu cuenta", target: "account" },
    };
  }
  if (set.has("target_retirement_age_missing")) {
    return {
      warning: "target_retirement_age_missing",
      tone: "warn",
      label: "Falta tu edad de jubilación objetivo",
      action: { label: "Elegir edad", target: "retirement" },
    };
  }
  return { warning: null, tone: "ok", label: "En plan", action: null };
}

/** Copy de cada razón por la que el hito no existe POR CONSTRUCCIÓN (`jubilacion_absent_reason`). */
const ABSENT_REASON_ES: Record<string, string> = {
  household_aggregate: "El hogar suma varios planes: mira la tarjeta de cada persona",
  no_retirement_trigger: "Sin objetivo ni edad objetivo: este plan no se jubila",
};

export type PlanMilestone = {
  /** Cifra grande de la tarjeta: la fecha de la jubilación efectiva, o el motivo de su ausencia. */
  value: string;
  /** Línea de apoyo: la edad a la que ocurre. `null` cuando no hay edad resoluble. */
  detail: string | null;
  /** `false` ⇒ `value` es una explicación, no una fecha (la vista la pinta apagada). */
  reached: boolean;
};

/**
 * El próximo hito del plan: la **jubilación EFECTIVA** (`jubilacion_*`, que desde 5.0.0 es el mes
 * en que el motor te jubila de verdad — cruce o edad, R8), nunca el cruce del objetivo.
 *
 * `null` en el índice significa cosas distintas según venga o no acompañado de razón, y la
 * diferencia es justo lo que hay que decir en pantalla: **con** razón, la pregunta no aplica en
 * esta respuesta; **sin** razón, hay plan y no se jubila dentro del horizonte.
 *
 * @deprecated — retirar en U1b/U2. U9 sustituye el trío «hito grande + edad + estado» por una
 * ORACIÓN (`lib/plan-sentence.ts`), que es lo que `planCardV2` construye. Sigue vivo mientras
 * `SummaryView.tsx` pinte la tarjeta antigua.
 */
export function planMilestone(input: {
  jubilacionMonthIndex?: number | null;
  jubilacionDateYmd?: string | null;
  jubilacionAge?: number | null;
  jubilacionAbsentReason?: string | null;
}): PlanMilestone {
  if (input.jubilacionMonthIndex == null) {
    const reason = input.jubilacionAbsentReason;
    return {
      value: reason
        ? (ABSENT_REASON_ES[reason] ?? "Sin hito que mostrar")
        : "Sin cruce en el horizonte",
      detail: null,
      reached: false,
    };
  }
  const ymd = input.jubilacionDateYmd?.trim();
  return {
    value: ymd ? formatDateDmy(ymd) : `Mes ${input.jubilacionMonthIndex}`,
    detail: input.jubilacionAge != null ? `a los ${input.jubilacionAge} años` : null,
    reached: true,
  };
}

/**
 * Las dos cifras al mes que la tarjeta añade con una estrategia por edad (5.0.0 WP5-2b):
 * decimal-strings **copiados** de `summary.plan`, sin una sola operación aritmética por el camino.
 * `null` = esta estrategia no responde a esa pregunta; **nunca cero**.
 */
export type PlanFigures = {
  /** €/mes — `required_savings_monthly`, que ES `required_contribution_monthly` de la serie. */
  requiredSavingsMonthly: string | null;
  /** €/mes — el margen, con la base que declare la estrategia. */
  disposableMonthly: string | null;
};

/** Una tarjeta de plan ya resuelta — la del usuario en «Yo», o una por miembro en «Hogar». */
export type PlanCardModel = {
  key: string;
  /** Nombre de la persona; `null` en la tarjeta propia (el título ya es «Tu plan»). */
  name: string | null;
  strategy: RetirementStrategyApi | null;
  milestone: PlanMilestone;
  status: PlanStatus;
  /** Ausente en las tarjetas que no publican cifras (miembros del hogar por cruce, plan ausente). */
  figures?: PlanFigures;
};

/**
 * Estado de la tarjeta cuando la fuente es el objeto `plan` del Resumen, que **no trae
 * `warnings[]`**: el rojo llega como el booleano `underfunded`.
 *
 * `true` = el plan está completo y no llega (D17); `false` = llega; **`null` = la pregunta no
 * aplica a esta estrategia**, y colapsarlo con `false` pintaría de verde un plan que nadie ha
 * evaluado. Se miran las DOS vías porque la tarjeta de miembro sí tiene avisos y la propia no.
 */
export function planStatusFromPlan(input: {
  underfunded?: boolean | null;
  warnings?: readonly string[] | null;
}): PlanStatus {
  if (input.underfunded === true) {
    return planStatusFromWarnings(["retire_at_age_underfunded"]);
  }
  return planStatusFromWarnings(input.warnings);
}

/**
 * Mes de la rejilla → fecha civil y edad, con el ancla de la proyección (`anchor_date_ymd`, el mes
 * 0) y la fecha de nacimiento del usuario.
 *
 * El objeto `plan` del Resumen publica el ÍNDICE y nada más —es un escalar del plan, no un punto
 * de la serie—, así que la fecha se resuelve aquí con el mismo ancla que usa el chart. Sin ancla
 * (o sin fecha de nacimiento) devuelve `null` en la mitad que no se puede saber en vez de
 * inventarse un día: una fecha aproximada en una tarjeta de estado se copia como si fuera exacta.
 */
export function resolvePlanMilestoneCivil(input: {
  monthIndex: number | null | undefined;
  anchorDateYmd?: string | null;
  birthDateIso?: string | null;
}): { ymd: string | null; age: number | null } {
  const mi = input.monthIndex;
  if (mi == null || !Number.isFinite(mi)) return { ymd: null, age: null };
  const anchor = input.anchorDateYmd ? parseYmdComponents(input.anchorDateYmd) : null;
  if (!anchor) return { ymd: null, age: null };
  const civil = addMonthsCivil(anchor.y, anchor.m, anchor.d, mi);
  const ymd = `${String(civil.y).padStart(4, "0")}-${String(civil.m).padStart(2, "0")}-${String(civil.d).padStart(2, "0")}`;
  const birth = input.birthDateIso ? parseYmdComponents(input.birthDateIso) : null;
  return {
    ymd,
    age: birth ? ageCompletedYearsCivil(civil, birth) : null,
  };
}

/** Los campos de la respuesta de la serie que sirven de respaldo cuando el Resumen declara el
 *  plan ausente (o cuando habla con un backend anterior a WP5-2b, que no publica `plan`). */
export type PlanSeriesFallback = Pick<
  ProjectionSeriesApi,
  | "strategy"
  | "jubilacion_month_index"
  | "jubilacion_date_ymd"
  | "jubilacion_age"
  | "jubilacion_absent_reason"
  | "warnings"
>;

/**
 * La tarjeta propia («Yo»), leyendo **`summary.plan`** — la fuente canónica desde WP5-2b — y
 * cayendo a la respuesta de la serie cuando el Resumen declara el plan ausente
 * (`absent_reason`) o cuando el backend no lo publica.
 *
 * Por qué el respaldo y no un hueco: `absent_reason: projection_unavailable` significa que el
 * Resumen no pudo calcular la proyección en ESA petición; la serie que el chart ya tiene cargada
 * sigue siendo la del mismo usuario y el mismo plan. Preferir el hueco dejaría la tarjeta en
 * blanco con datos en pantalla justo al lado. Lo que NO se mezcla nunca son las dos mitades: o
 * todo el hito sale del plan, o todo sale de la serie.
 *
 * @deprecated — retirar en U1b/U2. Su sustituta es `planCardV2`, que devuelve la tarjeta ANCHA de
 * U9 (frase · estrategia · «Éxito del plan» · aviso) en vez del trío hito/edad/estado.
 */
export function ownPlanCard(input: {
  plan?: SummaryPlanApi | null;
  series?: PlanSeriesFallback | null;
  anchorDateYmd?: string | null;
  birthDateIso?: string | null;
}): PlanCardModel | null {
  const plan = input.plan;
  const usePlan = plan != null && plan.absent_reason == null;
  if (usePlan) {
    const civil = resolvePlanMilestoneCivil({
      monthIndex: plan.jubilacion_month_index,
      anchorDateYmd: input.anchorDateYmd,
      birthDateIso: input.birthDateIso,
    });
    return {
      key: "mine",
      name: null,
      strategy: plan.strategy,
      milestone: planMilestone({
        jubilacionMonthIndex: plan.jubilacion_month_index,
        jubilacionDateYmd: civil.ymd,
        jubilacionAge: civil.age,
        // El plan sin `absent_reason` SIEMPRE tiene trigger: un índice nulo aquí es «no se jubila
        // dentro del horizonte», que es un resultado y lo dice `planMilestone` por defecto.
        jubilacionAbsentReason: null,
      }),
      status: planStatusFromPlan({ underfunded: plan.underfunded }),
      figures: {
        requiredSavingsMonthly: plan.required_savings_monthly,
        disposableMonthly: plan.disposable_monthly,
      },
    };
  }
  const series = input.series;
  if (!series) return null;
  return {
    key: "mine",
    name: null,
    strategy: series.strategy ?? null,
    milestone: planMilestone({
      jubilacionMonthIndex: series.jubilacion_month_index,
      jubilacionDateYmd: series.jubilacion_date_ymd,
      jubilacionAge: series.jubilacion_age,
      jubilacionAbsentReason: series.jubilacion_absent_reason,
    }),
    status: planStatusFromWarnings(series.warnings),
  };
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
// V2 — la tarjeta ANCHA del Resumen (U9)
// ═════════════════════════════════════════════════════════════════════════════════════════════

/** El aviso, ya listo para su fila: texto + a dónde se va a arreglarlo. */
export type PlanCardWarningV2 = {
  text: string;
  actionLabel: string;
  target: "account" | "retirement";
};

/** El KPI «Éxito del plan» dentro de la tarjeta. Es el MISMO sorteo que dibuja la sección
 *  «Riesgo» de Jubilación: aquí solo se rotula, jamás se recalcula. */
export type PlanCardSuccessV2 = {
  label: string;
  value: string;
  tone: "default" | "warn" | "danger";
  /** El umbral — el denominador del semáforo, sin el que «87 de cada 100» no se puede auditar. */
  parenthetical?: string;
  /** Los escenarios que no llegan a jubilarse, o la razón de que no haya cifra. */
  detail?: string;
};

export type PlanCardV2 = {
  /** La ORACIÓN del plan (`planSentence`), que es el título de la tarjeta. */
  title: string;
  /** Estrategia + el hito secundario que esa estrategia añade. */
  subtitle: string;
  tone: PlanStatusTone;
  success: PlanCardSuccessV2 | null;
  warning: PlanCardWarningV2 | null;
};

/** La serie del usuario, con lo que la frase y el estado necesitan. */
export type PlanCardV2Series = PlanSentenceSeries & Pick<ProjectionSeriesApi, "warnings">;

export type PlanCardV2Input = {
  /** `summary.plan` — la fuente canónica del ESTADO y del éxito. */
  plan?: SummaryPlanApi | null;
  /** La serie que el chart de la misma pantalla ya tiene cargada. */
  series?: PlanCardV2Series | null;
  monthLabel: (monthIndex: number) => string;
  ageMode?: PlanSentenceAgeMode;
  /** Edad objetivo GUARDADA del perfil. */
  targetRetirementAge: number | null;
};

/** `SummaryPlanApi` visto como serie para la frase. Los hitos secundarios van a `null` porque el
 *  objeto `plan` **no los publica**: es un resumen de cinco escalares, no la simulación. */
function planAsSentenceSeries(plan: SummaryPlanApi): PlanSentenceSeries {
  return {
    strategy: plan.strategy,
    jubilacion_month_index: plan.jubilacion_month_index,
    jubilacion_age: null,
    coast_fire_month_index: null,
    partial_retirement_month_index: null,
    pension_start_month_index: null,
    underfunded: plan.underfunded,
  };
}

/** Frase corta del hito secundario de cada estrategia; `null` cuando la estrategia no tiene uno. */
function secondaryPhrase(parts: PlanSentenceParts): string | null {
  if (parts.secondaryLabel == null) return null;
  switch (parts.secondaryKind) {
    case "coast":
      return `dejas de aportar en ${parts.secondaryLabel}`;
    case "partial":
      return `desde ${parts.secondaryLabel}`;
    case "pension":
      return `pensión desde ${parts.secondaryLabel}`;
    default:
      return null;
  }
}

/**
 * La tarjeta «Tu plan» del Resumen, rediseñada (U9): **una sola tarjeta ancha** con la frase de
 * arriba, la estrategia debajo, el KPI «Éxito del plan» y una fila de aviso.
 *
 * Sustituye a la rejilla de tres tarjetas cortas cuyo hito («12/05/2043») no decía qué estrategia
 * lo produjo ni si el plan llegaba — tres datos sin sujeto.
 *
 * ## De dónde sale cada mitad, y por qué no se mezclan
 *
 * - **La FRASE sale de la serie** cuando la hay. La serie es estrictamente más rica —tiene el mes
 *   coast, el de media jornada y el de la pensión, que `summary.plan` no publica— y es el mismo
 *   objeto que el chart está dibujando dos centímetros más abajo. Sin serie, la frase se arma con
 *   `plan` y sus hitos secundarios quedan vacíos: se pierde el matiz, nunca se inventa.
 * - **El ESTADO y el ÉXITO salen de `plan`**, que es su fuente canónica (WP5-2b): `underfunded`
 *   como booleano y el sorteo de Monte Carlo ya resuelto por el servidor. Los avisos textuales
 *   (`birth_date_missing`, `target_retirement_age_missing`) solo viven en la serie, así que se
 *   leen de ahí — no son otra versión del mismo hecho, son hechos que `plan` no publica.
 *
 * La precedencia del aviso es la de siempre (`planStatusFromWarnings`): `retire_at_age_underfunded`
 * (rojo) > `birth_date_missing` > `target_retirement_age_missing` > «En plan».
 *
 * El `tone` de la tarjeta es el del ESTADO, no el de la frase: la frase describe el hito y el
 * estado describe el plan, y cuando discrepan manda el segundo (D17).
 */
export function planCardV2(input: PlanCardV2Input): PlanCardV2 {
  const plan = input.plan ?? null;
  const usePlan = plan != null && plan.absent_reason == null;
  const series = input.series ?? null;

  const sentenceSource: PlanSentenceSeries | null =
    series ?? (usePlan ? planAsSentenceSeries(plan) : null);

  const sentence = planSentence({
    series: sentenceSource,
    targetRetirementAge: input.targetRetirementAge,
    monthLabel: input.monthLabel,
    ageMode: input.ageMode,
  });

  const status = planStatusFromPlan({
    underfunded: usePlan ? plan.underfunded : (sentenceSource?.underfunded ?? null),
    warnings: series?.warnings ?? null,
  });

  const strategy = (usePlan ? plan.strategy : null) ?? sentence.parts.strategy;
  const secondary = secondaryPhrase(sentence.parts);
  const subtitle = [
    strategy != null ? RETIREMENT_STRATEGY_LABEL[strategy] : "Sin estrategia",
    secondary,
  ]
    .filter((x): x is string => x != null)
    .join(" · ");

  const successTile = summarySuccessTile(plan);

  return {
    title: sentence.text,
    subtitle,
    // El tono es el del ESTADO — salvo cuando no hay NADA que evaluar (ni plan ni serie): ahí el
    // estado diría «En plan» en verde sobre una tarjeta que dice «Sin plan que mostrar», y «no
    // sé nada de ti» no es «vas bien».
    tone: sentenceSource == null ? sentence.tone : status.tone,
    success:
      successTile == null
        ? null
        : {
            label: "Éxito del plan",
            value: successTile.value,
            tone: successTile.tone,
            parenthetical: successTile.parenthetical,
            detail: successTile.detail,
          },
    warning:
      status.warning == null || status.action == null
        ? null
        : {
            text: status.label,
            actionLabel: status.action.label,
            target: status.action.target,
          },
  };
}
