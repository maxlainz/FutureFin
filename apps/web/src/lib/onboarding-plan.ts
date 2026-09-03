/**
 * Modelo PURO del paso «Tu plan» del asistente de primera vez (5.0.0, rediseño UX, decisión U8,
 * issue #207).
 *
 * Antes de U8 este paso pedía inflación + tasa de retirada segura (SWR) y mandaba un PATCH a
 * `/v1/installation` más otro a `/v1/auth/me/retirement-profile`. U8 lo sustituye por lo que de
 * verdad hace falta para tener un plan: fecha de nacimiento, la estrategia (las mismas 5
 * tarjetas de Jubilación) y SOLO los campos esenciales que esa estrategia exige. La inflación y
 * el SWR se quedan en sus valores por defecto (2,5 % / 3,5 %) y ya no se tocan desde aquí — quien
 * quiera cambiarlos lo hace luego en Ajustes → Plan / Jubilación.
 *
 * Qué campo hace falta por estrategia lo decide `requiredPlanFields`/`planGroupFields`
 * (`lib/plan-fields.ts`, U2/U12) — la ÚNICA fuente de verdad de la casa sobre visibilidad y
 * obligatoriedad. `onboardingPlanFields` es un envoltorio fino sobre ella, con el contexto de
 * quien todavía no tiene NADA configurado (sin pensión, regla de retirada por defecto, modo del
 * objetivo por defecto). Duplicar esa tabla aquí sería exactamente el fallo que U12 existe para
 * impedir: este asistente y la línea de supuestos de Jubilación discreparían sobre qué pide cada
 * estrategia.
 *
 * La fecha de nacimiento vive FUERA de esa tabla a propósito: en `plan-fields.ts` el campo
 * `birth_date` solo aparece cuando `!hasBirthDate`, así que aquí se le pasa siempre
 * `hasBirthDate: true` para que la tabla nunca la incluya, y este módulo decide su propia
 * obligatoriedad con `strategyNeedsBirthDate` — la misma regla (privada, no exportada) que usa
 * `plan-fields.ts`, duplicada aquí con un comentario en vez de exportada desde allí para no tocar
 * un fichero que no es de este paquete de trabajo.
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
  PartialRetirementApi,
  PensionPlanApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
} from "../api/types";
import { utcTodayYmd } from "./dates";
import { toApiDecimalString } from "./format";
import {
  planGroupFields,
  type PlanFieldDescriptorPlan,
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
 * Los siete campos que el paso puede llegar a pintar. Los cinco «esenciales» viven SIEMPRE en el
 * estado aunque la estrategia activa no los muestre ahora mismo: cambiar de tarjeta no debe
 * perder lo que el usuario ya había escrito en otra, y `validateOnboardingPlan` los ignora salvo
 * que `onboardingPlanFields` diga que la estrategia actual los necesita.
 */
export type OnboardingPlanState = {
  /** `""` o el valor nativo de `<input type="date">` (`"YYYY-MM-DD"`). */
  birthDate: string;
  strategy: RetirementStrategyApi;
  /** Texto tal cual lo escribe el usuario (sin parsear) — mismo patrón que el resto del wizard. */
  targetRetirementAge: string;
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
    partialStartAge: "",
    partialIncome: "",
    pensionAmount: "",
    pensionStartAge: "",
  };
}

// ---------------------------------------------------------------------------
// Qué campos pide cada estrategia — envoltorio sobre `lib/plan-fields.ts`
// ---------------------------------------------------------------------------

/** Contexto de quien llega al asistente sin nada configurado todavía. Ningún eje de aquí decide
 *  la lista de campos `plan` que son `required` salvo la estrategia — `hasPension`, `ruleKind`,
 *  `effectiveBasis` y `fireNumberMode` solo mueven el grupo `advanced`, que este asistente no
 *  pregunta nunca (ver el docblock del fichero). */
const ONBOARDING_FIELDS_BASE_CONTEXT: Omit<
  PlanFieldsContext,
  "strategy" | "strategyForcesBasis"
> = {
  // La fecha de nacimiento se pide FUERA de esta tabla (ver `strategyNeedsBirthDate` más abajo):
  // con `hasBirthDate: true` la tabla nunca añade el campo `birth_date` a la lista.
  hasBirthDate: true,
  hasPension: false,
  ruleKind: "fixed_real",
  effectiveBasis: "perpetuity",
  fireNumberMode: "annual_expense",
};

/**
 * Los campos esenciales de una estrategia, en orden de lectura, con su rótulo canónico —
 * exactamente los que `requiredPlanFields` marcaría `required` para quien aún no tiene plan.
 * `asap` no devuelve ninguno; `pension_bridge` devuelve `pension_amount` + `pension_start_age`;
 * `partial` devuelve `partial_start_age` + `partial_income`; `retire_at_age`/`coast` devuelven
 * `target_retirement_age`.
 */
export function onboardingPlanFields(
  strategy: RetirementStrategyApi,
): PlanFieldDescriptorPlan[] {
  const ctx: PlanFieldsContext = {
    ...ONBOARDING_FIELDS_BASE_CONTEXT,
    strategy,
    strategyForcesBasis: strategy === "pension_bridge",
  };
  return planGroupFields(ctx).filter((f) => f.required);
}

/**
 * Espejo de la función homónima (privada, no exportada) en `lib/plan-fields.ts`: las estrategias
 * cuyo disparador es una EDAD no se pueden simular tal y como se han pedido sin fecha de
 * nacimiento (sin ella el motor las degrada a «Cuanto antes»). `asap` y `pension_bridge` se
 * disparan por cruce de capital o por la pensión, así que no la necesitan — el campo se enseña
 * igual (es más fácil rellenarla ahora que volver a «Tu cuenta» luego), pero no bloquea «Continuar».
 */
export function strategyNeedsBirthDate(s: RetirementStrategyApi): boolean {
  return s === "retire_at_age" || s === "coast" || s === "partial";
}

// ---------------------------------------------------------------------------
// Validación
// ---------------------------------------------------------------------------

export type OnboardingPlanFieldKey =
  | "birthDate"
  | "targetRetirementAge"
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

  // --- Fecha de nacimiento ------------------------------------------------------------------
  const birthDate = state.birthDate.trim();
  if (birthDate === "") {
    if (strategyNeedsBirthDate(state.strategy)) {
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

  // --- Los esenciales de la estrategia activa -----------------------------------------------
  const fields = new Set(onboardingPlanFields(state.strategy).map((f) => f.id));

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

  if (fields.has("partial_start_age")) {
    const age = parseAge(state.partialStartAge);
    if (age === null || age < MIN_PROFILE_AGE || age > MAX_HORIZON_LIFESPAN_AGE) {
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
 * ha escrito), `strategy` y, según la estrategia, `target_retirement_age` (escalar) o el bloque
 * `partial_retirement`/`pension` completo — nunca los dos bloques a la vez, nunca
 * `withdrawal_rule` (la regla de retirada se queda en su default `fixed_real`, igual que el SWR y
 * la inflación se quedan en el suyo: este asistente no los pregunta).
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

  const fields = new Set(onboardingPlanFields(state.strategy).map((f) => f.id));

  if (fields.has("target_retirement_age")) {
    const age = parseAge(state.targetRetirementAge);
    if (age !== null) patch.target_retirement_age = age;
  }

  if (fields.has("partial_start_age") || fields.has("partial_income")) {
    const age = parseAge(state.partialStartAge) ?? MIN_PROFILE_AGE;
    const income =
      state.partialIncome.trim() === "" ? "0" : toApiDecimalString(state.partialIncome);
    const partial: PartialRetirementApi = {
      starts_at_age: age,
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
    };
    patch.pension = pension;
  }

  return patch;
}
