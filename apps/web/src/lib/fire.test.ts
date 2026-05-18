/**
 * Fase 4.6 — Paridad cliente↔servidor sobre el cálculo FIRE.
 *
 * Carga los mismos casos canónicos que `apps/api/tests/fire_parity.rs`. Para cada uno reproduce
 * la fórmula completa del cliente (`computeFireAnnualNeedNetEur` + `grossUpNetAnnualFire`) y
 * compara con `expected_target_nw` (± 1 €).
 *
 * Si alguien toca los tramos fiscales o la fórmula en `lib/fire.ts` y no actualiza el JSON, o
 * viceversa cambia el servidor sin actualizar el JSON, uno de los dos suites falla. El JSON es
 * la fuente de verdad.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { FireSettingsApi } from "../api/types";
import { computeFireAnnualNeedNetEur, grossUpNetAnnualFire } from "./fire";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/fire-parity.json",
);

type Case = {
  name: string;
  fire_settings: FireSettingsApi;
  monthly: {
    income: string;
    income_retirement: string;
    expense_retirement: string;
  };
  expected_target_nw: number | null;
};

function loadCases(): Case[] {
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  const fixture = JSON.parse(raw) as { cases: Case[] };
  return fixture.cases;
}

/** Reproduce el contrato del servidor: gross_up(need_annual) / (swr/100). */
function clientTargetNw(c: Case): number | null {
  const fs = c.fire_settings;
  // El cliente DEBE pasar expense_retirement (no expense_regular) para alinearse con el
  // servidor. RetirementView lo hace tras Fase 4.6.
  const need = computeFireAnnualNeedNetEur(
    fs,
    c.monthly.expense_retirement,
    c.monthly.income,
    c.monthly.income_retirement,
  );
  if (need === null || need <= 0) return null;
  const swr = Number(fs.swr_pct);
  if (!Number.isFinite(swr) || swr <= 0) return null;
  const gross = grossUpNetAnnualFire(need, fs.tax_brackets, fs.taxes_enabled);
  return gross / (swr / 100);
}

describe("FIRE parity (cliente vs fixture compartido)", () => {
  const cases = loadCases();

  it("loads at least 4 canonical cases", () => {
    expect(cases.length).toBeGreaterThanOrEqual(4);
  });

  for (const c of cases) {
    it(`case "${c.name}" matches ±1 €`, () => {
      const actual = clientTargetNw(c);
      if (c.expected_target_nw === null) {
        expect(actual).toBeNull();
        return;
      }
      expect(actual).not.toBeNull();
      const diff = Math.abs((actual as number) - c.expected_target_nw);
      expect(diff).toBeLessThanOrEqual(1.0);
    });
  }
});
