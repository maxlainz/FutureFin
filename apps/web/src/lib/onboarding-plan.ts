/**
 * Modelo PURO del paso «Tu plan» del asistente de primera vez (5.0.0, rediseño UX, decisión U8,
 * issue #207).
 *
 * Antes de U8 este paso pedía inflación + tasa de retirada segura (SWR) y mandaba un PATCH a
 * `/v1/installation` más otro a `/v1/auth/me/retirement-profile`. U8 lo sustituye por lo que de
 * verdad hace falta para tener un plan: fecha de nacimiento, la estrategia (las mismas 4
 * tarjetas de Jubilación desde C7, que retiró «Puente hasta la pensión» del selector) y SOLO los
 * campos esenciales que esa estrategia **y su modo** exigen. La inflación, el SWR y el umbral de
 * éxito se quedan en sus valores por defecto (2,5 % / 3,5 % / 95 %) y ya no se tocan desde aquí —
 * quien quiera cambiarlos lo hace luego en Ajustes → Plan / Jubilación.
 *
 * Qué campo hace falta por estrategia lo decide `requiredPlanFields`/`planFields`
 * (`lib/plan-fields.ts`, U2/U12) — la ÚNICA fuente de verdad de la casa sobre visibilidad y
 * obligatoriedad. `onboardingPlanFields` es un envoltorio fino sobre ella, con el contexto de
 * quien todavía no tiene NADA configurado (sin pensión, regla de retirada por defecto, modo del
 * objetivo por defecto). Duplicar esa tabla aquí sería exactamente el fallo que U12 existe para
 * impedir: este asistente y el formulario de Jubilación discreparían sobre qué pide cada
 * estrategia.
 *
 * La fecha de nacimiento vive FUERA de esa tabla a propósito: en `plan-fields.ts` el campo
 * `birth_date` solo aparece cuando `!hasBirthDate`, así que aquí se le pasa siempre
 * `hasBirthDate: true` para que la tabla nunca la incluya, y este módulo decide su propia
 * obligatoriedad con `strategyNeedsBirthDate` — la misma regla (privada, no exportada) que usa
 * `plan-fields.ts`, duplicada aquí con un comentario en vez de exportada desde allí para no tocar
 * un fichero que no es de este paquete de trabajo. **Los dos tienen que decir lo mismo**, y desde
 * C5 dicen más que antes: la fecha de nacimiento es obligatoria también con una pensión declarada,
 * no solo con las estrategias por edad.
 *
 * Validación y construcción del PATCH SÍ son propias de este módulo: `retirementProfileIssue`
 * (`lib/retirementProfile.ts`) valida el perfil ENTERO ya resuelto, con sus defaults aplicados y
 * sus tipos ya sólidos (edades como `number`, importes como decimal-string ya normalizado); este
 * asistente valida un FORMULARIO a medio rellenar, con campos de texto vacíos que en el perfil
 * resuelto nunca existen. Los códigos y el texto de los mensajes se mantienen alineados A MANO
 * con `validate_retirement_profile` (`apps/api/src/handlers/retirement_profile.rs`) y con
 * `lib/errorMessages.ts` — copiados aquí, no importados, para no acoplar este módulo a un
 * catálogo compartido que cambia por razones ajenas a este paso.
 *
 * Una desviación deliberada respecto al perfil completo: `retirementProfileIssue` acepta un
 * ingreso de media jornada en cero («año sabático declarado», ver su comentario). Este asistente
 * exige un importe estrictamente positivo para los DOS importes que pregunta (ingreso de media
 * jornada y pensión) — es un formulario de alta mínimo, no el editor completo, y un cero ahí no
 * se distingue de un campo que se ha dejado a medias. Quien de verdad quiera declarar un año
 * sabático a coste cero lo hace después desde Jubilación, donde ese matiz sí tiene sitio.
 */

import type {
  CoastModeApi,
  PartialRetirementApi,
  PensionPlanApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
} from "../api/types";
import { utcTodayYmd } from "./dates";
import { toApiDecimalString } from "./format";
import {
  planFields,
  type PartialModeApi,
  type PlanFieldDescriptor,
  type PlanFieldsContext,
} from "./plan-fields";
import {
  MAX_HORIZON_LIFESPAN_AGE,
  MIN_PENSION_AGE,
  MIN_PROFILE_AGE,
} from "./retirementProfile";

// ---------------------------------------------------------------------------
// Estado del formulario
// ---------------------------------------------------------------------------

/**
 * Todo lo que el paso puede llegar a pintar. Los esenciales viven SIEMPRE en el estado aunque la
 * estrategia activa no los muestre ahora mismo: cambiar de tarjeta no debe perder lo que el
 * usuario ya había escrito en otra, y `validateOnboardingPlan` los ignora salvo que
 * `onboardingPlanFields` diga que la estrategia actual los necesita.
 *
 * **Los dos MODOS del modelo v2 también son estado** (M10/M11), y no un detalle de la vista: en
 * `coast` y en `partial` deciden QUÉ campo es obligatorio —la edad de jubilación o la de dejar de
 * aportar; la edad de inicio de la fase o ninguna—, así que sin ellos aquí este módulo no podría
 * contestar «¿puedo dejarle avanzar?». Los dos arrancan en su modo A, que es el default del
 * servidor.
 */
export type OnboardingPlanState = {
  /** `""` o el valor nativo de `<input type="date">` (`"YYYY-MM-DD"`). */
  birthDate: string;
  strategy: RetirementStrategyApi;
  /** Texto tal cual lo escribe el usuario (sin parsear) — mismo patrón que el resto del wizard. */
  targetRetirementAge: string;
  /** M10: `fixed_retirement_age` (A, default) fija la edad de jubilación y el servidor resuelve
   *  cuándo se puede dejar de aportar; `fixed_stop_age` (B) fija la parada. Solo se mira con
   *  `coast`. */
  coastMode: CoastModeApi;
  /** Solo se pregunta en `coast` modo B. */
  coastStopAge: string;
  /** M11: `at_age` (A, default) pregunta la edad de inicio de la fase; `asap` (B) la resuelve el
   *  servidor. Solo se mira con `partial`. */
  partialMode: PartialModeApi;
  partialStartAge: string;
  partialIncome: string;
  pensionAmount: string;
  pensionStartAge: string;
};

export function emptyOnboardingPlanState(): OnboardingPlanState {
  return {
    birthDate: "",
    strategy: "asap",
    targetRetirementAge: "",
    coastMode: "fixed_retirement_age",
    coastStopAge: "",
    partialMode: "at_age",
    partialStartAge: "",
    partialIncome: "",
    pensionAmount: "",
    pensionStartAge: "",
  };
}

// ---------------------------------------------------------------------------
// Qué campos pide cada estrategia — envoltorio sobre `lib/plan-fields.ts`
// ---------------------------------------------------------------------------

/** Contexto de quien llega al asistente sin nada configurado todavía: sin pensión, con la regla
 *  de retirada por defecto, el modo del gasto por defecto y el puente apagado. Ninguno de esos
 *  ejes hace obligatorio ningún campo, así que ninguno cambia lo que este paso pregunta; están
 *  porque la tabla es total y quien la llame tiene que decidirlos. Lo que SÍ lo cambia va aparte,
 *  en `OnboardingPlanContext`. */
const ONBOARDING_FIELDS_BASE_CONTEXT: Omit<
  PlanFieldsContext,
  "strategy" | "coastMode" | "partialMode" | "hasPension"
> = {
  // La fecha de nacimiento se pide FUERA de esta tabla (ver `strategyNeedsBirthDate` más abajo):
  // con `hasBirthDate: true` la tabla nunca añade el campo `birth_date` a la lista.
  hasBirthDate: true,
  ruleKind: "fixed_real",
  fireNumberMode: "annual_expense",
  bridgeEnabled: false,
};

/**
 * Lo que, además de la estrategia, decide qué campos son obligatorios en el modelo v2. Todo
 * opcional y con el default del servidor: `onboardingPlanFields("coast")` a secas sigue
 * significando «coast en su modo A», que es con lo que arranca el formulario.
 */
export type OnboardingPlanContext = {
  coastMode?: CoastModeApi;
  partialMode?: PartialModeApi;
  /** Hay pensión declarada en el borrador. Con el modelo v2 la pensión ya no es una estrategia
   *  (C7): es un bloque que, en cuanto se abre, exige su importe, su edad y —C5— la fecha de
   *  nacimiento. */
  hasPension?: boolean;
};

/**
 * Los campos esenciales de una estrategia **y su modo**, en orden de lectura, con su rótulo
 * canónico — exactamente los que `requiredPlanFields` marcaría `required` para quien aún no tiene
 * plan. `asap` no devuelve ninguno; `retire_at_age` devuelve `target_retirement_age`; `coast`
 * devuelve `target_retirement_age` en modo A y `coast_stop_age` en modo B; `partial` devuelve
 * `partial_start_age` + `partial_income` en modo A y **solo el ingreso** en modo B (la edad de
 * inicio la resuelve el servidor); con pensión declarada se añaden su importe y su edad.
 *
 * `ctx` es opcional para que la vista pueda seguir preguntando por la estrategia sola mientras no
 * ofrezca los modos; los defaults son los del servidor (modo A en las dos, sin pensión).
 */
export function onboardingPlanFields(
  strategy: RetirementStrategyApi,
  ctx: OnboardingPlanContext = {},
): PlanFieldDescriptor[] {
  const full: PlanFieldsContext = {
    ...ONBOARDING_FIELDS_BASE_CONTEXT,
    strategy,
    coastMode: ctx.coastMode ?? "fixed_retirement_age",
    partialMode: ctx.partialMode ?? "at_age",
    hasPension: ctx.hasPension ?? false,
  };
  return planFields(full).filter((f) => f.required);
}

/** `true` ⟺ el borrador ha abierto el bloque de pensión: basta con haber escrito UNO de sus dos
 *  campos. Medio bloque es un bloque — y lo que dispara C5 es haber declarado la pensión, no
 *  haberla terminado de rellenar. */
function draftHasPension(state: OnboardingPlanState): boolean {
  return state.pensionAmount.trim() !== "" || state.pensionStartAge.trim() !== "";
}

/** El contexto de `plan-fields` que corresponde a un borrador concreto. */
function contextOf(state: OnboardingPlanState): OnboardingPlanContext {
  return {
    coastMode: state.coastMode,
    partialMode: state.partialMode,
    hasPension: draftHasPension(state),
  };
}

/**
 * Espejo de `needsBirthDate` (privada, no exportada) en `lib/plan-fields.ts`, con sus DOS motivos:
 *
 *  - las estrategias cuyo disparador es una EDAD (`retire_at_age`, `coast`, `partial`) no se
 *    pueden simular como se han pedido sin ella;
 *  - **y, desde el modelo v2, tampoco un plan con pensión declarada** (C5): la pensión entra en el
 *    bucle a una edad, y sin fecha de nacimiento el servidor no publica ni fecha, ni éxito, ni
 *    capital necesario (`plan_absent_reason: "birth_date_missing"`). Por eso el segundo argumento
 *    existe: «Cuanto antes» sin pensión sigue sin necesitarla, «Cuanto antes» CON pensión sí.
 *
 * `asap` sin pensión no la necesita — el campo se enseña igual (es más fácil rellenarla ahora que
 * volver a «Tu cuenta» luego), pero no bloquea «Continuar».
 */
export function strategyNeedsBirthDate(
  s: RetirementStrategyApi,
  hasPension = false,
): boolean {
  if (hasPension) return true;
  return s === "retire_at_age" || s === "coast" || s === "partial";
}

// ---------------------------------------------------------------------------
// Validación
// ---------------------------------------------------------------------------

export type OnboardingPlanFieldKey =
  | "birthDate"
  | "targetRetirementAge"
  | "coastStopAge"
  | "partialStartAge"
  | "partialIncome"
  | "pensionAmount"
  | "pensionStartAge";

export type OnboardingPlanIssue = {
  field: OnboardingPlanFieldKey;
  /** Código estable, alineado a mano con `validate_retirement_profile` donde el campo existe
   *  allí; los que no tienen equivalente en el perfil resuelto (p. ej. `birth_date_required`,
   *  propio de este formulario a medio rellenar) llevan un código propio. */
  code: string;
  message: string;
};

/** Solo dígitos, sin signo: una edad no puede ser negativa ni llevar decimales. */
function parseAge(raw: string): number | null {
  const t = raw.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return Number.isSafeInteger(n) ? n : null;
}

/**
 * Un importe tecleado (es-ES: coma decimal, punto de millar), parseado con la MISMA función que
 * `buildOnboardingPlanPatch` usa para construir el PATCH (`toApiDecimalString`) — así la validación
 * nunca acepta algo que el PATCH luego no sabría normalizar, ni al revés. `null` si no parsea
 * (`parseDisplayDecimal` por sí solo no basta aquí: no entiende `1.234,56`, y un importe así es
 * exactamente lo que este campo espera que se teclee).
 */
function parsedAmount(raw: string): number | null {
  try {
    const s = toApiDecimalString(raw);
    if (s === "") return null;
    const n = Number(s);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

const AGE_RANGE_TEXT = `entre los ${MIN_PROFILE_AGE} y los ${MAX_HORIZON_LIFESPAN_AGE} años`;
const PENSION_AGE_RANGE_TEXT = `entre los ${MIN_PENSION_AGE} y los ${MAX_HORIZON_LIFESPAN_AGE} años`;

/**
 * Fecha de nacimiento válida ⟺ formato `YYYY-MM-DD`, fecha de calendario real (rechaza
 * `"2023-02-30"`), año ≥ 1900 y no futura — mismas tres cotas que `validate_birth_date`
 * (`apps/api/src/handlers/auth.rs`), en el mismo orden.
 */
function birthDateIssue(raw: string): OnboardingPlanIssue | null {
  const t = raw.trim();
  const FORMAT_ISSUE: OnboardingPlanIssue = {
    field: "birthDate",
    code: "birth_date_format",
    message: "La fecha de nacimiento debe tener el formato AAAA-MM-DD.",
  };
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t);
  if (!m) return FORMAT_ISSUE;
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  const asDate = new Date(Date.UTC(y, mo - 1, d));
  const isRealCalendarDate =
    asDate.getUTCFullYear() === y &&
    asDate.getUTCMonth() === mo - 1 &&
    asDate.getUTCDate() === d;
  if (!isRealCalendarDate) return FORMAT_ISSUE;
  if (y < 1900) {
    return {
      field: "birthDate",
      code: "birth_date_too_old",
      message: "La fecha de nacimiento debe ser posterior a 1900.",
    };
  }
  if (t > utcTodayYmd()) {
    return {
      field: "birthDate",
      code: "birth_date_future",
      message: "La fecha de nacimiento no puede ser futura.",
    };
  }
  return null;
}

/**
 * Lista de problemas del borrador, en el mismo orden en que se pintan los campos: fecha de
 * nacimiento, estrategia (nunca falla: siempre hay una tarjeta marcada), y los esenciales que
 * `onboardingPlanFields(state.strategy)` diga que hacen falta. Lista vacía ⟺ `Continuar` se
 * puede pulsar y `buildOnboardingPlanPatch` produce un cuerpo que el servidor va a aceptar.
 */
export function validateOnboardingPlan(
  state: OnboardingPlanState,
): OnboardingPlanIssue[] {
  const issues: OnboardingPlanIssue[] = [];
  const hasPension = draftHasPension(state);

  // --- Fecha de nacimiento ------------------------------------------------------------------
  const birthDate = state.birthDate.trim();
  if (birthDate === "") {
    if (strategyNeedsBirthDate(state.strategy, hasPension)) {
      issues.push({
        field: "birthDate",
        code: "birth_date_required",
        message: "Esta estrategia necesita tu fecha de nacimiento para poder simularse.",
      });
    }
  } else {
    const issue = birthDateIssue(birthDate);
    if (issue) issues.push(issue);
  }

  // --- Los esenciales de la estrategia activa Y SU MODO --------------------------------------
  const fields = new Set(
    onboardingPlanFields(state.strategy, contextOf(state)).map((f) => f.id),
  );

  if (fields.has("target_retirement_age")) {
    const age = parseAge(state.targetRetirementAge);
    if (age === null) {
      issues.push({
        field: "targetRetirementAge",
        code: "target_retirement_age_required",
        message: "Esa estrategia necesita que digas a qué edad quieres jubilarte.",
      });
    } else if (age < MIN_PROFILE_AGE || age > MAX_HORIZON_LIFESPAN_AGE) {
      issues.push({
        field: "targetRetirementAge",
        code: "retirement_age_out_of_range",
        message: `La edad de jubilación tiene que estar ${AGE_RANGE_TEXT}.`,
      });
    }
  }

  if (fields.has("coast_stop_age")) {
    const age = parseAge(state.coastStopAge);
    if (age === null) {
      issues.push({
        field: "coastStopAge",
        code: "coast_stop_age_required",
        message: "Ese modo necesita la edad a la que dejas de aportar.",
      });
    } else if (age < MIN_PROFILE_AGE || age > MAX_HORIZON_LIFESPAN_AGE) {
      issues.push({
        field: "coastStopAge",
        code: "coast_stop_age_out_of_range",
        message: `La edad a la que dejas de aportar tiene que estar ${AGE_RANGE_TEXT}.`,
      });
    }
  }

  if (fields.has("partial_start_age")) {
    const age = parseAge(state.partialStartAge);
    if (age === null) {
      // Vacía o a medio teclear: código propio, el mismo que el servidor (`partial_start_age_
      // required`). Antes se colaba por «fuera de rango», que decía la verdad equivocada.
      issues.push({
        field: "partialStartAge",
        code: "partial_start_age_required",
        message: "Ese modo necesita la edad a la que empiezas la media jornada.",
      });
    } else if (age < MIN_PROFILE_AGE || age > MAX_HORIZON_LIFESPAN_AGE) {
      issues.push({
        field: "partialStartAge",
        code: "partial_age_out_of_range",
        message: `La edad de inicio de la media jornada tiene que estar ${AGE_RANGE_TEXT}.`,
      });
    } else {
      // Coherente con la edad de jubilación total SI las dos están presentes — en la práctica
      // solo ocurre si el usuario había escrito una edad total con otra estrategia y luego
      // cambió a «Media jornada» sin borrarla; el estado conserva ambas (ver docblock del tipo).
      const totalAge = parseAge(state.targetRetirementAge);
      if (totalAge !== null && age >= totalAge) {
        issues.push({
          field: "partialStartAge",
          code: "partial_not_before_retirement",
          message: "La media jornada tiene que empezar antes de la jubilación total.",
        });
      }
    }
  }

  if (fields.has("partial_income")) {
    const t = state.partialIncome.trim();
    if (t === "") {
      issues.push({
        field: "partialIncome",
        code: "partial_income_not_positive",
        message: "El ingreso mensual en media jornada debe ser mayor que cero.",
      });
    } else {
      const n = parsedAmount(t);
      if (n === null) {
        issues.push({
          field: "partialIncome",
          code: "decimal_invalid",
          message:
            "Esa cantidad no se entiende como número. Escríbela solo con cifras y, si hace falta, un decimal.",
        });
      } else if (n <= 0) {
        issues.push({
          field: "partialIncome",
          code: "partial_income_not_positive",
          message: "El ingreso mensual en media jornada debe ser mayor que cero.",
        });
      }
    }
  }

  if (fields.has("pension_amount")) {
    const t = state.pensionAmount.trim();
    if (t === "") {
      issues.push({
        field: "pensionAmount",
        code: "pension_amount_not_positive",
        message: "El importe de la pensión debe ser mayor que cero.",
      });
    } else {
      const n = parsedAmount(t);
      if (n === null) {
        issues.push({
          field: "pensionAmount",
          code: "decimal_invalid",
          message:
            "Esa cantidad no se entiende como número. Escríbela solo con cifras y, si hace falta, un decimal.",
        });
      } else if (n <= 0) {
        issues.push({
          field: "pensionAmount",
          code: "pension_amount_not_positive",
          message: "El importe de la pensión debe ser mayor que cero.",
        });
      }
    }
  }

  if (fields.has("pension_start_age")) {
    const age = parseAge(state.pensionStartAge);
    if (age === null || age < MIN_PENSION_AGE || age > MAX_HORIZON_LIFESPAN_AGE) {
      issues.push({
        field: "pensionStartAge",
        code: "pension_age_out_of_range",
        message: `La edad a la que empieza la pensión tiene que estar ${PENSION_AGE_RANGE_TEXT}.`,
      });
    }
  }

  return issues;
}

// ---------------------------------------------------------------------------
// PATCH exacto
// ---------------------------------------------------------------------------

/**
 * El cuerpo EXACTO de `PATCH /v1/auth/me/retirement-profile` para este paso: `birth_date` (si se
 * ha escrito), `strategy` y, según la estrategia Y SU MODO, `coast_mode` + una de las dos edades,
 * o el bloque `partial_retirement`/`pension` completo — nunca los dos bloques a la vez, nunca
 * `withdrawal_rule` (la regla de retirada se queda en su default `fixed_real`, igual que el SWR,
 * el umbral de éxito y la inflación se quedan en el suyo: este asistente no los pregunta).
 *
 * **`coast_mode` viaja siempre que la estrategia sea `coast`**, incluso en su modo A, que es el
 * default del servidor: mandar el default explícito cuesta un campo y evita el fallo que este
 * paso no puede permitirse — que alguien vuelva al asistente con un perfil ya en modo B y elija
 * «fijo la edad de jubilación» sin que el PATCH lo diga. En modo A **no** se manda
 * `coast_stop_age`: el PATCH es tri-estado y una clave ausente no cambia nada, así que una parada
 * guardada antes se conserva sin aplicarse (mismo criterio que el servidor, que la conserva
 * aunque no toque).
 *
 * Asume un estado que ya ha pasado `validateOnboardingPlan` con lista vacía — igual que el resto
 * de formularios de la casa (p. ej. `buildAssetWriteBody`), no vuelve a validar. Sobre un estado
 * inválido puede construir un PATCH que el servidor rechace; por eso «Continuar» se deshabilita
 * mientras `validateOnboardingPlan(state).length > 0`.
 */
export function buildOnboardingPlanPatch(
  state: OnboardingPlanState,
): RetirementProfilePatchApi {
  const patch: RetirementProfilePatchApi = { strategy: state.strategy };

  const birthDate = state.birthDate.trim();
  if (birthDate !== "") patch.birth_date = birthDate;

  const fields = new Set(
    onboardingPlanFields(state.strategy, contextOf(state)).map((f) => f.id),
  );

  if (state.strategy === "coast") patch.coast_mode = state.coastMode;

  if (fields.has("target_retirement_age")) {
    const age = parseAge(state.targetRetirementAge);
    if (age !== null) patch.target_retirement_age = age;
  }

  if (fields.has("coast_stop_age")) {
    const age = parseAge(state.coastStopAge);
    if (age !== null) patch.coast_stop_age = age;
  }

  if (fields.has("partial_start_age") || fields.has("partial_income")) {
    // Modo B («en cuanto pueda»): la edad de inicio viaja `null` a propósito — la resuelve el
    // servidor (`earliest_partial_start`) y la publica en `partial_start_month_index`. Mandar
    // aquí un número inventado fijaría la fase a una edad que nadie eligió.
    const startsAtAge =
      state.partialMode === "at_age"
        ? (parseAge(state.partialStartAge) ?? MIN_PROFILE_AGE)
        : null;
    const income =
      state.partialIncome.trim() === "" ? "0" : toApiDecimalString(state.partialIncome);
    const partial: PartialRetirementApi = {
      mode: state.partialMode,
      starts_at_age: startsAtAge,
      income_monthly_today: income,
      expense_basis: "retirement",
    };
    patch.partial_retirement = partial;
  }

  if (fields.has("pension_amount") || fields.has("pension_start_age")) {
    const age = parseAge(state.pensionStartAge) ?? MIN_PENSION_AGE;
    const amount =
      state.pensionAmount.trim() === "" ? "0" : toApiDecimalString(state.pensionAmount);
    const pension: PensionPlanApi = {
      monthly_amount_today: amount,
      starts_at_age: age,
      indexed: true,
      fraction_while_partial: "0",
      // El puente se activa en Jubilación, nunca aquí: es un ajuste fino sobre una pensión ya
      // declarada (C7), y este paso solo recoge lo mínimo para tener un plan.
      bridge_enabled: false,
      bridge_max_pct: null,
      bridge_max_years: null,
    };
    patch.pension = pension;
  }

  return patch;
}
