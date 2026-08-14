/**
 * Normalización del JSONB de fire_settings recibido de la API. Cubre en concreto el eje
 * `savings_source` (v1.9.0): ausente / válido / desconocido → default `budget`.
 */

import { describe, expect, it } from "vitest";
import type { FireSettingsApi } from "../api/types";
import {
  defaultFireSettingsApi,
  normalizeInstallationFireSettings,
  parseSavingsSource,
  savingsAvgParenthetical,
  savingsSourceUsesTransactions,
} from "./fire";

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
      fire_number_mode: "annual_expense",
      fire_number_manual_amount: null,
      fire_number_expense_adjustment_pct: null,
      swr_pct: "3.5",
      taxes_enabled: true,
      tax_brackets: [{ up_to: null, pct: "30" }],
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe("budget");
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

describe("savingsAvgParenthetical", () => {
  it("modo budget (A) → sin paréntesis", () => {
    expect(savingsAvgParenthetical("budget", 6)).toBeUndefined();
  });

  it("transactions_avg (modo B) con 6 meses", () => {
    expect(savingsAvgParenthetical("transactions_avg", 6)).toBe(
      "promedio de 6 meses",
    );
  });

  it("budget_income_real_expense (modo C) con 1 mes → singular", () => {
    expect(savingsAvgParenthetical("budget_income_real_expense", 1)).toBe(
      "promedio de 1 mes",
    );
  });

  it("fuente ausente → sin paréntesis", () => {
    expect(savingsAvgParenthetical(undefined, 6)).toBeUndefined();
  });

  it("0 meses (fallback del servidor a budget) o meses ausentes → sin paréntesis", () => {
    expect(savingsAvgParenthetical("transactions_avg", 0)).toBeUndefined();
    expect(savingsAvgParenthetical("transactions_avg", undefined)).toBeUndefined();
  });
});
