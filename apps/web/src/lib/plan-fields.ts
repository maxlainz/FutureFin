/**
 * **La única fuente de verdad de qué campos del plan de jubilación se ven, y en qué tarjeta**
 * (5.0.0; decisiones U2 y U12 de #207, reorganizada por V3 de la tercera vuelta de UX y
 * reescrita por el **modelo v2** — «el éxito define la fecha», C1–C8).
 *
 * U2 en una frase: **solo se enseñan los campos que importan para la estrategia elegida; los que
 * no, no se enseñan en absoluto** — ni en gris, ni «por defecto», ni plegados. Un campo visible
 * anuncia que la simulación lo va a mirar, y en la mayoría de las estrategias eso era mentira
 * para la mitad del formulario.
 *
 * **Qué cambió con el modelo v2.** Tres cosas, y ninguna es cosmética:
 *
 *  1. **El eje de visibilidad ya no es solo la estrategia: es la estrategia Y SU MODO.** `coast` y
 *     `partial` tienen dos modos cada una (M10/M11) y cada modo pregunta una edad DISTINTA: en
 *     coast A fijas la edad de jubilación y el servidor resuelve cuándo puedes dejar de aportar;
 *     en coast B fijas la parada y la fecha sale donde salga — y entonces `target_retirement_age`
 *     **no se pinta**, porque no la mira nadie. Igual con `partial`: en modo «en cuanto pueda» la
 *     edad de inicio la resuelve el servidor (`partial_start_month_index`) y preguntarla sería
 *     pedir un dato que la simulación va a ignorar.
 *  2. **El puente dejó de ser una estrategia y pasó a ser un ajuste de la tarjeta Pensión** (C7):
 *     tres campos (`bridge_enabled` + dos números) disponibles en CUALQUIER estrategia, y solo
 *     cuando hay pensión declarada — sin pensión no hay nada a lo que hacer de puente.
 *  3. **Vuelve `success_threshold_pct`** (C3), y vuelve el PRIMERO de la tarjeta «Retirada»: en v2
 *     el umbral no es un corte de semáforo, es **la restricción que decide la fecha válida**. Es,
 *     de largo, el campo que más mueve el resultado de esta pantalla, así que va arriba.
 *
 * Con el modelo v2 murieron `target_basis` y `bridge_discount_basis`: la pensión es un flujo de
 * caja (C1/M4), no descuenta ningún objetivo, y no queda ninguna «base del objetivo» que elegir.
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
  CoastModeApi,
  FireNumberModeApi,
  PartialRetirementApi,
  RetirementProfileApi,
  RetirementStrategyApi,
  WithdrawalRuleKindApi,
} from "../api/types";

/** Modo de la fase de media jornada (M11), con el mismo literal que el wire. Se declara aquí
 *  —y no se importa un alias de `PartialRetirementApi["mode"]` en cada firma— para que las
 *  condiciones de la tabla se lean como lo que son. */
export type PartialModeApi = PartialRetirementApi["mode"];

/** Todo campo del plan que la vista puede pintar. Cerrado a propósito: un id nuevo obliga a
 *  colocarlo en la tabla, y por tanto a decidir en qué estrategias existe y en qué tarjeta cae.
 *
 *  **Un id sigue sin volver**: `cash_buffer_months` (V6/M6 — el colchón como MECANISMO se retiró
 *  del motor; no hay nada que configurar). `success_threshold_pct`, en cambio, **sí vuelve**: V7
 *  lo retiró cuando el umbral era un corte de semáforo fijo al 100 %, y el modelo v2 lo devuelve
 *  al perfil como la restricción que decide la fecha válida (C3, 80–100, default 95).
 *
 *  **Dos ids murieron con el modelo v2** y no vuelven sin deshacer C1/M4: `target_basis` y
 *  `bridge_discount_basis`. La pensión es un flujo de caja, no un descuento sobre un objetivo, y
 *  no queda objetivo que dimensionar: no hay base que elegir ni descuento que aplicar. */
export type PlanFieldId =
  | "birth_date"
  | "coast_mode"
  | "target_retirement_age"
  | "coast_stop_age"
  | "partial_mode"
  | "partial_start_age"
  | "partial_income"
  | "partial_expense_basis"
  | "pension_amount"
  | "pension_start_age"
  | "pension_indexed"
  | "pension_fraction_while_partial"
  | "bridge_enabled"
  | "bridge_max_pct"
  | "bridge_max_years"
  | "fire_number_mode"
  | "fire_number_manual_amount"
  | "success_threshold_pct"
  | "swr_pct"
  | "withdrawal_rule_kind"
  | "hybrid_end_pct"
  | "guardrails_band_pct"
  | "guardrails_adjust_pct"
  | "spend_mode"
  | "horizon_lifespan_age";

/**
 * Las tarjetas por tema (V3). **Seis, no siete**: la lista del owner incluía «Riesgo», y sigue sin
 * existir aunque el umbral haya vuelto — su sitio es la tarjeta «Retirada», junto a la tasa, y por
 * el mismo motivo por el que vuelve: las dos cifras deciden JUNTAS la fecha válida (el umbral dice
 * cuántos escenarios tienen que aguantar; la tasa, cuánto se saca el primer año), y separarlas en
 * dos cuadros habría obligado a explicar en cada uno la mitad que faltaba. Lo que el owner quería
 * leer bajo el nombre «Riesgo» sigue en la página: es el bloque homónimo del panel «Resultado»,
 * con el éxito, la banda coloreada y el desglose de fallos.
 *
 * `strategy` es la excepción a «una tarjeta son sus campos»: su contenido no sale de esta tabla,
 * es el radiogroup de las CUATRO estrategias (C7 retiró «Puente hasta la pensión» del selector),
 * así que siempre se pinta aunque no tenga campos.
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
 * módulo no debe volver a decidir si hay pensión o en qué modo está una estrategia — eso ya lo
 * dice el perfil resuelto que devuelve el servidor, y duplicar la derivación aquí es cómo se abre
 * una divergencia con lo que la proyección de verdad simuló.
 *
 * **Perdió dos ejes con el modelo v2** (`effectiveBasis`, `strategyForcesBasis`): no hay base del
 * objetivo porque no hay objetivo. **Ganó tres**: los dos MODOS —`coastMode`, `partialMode`— que
 * deciden qué edad se pregunta, y `bridgeEnabled`, que abre los dos números del puente.
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
  /** Modo del número FIRE: decide si el importe manual entra en el formulario. */
  fireNumberMode: FireNumberModeApi;
  /** M10: `fixed_retirement_age` (A) pregunta la edad de jubilación; `fixed_stop_age` (B)
   *  pregunta la edad de PARADA y esconde la de jubilación. Solo se mira con `coast`. */
  coastMode: CoastModeApi;
  /** M11: `at_age` (A) pregunta la edad de inicio de la fase; `asap` (B) la resuelve el servidor
   *  y no se pregunta. Solo se mira con `partial`. */
  partialMode: PartialModeApi;
  /** C7: el puente está activado. Abre sus dos números, y solo con pensión declarada. */
  bridgeEnabled: boolean;
};

/** Contexto sin la estrategia — la forma que pide `requiredPlanFields(strategy, ctx)`. */
export type PlanFieldsContextWithoutStrategy = Omit<PlanFieldsContext, "strategy">;

/**
 * Cuándo la fecha de nacimiento es OBLIGATORIA (C5).
 *
 * Dos motivos, y el segundo es nuevo del modelo v2:
 *
 *  - Las estrategias por EDAD (`retire_at_age`, `coast`, `partial`) no se pueden simular como se
 *    han pedido sin ella: sin edad no hay mes contra el que resolver nada.
 *  - **Con una pensión declarada, tampoco**: la pensión entra en el bucle a una EDAD, y sin fecha
 *    de nacimiento no se sabe si ya se cobra en la fecha válida. El servidor lo dice con todas las
 *    letras — `plan_absent_reason: "birth_date_missing"` deja el bloque «plan» entero sin publicar
 *    y `pension_absent_reason` explica que la pensión no participa. Un plan «Cuanto antes» con
 *    pensión y sin fecha de nacimiento se guardaba tan tranquilo y salía sin fecha, sin éxito y
 *    sin capital necesario: exactamente el hueco silencioso que `required` existe para tapar.
 */
function needsBirthDate(ctx: PlanFieldsContext): boolean {
  if (ctx.hasPension) return true;
  return (
    ctx.strategy === "retire_at_age" ||
    ctx.strategy === "coast" ||
    ctx.strategy === "partial"
  );
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
 * La tabla U2 del modelo v2, en orden de lectura y agrupada por TARJETA (V3).
 *
 * | Campo | Tarjeta | ¿Cuándo se ve? | ¿Obligatorio? |
 * |---|---|---|---|
 * | `birth_date` | edades | **solo si falta** | sí con pensión declarada o en `retire_at_age`/`coast`/`partial` (C5) |
 * | `coast_mode` | edades | `coast` | no (tiene default) |
 * | `target_retirement_age` | edades | `retire_at_age`; `coast` **modo A**; `partial` | sí salvo en `partial` |
 * | `coast_stop_age` | edades | `coast` **modo B** | sí |
 * | `partial_mode` | edades | `partial` | no (tiene default) |
 * | `partial_start_age` | edades | `partial` **modo A** (`at_age`) | sí |
 * | `partial_income` | edades | `partial` | sí |
 * | `partial_expense_basis` | edades | `partial` | no |
 * | `pension_amount`, `pension_start_age` | pensión | siempre (la casilla vive ahí) | sí con pensión declarada |
 * | `pension_indexed` | pensión | hay pensión | no |
 * | `pension_fraction_while_partial` | pensión | `partial` **y** hay pensión | no |
 * | `bridge_enabled` | pensión | hay pensión (CUALQUIER estrategia, C7) | no |
 * | `bridge_max_pct`, `bridge_max_years` | pensión | hay pensión **y** el puente está activado | sí |
 * | `fire_number_mode` | gasto | siempre | no |
 * | `fire_number_manual_amount` | gasto | modo `manual` | sí |
 * | `success_threshold_pct` | retirada | siempre, **la primera** | no |
 * | `swr_pct`, `withdrawal_rule_kind` | retirada | siempre | no |
 * | `hybrid_end_pct` | retirada | regla `hybrid` | no |
 * | `guardrails_band_pct`, `guardrails_adjust_pct` | retirada | regla `guardrails` | no |
 * | `spend_mode` | retirada | regla ≠ `fixed_real` | no |
 * | `horizon_lifespan_age` | horizonte | siempre | no |
 *
 * **Por qué el modo va ANTES que su edad.** En `coast` y en `partial` la pregunta «¿qué fijas
 * tú?» decide cuál de las dos edades tiene sentido; ponerla debajo obligaría a leer la edad, luego
 * el modo, y luego volver a mirar si la edad seguía significando lo mismo. Y por eso mismo la edad
 * que el modo NO usa desaparece en vez de quedarse en gris: en coast B la fecha de jubilación
 * **la resuelve el sorteo**, y un campo «Edad de jubilación objetivo» ahí anunciaría un control
 * sobre algo que el usuario acaba de delegar.
 *
 * **Por qué el puente vive en «Pensión» y no en «Retirada»** (C7). Sus dos números son una tasa y
 * unos años, que suenan a retirada; pero el puente no existe sin una pensión a la que llegar —su
 * ventana se mide hacia atrás desde el inicio de la pensión— y ponerlo junto al SWR habría dejado
 * un ajuste huérfano visible sin pensión declarada. Los dos números son `required` cuando el
 * interruptor está encendido: el servidor los rellena con sus defaults al activarlo, pero un
 * `null` que llegue de otra vía (API, MCP, un perfil a medias) es un puente sin tope, y eso no se
 * puede simular como se pidió.
 *
 * **U4 en la tabla**: no existe `withdrawal_pct` ni `hybrid_start_pct`. El porcentaje de la
 * regla es `swr_pct`, el mismo que topa la tasa inicial en la fecha; la híbrida solo añade el
 * «baja al X %» (`hybrid_end_pct`) y las bandas su banda y su ajuste. Enseñar dos porcentajes de
 * retirada obligaba a explicar cuál mandaba, y la respuesta honesta era «depende de la pantalla».
 */
export function planFields(ctx: PlanFieldsContext): PlanFieldDescriptor[] {
  const out: PlanFieldDescriptor[] = [];
  const f = (
    id: PlanFieldId,
    card: PlanCardId,
    required: boolean,
    label: string,
  ) => out.push({ id, card, required, label });

  const isCoast = ctx.strategy === "coast";
  const isPartial = ctx.strategy === "partial";
  /** Coast modo B: el usuario fija la parada y la fecha de jubilación sale del sorteo. */
  const coastFixesStop = isCoast && ctx.coastMode === "fixed_stop_age";

  // ── Edades ───────────────────────────────────────────────────────────────────────────────
  // La fecha de nacimiento solo aparece cuando FALTA: pedirla otra vez a quien ya la tiene es
  // ruido, y su sitio natural es «Tu cuenta».
  if (!ctx.hasBirthDate) {
    f("birth_date", "ages", needsBirthDate(ctx), "Fecha de nacimiento");
  }
  // El modo primero: decide cuál de las dos edades siguientes existe.
  if (isCoast) {
    f("coast_mode", "ages", false, "Qué fijas tú");
  }
  if (ctx.strategy === "retire_at_age" || (isCoast && !coastFixesStop) || isPartial) {
    f("target_retirement_age", "ages", !isPartial, targetAgeLabel(ctx.strategy));
  }
  if (coastFixesStop) {
    f("coast_stop_age", "ages", true, "Edad en que dejo de aportar");
  }
  if (isPartial) {
    f("partial_mode", "ages", false, "Cuándo empieza la media jornada");
    if (ctx.partialMode === "at_age") {
      f("partial_start_age", "ages", true, "Edad de inicio de la media jornada");
    }
    f("partial_income", "ages", true, "Ingreso mensual en media jornada");
    f("partial_expense_basis", "ages", false, "Gasto durante la media jornada");
  }

  // ── Pensión ──────────────────────────────────────────────────────────────────────────────
  // Se ofrece SIEMPRE (la casilla es parte de la tarjeta) y es obligatoria en cuanto se declara:
  // un bloque abierto a medias entra en el bucle como una renta de 0 € sin que nadie lo diga.
  f("pension_amount", "pension", ctx.hasPension, "Pensión mensual");
  f("pension_start_age", "pension", ctx.hasPension, "Edad de inicio de la pensión");
  if (ctx.hasPension) {
    f("pension_indexed", "pension", false, "Pensión indexada a la inflación");
    if (isPartial) {
      f(
        "pension_fraction_while_partial",
        "pension",
        false,
        "Pensión cobrada durante la media jornada",
      );
    }
    f("bridge_enabled", "pension", false, "Puente hasta la pensión");
    if (ctx.bridgeEnabled) {
      f("bridge_max_pct", "pension", true, "Tasa máxima durante el puente");
      f("bridge_max_years", "pension", true, "Años máximos de puente");
    }
  }

  // ── Gasto en jubilación ──────────────────────────────────────────────────────────────────
  f("fire_number_mode", "spending", false, "Cómo se calcula el gasto de jubilación");
  if (ctx.fireNumberMode === "manual") {
    f("fire_number_manual_amount", "spending", true, "Gasto anual manual");
  }

  // ── Retirada ─────────────────────────────────────────────────────────────────────────────
  // El umbral va el PRIMERO: en v2 es la restricción que decide la fecha válida, no un corte de
  // semáforo. La tasa viene detrás porque las dos se leen juntas (ver `PLAN_CARD_COPY`).
  f("success_threshold_pct", "withdrawal", false, "Umbral de éxito");
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
 * sino el radiogroup de las cuatro estrategias, así que «vacía» ahí no significa «sin contenido».
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
 * Los dos modos se leen donde el wire los pone y **no se inventan**: `coast_mode` está en la raíz
 * del perfil (existe siempre, con su default), mientras que el modo de la fase parcial vive
 * DENTRO de `partial_retirement`, que es `null` mientras no haya fase. Sin bloque no hay modo que
 * leer, y el `at_age` de ese caso es inerte: la tabla solo mira `partialMode` con la estrategia
 * `partial`, y esa estrategia siempre trae su bloque.
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
    fireNumberMode: profile.fire_number_mode,
    coastMode: profile.coast_mode,
    partialMode: profile.partial_retirement?.mode ?? "at_age",
    bridgeEnabled: profile.pension?.bridge_enabled ?? false,
  };
}
