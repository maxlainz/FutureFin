/**
 * Normalización del JSONB de fire_settings recibido de la API. Cubre en concreto el eje
 * `savings_source` (v1.9.0): ausente / válido / desconocido → default `budget`.
 */

import { describe, expect, it } from "vitest";
import type { FireSettingsApi, SavingsAvgBasisApi } from "../api/types";
import {
  defaultFireSettingsApi,
  normalizeInstallationFireSettings,
  parseSavingsSource,
  runwaySwrParenthetical,
  savingsBasisParenthetical,
  savingsSourceUsesTransactions,
} from "./fire";
import { defaultRetirementProfileApi } from "./retirementProfile";

describe("normalizeInstallationFireSettings — savings_source", () => {
  it("default settings incluyen savings_source = budget", () => {
    expect(defaultFireSettingsApi().savings_source).toBe("budget");
  });

  it("null/undefined → default (budget)", () => {
    expect(normalizeInstallationFireSettings(null).savings_source).toBe(
      "budget",
    );
    expect(normalizeInstallationFireSettings(undefined).savings_source).toBe(
      "budget",
    );
  });

  it("campo ausente → budget", () => {
    const raw = {
      taxes_enabled: true,
      tax_brackets: [{ up_to: null, pct: "30" }],
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe("budget");
  });

  it("un fire_settings de 4.15.x con los cuatro ejes mudados no los arrastra (5.0.0)", () => {
    // El JSONB guardado por 4.15.x traía modo, importe manual, SWR y edad límite dentro. Desde
    // 5.0.0 son personales (perfil de jubilación, D13): el normalizador debe IGNORARLOS, no
    // reexportarlos, o el PATCH del hogar los volvería a escribir donde ya no viven.
    const out = normalizeInstallationFireSettings({
      fire_number_mode: "manual",
      fire_number_manual_amount: "500000",
      swr_pct: "3.0",
      horizon_lifespan_age: 100,
      taxes_enabled: true,
      tax_brackets: [{ up_to: null, pct: "30" }],
    } as unknown as FireSettingsApi);
    expect(Object.keys(out).sort()).toEqual(
      [
        "expense_avg_window_mode",
        "expense_avg_window_months",
        "income_avg_window_mode",
        "income_avg_window_months",
        "savings_source",
        "tax_brackets",
        "taxable_gain_ratio",
        "taxes_enabled",
      ].sort(),
    );
  });

  it("valor válido transactions_avg se preserva", () => {
    const raw = {
      ...defaultFireSettingsApi(),
      savings_source: "transactions_avg",
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe(
      "transactions_avg",
    );
  });

  it("valor válido budget se preserva", () => {
    const raw = {
      ...defaultFireSettingsApi(),
      savings_source: "budget",
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe("budget");
  });

  it("valor válido budget_income_real_expense se preserva", () => {
    const raw = {
      ...defaultFireSettingsApi(),
      savings_source: "budget_income_real_expense",
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe(
      "budget_income_real_expense",
    );
  });

  it("valor desconocido → budget", () => {
    const raw = {
      ...defaultFireSettingsApi(),
      // Fuerza un valor fuera del enum para simular un backend futuro / dato corrupto.
      savings_source: "monte_carlo" as unknown,
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe("budget");
  });
});

describe("parseSavingsSource", () => {
  it("preserva los 3 valores válidos", () => {
    expect(parseSavingsSource("budget")).toBe("budget");
    expect(parseSavingsSource("transactions_avg")).toBe("transactions_avg");
    expect(parseSavingsSource("budget_income_real_expense")).toBe(
      "budget_income_real_expense",
    );
  });

  it("desconocido / vacío / null → budget", () => {
    expect(parseSavingsSource("monte_carlo")).toBe("budget");
    expect(parseSavingsSource("")).toBe("budget");
    expect(parseSavingsSource(null)).toBe("budget");
    expect(parseSavingsSource(undefined)).toBe("budget");
  });
});

describe("savingsSourceUsesTransactions", () => {
  it("transactions_avg (modo B) → true", () => {
    expect(savingsSourceUsesTransactions("transactions_avg")).toBe(true);
  });

  it("budget_income_real_expense (modo C) → true", () => {
    expect(savingsSourceUsesTransactions("budget_income_real_expense")).toBe(
      true,
    );
  });

  it("budget (modo A) → false", () => {
    expect(savingsSourceUsesTransactions("budget")).toBe(false);
  });

  it("undefined → false", () => {
    expect(savingsSourceUsesTransactions(undefined)).toBe(false);
  });
});

describe("savingsBasisParenthetical", () => {
  const avg = (over: Partial<SavingsAvgBasisApi> = {}): SavingsAvgBasisApi => ({
    basis: "average",
    avg_months: 3,
    window_months: 3,
    window_mode: "calendar",
    first_month: "2026-05",
    last_month: "2026-07",
    has_gaps: false,
    ...over,
  });

  it("lado que salió del presupuesto → sin paréntesis", () => {
    expect(
      savingsBasisParenthetical({
        basis: "budget",
        avg_months: 0,
        window_months: 0,
        has_gaps: false,
      }),
    ).toBeUndefined();
  });

  it("rango contiguo → «media de may–jul 2026»", () => {
    expect(savingsBasisParenthetical(avg())).toBe("media de may 2026–jul 2026");
  });

  it("un solo mes → no repite el rango", () => {
    expect(
      savingsBasisParenthetical(
        avg({ avg_months: 1, first_month: "2026-07", last_month: "2026-07" }),
      ),
    ).toBe("media de jul 2026");
  });

  it("con huecos NO finge un rango contiguo", () => {
    expect(
      savingsBasisParenthetical(
        avg({ avg_months: 3, first_month: "2024-01", last_month: "2026-07", has_gaps: true }),
      ),
    ).toBe("media de 3 meses con datos, hasta jul 2026");
  });

  it("sin meses con datos → sin paréntesis", () => {
    expect(savingsBasisParenthetical(avg({ avg_months: 0 }))).toBeUndefined();
  });

  it("ausente → sin paréntesis", () => {
    expect(savingsBasisParenthetical(undefined)).toBeUndefined();
  });
});

describe("runwaySwrParenthetical", () => {
  // 5.0.0: lee el PERFIL de jubilación del usuario, no `fire_settings` — el SWR dejó de ser del
  // hogar (D13).
  it("sin perfil (sesión sin cargar) → etiqueta sin número, no el default", () => {
    expect(runwaySwrParenthetical(null)).toBe("dentro del SWR");
    expect(runwaySwrParenthetical(undefined)).toBe("dentro del SWR");
  });

  it("formatea el SWR configurado con un decimal y ' %' (es-ES)", () => {
    expect(
      runwaySwrParenthetical({ ...defaultRetirementProfileApi(), swr_pct: "3.5" }),
    ).toBe("dentro del SWR 3,5 %");
    expect(
      runwaySwrParenthetical({ ...defaultRetirementProfileApi(), swr_pct: "4" }),
    ).toBe("dentro del SWR 4,0 %");
  });

  it("swr_pct vacío → el normalizador cae al default 3,5", () => {
    expect(
      runwaySwrParenthetical({ ...defaultRetirementProfileApi(), swr_pct: "" }),
    ).toBe("dentro del SWR 3,5 %");
  });
});

describe("ventanas del promedio — round-trip obligatorio", () => {
  // TRAMPA QUE FIJA ESTE TEST: `PATCH /v1/installation` manda el objeto fire_settings COMPLETO y
  // el `#[serde(default)]` del servidor rellena lo ausente con los DEFAULTS. Si `normalize` no
  // devolviera las cuatro ventanas, guardar cualquier otro ajuste (el SWR, un tramo fiscal) las
  // resetearía a 3/12 en silencio — sin error, sin aviso, y con la proyección moviéndose.
  it("conserva las cuatro ventanas del servidor", () => {
    const out = normalizeInstallationFireSettings({
      savings_source: "transactions_avg",
      income_avg_window_months: 6,
      income_avg_window_mode: "data",
      expense_avg_window_months: 24,
      expense_avg_window_mode: "calendar",
    } as unknown as FireSettingsApi);
    expect(out.income_avg_window_months).toBe(6);
    expect(out.income_avg_window_mode).toBe("data");
    expect(out.expense_avg_window_months).toBe(24);
    expect(out.expense_avg_window_mode).toBe("calendar");
  });

  it("ausentes → defaults 3/12 en calendario", () => {
    const out = normalizeInstallationFireSettings({
      taxes_enabled: true,
    } as unknown as FireSettingsApi);
    expect(out.income_avg_window_months).toBe(3);
    expect(out.expense_avg_window_months).toBe(12);
    expect(out.income_avg_window_mode).toBe("calendar");
    expect(out.expense_avg_window_mode).toBe("calendar");
  });

  it("valores fuera de cota o basura → el default del lado, nunca 0", () => {
    const out = normalizeInstallationFireSettings({
      income_avg_window_months: 0,
      expense_avg_window_months: 999,
      income_avg_window_mode: "monte_carlo",
    } as unknown as FireSettingsApi);
    expect(out.income_avg_window_months).toBe(3);
    expect(out.expense_avg_window_months).toBe(12);
    expect(out.income_avg_window_mode).toBe("calendar");
  });
});
