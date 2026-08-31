/**
 * Principal derivado del plan de pago por modelo de amortización (4.2.0), lado cliente.
 *
 * Dos bloques:
 *
 * 1. **Paridad** contra `apps/api/tests/fixtures/liability-derived-principal-parity.json`, el mismo
 *    JSON que consume `apps/api/tests/liability_derived_principal_parity.rs`. La vista previa del
 *    formulario calcula en `number` (f64) lo que el servidor calcula en `Decimal`: si las dos
 *    fórmulas se separan, uno de los dos suites falla. El JSON es la fuente de verdad.
 * 2. **Puertas de la vista previa**: qué combinaciones NO producen número. La regla es que la
 *    previsualización calla siempre que el POST fuese a devolver 400 — enseñar un importe que el
 *    guardado no va a producir es peor que no enseñar nada.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { LiabilityRepaymentModelApi,
  LiabilityApiRow,
} from "../api/types";
import {
  REPAYMENT_MODEL_LABEL,
  REPAYMENT_MODEL_ORDER,
  liabilitiesApproxMonthlyInterestSum,
  liabilitiesWeightedAprPercent,
  liabilityAccruesInterest,
  liabilityDerivedPrincipalNum,
  liabilityDerivedPrincipalPreview,
  presentValueOfPayments,
} from "./ledger";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/liability-derived-principal-parity.json",
);

type Case = {
  name: string;
  repayment_model: LiabilityRepaymentModelApi;
  payment_amount: string;
  intervals: number;
  apr_percent: string | null;
  expected_principal: number;
};

function loadFixture(): { cases: Case[]; tolerance: number } {
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  const fixture = JSON.parse(raw) as {
    cases: Case[];
    _tolerance_eur: number;
  };
  return { cases: fixture.cases, tolerance: fixture._tolerance_eur };
}

describe("principal derivado — paridad cliente vs fixture compartido", () => {
  const { cases, tolerance } = loadFixture();

  it("carga los casos canónicos", () => {
    expect(cases.length).toBeGreaterThanOrEqual(5);
  });

  for (const c of cases) {
    it(`caso "${c.name}" coincide ±${tolerance} €`, () => {
      const actual = liabilityDerivedPrincipalNum(
        Number(c.payment_amount),
        c.intervals,
        c.repayment_model,
        c.apr_percent === null ? null : Number(c.apr_percent),
      );
      expect(actual).not.toBeNull();
      expect(Math.abs((actual as number) - c.expected_principal)).toBeLessThanOrEqual(
        tolerance,
      );
    });
  }
});

describe("present value — casos frontera de la fórmula", () => {
  it("TIN ausente o ≤ 0 degenera en Σ cuotas, sin pasar por la potencia", () => {
    // Límite exacto de P = M·(1−(1+i)^−n)/i cuando i → 0. Mismo atajo que el engine: calcularlo
    // con la transcendental metería error de redondeo en el caso más común.
    expect(presentValueOfPayments(500, 200, null)).toBe(100000);
    expect(presentValueOfPayments(500, 200, 0)).toBe(100000);
    expect(presentValueOfPayments(500, 200, -3)).toBe(100000);
  });

  it("el valor actual es siempre menor que la suma de cuotas con TIN > 0", () => {
    expect(presentValueOfPayments(500, 200, 3)).toBeLessThan(500 * 200);
  });

  it("a mayor TIN, menor principal para el mismo plan", () => {
    const low = presentValueOfPayments(800, 360, 2);
    const high = presentValueOfPayments(800, 360, 6);
    expect(high).toBeLessThan(low);
  });
});

describe("liabilityDerivedPrincipalNum — modelos que no derivan", () => {
  it("interest_only y revolving no producen número (derive_not_supported_for_model)", () => {
    expect(liabilityDerivedPrincipalNum(500, 200, "interest_only", 3)).toBeNull();
    expect(liabilityDerivedPrincipalNum(500, 200, "revolving", 3)).toBeNull();
  });

  it("french sin TIN > 0 no produce número (apr_required_for_model)", () => {
    expect(liabilityDerivedPrincipalNum(500, 200, "french", null)).toBeNull();
    expect(liabilityDerivedPrincipalNum(500, 200, "french", 0)).toBeNull();
  });

  it("cuota o intervalos no positivos no producen número", () => {
    expect(liabilityDerivedPrincipalNum(0, 200, "fixed_payments", null)).toBeNull();
    expect(liabilityDerivedPrincipalNum(500, 0, "fixed_payments", null)).toBeNull();
  });
});

describe("liabilityDerivedPrincipalPreview — puertas de la vista previa", () => {
  // Fecha muy lejana: el número exacto de intervalos depende de «hoy», así que los tests solo
  // afirman si HAY o NO HAY vista previa, nunca su importe (eso lo fija la paridad de arriba).
  const FAR = "2099-12-31";

  it("sin modelo explícito conserva el comportamiento histórico (Σ cuotas)", () => {
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", FAR, "UTC", "EUR"),
    ).not.toBeNull();
  });

  it("weekly solo se previsualiza en fixed_payments", () => {
    expect(
      liabilityDerivedPrincipalPreview("500", "weekly", FAR, "UTC", "EUR", "fixed_payments"),
    ).not.toBeNull();
    expect(
      liabilityDerivedPrincipalPreview("500", "weekly", FAR, "UTC", "EUR", "french", "3"),
    ).toBeNull();
  });

  it("french previsualiza con TIN > 0 y calla sin él", () => {
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", FAR, "UTC", "EUR", "french", "3"),
    ).not.toBeNull();
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", FAR, "UTC", "EUR", "french", ""),
    ).toBeNull();
  });

  it("interest_only y revolving nunca previsualizan", () => {
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", FAR, "UTC", "EUR", "interest_only", "3"),
    ).toBeNull();
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", FAR, "UTC", "EUR", "revolving", "3"),
    ).toBeNull();
  });

  it("sin plan (frecuencia o fecha fin) no hay vista previa", () => {
    expect(liabilityDerivedPrincipalPreview("500", "", FAR, "UTC", "EUR")).toBeNull();
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", "  ", "UTC", "EUR"),
    ).toBeNull();
  });

  it("una fecha fin anterior a hoy no produce vista previa", () => {
    expect(
      liabilityDerivedPrincipalPreview("500", "monthly", "2000-01-01", "UTC", "EUR"),
    ).toBeNull();
  });
});

describe("etiquetas de modelo", () => {
  it("hay etiqueta para los cuatro modelos del wire y el orden los cubre todos", () => {
    expect(REPAYMENT_MODEL_ORDER).toHaveLength(4);
    // INVERTIDO en 4.7.0 (#144): el primero era `fixed_payments` (el default histórico);
    // desde la Ola 3 el francés encabeza el select porque ES el default y el préstamo típico.
    expect(REPAYMENT_MODEL_ORDER[0]).toBe("french");
    for (const m of REPAYMENT_MODEL_ORDER) {
      expect(REPAYMENT_MODEL_LABEL[m]).toBeTruthy();
    }
  });
});

describe("liabilityAccruesInterest — el predicado único de #121", () => {
  const TODAY = "2026-08-31";
  const base: LiabilityApiRow = {
    id: "l1",
    category_id: "c1",
    label: "Hipoteca",
    type_tag: null,
    repayment_model: "french",
    principal: "50000.0000",
    apr_percent: "5.0000",
    payment_amount: "300.0000",
    payment_frequency: "monthly",
    payment_end_date: "2031-01-01",
    plan_expired_with_balance: false,
    min_payment_pct: null,
    min_payment_eur: null,
    notes: null,
    sort_index: 0,
  };

  it("francés con TIN y plan vivo devenga", () => {
    expect(liabilityAccruesInterest(base, TODAY)).toBe(true);
  });

  it("el modelo sin intereses nunca devenga", () => {
    expect(
      liabilityAccruesInterest(
        { ...base, repayment_model: "fixed_payments", apr_percent: null },
        TODAY,
      ),
    ).toBe(false);
  });

  it("sin TIN (o TIN 0) no devenga", () => {
    expect(liabilityAccruesInterest({ ...base, apr_percent: null }, TODAY)).toBe(false);
    expect(liabilityAccruesInterest({ ...base, apr_percent: "0" }, TODAY)).toBe(false);
  });

  it("plan vencido (fin < hoy) no devenga — el saldo queda congelado (#145)", () => {
    expect(
      liabilityAccruesInterest({ ...base, payment_end_date: "2026-08-30" }, TODAY),
    ).toBe(false);
    // El día exacto del fin todavía cuenta (mismo `>=` que el engine y el filtro SQL).
    expect(
      liabilityAccruesInterest({ ...base, payment_end_date: "2026-08-31" }, TODAY),
    ).toBe(true);
  });

  it("sin cuota no hay plan vivo, no devenga", () => {
    expect(liabilityAccruesInterest({ ...base, payment_amount: null }, TODAY)).toBe(false);
  });
});

describe("las dos KPIs de Pasivos filtran por el predicado (#121)", () => {
  const TZ = "UTC";
  const mkRow = (over: Partial<LiabilityApiRow>): LiabilityApiRow => ({
    id: "x",
    category_id: "c1",
    label: "L",
    type_tag: null,
    repayment_model: "french",
    principal: "50000.0000",
    apr_percent: "5.0000",
    payment_amount: "300.0000",
    payment_frequency: "monthly",
    payment_end_date: "2099-01-01",
    plan_expired_with_balance: false,
    min_payment_pct: null,
    min_payment_eur: null,
    notes: null,
    sort_index: 0,
    ...over,
  });

  it("el TIN medio ponderado ignora el plan vencido: 5 % vencido + 10 % vivo ⇒ 10,0000 %", () => {
    // A mano, el pin del spike: dos pasivos de 50.000 € — el del 5 % con plan vencido queda
    // fuera del cálculo ENTERO, así que la media es la del vivo al 10 %. Hasta 4.6.0 salía
    // 7,5 % (promediaba números declarados, no coste real).
    const rows = [
      mkRow({ id: "a", apr_percent: "5", payment_end_date: "2020-01-01" }),
      mkRow({ id: "b", apr_percent: "10" }),
    ];
    expect(liabilitiesWeightedAprPercent(rows, TZ)).toBeCloseTo(10.0, 4);
  });

  it("el interés mensual aprox. cobra solo lo que devenga: 100.000 al 6 % vivo ⇒ 500,00 €", () => {
    // A mano: 100.000 × 6 % ÷ 12 = 500. El sin-intereses de 40.000 y el vencido de 9.999 al
    // 5 % suman 0 — la misma base que la simulación.
    const rows = [
      mkRow({ id: "a", principal: "100000", apr_percent: "6" }),
      mkRow({
        id: "b",
        principal: "40000",
        repayment_model: "fixed_payments",
        apr_percent: null,
      }),
      mkRow({ id: "c", principal: "9999", apr_percent: "5", payment_end_date: "2020-01-01" }),
    ];
    expect(liabilitiesApproxMonthlyInterestSum(rows, TZ)).toBeCloseTo(500.0, 6);
  });
});

describe("liabilityDerivedPrincipalNum — la rama única de 4.7.0", () => {
  it("fixed_payments con TIN > 0 ya no deriva: el servidor lo rechaza (apr_forbidden_for_model)", () => {
    expect(liabilityDerivedPrincipalNum(500, 200, "fixed_payments", 3)).toBeNull();
  });
});
