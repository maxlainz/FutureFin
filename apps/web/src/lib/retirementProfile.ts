/**
 * Perfil de jubilación POR USUARIO en cliente: defaults, normalización, espejo de las cotas del
 * servidor y constructor del PATCH mínimo (5.0.0, issue #207, decisión D13).
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
 *     `target_retirement_age` o `cash_buffer_months`. Mandar el perfil entero resetearía en
 *     silencio lo que el usuario no tocó: es el bug que el tri-estado del servidor existe para
 *     esquivar, y mandarlo completo lo reintroduciría desde este lado.
 *
 * Las cotas están DUPLICADAS a propósito (aquí y en Rust): son el contrato publicado del
 * formulario. Si cambian allí, cambian aquí — `retirementProfile.test.ts` recorre la tabla
 * entera para que la divergencia sea un test rojo y no un 400 en producción.
 */

import type {
  BridgeDiscountBasisApi,
  FireNumberModeApi,
  PartialExpenseBasisApi,
  PartialRetirementApi,
  PensionPlanApi,
  RetirementProfileApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
  SpendModeApi,
  TargetBasisApi,
  WithdrawalRuleApi,
  WithdrawalRuleKindApi,
} from "../api/types";
import { parseDisplayDecimal } from "./format";

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
/** Colchón de caja máximo, en meses de gasto. Sigue siendo la cota del override explícito por
 *  API/MCP; la SPA ya no ofrece el campo (V6: el colchón se deriva del tope de la regla). */
export const MAX_CASH_BUFFER_MONTHS = 60;
/** Techo del SWR (%). El eje se movió al perfil; la cota no cambió. */
export const MAX_SWR_PCT = 4;
/** Cotas de la edad límite del horizonte (siguen viviendo en `installation.rs`, reusadas aquí). */
export const MIN_HORIZON_LIFESPAN_AGE = 85;
export const MAX_HORIZON_LIFESPAN_AGE = 105;

/** Edades ofrecidas por el selector de horizonte (las mismas cinco de 4.9.0). */
export const HORIZON_LIFESPAN_AGE_OPTIONS = [85, 90, 95, 100, 105] as const;

export const RETIREMENT_STRATEGIES: readonly RetirementStrategyApi[] = [
  "asap",
  "retire_at_age",
  "coast",
  "partial",
  "pension_bridge",
] as const;

/** Nombres de producto (D33). No los inventes en la vista: viven aquí una sola vez. */
export const RETIREMENT_STRATEGY_LABEL: Record<RetirementStrategyApi, string> = {
  asap: "Cuanto antes (FIRE clásico)",
  retire_at_age: "A una edad fija",
  coast: "Ahorrar ahora y dejar crecer (Coast FIRE)",
  partial: "Media jornada",
  pension_bridge: "Puente hasta la pensión",
};

/** Una frase por estrategia — lo que hace, no cómo se implementa. */
export const RETIREMENT_STRATEGY_BLURB: Record<RetirementStrategyApi, string> = {
  asap:
    "Ahorras todo lo que puedes y te jubilas el mes en que tu patrimonio líquido cubre el objetivo.",
  retire_at_age:
    "Eliges la edad; el plan te dice cuánto necesitas ahorrar y cuánto margen te sobra.",
  coast:
    "Aportas fuerte hasta que el capital llegue solo a tu edad objetivo; después, cada euro es margen.",
  partial:
    "Reduces jornada a una edad y cubres el hueco con el capital hasta el cruce total.",
  pension_bridge:
    "Te jubilas por cruce y vives del capital hasta que llegue la pensión pública; el objetivo se dimensiona con ese puente.",
};

export const WITHDRAWAL_RULE_KIND_LABEL: Record<WithdrawalRuleKindApi, string> = {
  fixed_real: "Gasto fijo en euros de hoy",
  percent_of_balance: "Un % del saldo cada año",
  hybrid: "Híbrida (empiezo alto y bajo)",
  guardrails: "Con bandas (Guyton-Klinger)",
};

export const BRIDGE_DISCOUNT_BASIS_LABEL: Record<BridgeDiscountBasisApi, string> = {
  expected_return: "Rentabilidad esperada de tus líquidos",
  swr: "Tu tasa segura de retirada",
  none: "Sin descuento (más conservador)",
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
 * El perfil de quien no ha tocado nada. Reproduce la jubilación de 4.15.x (cruce de líquido,
 * objetivo perpetuo, SWR 3,5 %, horizonte a 90) — el mismo `default_retirement_profile()` del
 * servidor. Si esto se moviera, el formulario propondría por defecto un plan distinto al que la
 * proyección está simulando.
 */
export function defaultRetirementProfileApi(): RetirementProfileApi {
  return {
    strategy: "asap",
    target_retirement_age: null,
    fire_number_mode: "annual_expense",
    fire_number_manual_amount: null,
    swr_pct: "3.5",
    horizon_lifespan_age: 90,
    target_basis: null,
    bridge_discount_basis: "expected_return",
    withdrawal_rule: defaultWithdrawalRuleApi(),
    pension: null,
    partial_retirement: null,
    cash_buffer_months: null,
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

export function parseRetirementStrategy(v: unknown): RetirementStrategyApi {
  return pick(v, RETIREMENT_STRATEGIES, "asap");
}

export function parseTargetBasis(v: unknown): TargetBasisApi | null {
  return v === "perpetuity" || v === "bridge_to_pension" ? v : null;
}

export function parseBridgeDiscountBasis(v: unknown): BridgeDiscountBasisApi {
  return pick(v, ["expected_return", "swr", "none"] as const, "expected_return");
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
 * Espejo de `resolve_retirement_profile`: defaults y clamps en lectura, en el MISMO orden (el
 * horizonte primero, porque es el techo de todas las edades del perfil).
 *
 * Lo que NO hace: derivar `target_basis`. Aquí se conserva tal cual llega —`null` incluido—
 * porque el formulario necesita distinguir «no lo he elegido» de «he elegido esto» para no
 * congelar la derivación del servidor al guardar cualquier otro campo. La derivación para
 * PINTAR vive en `effectiveTargetBasis`.
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

  const pension: PensionPlanApi | null =
    raw.pension && typeof raw.pension === "object"
      ? {
          monthly_amount_today: clampDecimalString(
            raw.pension.monthly_amount_today,
            0,
            Number.MAX_SAFE_INTEGER,
            "0",
          ),
          starts_at_age: clampInt(
            raw.pension.starts_at_age,
            Math.min(MIN_PENSION_AGE, horizon),
            horizon,
            Math.min(MIN_PENSION_AGE, horizon),
          ),
          indexed: raw.pension.indexed !== false,
          fraction_while_partial: clampDecimalString(
            raw.pension.fraction_while_partial,
            0,
            1,
            "0",
          ),
        }
      : null;

  const partial: PartialRetirementApi | null =
    raw.partial_retirement && typeof raw.partial_retirement === "object"
      ? {
          starts_at_age: clampInt(
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
    target_retirement_age:
      raw.target_retirement_age == null
        ? null
        : clampInt(raw.target_retirement_age, MIN_PROFILE_AGE, horizon, MIN_PROFILE_AGE),
    fire_number_mode: parseFireNumberMode(raw.fire_number_mode),
    fire_number_manual_amount:
      raw.fire_number_manual_amount == null ||
      String(raw.fire_number_manual_amount).trim() === ""
        ? null
        : String(raw.fire_number_manual_amount),
    swr_pct: clampDecimalString(raw.swr_pct, 0, MAX_SWR_PCT, base.swr_pct),
    horizon_lifespan_age: horizon,
    target_basis: parseTargetBasis(raw.target_basis),
    bridge_discount_basis: parseBridgeDiscountBasis(raw.bridge_discount_basis),
    withdrawal_rule: normalizeWithdrawalRule(raw.withdrawal_rule),
    pension,
    partial_retirement: partial,
    cash_buffer_months:
      raw.cash_buffer_months == null
        ? null
        : clampInt(raw.cash_buffer_months, 0, MAX_CASH_BUFFER_MONTHS, 0),
  };
}

/**
 * La base del objetivo que se APLICA, derivada igual que el servidor (R6): `pension_bridge` la
 * fuerza a puente; sin elección explícita, hay puente si hay pensión declarada y perpetuidad si
 * no. Es lo que se pinta como seleccionado en el radio.
 */
export function effectiveTargetBasis(p: RetirementProfileApi): TargetBasisApi {
  if (p.strategy === "pension_bridge") return "bridge_to_pension";
  if (p.target_basis) return p.target_basis;
  return p.pension ? "bridge_to_pension" : "perpetuity";
}

/** De dónde sale la base del objetivo que se está aplicando. */
export type TargetBasisSource =
  /** El usuario la eligió a mano y está guardada. */
  | "stored"
  /** Nadie la ha elegido: la deriva el servidor (R6) y se mueve sola al declarar una pensión. */
  | "derived"
  /** La estrategia la impone (`pension_bridge` ES el puente); la elección guardada no pinta nada. */
  | "forced_by_strategy";

/**
 * Origen de `effectiveTargetBasis`, para que el formulario pueda rotular «(derivada)» en vez de
 * enseñar una elección que nadie hizo.
 *
 * Sin esta distinción, el radio marcado se lee como una decisión del usuario: la reenviaría en el
 * primer PATCH, y declarar una pensión después ya no movería la base del objetivo, que se quedaría
 * en la perpetuidad conservadora que nadie pidió. **Exige que `p.target_basis` lleve la elección
 * ALMACENADA** (`withStoredTargetBasis`), no la resuelta que publica el servidor.
 */
export function targetBasisSource(p: RetirementProfileApi): TargetBasisSource {
  if (p.strategy === "pension_bridge") return "forced_by_strategy";
  return p.target_basis == null ? "derived" : "stored";
}

/**
 * El perfil que llega del servidor, con `target_basis` sustituido por la elección **ALMACENADA**
 * (`target_basis_stored` de la respuesta; `null` = derivada).
 *
 * `profile.target_basis` viaja siempre RESUELTO, así que un formulario que lo use como estado
 * pierde la única información que necesita para no congelar la derivación al guardar cualquier
 * otro campo. Lo que se pinta sigue saliendo de `effectiveTargetBasis`, que deriva con la misma
 * regla R6 que el servidor: la sustitución no cambia lo que ve el usuario, solo lo que se manda.
 *
 * `stored === undefined` = backend anterior a WP5-2, que no publica la elección almacenada: se
 * conserva el valor resuelto (el comportamiento de 4.15.x) porque inventar un `null` diría
 * «derivada» sobre una elección que quizá sí existe.
 */
export function withStoredTargetBasis(
  profile: RetirementProfileApi,
  stored: TargetBasisApi | null | undefined,
): RetirementProfileApi {
  if (stored === undefined) return profile;
  return { ...profile, target_basis: stored };
}

/** `true` para las estrategias cuyo trigger es una EDAD (y que por tanto la exigen). */
export function strategyRequiresTargetAge(s: RetirementStrategyApi): boolean {
  return s === "retire_at_age" || s === "coast";
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

  // --- Estrategia ------------------------------------------------------------------------
  if (strategyRequiresTargetAge(p.strategy) && p.target_retirement_age == null) {
    return "target_retirement_age_required";
  }
  if (p.strategy === "pension_bridge" && p.pension == null) {
    return "pension_required_for_bridge";
  }

  // --- Edades ----------------------------------------------------------------------------
  if (p.target_retirement_age != null) {
    const a = p.target_retirement_age;
    if (!Number.isInteger(a) || a < MIN_PROFILE_AGE || a > horizon) {
      return "retirement_age_out_of_range";
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
  }
  if (p.partial_retirement) {
    const a = p.partial_retirement.starts_at_age;
    if (!Number.isInteger(a) || a < MIN_PROFILE_AGE || a > horizon) {
      return "partial_age_out_of_range";
    }
    // Un ingreso vacío es un año sabático declarado (0 €/mes), no un error: el bloque de media
    // jornada existe precisamente para poder no cobrar nada durante la fase.
    const inc = decimalOrZero(p.partial_retirement.income_monthly_today);
    if (inc === null) return "decimal_invalid";
    if (inc < 0) return "partial_income_negative";
    if (p.target_retirement_age != null && a >= p.target_retirement_age) {
      return "partial_not_before_retirement";
    }
  }

  // --- Colchón ---------------------------------------------------------------------------
  //
  // El UMBRAL de éxito ya no se valida aquí porque ya no existe (V7): el corte es fijo al 100 %
  // y lo decide el servidor. El PATCH sigue tolerando el campo por compatibilidad, pero la SPA
  // no lo escribe, así que no hay nada que este espejo pueda rechazar.
  if (p.cash_buffer_months != null) {
    if (!Number.isInteger(p.cash_buffer_months) || p.cash_buffer_months < 0) {
      return "cash_buffer_out_of_range";
    }
    if (p.cash_buffer_months > MAX_CASH_BUFFER_MONTHS) return "cash_buffer_out_of_range";
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
    sameDecimalZeroDefault(a.fraction_while_partial, b.fraction_while_partial)
  );
}

function samePartial(
  a: PartialRetirementApi | null,
  b: PartialRetirementApi | null,
): boolean {
  if (a == null || b == null) return a === b;
  return (
    a.starts_at_age === b.starts_at_age &&
    sameDecimalZeroDefault(a.income_monthly_today, b.income_monthly_today) &&
    a.expense_basis === b.expense_basis
  );
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
 *  * **`target_basis` solo viaja si el borrador lo cambia.** El servidor lo DERIVA cuando está
 *    sin fijar (R6); mandarlo en cada PATCH congelaría esa derivación con el valor que se estaba
 *    enseñando, y al declarar después una pensión el objetivo se quedaría en perpetuidad —la
 *    opción conservadora que nadie pidió— sin ningún aviso. Para que esa comparación signifique
 *    «el usuario ha tocado el radio» y no «el servidor resolvió otra cosa», los dos lados deben
 *    llevar la elección ALMACENADA (`withStoredTargetBasis`): con el valor RESUELTO en `before`,
 *    un perfil derivado a `perpetuity` y un radio que el usuario marca en `perpetuity` se ven
 *    iguales y la fijación explícita no se manda nunca. El `null` del borrador («volver a la
 *    derivada») viaja como `null` explícito, que es lo que el tri-estado del servidor espera.
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
  if (before.target_basis !== after.target_basis) patch.target_basis = after.target_basis;
  if (before.bridge_discount_basis !== after.bridge_discount_basis) {
    patch.bridge_discount_basis = after.bridge_discount_basis;
  }
  if (!sameWithdrawalRule(before.withdrawal_rule, after.withdrawal_rule)) {
    patch.withdrawal_rule = withdrawalRuleForWire(after.withdrawal_rule);
  }
  if (!samePension(before.pension, after.pension)) {
    patch.pension = after.pension
      ? {
          ...after.pension,
          monthly_amount_today: decimalStringForWire(after.pension.monthly_amount_today),
          fraction_while_partial: decimalStringForWire(
            after.pension.fraction_while_partial,
          ),
        }
      : null;
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
  if (before.cash_buffer_months !== after.cash_buffer_months) {
    patch.cash_buffer_months = after.cash_buffer_months;
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

/** Bloque de pensión de partida al activar la casilla (importe vacío: lo pone el usuario). */
export function newPensionPlanDraft(): PensionPlanApi {
  return {
    monthly_amount_today: "",
    starts_at_age: 67,
    indexed: true,
    fraction_while_partial: "0",
  };
}

/** Bloque de media jornada de partida al elegir la estrategia o activar la casilla. */
export function newPartialRetirementDraft(): PartialRetirementApi {
  return {
    starts_at_age: 60,
    income_monthly_today: "",
    expense_basis: "retirement",
  };
}
