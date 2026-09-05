/**
 * Las tres piezas de LÓGICA del formulario de Jubilación que no son JSX (5.0.0, rediseño UX
 * U1b; decisiones S3, S6 y la guarda de autosave de U2 en #207).
 *
 * Viven aquí y no dentro de `RetirementView.tsx` porque las tres se rompen en silencio: una
 * conversión porcentaje↔fracción equivocada multiplica o divide por 100 lo que el usuario cree
 * estar declarando, un indicador de guardado que miente convierte un error en «guardado», y una
 * guarda de obligatorios mal calculada deja el autosave apagado para siempre sin decir por qué.
 * Ninguna de las tres se ve mirando la pantalla, y las tres se prueban en una línea.
 *
 * Lo que este módulo NO hace: decidir qué campos se ven (eso es `lib/plan-fields.ts`, la tabla
 * U2), validar el perfil (eso es `retirementProfileIssue`, espejo del servidor) ni saber nada
 * de React.
 */

import type { RetirementProfileApi, WithdrawalRuleApi } from "../api/types";
import type { PlanCardId, PlanFieldId } from "./plan-fields";
import type { HelpTextId } from "./helpTexts";
import { DISPLAY_NUMBER_LOCALE, formatPercentAmount, parseDisplayDecimal } from "./format";
import { effectiveWithdrawalPct, type PctSourceApi } from "./retirementProfile";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// S3 — la fracción de pensión durante la media jornada se EDITA en porcentaje
// ─────────────────────────────────────────────────────────────────────────────────────────────

/**
 * `"0.4"` → `"40"`. La API guarda una FRACCIÓN (0 a 1) y la pantalla pide un PORCENTAJE (0 a
 * 100): el campo anterior rotulaba «Parte que cobras en media jornada (0 a 1)» y quien escribía
 * `40` declaraba cobrar 40 veces su pensión, que el servidor recortaba a 1 sin decir nada.
 *
 * Un valor ilegible o ausente devuelve `""` — el campo vacío es un estado legítimo (equivale a
 * cero) y rellenarlo con un `0` inventado sería escribir por el usuario.
 */
export function percentFromFraction(raw: string | null | undefined): string {
  const n = parseDisplayDecimal(String(raw ?? ""));
  if (n == null || !Number.isFinite(n)) return "";
  // El redondeo a 4 decimales mata el `0.30000000000000004` de la coma flotante sin tocar
  // ninguna fracción que un humano pueda teclear.
  const pct = Math.round(n * 100 * 10000) / 10000;
  return String(pct);
}

/**
 * `"40"` → `"0.4"`, el camino de vuelta para el wire. Vacío devuelve `"0"`: la API solo acepta
 * decimales y el cero es lo que «no cobro nada durante la fase» significa.
 *
 * **No acota.** Un 140 % lo rechaza la guarda (`pension_fraction_out_of_range`) con el mismo
 * código que el servidor; recortarlo aquí escondería el error justo cuando hay que enseñarlo.
 */
export function fractionFromPercent(raw: string | null | undefined): string {
  const t = String(raw ?? "").trim();
  if (t === "") return "0";
  const n = parseDisplayDecimal(t);
  if (n == null || !Number.isFinite(n)) return t.replace(",", ".");
  const fraction = Math.round((n / 100) * 1000000) / 1000000;
  return String(fraction);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// S6 — UN solo indicador de guardado, en la cabecera de la página
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type SaveIndicatorTone = "muted" | "danger";

export type SaveIndicator = { text: string; tone: SaveIndicatorTone };

export type SaveIndicatorInput = {
  /** Hay un PATCH en vuelo. */
  saving: boolean;
  /** Instante del último guardado con éxito (epoch ms). `null` = todavía ninguno. */
  savedAtMs: number | null;
  /** «Ahora» — lo inyecta la vista con su propio reloj, para que el test no dependa del real. */
  nowMs: number;
  /** El último intento falló (banner aparte; aquí solo el estado). */
  error?: boolean;
  /**
   * El autosave está PARADO por un obligatorio sin rellenar (U2). No es un error: es que
   * todavía no hay nada que guardar de verdad, y decirlo aquí evita que el usuario se quede
   * esperando un «Guardado» que no va a llegar.
   */
  blocked?: boolean;
};

function int(n: number): string {
  return new Intl.NumberFormat(DISPLAY_NUMBER_LOCALE, {
    maximumFractionDigits: 0,
  }).format(n);
}

/**
 * El rótulo del ÚNICO indicador de estado de la página (S6), que sustituye a los seis
 * «Guardado automático.» al pie de cada panel.
 *
 * La precedencia no es negociable y es lo que el test fija: **error > guardando > bloqueado >
 * guardado**. Con seis pies repartidos, un panel podía decir «Guardado automático.» mientras
 * otro tenía un 400 en la mano; con uno solo, lo que manda es el peor estado vivo.
 *
 * Los plazos se cuentan en segundos hasta el minuto, en minutos hasta la hora y en horas
 * después. Por debajo de cinco segundos **no se enseña un plazo**: «hace 0 s» parpadea en cada
 * render y no añade nada sobre «Guardado».
 */
export function saveIndicatorLabel(input: SaveIndicatorInput): SaveIndicator {
  if (input.error) return { text: "No se pudo guardar", tone: "danger" };
  if (input.saving) return { text: "Guardando…", tone: "muted" };
  if (input.blocked) return { text: "Sin guardar · falta un dato", tone: "danger" };
  if (input.savedAtMs == null) return { text: "Guardado automático", tone: "muted" };

  const deltaMs = Math.max(0, input.nowMs - input.savedAtMs);
  const seconds = Math.floor(deltaMs / 1000);
  if (seconds < 5) return { text: "Guardado", tone: "muted" };
  if (seconds < 60) return { text: `Guardado · hace ${int(seconds)} s`, tone: "muted" };
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return { text: `Guardado · hace ${int(minutes)} min`, tone: "muted" };
  const hours = Math.floor(minutes / 60);
  return { text: `Guardado · hace ${int(hours)} h`, tone: "muted" };
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// U2 — qué obligatorio falta, y por tanto por qué el autosave no ha salido
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type MissingPlanFieldsInput = {
  profile: RetirementProfileApi;
  /** Los ids que `requiredPlanFields(strategy, ctx)` haya devuelto para este perfil. */
  required: readonly PlanFieldId[];
  /** La fecha de nacimiento del usuario (`""`/`null` = no la tiene). */
  birthDate: string | null;
};

/**
 * De los obligatorios de la estrategia, cuáles siguen **vacíos** — en el orden en que llegan,
 * que es el orden del formulario.
 *
 * Es lo que apaga el autosave (U2: «una estrategia que no tiene su dato NO se guarda hasta que
 * lo tiene») y lo que pone el «obligatorio» junto al campo. Se calcula aparte de
 * `retirementProfileIssue` porque contestan preguntas distintas: aquella dice si el servidor
 * ACEPTARÍA el PATCH; esta, si la estrategia está completa. Un plan «A una edad fija» sin fecha
 * de nacimiento se guarda perfectamente y el motor lo degrada a «Cuanto antes» — precisamente el
 * caso que hay que impedir, y que ninguna validación del servidor puede cazar.
 *
 * **Vacío no es cero**, y hay dos campos donde eso importa y por eso NO se declaran vacíos
 * nunca:
 *
 * - `partial_income`: un ingreso en blanco es un año sabático declarado (0 €/mes), que es
 *   exactamente para lo que existe la fase; bloquear ahí dejaría el plan sin guardar por un
 *   dato que el usuario ya ha contestado.
 * - `partial_start_age` / `pension_start_age`: son enteros del bloque y el bloque no existe a
 *   medias — si el bloque está, la edad está.
 */
export function missingRequiredPlanFields(
  input: MissingPlanFieldsInput,
): PlanFieldId[] {
  const { profile, birthDate } = input;
  const filled = (id: PlanFieldId): boolean => {
    switch (id) {
      case "birth_date":
        return String(birthDate ?? "").trim() !== "";
      case "target_retirement_age":
        return profile.target_retirement_age != null;
      case "partial_start_age":
        return profile.partial_retirement != null;
      case "partial_income":
        return profile.partial_retirement != null;
      case "pension_amount":
        return (
          profile.pension != null &&
          String(profile.pension.monthly_amount_today ?? "").trim() !== ""
        );
      case "pension_start_age":
        return profile.pension != null;
      case "fire_number_manual_amount":
        return String(profile.fire_number_manual_amount ?? "").trim() !== "";
      default:
        // Todo lo demás tiene default resuelto por el servidor: no puede faltar.
        return true;
    }
  };
  return input.required.filter((id) => !filled(id));
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// U4 — qué porcentaje retira de verdad la regla, y de dónde salió
// ─────────────────────────────────────────────────────────────────────────────────────────────

export type WithdrawalPctNoteInput = {
  rule: WithdrawalRuleApi;
  /** El SWR del perfil: el porcentaje que la regla hereda cuando no tiene uno propio. */
  swrPct: string;
  /** `pct_source` del servidor. `null` = backend anterior a U4, o literal desconocido. */
  pctSource: PctSourceApi | null;
};

/**
 * La línea bajo el selector de regla: **el porcentaje que va a retirar y su procedencia**.
 *
 * Existe porque la pantalla tiene UN solo porcentaje editable (el SWR) y la regla puede estar
 * retirando otro: cuando alguien fija `withdrawal_rule.pct` por API o por MCP, mover el slider
 * deja de mover la regla. Callarlo dejaba al usuario ajustando un control que su plan ignora.
 *
 * La decisión se toma por el **valor**, no por `pct_source`: un porcentaje presente es uno que
 * alguien escribió (`normalizeWithdrawalRule` suelta los heredados en la lectura), y un backend
 * anterior a U4 no publica procedencia ninguna — dar por heredado lo que no lo es haría que la
 * nota dijera «tu tasa de retirada» sobre un 4 % con el SWR al 3. `pctSource` solo se mira para
 * el caso contrario: un servidor que SÍ dice «esto lo heredé» manda sobre la heurística.
 *
 * `null` en `fixed_real`: esa regla no retira un porcentaje, retira la necesidad declarada.
 */
export function withdrawalPctNote(input: WithdrawalPctNoteInput): string | null {
  const { rule, swrPct, pctSource } = input;
  if (rule.kind === "fixed_real") return null;
  const written = rule.kind === "hybrid" ? rule.start_pct : rule.pct;
  const inherited = written == null || pctSource === "swr";
  const effective = effectiveWithdrawalPct(rule, swrPct);
  if (effective == null) return null;
  return inherited
    ? `Retira el ${formatPercentAmount(effective)}: tu tasa de retirada.`
    : `Regla al ${formatPercentAmount(effective)}, fijado por API.`;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// El cableado campo → ayuda, como DATO
// ─────────────────────────────────────────────────────────────────────────────────────────────

/**
 * Qué entrada del catálogo de descripciones acompaña a cada campo del plan.
 *
 * Es una tabla y no una prop de ayuda repartida por el JSX para que el cableado se pueda leer —y
 * probar— de una vez: con veinte campos renderizados por id, un ternario perdido deja un
 * campo sin ayuda y nadie lo nota. Los campos que no aparecen aquí **no llevan icono a
 * propósito**: su rótulo se explica solo, y un interrogante que lo repite es ruido.
 *
 * **Por qué el valor es un objeto `{ helpId }` y no el id pelado**: el escáner de
 * `helpTexts.test.ts` reconoce tres formas —la prop JSX, la clave de objeto y el acceso directo
 * al catálogo— y ninguna casa con `target_retirement_age: "retirement.target_age"`, donde la
 * clave es el campo y no la ayuda. (Este párrafo no escribe ninguno de los tres patrones a
 * propósito: un comentario que contiene el patrón que explica se cuenta a sí mismo, y el
 * escáner registró una vez un id llamado «…» sacado justo de aquí.) Con el id suelto,
 * la mitad «ningún texto huérfano» del test habría empujado a **borrar diez textos vivos** — el
 * fallo exacto que ese test existe para impedir, del revés, y el mismo que ya obligó a añadir la
 * forma de objeto cuando los KPIs por estrategia se movieron a `lib/retirement-tiles.ts`.
 */
export const PLAN_FIELD_HELP: Partial<Record<PlanFieldId, { helpId: HelpTextId }>> = {
  target_retirement_age: { helpId: "retirement.target_age" },
  partial_start_age: { helpId: "retirement.partial" },
  partial_income: { helpId: "retirement.partial" },
  pension_amount: { helpId: "retirement.pension" },
  pension_start_age: { helpId: "retirement.pension" },
  swr_pct: { helpId: "settings.swr" },
  withdrawal_rule_kind: { helpId: "retirement.withdrawal_rule" },
  hybrid_end_pct: { helpId: "retirement.withdrawal_rule" },
  guardrails_band_pct: { helpId: "retirement.withdrawal_rule" },
  guardrails_adjust_pct: { helpId: "retirement.withdrawal_rule" },
  spend_mode: { helpId: "retirement.spend_mode" },
  target_basis: { helpId: "retirement.target_basis" },
  bridge_discount_basis: { helpId: "retirement.bridge_discount" },
  pension_indexed: { helpId: "retirement.pension" },
  pension_fraction_while_partial: { helpId: "retirement.pension" },
  partial_expense_basis: { helpId: "retirement.partial" },
  horizon_lifespan_age: { helpId: "settings.horizon_age" },
};

/**
 * Título y FRASE de cada tarjeta de configuración (V3 de la tercera vuelta de UX, F9/F10).
 *
 * Sustituye a `ADVANCED_SECTION_LABEL`, que solo tenía rótulos porque el acordeón «Avanzado» los
 * usaba como separadores. El owner pidió lo contrario de un separador: «cada cuadro abre con una
 * frase de qué hace». Por eso ninguna frase describe el CONTROL —eso ya lo dice el rótulo del
 * campo— sino **qué cambia y qué implica cambiarlo**: una tarjeta que solo dijera «aquí van las
 * edades» habría dejado el formulario exactamente igual de mudo, con un título más.
 *
 * Las frases son el contrato de esta pantalla y `retirement-form.test.ts` las fija: título corto,
 * frase de más de 40 caracteres acabada en punto, una entrada por tarjeta. Ese test no juzga
 * prosa: caza la entrada que alguien añade sin frase al meter una tarjeta nueva.
 *
 * `PLAN_FIELD_HELP` no se toca: la ayuda POR CAMPO sigue colgando de su rótulo, y una frase de
 * tarjeta no la sustituye — explican cosas de tamaño distinto.
 */
export const PLAN_CARD_COPY: Record<PlanCardId, { title: string; blurb: string }> = {
  strategy: {
    title: "Estrategia",
    blurb:
      "Elige qué dispara tu jubilación. Cambiarla cambia lo que te preguntamos aquí abajo y cómo " +
      "se dimensiona tu objetivo.",
  },
  ages: {
    title: "Edades",
    blurb:
      "Las edades que fijan tu calendario. Se convierten en meses con tu fecha de nacimiento: sin " +
      "ella, la simulación te jubila por capital y no por edad.",
  },
  pension: {
    title: "Pensión",
    blurb:
      "Una renta con fecha de inicio. Los años anteriores los paga tu capital entero, así que la " +
      "edad a la que empieza mueve el objetivo, no solo el flujo de caja.",
  },
  spending: {
    title: "Gasto en jubilación",
    blurb:
      "De dónde sale el gasto anual que tu plan tiene que cubrir. Es el número que multiplica tu " +
      "objetivo: cambiarlo lo mueve todo.",
  },
  withdrawal: {
    title: "Retirada",
    blurb:
      "Cuánto sacas cada año una vez jubilado. El mismo porcentaje dimensiona el objetivo y " +
      "alimenta la regla: subirlo adelanta la fecha y sube el riesgo de quedarte sin capital.",
  },
  horizon: {
    title: "Horizonte",
    blurb:
      "Hasta qué edad tiene que durar el dinero. Alargarlo no mueve tu fecha de jubilación: mueve " +
      "cuántos escenarios llegan al final con capital.",
  },
};
