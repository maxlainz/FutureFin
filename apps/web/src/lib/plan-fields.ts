/**
 * **La única fuente de verdad de qué campos del plan de jubilación se ven, y en qué tarjeta**
 * (5.0.0; decisiones U2 y U12 de #207, reorganizada por V3 de la tercera vuelta de UX).
 *
 * U2 en una frase: **solo se enseñan los campos que importan para la estrategia elegida; los que
 * no, no se enseñan en absoluto** — ni en gris, ni «por defecto», ni plegados. Un campo visible
 * anuncia que la simulación lo va a mirar, y en cuatro de las cinco estrategias eso es mentira
 * para la mitad del formulario.
 *
 * **Qué cambió en V3 (F9/F10)**: el eje de agrupación. Hasta aquí la tabla repartía los campos en
 * dos GRUPOS —«plan» (lo que se pregunta) y «avanzado» (los supuestos)— y el segundo vivía dentro
 * de un acordeón «Avanzado» con seis secciones. El owner lo leyó como «un cajón de sastre mal
 * explicado»: tres mandos de pensión quedaban a dos pantallas de la casilla «Cuento con una
 * pensión», y `partial_expense_basis` lejos de la media jornada. El eje nuevo es **el TEMA**
 * (`PlanCardId`), todo a la vista, cada tarjeta con su frase de qué hace. No cambia ni una
 * condición de visibilidad: es exactamente la misma tabla, ordenada por otro criterio.
 *
 * Con el acordeón se fue su contrapeso U12, la línea «Supuestos» (`lib/assumptions-line.ts`,
 * retirada): existía para enunciar lo que el acordeón escondía. Sin acordeón no hay nada
 * escondido que enunciar — todo supuesto en vigor está en su tarjeta, a la vista.
 *
 * Tres cosas que este módulo NO hace:
 *
 *  1. **No valida.** La guarda de validez es `retirementProfileIssue` (`lib/retirementProfile.ts`),
 *     espejo del servidor con sus mismos códigos. Aquí `required` significa «sin esto la
 *     estrategia no se puede simular tal y como el usuario la ha pedido», que es lo que el
 *     asistente de alta necesita saber para no dejarle avanzar.
 *  2. **No conoce el `<input>`.** Devuelve descriptores; la vista decide el control.
 *  3. **No decide el orden visual dentro de la tarjeta.** El orden del array ES el orden de
 *     lectura, y las tarjetas salen contiguas por construcción; agrupar es cosa de quien pinta.
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
 *  colocarlo en la tabla, y por tanto a decidir en qué estrategias existe y en qué tarjeta cae.
 *
 *  **Dos ids se retiraron en la tercera vuelta de UX** y no vuelven sin deshacer una decisión del
 *  owner: `cash_buffer_months` (V6 — el colchón se DERIVA del tope de tu regla de ahorro y la SPA
 *  solo lo informa) y `success_threshold_pct` (V7 — el corte del semáforo es fijo al 100 % y ya no
 *  es del usuario). Los dos siguen existiendo en el perfil por API y por MCP; lo que desapareció
 *  es su campo de formulario. */
export type PlanFieldId =
  | "birth_date"
  | "target_retirement_age"
  | "partial_start_age"
  | "partial_income"
  | "partial_expense_basis"
  | "pension_amount"
  | "pension_start_age"
  | "pension_indexed"
  | "pension_fraction_while_partial"
  | "target_basis"
  | "bridge_discount_basis"
  | "fire_number_mode"
  | "fire_number_manual_amount"
  | "swr_pct"
  | "withdrawal_rule_kind"
  | "hybrid_end_pct"
  | "guardrails_band_pct"
  | "guardrails_adjust_pct"
  | "spend_mode"
  | "horizon_lifespan_age";

/**
 * Las tarjetas por tema (V3). **Seis, no siete**: la lista del owner incluía «Riesgo», pero tras
 * V6 y V7 esa tarjeta se quedó SIN un solo campo editable —el colchón se deriva, el umbral no
 * existe— y una tarjeta vacía no se pinta. Lo que el owner quería leer bajo ese nombre sigue en la
 * página: es el bloque «Riesgo» del panel «Resultado», con el éxito, la banda coloreada y la línea
 * informativa del colchón. Un id aquí que nadie pudiera producir sería un token huérfano que
 * «parece vivo» y acaba usándose para otra cosa (el precedente de la casa es `--proj-jub`).
 *
 * `strategy` es la excepción a «una tarjeta son sus campos»: su contenido no sale de esta tabla,
 * es el radiogroup de las cinco estrategias, así que siempre se pinta aunque no tenga campos.
 */
export type PlanCardId =
  | "strategy"
  | "ages"
  | "pension"
  | "spending"
  | "withdrawal"
  | "horizon";

/** Orden de lectura de las tarjetas: primero QUÉ dispara la jubilación, luego CUÁNDO, con QUÉ
 *  rentas, CUÁNTO se gasta, CÓMO se saca y HASTA cuándo tiene que durar. */
export const PLAN_CARD_ORDER: readonly PlanCardId[] = [
  "strategy",
  "ages",
  "pension",
  "spending",
  "withdrawal",
  "horizon",
];

/**
 * Un campo visible y la tarjeta donde vive.
 *
 * `card` es TOTAL (no `card?`) a propósito: el descriptor anterior usaba una unión discriminada
 * por `group` para que un campo «avanzado» no pudiera compilar sin sección. La misma protección,
 * más barata: sin tarjeta un campo no se pintaría en ningún sitio, y el compilador lo caza al
 * añadirlo a la tabla.
 */
export type PlanFieldDescriptor = {
  id: PlanFieldId;
  card: PlanCardId;
  /** `true` ⟺ la estrategia elegida no se puede simular como se ha pedido sin este dato. Los
   *  supuestos con default resuelto por el servidor son siempre `false`. */
  required: boolean;
  /** Rótulo canónico. Varía con la estrategia en `target_retirement_age` (ver la tabla). */
  label: string;
};

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
 * La tabla U2, en orden de lectura y agrupada por TARJETA (V3).
 *
 * | Campo | Tarjeta | ¿Cuándo se ve? | ¿Obligatorio? |
 * |---|---|---|---|
 * | `birth_date` | edades | **solo si falta** | sí en `retire_at_age`/`coast`/`partial` |
 * | `target_retirement_age` | edades | `retire_at_age`, `coast`, `partial` | sí salvo en `partial` |
 * | `partial_start_age`, `partial_income` | edades | `partial` | sí |
 * | `partial_expense_basis` | edades | `partial` | no |
 * | `pension_amount`, `pension_start_age` | pensión | siempre (la casilla vive ahí) | solo en `pension_bridge` |
 * | `pension_indexed` | pensión | hay pensión | no |
 * | `pension_fraction_while_partial` | pensión | `partial` **y** hay pensión | no |
 * | `target_basis` | pensión | hay pensión **y** la estrategia no impone la base | no |
 * | `bridge_discount_basis` | pensión | la base efectiva ES el puente | no |
 * | `fire_number_mode` | gasto | siempre | no |
 * | `fire_number_manual_amount` | gasto | modo `manual` | sí |
 * | `swr_pct`, `withdrawal_rule_kind` | retirada | siempre | no |
 * | `hybrid_end_pct` | retirada | regla `hybrid` | no |
 * | `guardrails_band_pct`, `guardrails_adjust_pct` | retirada | regla `guardrails` | no |
 * | `spend_mode` | retirada | regla ≠ `fixed_real` | no |
 * | `horizon_lifespan_age` | horizonte | siempre | no |
 *
 * **Por qué las dos mudanzas de V3.** Las tres piezas finas de la pensión (`pension_indexed`,
 * `pension_fraction_while_partial`) y las dos del objetivo (`target_basis`,
 * `bridge_discount_basis`) viven ahora en la MISMA tarjeta que la casilla «Cuento con una
 * pensión»: las cinco solo existen cuando hay pensión declarada, y separarlas obligaba a leer la
 * pensión dos veces en dos sitios de la página. `partial_expense_basis` baja a «Edades» por lo
 * mismo: es el gasto DURANTE la fase parcial, y su sitio es junto a la fase.
 *
 * **U4 en la tabla**: no existe `withdrawal_pct` ni `hybrid_start_pct`. El porcentaje de la
 * regla es `swr_pct`, el mismo que dimensiona el objetivo; la híbrida solo añade el «baja al
 * X %» (`hybrid_end_pct`) y las bandas su banda y su ajuste. Enseñar dos porcentajes de retirada
 * obligaba a explicar cuál mandaba, y la respuesta honesta era «depende de la pantalla».
 */
export function planFields(ctx: PlanFieldsContext): PlanFieldDescriptor[] {
  const out: PlanFieldDescriptor[] = [];
  const f = (
    id: PlanFieldId,
    card: PlanCardId,
    required: boolean,
    label: string,
  ) => out.push({ id, card, required, label });

  // ── Edades ───────────────────────────────────────────────────────────────────────────────
  // La fecha de nacimiento solo aparece cuando FALTA: pedirla otra vez a quien ya la tiene es
  // ruido, y su sitio natural es «Tu cuenta».
  if (!ctx.hasBirthDate) {
    f("birth_date", "ages", strategyNeedsBirthDate(ctx.strategy), "Fecha de nacimiento");
  }
  if (
    ctx.strategy === "retire_at_age" ||
    ctx.strategy === "coast" ||
    ctx.strategy === "partial"
  ) {
    f(
      "target_retirement_age",
      "ages",
      ctx.strategy !== "partial",
      targetAgeLabel(ctx.strategy),
    );
  }
  if (ctx.strategy === "partial") {
    f("partial_start_age", "ages", true, "Edad de inicio de la media jornada");
    f("partial_income", "ages", true, "Ingreso mensual en media jornada");
    f("partial_expense_basis", "ages", false, "Gasto durante la media jornada");
  }

  // ── Pensión ──────────────────────────────────────────────────────────────────────────────
  // Se ofrece SIEMPRE (la casilla es parte de la tarjeta), pero solo el puente la exige:
  // esconderla en las demás estrategias haría invisible el dato que más mueve el objetivo.
  f("pension_amount", "pension", ctx.strategy === "pension_bridge", "Pensión mensual");
  f(
    "pension_start_age",
    "pension",
    ctx.strategy === "pension_bridge",
    "Edad de inicio de la pensión",
  );
  if (ctx.hasPension) {
    f("pension_indexed", "pension", false, "Pensión indexada a la inflación");
    if (ctx.strategy === "partial") {
      f(
        "pension_fraction_while_partial",
        "pension",
        false,
        "Pensión cobrada durante la media jornada",
      );
    }
    if (!ctx.strategyForcesBasis) {
      f("target_basis", "pension", false, "Base del objetivo");
    }
  }
  if (ctx.effectiveBasis === "bridge_to_pension") {
    f("bridge_discount_basis", "pension", false, "Descuento del puente");
  }

  // ── Gasto en jubilación ──────────────────────────────────────────────────────────────────
  f("fire_number_mode", "spending", false, "Cómo se calcula el objetivo");
  if (ctx.fireNumberMode === "manual") {
    f("fire_number_manual_amount", "spending", true, "Objetivo manual");
  }

  // ── Retirada ─────────────────────────────────────────────────────────────────────────────
  f("swr_pct", "withdrawal", false, "Tasa de retirada");
  f("withdrawal_rule_kind", "withdrawal", false, "Regla de retirada");
  if (ctx.ruleKind === "hybrid") f("hybrid_end_pct", "withdrawal", false, "Baja al");
  if (ctx.ruleKind === "guardrails") {
    f("guardrails_band_pct", "withdrawal", false, "Banda");
    f("guardrails_adjust_pct", "withdrawal", false, "Ajuste");
  }
  if (ctx.ruleKind !== "fixed_real") {
    f("spend_mode", "withdrawal", false, "Cómo se aplica la regla");
  }

  // ── Horizonte ────────────────────────────────────────────────────────────────────────────
  f("horizon_lifespan_age", "horizon", false, "Edad límite del horizonte");

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

/** Una tarjeta a pintar, con sus campos en orden de lectura. */
export type PlanCardGroup = {
  card: PlanCardId;
  fields: PlanFieldDescriptor[];
};

/**
 * Las tarjetas que de verdad se pintan, en `PLAN_CARD_ORDER`.
 *
 * **Ninguna tarjeta vacía**: una tarjeta con su título, su frase y ningún control anuncia una
 * decisión que el usuario no puede tomar. Con `asap` y fecha de nacimiento conocida, «Edades»
 * desaparece entera — que es exactamente lo que U2 pide.
 *
 * **`strategy` es la única excepción y está siempre**: su contenido no son campos de esta tabla
 * sino el radiogroup de las cinco estrategias, así que «vacía» ahí no significa «sin contenido».
 * La vista lo sabe y la pinta aparte.
 */
export function planCardGroups(ctx: PlanFieldsContext): PlanCardGroup[] {
  const fields = planFields(ctx);
  const out: PlanCardGroup[] = [];
  for (const card of PLAN_CARD_ORDER) {
    const mine = fields.filter((f) => f.card === card);
    if (card === "strategy" || mine.length > 0) out.push({ card, fields: mine });
  }
  return out;
}

/**
 * Contexto derivado de un perfil YA resuelto (`normalizeRetirementProfile` o la respuesta del
 * servidor) más el único dato que no vive en él: si el usuario tiene fecha de nacimiento.
 *
 * La base efectiva sale de `effectiveTargetBasis` (R6, misma regla que Rust) y `strategyForcesBasis`
 * de la propia estrategia — una sola derivación para el formulario y cualquier otro consumidor.
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
