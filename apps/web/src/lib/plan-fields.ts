/**
 * **La única fuente de verdad de qué campos del plan de jubilación se ven** (5.0.0, rediseño UX
 * U1a; decisiones U2 y U12 de #207).
 *
 * U2 en una frase: **solo se enseñan los campos que importan para la estrategia elegida; los que
 * no, no se enseñan en absoluto** — ni en gris, ni «por defecto», ni plegados. Un campo visible
 * anuncia que la simulación lo va a mirar, y en cuatro de las cinco estrategias eso es mentira
 * para la mitad del formulario.
 *
 * U12 es su contrapeso, y por eso la tabla vive AQUÍ y no dentro de la vista: **nada se fuerza en
 * silencio**. Los supuestos que la estrategia impone (o hereda del default) siguen existiendo y se
 * enuncian en la línea «Supuestos» (`lib/assumptions-line.ts`), que se construye **a partir de
 * esta misma tabla**. Si la visibilidad viviera en un `if` de `RetirementView.tsx`, la línea de
 * supuestos y el formulario podrían discrepar sobre qué está en juego, que es exactamente el
 * fallo que U12 existe para impedir.
 *
 * Tres cosas que este módulo NO hace:
 *
 *  1. **No valida.** La guarda de validez es `retirementProfileIssue` (`lib/retirementProfile.ts`),
 *     espejo del servidor con sus mismos códigos. Aquí `required` significa «sin esto la
 *     estrategia no se puede simular tal y como el usuario la ha pedido», que es lo que el
 *     asistente de alta necesita saber para no dejarle avanzar.
 *  2. **No conoce el `<input>`.** Devuelve descriptores; la vista decide el control.
 *  3. **No decide el orden visual por sección.** El orden del array ES el orden de lectura, y las
 *     secciones salen contiguas por construcción; agrupar es cosa de quien pinta.
 */

import type {
  FireNumberModeApi,
  RetirementProfileApi,
  RetirementStrategyApi,
  TargetBasisApi,
  WithdrawalRuleKindApi,
} from "../api/types";
import { effectiveTargetBasis } from "./retirementProfile";

/** Todo campo del plan que la vista puede pintar. Cerrado a propósito: un id nuevo obliga a
 *  colocarlo en la tabla, y por tanto a decidir en qué estrategias existe. */
export type PlanFieldId =
  // --- grupo «plan»: lo que la estrategia PREGUNTA ---
  | "birth_date"
  | "target_retirement_age"
  | "partial_start_age"
  | "partial_income"
  | "pension_amount"
  | "pension_start_age"
  | "fire_number_mode"
  | "fire_number_manual_amount"
  // --- grupo «avanzado»: los supuestos ---
  | "swr_pct"
  | "withdrawal_rule_kind"
  | "hybrid_end_pct"
  | "guardrails_band_pct"
  | "guardrails_adjust_pct"
  | "spend_mode"
  | "target_basis"
  | "bridge_discount_basis"
  | "pension_indexed"
  | "pension_fraction_while_partial"
  | "partial_expense_basis"
  | "horizon_lifespan_age"
  | "cash_buffer_months"
  | "success_threshold_pct";

/** `plan` = lo que hay que contestar para tener plan. `advanced` = supuestos con default. */
export type PlanFieldGroup = "plan" | "advanced";

/** Secciones del bloque avanzado. Solo las llevan los campos `advanced`. */
export type PlanFieldSection =
  | "retirada"
  | "objetivo"
  | "media_jornada"
  | "pension"
  | "horizonte"
  | "riesgo";

/**
 * Un campo visible. La unión discriminada por `group` no es decoración: **un campo del grupo
 * `plan` no tiene sección y uno `advanced` la tiene siempre**, y dejarlo en un `section?` haría
 * que un descriptor avanzado sin sección compilara y se pintara fuera de todo bloque.
 */
export type PlanFieldDescriptorPlan = {
  id: PlanFieldId;
  group: "plan";
  section: null;
  /** `true` ⟺ la estrategia elegida no se puede simular como se ha pedido sin este dato. */
  required: boolean;
  /** Rótulo canónico. Varía con la estrategia en `target_retirement_age` (ver la tabla). */
  label: string;
};

export type PlanFieldDescriptorAdvanced = {
  id: PlanFieldId;
  group: "advanced";
  section: PlanFieldSection;
  /** Siempre `false`: todo supuesto tiene default resuelto por el servidor. */
  required: false;
  label: string;
};

export type PlanFieldDescriptor = PlanFieldDescriptorPlan | PlanFieldDescriptorAdvanced;

/**
 * Lo que hace falta para resolver la tabla. Son HECHOS ya derivados, no el perfil crudo: el
 * módulo no debe volver a decidir si hay pensión o cuál es la base efectiva — eso ya lo deciden
 * `retirementProfile.ts` (R6) y la respuesta del servidor, y duplicar la derivación aquí es cómo
 * se abre una divergencia con lo que la proyección de verdad simuló.
 *
 * `planFieldsContextFromProfile` construye este objeto desde un `RetirementProfileApi` resuelto.
 */
export type PlanFieldsContext = {
  strategy: RetirementStrategyApi;
  /** Hay bloque de pensión declarado (importe + edad), no «la pensión es posible». */
  hasPension: boolean;
  /** El usuario tiene fecha de nacimiento en «Tu cuenta». */
  hasBirthDate: boolean;
  ruleKind: WithdrawalRuleKindApi;
  /** La base que se APLICA (R6), no la almacenada. */
  effectiveBasis: TargetBasisApi;
  /** `true` ⟺ la estrategia impone la base y elegirla no pinta nada (`pension_bridge`). */
  strategyForcesBasis: boolean;
  /** Modo del número FIRE: decide si el importe manual entra en el formulario. */
  fireNumberMode: FireNumberModeApi;
};

/** Contexto sin la estrategia — la forma que pide `requiredPlanFields(strategy, ctx)`. */
export type PlanFieldsContextWithoutStrategy = Omit<PlanFieldsContext, "strategy">;

/** Estrategias que EXIGEN fecha de nacimiento: sin ella el motor degrada el plan a «Cuanto
 *  antes» y la simulación deja de ser la que el usuario pidió. */
function strategyNeedsBirthDate(s: RetirementStrategyApi): boolean {
  return s === "retire_at_age" || s === "coast" || s === "partial";
}

/**
 * Rótulo de la edad objetivo. En «Media jornada» **no** es «la edad a la que me jubilo» sino el
 * fin de la fase parcial, y llamarla igual que en «A una edad fija» hacía que el mismo campo
 * significara dos cosas en la misma pantalla (U2).
 */
function targetAgeLabel(s: RetirementStrategyApi): string {
  return s === "partial" ? "Edad de jubilación total" : "Edad de jubilación objetivo";
}

/**
 * La tabla U2, en orden de lectura.
 *
 * Grupo `plan` (lo que la estrategia pregunta):
 *
 * | Campo | ¿Cuándo se ve? | ¿Obligatorio? |
 * |---|---|---|
 * | `birth_date` | **solo si falta** | sí en `retire_at_age`/`coast`/`partial`; no en `asap`/`pension_bridge` |
 * | `target_retirement_age` | `retire_at_age`, `coast`, `partial` | sí salvo en `partial` (ahí es opcional y se rotula «edad de jubilación total») |
 * | `partial_start_age`, `partial_income` | `partial` | sí |
 * | `pension_amount`, `pension_start_age` | siempre (la casilla vive en el grupo) | solo en `pension_bridge` |
 * | `fire_number_mode` | siempre | no |
 * | `fire_number_manual_amount` | modo `manual` | sí |
 *
 * Grupo `advanced` (supuestos, todos con default y por tanto nunca obligatorios):
 *
 * | Campo | Sección | ¿Cuándo se ve? |
 * |---|---|---|
 * | `swr_pct` | retirada | siempre — es **el único porcentaje de retirada** (U4) |
 * | `withdrawal_rule_kind` | retirada | siempre |
 * | `hybrid_end_pct` | retirada | regla `hybrid` |
 * | `guardrails_band_pct`, `guardrails_adjust_pct` | retirada | regla `guardrails` |
 * | `spend_mode` | retirada | regla ≠ `fixed_real` (con gasto fijo no hay techo del que hablar) |
 * | `target_basis` | objetivo | hay pensión **y** la estrategia no impone la base |
 * | `bridge_discount_basis` | objetivo | la base efectiva ES el puente |
 * | `pension_indexed` | pension | hay pensión |
 * | `pension_fraction_while_partial` | pension | `partial` **y** hay pensión |
 * | `partial_expense_basis` | media_jornada | `partial` |
 * | `horizon_lifespan_age` | horizonte | siempre |
 * | `cash_buffer_months`, `success_threshold_pct` | riesgo | siempre |
 *
 * **U4 en la tabla**: no existe `withdrawal_pct` ni `hybrid_start_pct`. El porcentaje de la
 * regla es `swr_pct`, el mismo que dimensiona el objetivo; la híbrida solo añade el «baja al
 * X %» (`hybrid_end_pct`) y las bandas su banda y su ajuste. Enseñar dos porcentajes de retirada
 * obligaba a explicar cuál mandaba, y la respuesta honesta era «depende de la pantalla».
 */
export function planFields(ctx: PlanFieldsContext): PlanFieldDescriptor[] {
  const out: PlanFieldDescriptor[] = [];
  const plan = (id: PlanFieldId, required: boolean, label: string) =>
    out.push({ id, group: "plan", section: null, required, label });
  const adv = (id: PlanFieldId, section: PlanFieldSection, label: string) =>
    out.push({ id, group: "advanced", section, required: false, label });

  // ── Grupo «plan» ─────────────────────────────────────────────────────────────────────────
  // La fecha de nacimiento solo aparece cuando FALTA: pedirla otra vez a quien ya la tiene es
  // ruido, y su sitio natural es «Tu cuenta».
  if (!ctx.hasBirthDate) {
    plan("birth_date", strategyNeedsBirthDate(ctx.strategy), "Fecha de nacimiento");
  }
  if (
    ctx.strategy === "retire_at_age" ||
    ctx.strategy === "coast" ||
    ctx.strategy === "partial"
  ) {
    plan(
      "target_retirement_age",
      ctx.strategy !== "partial",
      targetAgeLabel(ctx.strategy),
    );
  }
  if (ctx.strategy === "partial") {
    plan("partial_start_age", true, "Edad de inicio de la media jornada");
    plan("partial_income", true, "Ingreso mensual en media jornada");
  }
  // La pensión se ofrece SIEMPRE (la casilla es parte del grupo), pero solo el puente la exige:
  // esconderla en las demás estrategias haría invisible el dato que más mueve el objetivo.
  plan("pension_amount", ctx.strategy === "pension_bridge", "Pensión mensual");
  plan("pension_start_age", ctx.strategy === "pension_bridge", "Edad de inicio de la pensión");
  plan("fire_number_mode", false, "Cómo se calcula el objetivo");
  if (ctx.fireNumberMode === "manual") {
    plan("fire_number_manual_amount", true, "Objetivo manual");
  }

  // ── Grupo «avanzado» · retirada ──────────────────────────────────────────────────────────
  adv("swr_pct", "retirada", "Tasa de retirada");
  adv("withdrawal_rule_kind", "retirada", "Regla de retirada");
  if (ctx.ruleKind === "hybrid") adv("hybrid_end_pct", "retirada", "Baja al");
  if (ctx.ruleKind === "guardrails") {
    adv("guardrails_band_pct", "retirada", "Banda");
    adv("guardrails_adjust_pct", "retirada", "Ajuste");
  }
  if (ctx.ruleKind !== "fixed_real") {
    adv("spend_mode", "retirada", "Cómo se aplica la regla");
  }

  // ── objetivo ─────────────────────────────────────────────────────────────────────────────
  if (ctx.hasPension && !ctx.strategyForcesBasis) {
    adv("target_basis", "objetivo", "Base del objetivo");
  }
  if (ctx.effectiveBasis === "bridge_to_pension") {
    adv("bridge_discount_basis", "objetivo", "Descuento del puente");
  }

  // ── pensión ──────────────────────────────────────────────────────────────────────────────
  if (ctx.hasPension) {
    adv("pension_indexed", "pension", "Pensión indexada a la inflación");
    if (ctx.strategy === "partial") {
      adv(
        "pension_fraction_while_partial",
        "pension",
        "Pensión cobrada durante la media jornada",
      );
    }
  }

  // ── media jornada ────────────────────────────────────────────────────────────────────────
  if (ctx.strategy === "partial") {
    adv("partial_expense_basis", "media_jornada", "Gasto durante la media jornada");
  }

  // ── horizonte y riesgo ───────────────────────────────────────────────────────────────────
  adv("horizon_lifespan_age", "horizonte", "Edad límite del horizonte");
  adv("cash_buffer_months", "riesgo", "Colchón de caja");
  adv("success_threshold_pct", "riesgo", "Umbral de éxito");

  return out;
}

/** `true` ⟺ el campo se pinta con este contexto. Atajo de `planFields` para la vista, que
 *  pregunta campo a campo mientras compone el formulario. */
export function isFieldVisible(id: PlanFieldId, ctx: PlanFieldsContext): boolean {
  return planFields(ctx).some((f) => f.id === id);
}

/**
 * Los campos que hay que contestar para que la estrategia se simule como se ha pedido.
 *
 * Los consumen dos sitios con la misma pregunta y distinta consecuencia: el asistente de alta
 * (¿puedo dejarle avanzar?) y la guarda del autosave (¿mando ya el PATCH?). Nunca incluye
 * supuestos: un default resuelto por el servidor no es un hueco.
 */
export function requiredPlanFields(
  strategy: RetirementStrategyApi,
  ctx: PlanFieldsContextWithoutStrategy,
): PlanFieldId[] {
  return planFields({ ...ctx, strategy })
    .filter((f) => f.required)
    .map((f) => f.id);
}

/** Solo el grupo `plan`, en orden. */
export function planGroupFields(ctx: PlanFieldsContext): PlanFieldDescriptorPlan[] {
  return planFields(ctx).filter((f): f is PlanFieldDescriptorPlan => f.group === "plan");
}

/** Solo el grupo `advanced`, en orden (las secciones salen contiguas). */
export function advancedGroupFields(ctx: PlanFieldsContext): PlanFieldDescriptorAdvanced[] {
  return planFields(ctx).filter((f): f is PlanFieldDescriptorAdvanced => f.group === "advanced");
}

/**
 * Contexto derivado de un perfil YA resuelto (`normalizeRetirementProfile` o la respuesta del
 * servidor) más el único dato que no vive en él: si el usuario tiene fecha de nacimiento.
 *
 * La base efectiva sale de `effectiveTargetBasis` (R6, misma regla que Rust) y `strategyForcesBasis`
 * de la propia estrategia — una sola derivación para el formulario, la línea de supuestos y
 * cualquier otro consumidor.
 */
export function planFieldsContextFromProfile(
  profile: RetirementProfileApi,
  hasBirthDate: boolean,
): PlanFieldsContext {
  return {
    strategy: profile.strategy,
    hasPension: profile.pension != null,
    hasBirthDate,
    ruleKind: profile.withdrawal_rule.kind,
    effectiveBasis: effectiveTargetBasis(profile),
    strategyForcesBasis: profile.strategy === "pension_bridge",
    fireNumberMode: profile.fire_number_mode,
  };
}
