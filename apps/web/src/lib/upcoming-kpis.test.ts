import { describe, expect, it } from "vitest";
import type { PlanningFlowApiRow } from "../api/types";
import { computeUpcomingKpis } from "./upcoming-kpis";

function flow(over: Partial<PlanningFlowApiRow>): PlanningFlowApiRow {
  return {
    id: "f",
    category_id: "c",
    direction: "outflow",
    title: "x",
    expected_amount: "100",
    amount_basis: "one_off",
    due_date: null,
    notes: null,
    sort_index: 0,
    show_in_chart: false,
    enabled: true,
    ...over,
  };
}

describe("computeUpcomingKpis", () => {
  it("suma puntuales en € y recurrentes al mes por separado", () => {
    const k = computeUpcomingKpis([
      flow({ direction: "inflow", expected_amount: "900" }),
      flow({ direction: "outflow", expected_amount: "5000" }),
      flow({ direction: "outflow", expected_amount: "400", amount_basis: "per_month" }),
      flow({ direction: "inflow", expected_amount: "150", amount_basis: "per_month" }),
    ]);
    expect(k.inflowSum).toBe(900);
    expect(k.outflowSum).toBe(5000);
    expect(k.netTotal).toBe(-4100);
    expect(k.recurringCount).toBe(2);
    expect(k.recurringMonthlyNet).toBe(-250);
  });

  it("los flujos apagados no cuentan en ninguna cifra", () => {
    const k = computeUpcomingKpis([
      flow({ direction: "inflow", expected_amount: "900", enabled: false }),
      flow({ direction: "outflow", expected_amount: "5000" }),
      flow({ expected_amount: "400", amount_basis: "per_month", enabled: false }),
    ]);
    expect(k.inflowSum).toBe(0);
    expect(k.outflowSum).toBe(5000);
    expect(k.netTotal).toBe(-5000);
    expect(k.recurringCount).toBe(0);
    expect(k.recurringMonthlyNet).toBe(0);
  });

  it("todo apagado = todo a cero", () => {
    const k = computeUpcomingKpis([flow({ enabled: false }), flow({ enabled: false, direction: "inflow" })]);
    expect(k).toEqual({
      inflowSum: 0,
      outflowSum: 0,
      netTotal: 0,
      recurringCount: 0,
      recurringMonthlyNet: 0,
    });
  });
});
