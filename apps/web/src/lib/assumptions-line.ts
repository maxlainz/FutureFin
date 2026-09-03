/**
 * La línea «Supuestos» (5.0.0, rediseño UX U1a; decisión **U12** de #207).
 *
 * U12 es el contrapeso exacto de U2. U2 esconde los campos que la estrategia no usa; sin nada
 * más, esconder un campo es **forzar su valor en silencio** — el usuario deja de ver que su plan
 * asume una tasa de retirada del 3,5 %, un horizonte a los 90 y un umbral de éxito del 95 %
 * porque nadie se los está preguntando. La regla de la casa es la contraria: nada se fuerza sin
 * decirlo.
 *
 * De ahí esta línea, **siempre visible**, que enuncia todos los supuestos en vigor:
 *
 * > Supuestos: retirada 3,5 % · gasto fijo en euros de hoy · horizonte 90 años · sin colchón ·
 * > umbral 95,0 %
 *
 * Y de ahí también que se construya **desde `lib/plan-fields.ts`** y no desde una lista propia:
 * un supuesto entra en la frase si y solo si su campo es visible en el formulario con este
 * perfil. Con dos listas independientes, añadir un eje nuevo dejaría una de las dos atrás y el
 * usuario vería un formulario y una frase que no describen el mismo plan — que es justo el fallo
 * que U12 existe para impedir.
 *
 * Todo porcentaje pasa por `formatPercentAmount` (un decimal y « %» detrás, §Formato de cifras
 * del design system): «95 %» escrito a mano acabaría siendo el único porcentaje de la app con
 * otra convención.
 */

import type {
  BridgeDiscountBasisApi,
  PartialExpenseBasisApi,
  RetirementProfileApi,
  SpendModeApi,
  TargetBasisApi,
  WithdrawalRuleApi,
} from "../api/types";
import { formatFractionAsPercent, formatPercentAmount, parseDisplayDecimal } from "./format";
import { isFieldVisible, planFieldsContextFromProfile } from "./plan-fields";
import { targetBasisSource, type TargetBasisSource } from "./retirementProfile";

export type AssumptionsContext = {
  /** El único hecho del contexto de campos que no vive en el perfil. */
  hasBirthDate: boolean;
  /**
   * De dónde sale la base del objetivo. Por defecto se deriva del propio perfil con
   * `targetBasisSource`, que **exige que `target_basis` lleve la elección ALMACENADA**
   * (`withStoredTargetBasis`): con el valor RESUELTO que publica el servidor, una base derivada
   * se anunciaría como elegida y la frase mentiría sobre quién tomó la decisión.
   */
  targetBasisSource?: TargetBasisSource;
};

/** Nombres CORTOS de la base — la línea es una enumeración, no un formulario. */
const BASIS_SHORT: Record<TargetBasisApi, string> = {
  perpetuity: "renta perpetua",
  bridge_to_pension: "puente",
};

/** Ídem para el descuento del puente (los rótulos largos viven en `retirementProfile.ts`). */
const DISCOUNT_SHORT: Record<BridgeDiscountBasisApi, string> = {
  expected_return: "rentabilidad esperada",
  swr: "tu tasa segura de retirada",
  none: "ninguno",
};

const SPEND_MODE_SHORT: Record<SpendModeApi, string> = {
  ceiling: "techo: retiras como mucho la regla",
  rule_is_spend: "la regla es tu gasto",
};

const PARTIAL_EXPENSE_SHORT: Record<PartialExpenseBasisApi, string> = {
  retirement: "gasto en media jornada: el de jubilación",
  regular: "gasto en media jornada: el de ahora",
};

/**
 * La regla de retirada en una cláusula.
 *
 * **U4 dentro de la frase**: la regla NO repite su porcentaje — el único porcentaje de retirada
 * de la app es `swr_pct`, que ya va delante. La híbrida solo añade a cuánto BAJA y las bandas su
 * banda y su ajuste, que son los dos parámetros que no son «el porcentaje de retirada».
 */
function withdrawalClause(rule: WithdrawalRuleApi): string {
  switch (rule.kind) {
    case "fixed_real":
      return "gasto fijo en euros de hoy";
    case "percent_of_balance":
      return "un % del saldo cada año";
    case "hybrid":
      return rule.end_pct == null || String(rule.end_pct).trim() === ""
        ? "híbrida"
        : `híbrida: baja al ${formatPercentAmount(String(rule.end_pct))}`;
    case "guardrails": {
      const band = rule.band_pct == null ? null : formatPercentAmount(String(rule.band_pct));
      const adj = rule.adjust_pct == null ? null : formatPercentAmount(String(rule.adjust_pct));
      if (band == null && adj == null) return "con bandas";
      if (adj == null) return `con bandas: ±${band}`;
      if (band == null) return `con bandas: ajuste ${adj}`;
      return `con bandas: ±${band}, ajuste ${adj}`;
    }
  }
}

function cashBufferClause(months: number | null): string {
  if (months == null || !Number.isFinite(months) || months <= 0) return "sin colchón";
  return `colchón ${months} ${months === 1 ? "mes" : "meses"}`;
}

/**
 * Los supuestos en vigor, ya en texto y en el mismo orden en que aparecen sus campos en el
 * formulario (`planFields`): retirada → objetivo → pensión → media jornada → horizonte → riesgo.
 *
 * Se publica aparte de `assumptionsLine` para que un test pueda comprobar qué entra y qué no sin
 * pelearse con los separadores, y para que la vista pueda pintarlos como chips si algún día hace
 * falta.
 */
export function assumptionParts(
  profile: RetirementProfileApi,
  ctx: AssumptionsContext,
): string[] {
  const fieldCtx = planFieldsContextFromProfile(profile, ctx.hasBirthDate);
  const visible = (id: Parameters<typeof isFieldVisible>[0]) => isFieldVisible(id, fieldCtx);
  const out: string[] = [];

  // ── retirada ─────────────────────────────────────────────────────────────────────────────
  out.push(`retirada ${formatPercentAmount(profile.swr_pct)}`);
  out.push(withdrawalClause(profile.withdrawal_rule));
  if (visible("spend_mode")) {
    out.push(SPEND_MODE_SHORT[profile.withdrawal_rule.spend_mode]);
  }

  // ── objetivo ─────────────────────────────────────────────────────────────────────────────
  if (visible("target_basis")) {
    const source = ctx.targetBasisSource ?? targetBasisSource(profile);
    const suffix = source === "derived" ? " (derivada)" : "";
    out.push(`base: ${BASIS_SHORT[fieldCtx.effectiveBasis]}${suffix}`);
  }
  if (visible("bridge_discount_basis")) {
    out.push(`descuento: ${DISCOUNT_SHORT[profile.bridge_discount_basis]}`);
  }

  // ── pensión ──────────────────────────────────────────────────────────────────────────────
  if (visible("pension_indexed")) {
    out.push(profile.pension?.indexed === false ? "pensión sin indexar" : "pensión indexada");
  }
  if (visible("pension_fraction_while_partial")) {
    const raw = profile.pension?.fraction_while_partial ?? "0";
    const n = parseDisplayDecimal(String(raw));
    out.push(
      n == null || n <= 0
        ? "sin pensión durante la media jornada"
        : `pensión durante la media jornada: ${formatFractionAsPercent(String(raw))}`,
    );
  }

  // ── media jornada ────────────────────────────────────────────────────────────────────────
  if (visible("partial_expense_basis")) {
    out.push(PARTIAL_EXPENSE_SHORT[profile.partial_retirement?.expense_basis ?? "retirement"]);
  }

  // ── horizonte y riesgo ───────────────────────────────────────────────────────────────────
  out.push(`horizonte ${profile.horizon_lifespan_age} años`);
  out.push(cashBufferClause(profile.cash_buffer_months));
  out.push(`umbral ${formatPercentAmount(String(profile.success_threshold_pct))}`);

  return out;
}

/** La línea completa, con su prefijo. Siempre devuelve algo: nunca hay «ningún supuesto». */
export function assumptionsLine(
  profile: RetirementProfileApi,
  ctx: AssumptionsContext,
): string {
  return `Supuestos: ${assumptionParts(profile, ctx).join(" · ")}`;
}
