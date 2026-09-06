/**
 * Perfil de jubilación POR USUARIO en cliente: defaults, normalización, espejo de las cotas del
 * servidor y constructor del PATCH mínimo (5.0.0, issue #207, decisión D13; **modelo v2**, C1-C8).
 *
 * Tres responsabilidades, y ninguna más:
 *
 *  1. **Defaults y clamps en LECTURA** (`normalizeRetirementProfile`) — espejo de
 *     `resolve_retirement_profile` (`apps/api/src/handlers/retirement_profile.rs`). Un backend
 *     viejo, un campo ausente o un valor imposible no pueden dejar el formulario en un estado
 *     que no se pueda guardar.
 *  2. **Guarda de validez en ESCRITURA** (`retirementProfileIssue`) — espejo de
 *     `validate_retirement_profile`, **mismos códigos estables** que devuelve el servidor, para
 *     que la frase que ve el usuario salga del catálogo único (`errorMessages.ts`) y no de una
 *     traducción paralela. El autosave NO lanza el PATCH con un valor que el servidor va a
 *     rechazar: el patrón de la casa es no prometer «Guardado automático» sobre un 400.
 *  3. **PATCH MÍNIMO y tri-estado** (`buildRetirementProfilePatch`) — solo las claves que
 *     cambian; `null` explícito para borrar `pension`, `partial_retirement`,
 *     `target_retirement_age` o `coast_stop_age`. Mandar el perfil entero resetearía en
 *     silencio lo que el usuario no tocó: es el bug que el tri-estado del servidor existe para
 *     esquivar, y mandarlo completo lo reintroduciría desde este lado.
 *
 * **Qué cambió en el modelo v2** (y por qué este módulo encogió): el éxito define la fecha. Ya no
 * hay objetivo descontado, así que se fueron `target_basis`, `bridge_discount_basis` y todo el
 * bloque R6 (`effectiveTargetBasis` y compañía); tampoco hay colchón de caja (`cash_buffer_months`
 * y su cota). En su lugar entran el **umbral de éxito** (`success_threshold_pct`, 80–100, default
 * 95: la restricción que decide la fecha válida), los **dos modos de coast** (`coast_mode` +
 * `coast_stop_age`), los **dos modos de jornada reducida** (`partial_retirement.mode`) y el
 * **puente**, que dejó de ser una estrategia para ser un ajuste de la tarjeta Pensión
 * (`bridge_enabled` + `bridge_max_pct` + `bridge_max_years`), disponible en cualquiera de las
 * cuatro estrategias (C7).
 *
 * Las cotas están DUPLICADAS a propósito (aquí y en Rust): son el contrato publicado del
 * formulario. Si cambian allí, cambian aquí — `retirementProfile.test.ts` recorre la tabla
 * entera para que la divergencia sea un test rojo y no un 400 en producción.
 */

import type {
  CoastModeApi,
  FireNumberModeApi,
  PartialExpenseBasisApi,
  PartialRetirementApi,
  PensionPlanApi,
  RetirementProfileApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
  SpendModeApi,
  WithdrawalRuleApi,
  WithdrawalRuleKindApi,
} from "../api/types";
import { parseDisplayDecimal } from "./format";

/** Los dos modos de arranque de la jornada reducida (M11). Se deriva del tipo del wire para que
 *  no puedan separarse: si la API gana un modo, este alias lo gana solo. */
export type PartialModeApi = PartialRetirementApi["mode"];

// ---------------------------------------------------------------------------
// Cotas — espejo de `retirement_profile.rs` §Cotas
// ---------------------------------------------------------------------------

/** Edad mínima de cualquier hito del perfil (no hay miembros por debajo). */
export const MIN_PROFILE_AGE = 18;
/** Edad mínima a la que se puede declarar que empieza una pensión. */
export const MIN_PENSION_AGE = 50;
/** Techo de los `pct` de las reglas de retirada (%), BRUTO de impuestos como el SWR. */
export const MAX_WITHDRAWAL_PCT = 20;
/** Techo de la banda y del ajuste de `guardrails` (%). */
export const MAX_GUARDRAIL_PCT = 50;
/**
 * Techo del SWR (%). **6 desde el modelo v2** (era 4): el SWR dejó de ser «la tasa que fija el
 * objetivo» —ese objetivo ya no existe— y pasó a ser el TOPE de venta ordinaria anual, que es una
 * palanca que el usuario sí puede querer subir. La cota de abajo sigue siendo 0.
 */
export const MAX_SWR_PCT = 6;
/** Cotas de la edad límite del horizonte (siguen viviendo en `installation.rs`, reusadas aquí). */
export const MIN_HORIZON_LIFESPAN_AGE = 85;
export const MAX_HORIZON_LIFESPAN_AGE = 105;

/** Edades ofrecidas por el selector de horizonte (las mismas cinco de 4.9.0). */
export const HORIZON_LIFESPAN_AGE_OPTIONS = [85, 90, 95, 100, 105] as const;

/**
 * Umbral de éxito (C3, entero): el porcentaje de caminos que tienen que aguantar hasta el
 * horizonte para que un mes se considere fecha válida. `100` significa CERO fallos de N, no
 * «casi todos»; por debajo de 100 el corte lo decide el límite inferior del intervalo de Wilson,
 * que es estable frente a la semilla y a N.
 */
export const MIN_SUCCESS_THRESHOLD_PCT = 80;
export const MAX_SUCCESS_THRESHOLD_PCT = 100;
export const DEFAULT_SUCCESS_THRESHOLD_PCT = 95;

/**
 * Techo del puente (%). **Es el mismo que el de las reglas de retirada, y a propósito**: durante
 * el puente el tope de la tasa inicial deja de ser el SWR y pasa a ser este porcentaje, así que
 * las dos magnitudes miden lo mismo (cuánto se puede vender al año) y compartir cota es lo que
 * impide que una acabe permitiendo lo que la otra prohíbe.
 */
export const MAX_BRIDGE_PCT = MAX_WITHDRAWAL_PCT;
/** Años máximos de antelación sobre el inicio de la pensión durante los que aplica el puente. */
export const MIN_BRIDGE_YEARS = 1;
export const MAX_BRIDGE_YEARS = 20;
/** Años del puente al activarlo, si no se dice otra cosa. */
export const DEFAULT_BRIDGE_YEARS = 7;

/**
 * La tasa del puente al ACTIVARLO, como decimal-string: `max(5, swr + 1)` %.
 *
 * Los dos términos existen por motivos distintos. El `swr + 1` mantiene la invariante del puente
 * —tiene que ser **estrictamente mayor** que el SWR, o no es un puente, es la misma tasa—, y el
 * suelo de 5 evita proponer un puente tan tímido que no adelante ninguna fecha cuando el SWR es
 * bajo. Con `MAX_SWR_PCT = 6` el resultado nunca pasa de 7, muy por debajo de `MAX_BRIDGE_PCT`.
 *
 * **El servidor rellena exactamente esto** cuando el puente está activo y el número falta (y es
 * también lo que recibe un perfil guardado con la estrategia retirada `pension_bridge` al migrar,
 * C7). Si esta fórmula se mueve aquí sin moverse allí, activar el puente enseñaría una tasa y
 * guardaría otra.
 */
export function defaultBridgePct(swrPct: string): string {
  const parsed = parseDisplayDecimal(swrPct);
  const swr = parsed === null ? 0 : Math.min(Math.max(parsed, 0), MAX_SWR_PCT);
  return roundDecimalString(Math.min(Math.max(5, swr + 1), MAX_BRIDGE_PCT));
}

/** Un número a decimal-string sin la basura binaria de `2.9 + 1` (`3.9000000000000004`). */
function roundDecimalString(n: number): string {
  return String(Math.round(n * 10000) / 10000);
}

/**
 * Las CUATRO estrategias del modelo v2 (C7). `pension_bridge` ya no está: el puente pasó a ser un
 * ajuste de la tarjeta Pensión disponible en cualquiera de estas cuatro.
 */
export const RETIREMENT_STRATEGIES: readonly RetirementStrategyApi[] = [
  "asap",
  "retire_at_age",
  "coast",
  "partial",
] as const;

/** Nombres de producto (D33). No los inventes en la vista: viven aquí una sola vez. */
export const RETIREMENT_STRATEGY_LABEL: Record<RetirementStrategyApi, string> = {
  asap: "Cuanto antes (FIRE clásico)",
  retire_at_age: "A una edad fija",
  coast: "Ahorrar ahora y dejar crecer (Coast FIRE)",
  partial: "Jornada reducida (Barista FIRE)",
};

/** Una frase por estrategia — lo que hace, no cómo se implementa. */
export const RETIREMENT_STRATEGY_BLURB: Record<RetirementStrategyApi, string> = {
  asap:
    "Ahorras todo lo que puedes y te jubilas en la primera fecha en la que tu plan aguanta hasta el final con el nivel de éxito que exijas.",
  retire_at_age:
    "Eliges la edad y el plan te dice cuánto tienes que aportar cada mes para llegar a ella con tu nivel de éxito.",
  coast:
    "Aportas fuerte y luego dejas de aportar. Puedes fijar la edad de jubilación —y el plan te dice cuándo puedes dejar de ahorrar— o fijar la edad a la que dejas de ahorrar y ver a qué fecha te lleva.",
  partial:
    "Bajas de jornada a una edad (o en cuanto el plan pueda permitírselo) y cubres el hueco con tu capital hasta la jubilación total. No es la jubilación parcial legal española: es una decisión tuya sobre tu plan, sin trámite ni cotización asociada.",
};

export const WITHDRAWAL_RULE_KIND_LABEL: Record<WithdrawalRuleKindApi, string> = {
  fixed_real: "Gasto fijo en euros de hoy",
  percent_of_balance: "Un % del saldo cada año",
  hybrid: "Híbrida (empiezo alto y bajo)",
  guardrails: "Con bandas (Guyton-Klinger)",
};

/** Los dos modos de coast (M10). El modo decide QUÉ edad pide el formulario, no dos planes. */
export const COAST_MODE_LABEL: Record<CoastModeApi, string> = {
  fixed_retirement_age: "Fijo la edad a la que me jubilo",
  fixed_stop_age: "Fijo la edad a la que dejo de aportar",
};

/** Los dos modos de arranque de la jornada reducida (M11). */
export const PARTIAL_MODE_LABEL: Record<PartialModeApi, string> = {
  at_age: "A la edad que yo diga",
  asap: "En cuanto el plan pueda permitírselo",
};

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

/**
 * De dónde salió el porcentaje de la regla de retirada (5.0.0 U4, `PctSource` del servidor).
 *
 * Vive AQUÍ y no en `api/types.ts` por la misma razón que el resto del perfil: es un campo del
 * bloque personal, y el cliente lo lee **defensivamente** — un backend anterior a U4 no lo
 * publica, y ausencia NO es `"swr"`.
 */
export type PctSourceApi = "swr" | "explicit";

/**
 * Una regla de retirada tal y como la publica el servidor tras U4: con el porcentaje ya
 * resuelto y la procedencia al lado. El campo se declara aquí (y no en `api/types.ts`) porque
 * `WithdrawalRuleApi` es el cuerpo que el formulario ESCRIBE, y el formulario no escribe
 * `pct_source` jamás: lo decide el servidor.
 */
export type WithdrawalRuleWithSourceApi = WithdrawalRuleApi & {
  pct_source?: PctSourceApi | null;
};

/**
 * `pct_source` leído sin creerse nada: `null` cuando el backend no lo publica (anterior a U4) o
 * cuando el literal no es de los dos conocidos.
 *
 * **El sesgo importa**: sin `pct_source` el cliente NO puede concluir que un `pct` guardado se
 * hereda del SWR, así que lo trata como explícito y lo conserva. Al revés —dar por heredado lo
 * que no lo es— el formulario borraría en silencio un porcentaje que alguien fijó por API.
 */
export function withdrawalPctSource(
  rule: WithdrawalRuleApi | WithdrawalRuleWithSourceApi | null | undefined,
): PctSourceApi | null {
  const raw = (rule as WithdrawalRuleWithSourceApi | null | undefined)?.pct_source;
  return raw === "swr" || raw === "explicit" ? raw : null;
}

/**
 * El porcentaje que la regla usa de verdad, en el `kind` que tiene uno (U4). `null` en
 * `fixed_real`, que no retira un porcentaje sino la necesidad declarada.
 *
 * Espejo de `resolve_withdrawal_rule`: `percent_of_balance`/`guardrails` usan `pct`, `hybrid`
 * usa `start_pct`, y el que falte hereda `swr_pct`.
 */
export function effectiveWithdrawalPct(
  rule: WithdrawalRuleApi,
  swrPct: string,
): string | null {
  switch (rule.kind) {
    case "fixed_real":
      return null;
    case "hybrid":
      return rule.start_pct ?? swrPct;
    default:
      return rule.pct ?? swrPct;
  }
}

/**
 * **El resolvedor único del porcentaje de retirada en cliente (U4)** — espejo exacto de
 * `resolve_withdrawal_rule` (`apps/api/src/handlers/retirement_profile.rs`).
 *
 * Rellena con `swr_pct` el porcentaje que la regla necesita y que nadie escribió. Se usa para
 * VALIDAR (la guarda tiene que juzgar lo que el motor va a retirar, no lo que hay tecleado) y
 * jamás para construir el PATCH: el formulario no manda `pct` ni `start_pct` nunca — ese es el
 * punto entero de U4, un solo porcentaje editable y es el SWR.
 */
export function resolveWithdrawalRule(
  rule: WithdrawalRuleApi,
  swrPct: string,
): WithdrawalRuleApi {
  switch (rule.kind) {
    case "fixed_real":
      return rule;
    case "hybrid":
      return rule.start_pct == null ? { ...rule, start_pct: swrPct } : rule;
    default:
      return rule.pct == null ? { ...rule, pct: swrPct } : rule;
  }
}

/** La regla de retirada de quien no ha tocado nada: exactamente el drenaje de 4.15.x. */
export function defaultWithdrawalRuleApi(): WithdrawalRuleApi {
  return {
    kind: "fixed_real",
    pct: null,
    start_pct: null,
    end_pct: null,
    band_pct: null,
    adjust_pct: null,
    spend_mode: "ceiling",
  };
}

/**
 * El perfil de quien no ha tocado nada: cruce por éxito (`asap`), SWR 3,5 %, horizonte a 90 y el
 * **umbral al 95 %** — el mismo `default_retirement_profile()` del servidor. Si esto se moviera,
 * el formulario propondría por defecto un plan distinto al que la proyección está simulando.
 *
 * `coast_mode` viaja siempre (aunque la estrategia no sea `coast`) porque es el que decide si
 * `target_retirement_age` es obligatoria: un `undefined` aquí haría que la guarda pidiera una
 * edad que la pantalla no está enseñando.
 */
export function defaultRetirementProfileApi(): RetirementProfileApi {
  return {
    strategy: "asap",
    target_retirement_age: null,
    fire_number_mode: "annual_expense",
    fire_number_manual_amount: null,
    swr_pct: "3.5",
    horizon_lifespan_age: 90,
    success_threshold_pct: DEFAULT_SUCCESS_THRESHOLD_PCT,
    coast_mode: "fixed_retirement_age",
    coast_stop_age: null,
    withdrawal_rule: defaultWithdrawalRuleApi(),
    pension: null,
    partial_retirement: null,
  };
}

// ---------------------------------------------------------------------------
// Parsers de enumerado — allow-list, nunca un cast
// ---------------------------------------------------------------------------

function pick<T extends string>(v: unknown, allowed: readonly T[], fallback: T): T {
  return typeof v === "string" && (allowed as readonly string[]).includes(v)
    ? (v as T)
    : fallback;
}

/**
 * `pension_bridge` es un literal RETIRADO (C7) que sigue vivo en perfiles guardados y en backups:
 * se pliega a `asap`, igual que `annual_expense_adjusted` se pliega a `annual_expense` más abajo.
 *
 * **Lo que este pliegue NO hace es activar el puente.** Eso lo hace el servidor al resolver el
 * perfil guardado (y lo anuncia con el aviso `strategy_pension_bridge_migrated`), porque es él
 * quien conoce el estado almacenado; el cliente solo ve el perfil YA migrado. Plegar aquí a
 * `asap` sin tocar la pensión es exactamente lo que hace falta para que el selector no se quede
 * en blanco ante una estrategia que ya no existe.
 */
export function parseRetirementStrategy(v: unknown): RetirementStrategyApi {
  if (v === "pension_bridge") return "asap";
  return pick(v, RETIREMENT_STRATEGIES, "asap");
}

export function parseCoastMode(v: unknown): CoastModeApi {
  return pick(v, ["fixed_retirement_age", "fixed_stop_age"] as const, "fixed_retirement_age");
}

export function parsePartialMode(v: unknown): PartialModeApi {
  return pick(v, ["at_age", "asap"] as const, "at_age");
}

export function parseWithdrawalRuleKind(v: unknown): WithdrawalRuleKindApi {
  return pick(
    v,
    ["fixed_real", "percent_of_balance", "hybrid", "guardrails"] as const,
    "fixed_real",
  );
}

export function parseSpendMode(v: unknown): SpendModeApi {
  return pick(v, ["ceiling", "rule_is_spend"] as const, "ceiling");
}

export function parsePartialExpenseBasis(v: unknown): PartialExpenseBasisApi {
  return pick(v, ["retirement", "regular"] as const, "retirement");
}

export function parseFireNumberMode(v: unknown): FireNumberModeApi {
  // `annual_expense_adjusted` es un modo retirado (Ola 1, #137) que sigue vivo en backups
  // antiguos: se pliega al modo que lo sustituyó, igual que hace `lib/fire.ts`.
  if (v === "annual_expense_adjusted") return "annual_expense";
  return pick(v, ["manual", "annual_expense", "current_income"] as const, "annual_expense");
}

// ---------------------------------------------------------------------------
// Normalización (defaults + clamps en LECTURA)
// ---------------------------------------------------------------------------

function clampInt(v: unknown, min: number, max: number, fallback: number): number {
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(n)));
}

/** Decimal-string acotado a `[min, max]`. Un valor ilegible cae al `fallback`. */
function clampDecimalString(
  v: unknown,
  min: number,
  max: number,
  fallback: string,
): string {
  if (v == null) return fallback;
  const n = parseDisplayDecimal(String(v));
  if (n === null) return fallback;
  return String(Math.min(max, Math.max(min, n)));
}

/** `pct` opcional de una regla: `null` se conserva (la regla no lo usa); un valor se acota. */
function clampOptionalPct(v: unknown, max: number): string | null {
  if (v == null || String(v).trim() === "") return null;
  const n = parseDisplayDecimal(String(v));
  if (n === null) return null;
  return String(Math.min(max, Math.max(0, n)));
}

export function normalizeWithdrawalRule(raw: unknown): WithdrawalRuleWithSourceApi {
  const base: WithdrawalRuleWithSourceApi = defaultWithdrawalRuleApi();
  if (!raw || typeof raw !== "object") return base;
  const r = raw as Partial<WithdrawalRuleApi>;
  const source = withdrawalPctSource(raw as WithdrawalRuleWithSourceApi);
  // **U4, y es la línea que hace que el formulario tenga UN solo porcentaje**: el servidor
  // publica la regla ya RESUELTA —`pct`/`start_pct` rellenos con el `swr_pct` y `pct_source:
  // "swr"` al lado—, así que leerla tal cual dejaría el borrador con un porcentaje heredado
  // dentro. A la siguiente escritura ese valor viajaría de vuelta como si alguien lo hubiera
  // fijado, y mover el slider del SWR ya no movería la regla: el porcentaje se habría
  // congelado sin que nadie lo decidiera. Se suelta aquí, en la LECTURA, para que el borrador
  // diga la verdad («no lo he fijado») y el porcentaje siga colgando del SWR.
  const inherited = source === "swr";
  return {
    kind: parseWithdrawalRuleKind(r.kind),
    pct: inherited ? null : clampOptionalPct(r.pct, MAX_WITHDRAWAL_PCT),
    start_pct: inherited ? null : clampOptionalPct(r.start_pct, MAX_WITHDRAWAL_PCT),
    end_pct: clampOptionalPct(r.end_pct, MAX_WITHDRAWAL_PCT),
    band_pct: clampOptionalPct(r.band_pct, MAX_GUARDRAIL_PCT),
    adjust_pct: clampOptionalPct(r.adjust_pct, MAX_GUARDRAIL_PCT),
    spend_mode: parseSpendMode(r.spend_mode),
    // La procedencia se CONSERVA aunque el valor se haya soltado: es lo que permite a la vista
    // decir «regla al X %, fijado por API» sin volver a preguntárselo al servidor.
    ...(source != null ? { pct_source: source } : {}),
  };
}

/**
 * Tasa del puente en LECTURA. Tres cosas, en este orden:
 *
 *  * ausente o ilegible **con el puente activo** → la tasa por defecto (`defaultBridgePct`), que
 *    es lo que rellena el servidor: dejarla vacía dejaría el formulario en un estado que la
 *    guarda rechaza y el autosave no podría guardar nunca;
 *  * cualquier valor se acota por arriba a `MAX_BRIDGE_PCT`;
 *  * un valor que **no supera el SWR** —con el puente activo— se sube también a la tasa por
 *    defecto: un puente que no es estrictamente mayor que el SWR no es un puente, es la misma
 *    tasa, y la invariante se restaura en lectura en vez de bloquear el guardado.
 *
 * Con el puente APAGADO el número es inerte: se conserva tal cual (solo con el techo aplicado)
 * para no destruir lo que alguien fijó por API o dejó preparado antes de encender el interruptor.
 */
function normalizeBridgePct(v: unknown, enabled: boolean, swrPct: string): string | null {
  const fallback = enabled ? defaultBridgePct(swrPct) : null;
  if (v == null || String(v).trim() === "") return fallback;
  const n = parseDisplayDecimal(String(v));
  if (n === null) return fallback;
  const capped = Math.min(Math.max(n, 0), MAX_BRIDGE_PCT);
  const swr = parseDisplayDecimal(swrPct) ?? 0;
  if (enabled && capped <= swr) return defaultBridgePct(swrPct);
  return roundDecimalString(capped);
}

/** Años del puente en LECTURA: mismo criterio que la tasa, con el rango `[1, 20]`. */
function normalizeBridgeYears(v: unknown, enabled: boolean): number | null {
  if (v == null) return enabled ? DEFAULT_BRIDGE_YEARS : null;
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n)) return enabled ? DEFAULT_BRIDGE_YEARS : null;
  return clampInt(n, MIN_BRIDGE_YEARS, MAX_BRIDGE_YEARS, DEFAULT_BRIDGE_YEARS);
}

/**
 * Espejo de `resolve_retirement_profile`: defaults y clamps en lectura, en el MISMO orden (el
 * horizonte primero, porque es el techo de todas las edades del perfil; el SWR antes que la
 * pensión, porque el puente se acota contra él).
 *
 * **Las claves que no conoce se caen** —el objeto que devuelve es un literal cerrado—, y ese es
 * el mecanismo por el que un JSONB de 4.15.x con `target_basis`, `bridge_discount_basis` o
 * `cash_buffer_months` deja de existir sin migrar nada en cliente. El servidor hace lo mismo con
 * su `#[serde(default)]` sin `deny_unknown_fields`.
 */
export function normalizeRetirementProfile(
  raw: RetirementProfileApi | null | undefined,
): RetirementProfileApi {
  const base = defaultRetirementProfileApi();
  if (!raw || typeof raw !== "object") return base;

  const horizon = clampInt(
    raw.horizon_lifespan_age,
    MIN_HORIZON_LIFESPAN_AGE,
    MAX_HORIZON_LIFESPAN_AGE,
    base.horizon_lifespan_age,
  );
  const swr = clampDecimalString(raw.swr_pct, 0, MAX_SWR_PCT, base.swr_pct);

  const targetAge =
    raw.target_retirement_age == null
      ? null
      : clampInt(raw.target_retirement_age, MIN_PROFILE_AGE, horizon, MIN_PROFILE_AGE);

  // La edad de parada de coast no puede caer más tarde que la jubilación que la espera: si hay
  // edad objetivo, ella es el techo; si no la hay, el horizonte.
  const coastStopAge =
    raw.coast_stop_age == null
      ? null
      : clampInt(raw.coast_stop_age, MIN_PROFILE_AGE, targetAge ?? horizon, MIN_PROFILE_AGE);

  const pension: PensionPlanApi | null =
    raw.pension && typeof raw.pension === "object"
      ? (() => {
          const bridgeEnabled = raw.pension?.bridge_enabled === true;
          return {
            monthly_amount_today: clampDecimalString(
              raw.pension?.monthly_amount_today,
              0,
              Number.MAX_SAFE_INTEGER,
              "0",
            ),
            starts_at_age: clampInt(
              raw.pension?.starts_at_age,
              Math.min(MIN_PENSION_AGE, horizon),
              horizon,
              Math.min(MIN_PENSION_AGE, horizon),
            ),
            indexed: raw.pension?.indexed !== false,
            fraction_while_partial: clampDecimalString(
              raw.pension?.fraction_while_partial,
              0,
              1,
              "0",
            ),
            bridge_enabled: bridgeEnabled,
            bridge_max_pct: normalizeBridgePct(
              raw.pension?.bridge_max_pct,
              bridgeEnabled,
              swr,
            ),
            bridge_max_years: normalizeBridgeYears(
              raw.pension?.bridge_max_years,
              bridgeEnabled,
            ),
          };
        })()
      : null;

  const partial: PartialRetirementApi | null =
    raw.partial_retirement && typeof raw.partial_retirement === "object"
      ? {
          mode: parsePartialMode(raw.partial_retirement.mode),
          // `null` se conserva: con `mode: "asap"` la edad de inicio la resuelve el servidor y
          // rellenarla aquí inventaría una decisión que el usuario no tomó.
          starts_at_age:
            raw.partial_retirement.starts_at_age == null
              ? null
              : clampInt(
                  raw.partial_retirement.starts_at_age,
                  MIN_PROFILE_AGE,
                  horizon,
                  MIN_PROFILE_AGE,
                ),
          income_monthly_today: clampDecimalString(
            raw.partial_retirement.income_monthly_today,
            0,
            Number.MAX_SAFE_INTEGER,
            "0",
          ),
          expense_basis: parsePartialExpenseBasis(raw.partial_retirement.expense_basis),
        }
      : null;

  return {
    strategy: parseRetirementStrategy(raw.strategy),
    target_retirement_age: targetAge,
    fire_number_mode: parseFireNumberMode(raw.fire_number_mode),
    fire_number_manual_amount:
      raw.fire_number_manual_amount == null ||
      String(raw.fire_number_manual_amount).trim() === ""
        ? null
        : String(raw.fire_number_manual_amount),
    swr_pct: swr,
    horizon_lifespan_age: horizon,
    success_threshold_pct:
      raw.success_threshold_pct == null
        ? DEFAULT_SUCCESS_THRESHOLD_PCT
        : clampInt(
            raw.success_threshold_pct,
            MIN_SUCCESS_THRESHOLD_PCT,
            MAX_SUCCESS_THRESHOLD_PCT,
            DEFAULT_SUCCESS_THRESHOLD_PCT,
          ),
    coast_mode: parseCoastMode(raw.coast_mode),
    coast_stop_age: coastStopAge,
    withdrawal_rule: normalizeWithdrawalRule(raw.withdrawal_rule),
    pension,
    partial_retirement: partial,
  };
}

/**
 * El puente RESUELTO: los dos números que el motor va a usar de verdad (o `null` si no hay
 * puente). Mismo papel que `resolveWithdrawalRule` para la regla de retirada — la guarda tiene
 * que juzgar **lo que se va a aplicar**, no lo que hay tecleado, y el PATCH tiene que mandar lo
 * que la pantalla estaba enseñando.
 *
 * Un campo vacío se trata como ausente (no como cero): es el estado natural justo después de
 * encender el interruptor, y el servidor lo rellena con el mismo default.
 */
export function resolveBridge(
  pension: PensionPlanApi,
  swrPct: string,
): { pct: string; years: number } | null {
  if (!pension.bridge_enabled) return null;
  const raw = String(pension.bridge_max_pct ?? "").trim();
  return {
    pct: raw === "" ? defaultBridgePct(swrPct) : raw,
    years: pension.bridge_max_years ?? DEFAULT_BRIDGE_YEARS,
  };
}

/**
 * La pensión con el puente encendido o apagado, con los defaults ya puestos al encenderlo. Es lo
 * que el interruptor de la tarjeta Pensión tiene que llamar: activar el puente y dejar los dos
 * números en `null` dejaría el borrador enseñando huecos donde el servidor va a guardar 5 % y 7
 * años.
 */
export function withBridgeEnabled(
  pension: PensionPlanApi,
  enabled: boolean,
  swrPct: string,
): PensionPlanApi {
  if (!enabled) {
    return { ...pension, bridge_enabled: false };
  }
  return {
    ...pension,
    bridge_enabled: true,
    bridge_max_pct: pension.bridge_max_pct ?? defaultBridgePct(swrPct),
    bridge_max_years: pension.bridge_max_years ?? DEFAULT_BRIDGE_YEARS,
  };
}

/**
 * `true` para las combinaciones cuyo trigger es una EDAD DE JUBILACIÓN (y que por tanto la
 * exigen). Espejo de `requires_target_age(strategy, coast_mode)`.
 *
 * `coast` **solo** la exige en su modo A (`fixed_retirement_age`): en el modo B el usuario fija
 * la edad a la que deja de aportar y la fecha de jubilación es el RESULTADO, así que pedirle
 * además la edad de jubilación sería pedirle la respuesta.
 */
export function strategyRequiresTargetAge(
  strategy: RetirementStrategyApi,
  coastMode: CoastModeApi,
): boolean {
  if (strategy === "retire_at_age") return true;
  return strategy === "coast" && coastMode === "fixed_retirement_age";
}

// ---------------------------------------------------------------------------
// Guarda de validez — espejo de `validate_retirement_profile`, mismos códigos
// ---------------------------------------------------------------------------

/** Vacío = 0 (los campos donde «no escribo nada» significa cero); ilegible = `null`. */
function decimalOrZero(v: string | null | undefined): number | null {
  const t = String(v ?? "").trim();
  if (t === "") return 0;
  return parseDisplayDecimal(t);
}

/** Vacío = `"0"` en el wire. La API solo acepta decimales, nunca la cadena vacía. */
function decimalStringForWire(v: string | null | undefined): string {
  const t = String(v ?? "").trim();
  return t === "" ? "0" : t;
}

/** Un `pct` obligatorio de la regla: ausente, ilegible o fuera de `(0, max]`. */
function pctIssue(v: string | null, max: number, rangeCode: string): string | null {
  if (v == null || String(v).trim() === "") return "withdrawal_pct_required";
  const n = parseDisplayDecimal(v);
  if (n === null) return "decimal_invalid";
  if (n <= 0 || n > max) return rangeCode;
  return null;
}

/**
 * Primer problema del perfil, como CÓDIGO estable del servidor (`null` = se puede guardar).
 * El orden reproduce el de `validate_retirement_profile` para que el usuario vea el mismo
 * primer error por las dos vías.
 */
export function retirementProfileIssue(p: RetirementProfileApi): string | null {
  // --- Los cuatro ejes movidos conservan sus códigos de 4.15.x ---------------------------
  const swr = parseDisplayDecimal(p.swr_pct);
  if (swr === null) return "decimal_invalid";
  if (swr < 0 || swr > MAX_SWR_PCT) return "swr_out_of_range";

  if (p.fire_number_mode === "manual") {
    if (
      p.fire_number_manual_amount == null ||
      String(p.fire_number_manual_amount).trim() === ""
    ) {
      return "fire_manual_amount_required";
    }
    const amt = parseDisplayDecimal(p.fire_number_manual_amount);
    if (amt === null) return "decimal_invalid";
    if (amt <= 0) return "fire_manual_amount_not_positive";
  }

  const horizon = p.horizon_lifespan_age;
  if (
    !Number.isInteger(horizon) ||
    horizon < MIN_HORIZON_LIFESPAN_AGE ||
    horizon > MAX_HORIZON_LIFESPAN_AGE
  ) {
    return "horizon_lifespan_age_out_of_range";
  }

  // --- Umbral de éxito (C3) ---------------------------------------------------------------
  // Entero: «95,5 % de los caminos» no significa nada — el umbral se compara contra un conteo.
  if (
    !Number.isInteger(p.success_threshold_pct) ||
    p.success_threshold_pct < MIN_SUCCESS_THRESHOLD_PCT ||
    p.success_threshold_pct > MAX_SUCCESS_THRESHOLD_PCT
  ) {
    return "success_threshold_out_of_range";
  }

  // --- Estrategia ------------------------------------------------------------------------
  if (
    strategyRequiresTargetAge(p.strategy, p.coast_mode) &&
    p.target_retirement_age == null
  ) {
    return "target_retirement_age_required";
  }
  if (
    p.strategy === "coast" &&
    p.coast_mode === "fixed_stop_age" &&
    p.coast_stop_age == null
  ) {
    return "coast_stop_age_required";
  }
  if (
    p.strategy === "partial" &&
    p.partial_retirement != null &&
    p.partial_retirement.mode === "at_age" &&
    p.partial_retirement.starts_at_age == null
  ) {
    return "partial_start_age_required";
  }

  // --- Edades ----------------------------------------------------------------------------
  if (p.target_retirement_age != null) {
    const a = p.target_retirement_age;
    if (!Number.isInteger(a) || a < MIN_PROFILE_AGE || a > horizon) {
      return "retirement_age_out_of_range";
    }
  }
  if (p.coast_stop_age != null) {
    // El techo es la jubilación que la espera (o el horizonte si no hay edad fijada): dejar de
    // aportar DESPUÉS de jubilarte no describe ningún plan.
    const a = p.coast_stop_age;
    const ceiling = p.target_retirement_age ?? horizon;
    if (!Number.isInteger(a) || a < MIN_PROFILE_AGE || a > ceiling) {
      return "coast_stop_age_out_of_range";
    }
  }
  if (p.pension) {
    const a = p.pension.starts_at_age;
    if (!Number.isInteger(a) || a < MIN_PENSION_AGE || a > horizon) {
      return "pension_age_out_of_range";
    }
    // El importe vacío NO es `decimal_invalid`: es el estado natural justo después de activar
    // la casilla, y «debe ser mayor que cero» es lo que hay que decirle a quien no ha escrito
    // nada todavía.
    const rawAmt = String(p.pension.monthly_amount_today).trim();
    if (rawAmt === "") return "pension_amount_not_positive";
    const amt = parseDisplayDecimal(rawAmt);
    if (amt === null) return "decimal_invalid";
    if (amt <= 0) return "pension_amount_not_positive";
    const fr = decimalOrZero(p.pension.fraction_while_partial);
    if (fr === null) return "decimal_invalid";
    if (fr < 0 || fr > 1) return "pension_fraction_out_of_range";

    // --- Puente (C2/C7): solo se juzga si está ENCENDIDO ---------------------------------
    // Apagado, los dos números son inertes y el motor no los mira; rechazar el guardado por un
    // valor invisible bloquearía el autosave sin que nada en pantalla lo explique.
    const bridge = resolveBridge(p.pension, p.swr_pct);
    if (bridge) {
      const pct = parseDisplayDecimal(bridge.pct);
      if (pct === null) return "decimal_invalid";
      if (pct <= 0 || pct > MAX_BRIDGE_PCT) return "bridge_max_pct_out_of_range";
      // El orden importa: primero «cabe en la escala», después «es un puente de verdad». Al
      // revés, subir el SWR por encima de 20 daría el mensaje equivocado.
      if (pct <= swr) return "bridge_max_pct_not_above_swr";
      if (
        !Number.isInteger(bridge.years) ||
        bridge.years < MIN_BRIDGE_YEARS ||
        bridge.years > MAX_BRIDGE_YEARS
      ) {
        return "bridge_max_years_out_of_range";
      }
    }
  }
  if (p.partial_retirement) {
    const a = p.partial_retirement.starts_at_age;
    // `null` es legítimo con `mode: "asap"` (la edad sale de la serie); con `at_age` ya lo ha
    // rechazado `partial_start_age_required` más arriba.
    if (a != null) {
      if (!Number.isInteger(a) || a < MIN_PROFILE_AGE || a > horizon) {
        return "partial_age_out_of_range";
      }
    }
    // Un ingreso vacío es un año sabático declarado (0 €/mes), no un error: el bloque de media
    // jornada existe precisamente para poder no cobrar nada durante la fase.
    const inc = decimalOrZero(p.partial_retirement.income_monthly_today);
    if (inc === null) return "decimal_invalid";
    if (inc < 0) return "partial_income_negative";
    if (a != null && p.target_retirement_age != null && a >= p.target_retirement_age) {
      return "partial_not_before_retirement";
    }
  }

  // U4 — se juzga el porcentaje EFECTIVO, no el escrito: `pct`/`start_pct` ausentes heredan
  // `swr_pct` (ya comprobado arriba contra `MAX_SWR_PCT`). Consecuencia declarada, la misma que
  // el servidor: con `swr_pct = 0` una regla basada en saldo y sin porcentaje propio es
  // `withdrawal_pct_out_of_range` — un plan que retira 0 % no es un plan.
  return withdrawalRuleIssue(resolveWithdrawalRule(p.withdrawal_rule, p.swr_pct));
}

/**
 * Cada `kind` exige SUS campos y no los de otro (espejo de `validate_withdrawal_rule`).
 *
 * **Corre sobre la regla YA RESUELTA** (`resolveWithdrawalRule`), igual que en Rust: `pct` y
 * `start_pct` no llegan aquí ausentes para los `kind` que los usan. Lo que sigue vivo de
 * `withdrawal_pct_required` son el `end_pct` de la híbrida y la banda/ajuste de las bandas, que
 * **no heredan nada** — no son porcentajes de retirada, son el suelo del latch y la reacción de
 * la regla.
 */
export function withdrawalRuleIssue(r: WithdrawalRuleApi): string | null {
  switch (r.kind) {
    case "fixed_real":
      return null;
    case "percent_of_balance":
      return pctIssue(r.pct, MAX_WITHDRAWAL_PCT, "withdrawal_pct_out_of_range");
    case "hybrid": {
      const s = pctIssue(r.start_pct, MAX_WITHDRAWAL_PCT, "withdrawal_pct_out_of_range");
      if (s) return s;
      const e = pctIssue(r.end_pct, MAX_WITHDRAWAL_PCT, "withdrawal_pct_out_of_range");
      if (e) return e;
      const start = parseDisplayDecimal(r.start_pct ?? "");
      const end = parseDisplayDecimal(r.end_pct ?? "");
      if (start === null || end === null) return "decimal_invalid";
      if (end >= start) return "hybrid_end_pct_not_below_start";
      return null;
    }
    case "guardrails": {
      const p = pctIssue(r.pct, MAX_WITHDRAWAL_PCT, "withdrawal_pct_out_of_range");
      if (p) return p;
      for (const v of [r.band_pct, r.adjust_pct]) {
        const issue = pctIssue(v, MAX_GUARDRAIL_PCT, "withdrawal_band_out_of_range");
        if (issue) return issue;
      }
      return null;
    }
  }
}

// ---------------------------------------------------------------------------
// PATCH mínimo y tri-estado
// ---------------------------------------------------------------------------

/** Igualdad de un decimal-string por VALOR: `"3.50"` y `"3.5"` son el mismo SWR. */
function sameDecimal(a: string | null, b: string | null): boolean {
  if (a == null || b == null) return a === b;
  const na = parseDisplayDecimal(a);
  const nb = parseDisplayDecimal(b);
  if (na === null || nb === null) return String(a).trim() === String(b).trim();
  return na === nb;
}

/** Como `sameDecimal`, pero donde la cadena vacía significa cero (ingreso parcial, fracción). */
function sameDecimalZeroDefault(a: string | null, b: string | null): boolean {
  return decimalOrZero(a) === decimalOrZero(b);
}

function sameWithdrawalRule(a: WithdrawalRuleApi, b: WithdrawalRuleApi): boolean {
  return (
    a.kind === b.kind &&
    a.spend_mode === b.spend_mode &&
    sameDecimal(a.pct, b.pct) &&
    sameDecimal(a.start_pct, b.start_pct) &&
    sameDecimal(a.end_pct, b.end_pct) &&
    sameDecimal(a.band_pct, b.band_pct) &&
    sameDecimal(a.adjust_pct, b.adjust_pct)
  );
}

function samePension(a: PensionPlanApi | null, b: PensionPlanApi | null): boolean {
  if (a == null || b == null) return a === b;
  return (
    sameDecimal(a.monthly_amount_today, b.monthly_amount_today) &&
    a.starts_at_age === b.starts_at_age &&
    a.indexed === b.indexed &&
    sameDecimalZeroDefault(a.fraction_while_partial, b.fraction_while_partial) &&
    a.bridge_enabled === b.bridge_enabled &&
    sameDecimal(a.bridge_max_pct, b.bridge_max_pct) &&
    a.bridge_max_years === b.bridge_max_years
  );
}

function samePartial(
  a: PartialRetirementApi | null,
  b: PartialRetirementApi | null,
): boolean {
  if (a == null || b == null) return a === b;
  return (
    a.mode === b.mode &&
    a.starts_at_age === b.starts_at_age &&
    sameDecimalZeroDefault(a.income_monthly_today, b.income_monthly_today) &&
    a.expense_basis === b.expense_basis
  );
}

/** La pensión lista para el wire: importes sin cadenas vacías y el puente ya resuelto. */
function pensionForWire(pension: PensionPlanApi, swrPct: string): PensionPlanApi {
  const bridge = resolveBridge(pension, swrPct);
  return {
    ...pension,
    monthly_amount_today: decimalStringForWire(pension.monthly_amount_today),
    fraction_while_partial: decimalStringForWire(pension.fraction_while_partial),
    // Con el puente apagado los dos van a `null`: son los valores que el contrato publica para
    // ese estado, y mandar números inertes invitaría a que el servidor los aplicara algún día.
    bridge_max_pct: bridge ? bridge.pct : null,
    bridge_max_years: bridge ? bridge.years : null,
  };
}

/**
 * Diferencia entre el perfil que tiene el servidor y el borrador del formulario, como PATCH
 * **mínimo**: solo las claves que cambian de verdad, con `null` explícito donde el usuario
 * borró un bloque opcional.
 *
 * Dos reglas que no son negociables:
 *
 *  * **Un decimal se compara por VALOR, no por texto.** Sin esto, teclear `3,50` sobre un
 *    `3.5` guardado mandaría un PATCH que no cambia nada, y cada pulsación de una coma sería
 *    una escritura y una invalidación de la cache de proyección.
 *  * **Los bloques viajan ENTEROS o no viajan.** `pension` (con sus tres campos de puente) y
 *    `partial_retirement` (con su `mode`) se mandan completos: qué campos son obligatorios
 *    dentro depende de otros campos del mismo bloque, y un merge parcial permitiría estados
 *    —«puente encendido sin tasa», «modo `at_age` sin edad»— que nadie escribió.
 *
 * `coast_stop_age` es el tri-estado nuevo: ausente no cambia nada, un número la fija y `null`
 * **la suelta**. El servidor la conserva aunque el modo no la use, igual que hace con
 * `target_retirement_age`, así que borrarla tiene que ser una orden explícita.
 */
export function buildRetirementProfilePatch(
  before: RetirementProfileApi,
  after: RetirementProfileApi,
): RetirementProfilePatchApi {
  const patch: RetirementProfilePatchApi = {};

  if (before.strategy !== after.strategy) patch.strategy = after.strategy;
  if (before.target_retirement_age !== after.target_retirement_age) {
    patch.target_retirement_age = after.target_retirement_age;
  }
  if (before.fire_number_mode !== after.fire_number_mode) {
    patch.fire_number_mode = after.fire_number_mode;
  }
  if (!sameDecimal(before.fire_number_manual_amount, after.fire_number_manual_amount)) {
    patch.fire_number_manual_amount = after.fire_number_manual_amount;
  }
  if (!sameDecimal(before.swr_pct, after.swr_pct)) patch.swr_pct = after.swr_pct;
  if (before.horizon_lifespan_age !== after.horizon_lifespan_age) {
    patch.horizon_lifespan_age = after.horizon_lifespan_age;
  }
  if (before.success_threshold_pct !== after.success_threshold_pct) {
    patch.success_threshold_pct = after.success_threshold_pct;
  }
  if (before.coast_mode !== after.coast_mode) patch.coast_mode = after.coast_mode;
  if (before.coast_stop_age !== after.coast_stop_age) {
    patch.coast_stop_age = after.coast_stop_age;
  }
  if (!sameWithdrawalRule(before.withdrawal_rule, after.withdrawal_rule)) {
    patch.withdrawal_rule = withdrawalRuleForWire(after.withdrawal_rule);
  }
  if (!samePension(before.pension, after.pension)) {
    patch.pension = after.pension ? pensionForWire(after.pension, after.swr_pct) : null;
  }
  if (!samePartial(before.partial_retirement, after.partial_retirement)) {
    patch.partial_retirement = after.partial_retirement
      ? {
          ...after.partial_retirement,
          income_monthly_today: decimalStringForWire(
            after.partial_retirement.income_monthly_today,
          ),
        }
      : null;
  }

  return patch;
}

/**
 * La regla lista para el wire: **sin `pct_source`** (U4).
 *
 * La procedencia la decide el SERVIDOR y solo él; reenviarla convertiría una lectura en una
 * orden. Los porcentajes heredados ya vienen sueltos de `normalizeWithdrawalRule`, así que lo
 * que queda en `pct`/`start_pct` cuando llega aquí es lo que alguien fijó de verdad por API —
 * y eso SÍ viaja: borrarlo porque el formulario no sabe editarlo sería perder el dato del
 * usuario en la primera pulsación de un campo vecino.
 */
export function withdrawalRuleForWire(rule: WithdrawalRuleApi): WithdrawalRuleApi {
  return {
    kind: rule.kind,
    pct: rule.pct,
    start_pct: rule.start_pct,
    end_pct: rule.end_pct,
    band_pct: rule.band_pct,
    adjust_pct: rule.adjust_pct,
    spend_mode: rule.spend_mode,
  };
}

/** `true` cuando el PATCH no nombra nada: el servidor lo rechazaría con `patch_empty`. */
export function isEmptyRetirementProfilePatch(p: RetirementProfilePatchApi): boolean {
  return Object.keys(p).length === 0;
}

/**
 * Bloque de pensión de partida al activar la casilla (importe vacío: lo pone el usuario).
 *
 * El puente arranca **apagado** —es un ajuste, no el estado natural de tener pensión— y por eso
 * sus dos números son `null`. El SWR entra igualmente porque es lo que decide con qué tasa se
 * enciende: el interruptor llama a `withBridgeEnabled` con este mismo valor y no puede
 * inventarse otro, o la pantalla enseñaría una tasa distinta de la que el servidor guarda.
 */
export function newPensionPlanDraft(swrPct: string): PensionPlanApi {
  return withBridgeEnabled(
    {
      monthly_amount_today: "",
      starts_at_age: 67,
      indexed: true,
      fraction_while_partial: "0",
      bridge_enabled: false,
      bridge_max_pct: null,
      bridge_max_years: null,
    },
    false,
    swrPct,
  );
}

/**
 * Bloque de media jornada de partida al elegir la estrategia o activar la casilla. Arranca en el
 * modo A (`at_age`) con una edad puesta: es el modo que el usuario puede razonar sin correr el
 * plan, y dejarlo en `asap` escondería la única decisión que la fase pide.
 */
export function newPartialRetirementDraft(): PartialRetirementApi {
  return {
    mode: "at_age",
    starts_at_age: 60,
    income_monthly_today: "",
    expense_basis: "retirement",
  };
}
