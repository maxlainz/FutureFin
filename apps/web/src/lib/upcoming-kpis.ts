/**
 * KPIs de la pestaña «Simulación» (antes «Próximos»), calculados en cliente sobre las filas.
 *
 * Dos reglas, las dos con su espejo en el servidor (`planning_flow_totals_in_out`,
 * `apps/api/src/handlers/summary.rs`):
 *
 * - **Solo cuentan los flujos ENCENDIDOS** (`enabled`, 5.0.0). Un apagado sigue en la lista, pero
 *   no entra en ninguna de estas cifras — igual que no entra en la proyección.
 * - **Los totales en € suman SOLO puntuales** (#148): el `expected_amount` de un recurrente son
 *   €/MES y mezclarlo con un total sería un error de magnitud. Su neto va aparte, al mes.
 */
import type { PlanningFlowApiRow } from "../api/types";
import { parseDisplayDecimal } from "./format";

export type UpcomingKpis = {
  /** Σ entradas puntuales encendidas, €. */
  inflowSum: number;
  /** Σ salidas puntuales encendidas, €. */
  outflowSum: number;
  /** `inflowSum − outflowSum`, €. */
  netTotal: number;
  /** Nº de recurrentes encendidos: decide si se pinta la tarjeta «Recurrentes». */
  recurringCount: number;
  /** Entradas − salidas recurrentes encendidas, €/MES. */
  recurringMonthlyNet: number;
};

export function computeUpcomingKpis(flows: readonly PlanningFlowApiRow[]): UpcomingKpis {
  let inflowSum = 0;
  let outflowSum = 0;
  let recurringCount = 0;
  let recurringMonthlyNet = 0;
  for (const f of flows) {
    if (!f.enabled) continue;
    const amt = parseDisplayDecimal(f.expected_amount) ?? 0;
    const sign = f.direction === "inflow" ? 1 : f.direction === "outflow" ? -1 : 0;
    if (f.amount_basis === "per_month") {
      recurringCount += 1;
      recurringMonthlyNet += sign * amt;
    } else if (sign > 0) {
      inflowSum += amt;
    } else if (sign < 0) {
      outflowSum += amt;
    }
  }
  return {
    inflowSum,
    outflowSum,
    netTotal: inflowSum - outflowSum,
    recurringCount,
    recurringMonthlyNet,
  };
}
