/**
 * Modelo PURO de la tarjeta «Tu plan» del Resumen (5.0.0, D27/D32 → U9/U10, modelo v2 C1-C8) y
 * del ESTADO que comparten esa tarjeta y las líneas del hogar.
 *
 * Dos piezas viven aquí:
 *
 *  1. **El ESTADO** (`planStatusFromWarnings`/`planStatusFromPlan`): sale de un array de
 *     literales cerrados (`warnings[]`) más los dos escalares del modelo v2 (`plan_state` y
 *     `contribution_underfunded`), con PRECEDENCIA entre ellos — y esa precedencia es
 *     exactamente el tipo de regla que se rompe en silencio al añadir el aviso siguiente. Con la
 *     regla aquí, un test la fija (`plan-card.test.ts`). `resolvePlanMilestoneCivil` vive junto
 *     al estado porque las dos cosas fechan/traducen el mismo escalar del servidor.
 *  2. **`planCardV2`** (más abajo): la tarjeta ANCHA de U9 — una ORACIÓN
 *     (`lib/plan-sentence.ts`) más los KPI «Éxito del plan» y «Capital necesario hoy», y el
 *     aviso. Es el único modelo que consume `SummaryView.tsx` desde U2.
 */

import type { SummaryPlanApi } from "../api/types";
import { formatCurrencyAmount } from "./format";
import {
  addMonthsCivil,
  ageCompletedYearsCivil,
  parseYmdComponents,
} from "./dates";
import {
  planSentence,
  type PlanSentenceAgeMode,
  type PlanSentenceParts,
  type PlanSentenceSeries,
  type PlanSentenceTone,
} from "./plan-sentence";
import { RETIREMENT_STRATEGY_LABEL } from "./retirementProfile";
import { scenariosPerHundred, summarySuccessTile } from "./risk-bands";

/**
 * Avisos que el plan sabe leer. Literales cerrados del contrato (`warnings[]` de
 * `GET /v1/projection/series` y de `members[]`), más `contribution_underfunded`, que **no es un
 * literal de `warnings[]`**: viaja como booleano y se traduce a esta lista para que la
 * precedencia sea una sola tabla.
 *
 * El modelo v2 retiró `retire_at_age_underfunded` del motor: la lectura equivalente es
 * `contribution_underfunded` («ni con todo el sobrante llegas»), que además responde a una
 * pregunta más estrecha —la aportación— y no a un veredicto de la estrategia entera.
 */
export type PlanWarning =
  | "contribution_underfunded"
  | "birth_date_missing"
  | "target_retirement_age_missing"
  | "coast_not_reachable"
  | "partial_never_starts"
  | "partial_never_fully_retires";

export type PlanStatusTone = PlanSentenceTone;

export type PlanStatus = {
  /** El aviso que ganó la precedencia, o `null` cuando no hay ninguno («En plan»). */
  warning: PlanWarning | null;
  tone: PlanStatusTone;
  label: string;
  /** Adónde lleva el enlace de arreglo, cuando hay algo que arreglar. */
  action: { label: string; target: "account" | "retirement" } | null;
};

/** La tabla de precedencia, en orden. Es una LISTA y no una cadena de `if` sueltos porque el
 *  orden ES la regla: `warnings[]` puede traer varios literales y la tarjeta enseña una línea. */
const WARNING_STATUS: Array<{ warning: PlanWarning } & Omit<PlanStatus, "warning">> = [
  // Primero lo que invalida el plan (está completo y NO llega, o no hay plan que evaluar)…
  {
    warning: "contribution_underfunded",
    tone: "danger",
    label: "Ni ahorrando todo tu sobrante llegas a tu edad objetivo",
    action: { label: "Revisar tu plan", target: "retirement" },
  },
  {
    warning: "birth_date_missing",
    tone: "danger",
    label: "Falta tu fecha de nacimiento",
    action: { label: "Tu cuenta", target: "account" },
  },
  {
    warning: "coast_not_reachable",
    tone: "danger",
    label: "Ni aportando siempre llegas a tu edad objetivo",
    action: { label: "Revisar tu plan", target: "retirement" },
  },
  {
    warning: "partial_never_starts",
    tone: "danger",
    label: "Tu plan no puede permitirse la jornada reducida",
    action: { label: "Revisar tu plan", target: "retirement" },
  },
  {
    warning: "partial_never_fully_retires",
    tone: "danger",
    label: "Empiezas la jornada reducida pero no llegas a jubilarte del todo",
    action: { label: "Revisar tu plan", target: "retirement" },
  },
  // …y después lo que le falta.
  {
    warning: "target_retirement_age_missing",
    tone: "warn",
    label: "Falta tu edad de jubilación objetivo",
    action: { label: "Elegir edad", target: "retirement" },
  },
];

/**
 * Precedencia deliberada: **primero lo que invalida el plan, después lo que le falta**.
 *
 * - `contribution_underfunded` gana siempre: el plan está completo y NO llega. Es un resultado,
 *   no un hueco de configuración.
 * - `birth_date_missing` es rojo en el modelo v2 y no ámbar como en 4.x: **sin fecha de
 *   nacimiento el servidor no publica fecha, ni éxito, ni capital necesario** (C5), así que no
 *   es «falta un dato y el hito de al lado sigue valiendo» — es «no hay plan».
 * - Los tres fallos de solve (`coast_not_reachable`, `partial_never_starts`,
 *   `partial_never_fully_retires`) son rojos por la misma razón: el plan configurado no existe.
 * - `target_retirement_age_missing` es ámbar: falta un dato y la estrategia degradó a «Cuanto
 *   antes», así que el hito que se ve al lado es el de otra simulación — decirlo es el objetivo.
 * - Sin avisos: «En plan».
 *
 * Los literales que no conoce se ignoran (un aviso nuevo del servidor nunca deja la tarjeta sin
 * estado; a lo sumo dice «En plan» hasta que alguien lo traduzca aquí). Eso incluye a propósito
 * `no_volatility_declared` y `strategy_pension_bridge_migrated`, que son informativos y viven en
 * «Riesgo» y en la tarjeta de Pensión: subirlos aquí llenaría de ámbar el Resumen de casi todo
 * el mundo.
 */
export function planStatusFromWarnings(
  warnings: readonly string[] | null | undefined,
): PlanStatus {
  const set = new Set(warnings ?? []);
  for (const entry of WARNING_STATUS) {
    if (set.has(entry.warning)) {
      return { warning: entry.warning, tone: entry.tone, label: entry.label, action: entry.action };
    }
  }
  return { warning: null, tone: "ok", label: "En plan", action: null };
}

/** Copy de cada razón por la que el bloque «plan» no viaja. `absent_reason` del Resumen y
 *  `plan_absent_reason` de la serie son listas DISTINTAS y las dos se traducen aquí: la tarjeta
 *  puede alimentarse de cualquiera de las dos fuentes. */
const ABSENT_STATUS: Record<string, Omit<PlanStatus, "warning">> = {
  birth_date_missing: {
    tone: "danger",
    label: "Falta tu fecha de nacimiento",
    action: { label: "Tu cuenta", target: "account" },
  },
  household_aggregate: {
    tone: "warn",
    label: "El hogar no tiene un plan propio",
    action: null,
  },
  household_not_solved: {
    tone: "warn",
    label: "El hogar no tiene un plan propio",
    action: null,
  },
  projection_unavailable: {
    tone: "warn",
    label: "No se pudo calcular tu proyección",
    action: null,
  },
  months_override: {
    tone: "warn",
    label: "Esta simulación usa un horizonte forzado",
    action: null,
  },
  no_liquid_assets: {
    tone: "warn",
    label: "Sin activos líquidos no hay plan que sostener",
    action: null,
  },
};

/**
 * Estado de la tarjeta cuando la fuente es el objeto `plan` del Resumen, que **no trae
 * `warnings[]`**: el estado llega como los escalares `plan_state`, `absent_reason` y el booleano
 * de infra-financiación.
 *
 * `underfunded`: `true` = el plan está completo y no llega; `false` = llega; **`null` = la
 * pregunta no aplica a esta estrategia**, y colapsarlo con `false` pintaría de verde un plan que
 * nadie ha evaluado. Se miran las DOS vías porque la fila de miembro sí tiene avisos y la propia
 * no.
 */
export function planStatusFromPlan(input: {
  planState?: SummaryPlanApi["plan_state"] | null;
  absentReason?: string | null;
  underfunded?: boolean | null;
  warnings?: readonly string[] | null;
}): PlanStatus {
  if (input.planState === "pending") {
    return { warning: null, tone: "warn", label: "Calculando tu plan…", action: null };
  }
  if (input.underfunded === true) {
    return planStatusFromWarnings(["contribution_underfunded"]);
  }
  // Un aviso explícito gana a la razón de ausencia: dice QUÉ falta, no solo que falta algo.
  const fromWarnings = planStatusFromWarnings(input.warnings);
  if (fromWarnings.warning != null) return fromWarnings;
  const reason = input.absentReason ?? null;
  if (reason != null) {
    const copy = ABSENT_STATUS[reason];
    if (copy) return { warning: null, ...copy };
    return { warning: null, tone: "warn", label: "Tu plan no está disponible", action: null };
  }
  return fromWarnings;
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

// ═════════════════════════════════════════════════════════════════════════════════════════════
// V2 — la tarjeta ANCHA del Resumen (U9)
// ═════════════════════════════════════════════════════════════════════════════════════════════

/** El aviso, ya listo para su fila: texto + a dónde se va a arreglarlo. */
export type PlanCardWarningV2 = {
  text: string;
  actionLabel: string;
  target: "account" | "retirement";
};

/** Un KPI de la tarjeta. Es el MISMO plan que pinta Jubilación: aquí solo se rotula, jamás se
 *  recalcula. */
export type PlanCardKpiV2 = {
  label: string;
  value: string;
  tone: "default" | "warn" | "danger";
  /** El SUJETO de la cifra, sin el que un «87,0 %» pelado no dice de qué. */
  parenthetical?: string;
  /** Segundo slot: la razón de que no haya cifra, o el matiz que la acompaña. */
  detail?: string;
};

/** Nombre histórico del KPI de éxito; se conserva para no romper importaciones. */
export type PlanCardSuccessV2 = PlanCardKpiV2;

export type PlanCardV2 = {
  /** La ORACIÓN del plan (`planSentence`), que es el título de la tarjeta. */
  title: string;
  /** Estrategia + el hito secundario que esa estrategia añade. */
  subtitle: string;
  tone: PlanStatusTone;
  /** «Éxito del plan» — el KPI central del modelo v2. */
  success: PlanCardKpiV2 | null;
  /** «Capital necesario hoy» — el líquido que sostendría el plan si te jubilaras ya. */
  neededCapital: PlanCardKpiV2 | null;
  warning: PlanCardWarningV2 | null;
};

/** La serie del usuario, con lo que la frase y el estado necesitan. `warnings` ya viaja dentro
 *  de `PlanSentenceSeries`: los avisos textuales solo existen ahí. */
export type PlanCardV2Series = PlanSentenceSeries;

export type PlanCardV2Input = {
  /** `summary.plan` — la fuente canónica del ESTADO, del éxito y del capital necesario. */
  plan?: SummaryPlanApi | null;
  /** La serie que el chart de la misma pantalla ya tiene cargada. */
  series?: PlanCardV2Series | null;
  monthLabel: (monthIndex: number) => string;
  /** Edad cumplida en un mes de la rejilla (ver `PlanSentenceInput.ageAt`). */
  ageAt?: (monthIndex: number) => number | null;
  currencyIso: string;
  ageMode?: PlanSentenceAgeMode;
  /** Edad objetivo GUARDADA del perfil. */
  targetRetirementAge: number | null;
};

/** Frase corta del hito secundario de cada estrategia; `null` cuando la estrategia no tiene uno. */
function secondaryPhrase(parts: PlanSentenceParts): string | null {
  if (parts.secondaryLabel == null) return null;
  switch (parts.secondaryKind) {
    case "coast":
      return `dejas de aportar en ${parts.secondaryLabel}`;
    case "partial":
      // Sin repetir «jornada reducida»: el rótulo de la estrategia ya lo dice justo antes.
      return `desde ${parts.secondaryLabel}`;
    case "pension":
      return `pensión desde ${parts.secondaryLabel}`;
    default:
      return null;
  }
}

/**
 * La FORMA CORTA de la frase, cuando la serie no está cargada y solo hay `summary.plan`.
 *
 * `plan` publica cinco escalares del plan —fecha válida, éxito, umbral, capital y estado— y **no
 * publica edades ni hitos secundarios**, así que la frase corta dice exactamente eso y nada más:
 * fecha + éxito. Rellenar la edad con la guardada del perfil sería el bug B5 otra vez (rotular
 * una edad que el motor no leyó); inventar un hito secundario, peor.
 */
function summaryShortSentence(
  plan: SummaryPlanApi,
  monthLabel: (monthIndex: number) => string,
): { text: string; tone: PlanStatusTone } {
  if (plan.plan_state === "pending") {
    return { text: "Calculando tu fecha…", tone: "warn" };
  }
  if (plan.plan_state === "absent") {
    const reason = plan.absent_reason ?? null;
    const copy = reason == null ? null : ABSENT_STATUS[reason];
    return {
      text: copy?.label ?? "Tu plan no está disponible",
      tone: copy?.tone ?? "warn",
    };
  }
  const n = scenariosPerHundred(plan.success_of_plan);
  const mi = plan.safe_date_month_index ?? plan.jubilacion_month_index ?? null;
  if (mi == null) {
    const u = plan.success_threshold_pct;
    return {
      text:
        u == null
          ? "Con tu plan no hay ninguna fecha que aguante tu umbral."
          : `Con tu plan no hay ninguna fecha en la que aguanten ${u} de cada 100 escenarios.`,
      tone: "danger",
    };
  }
  const head = mi <= 0 ? "Con tu plan ya puedes jubilarte" : `Con tu plan te jubilas en ${monthLabel(mi)}`;
  return {
    text: n == null ? `${head}.` : `${head}: aguantan ${n} de cada 100 escenarios.`,
    tone: "ok",
  };
}

/**
 * La tarjeta «Tu plan» del Resumen (U9): **una sola tarjeta ancha** con la frase de arriba, la
 * estrategia debajo, los dos KPI del modelo v2 y una fila de aviso.
 *
 * ## De dónde sale cada mitad, y por qué no se mezclan
 *
 * - **La FRASE sale de la serie** cuando la hay. La serie es estrictamente más rica —trae el mes
 *   coast, el de jornada reducida, las fechas al 100/90 y las edades que el servidor calculó— y
 *   es el mismo objeto que el chart está dibujando dos centímetros más abajo. Sin serie, la
 *   frase cae a la forma CORTA de `summary.plan` (fecha + éxito): se pierde el matiz, nunca se
 *   inventa.
 * - **El ESTADO, el ÉXITO y el CAPITAL NECESARIO salen de `plan`**, su fuente canónica: el
 *   mismo cache de plan que resolvió la fecha, no un segundo sorteo. Los avisos textuales
 *   (`birth_date_missing`, `coast_not_reachable`…) solo viven en la serie, así que se leen de
 *   ahí — no son otra versión del mismo hecho, son hechos que `plan` no publica.
 *
 * El `tone` de la tarjeta es el del ESTADO, no el de la frase: la frase describe el hito y el
 * estado describe el plan, y cuando discrepan manda el segundo.
 */
export function planCardV2(input: PlanCardV2Input): PlanCardV2 {
  const plan = input.plan ?? null;
  const usePlan = plan != null && plan.absent_reason == null;
  const series = input.series ?? null;

  const sentence =
    series != null
      ? planSentence({
          series,
          targetRetirementAge: input.targetRetirementAge,
          monthLabel: input.monthLabel,
          ageAt: input.ageAt,
          currencyIso: input.currencyIso,
          ageMode: input.ageMode,
        })
      : null;
  const short = sentence == null && plan != null ? summaryShortSentence(plan, input.monthLabel) : null;

  const status = planStatusFromPlan({
    planState: plan?.plan_state ?? null,
    absentReason: plan?.absent_reason ?? series?.plan_absent_reason ?? null,
    underfunded: series?.contribution_underfunded ?? (usePlan ? plan.underfunded : null),
    warnings: series?.warnings ?? null,
  });

  const strategy = (usePlan ? plan.strategy : null) ?? sentence?.parts.strategy ?? null;
  const secondary = sentence == null ? null : secondaryPhrase(sentence.parts);
  const subtitle = [
    strategy != null ? RETIREMENT_STRATEGY_LABEL[strategy] : "Sin estrategia",
    secondary,
  ]
    .filter((x): x is string => x != null)
    .join(" · ");

  const successTile = summarySuccessTile(plan);
  const neededCapital = plan?.needed_capital_today ?? null;

  return {
    title: sentence?.text ?? short?.text ?? "Sin plan que mostrar",
    subtitle,
    // El tono es el del ESTADO — salvo cuando no hay NADA que evaluar (ni plan ni serie): ahí el
    // estado diría «En plan» en verde sobre una tarjeta que dice «Sin plan que mostrar», y «no
    // sé nada de ti» no es «vas bien».
    tone: sentence == null && short == null ? "warn" : status.tone,
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
    // Sin cifra no se pinta el KPI: un guion mudo se leería como «tu plan no necesita capital».
    neededCapital:
      neededCapital == null
        ? null
        : {
            label: "Capital necesario hoy",
            value: formatCurrencyAmount(neededCapital, input.currencyIso),
            tone: "default",
            parenthetical: "el líquido que sostendría tu plan si te jubilaras ya",
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
