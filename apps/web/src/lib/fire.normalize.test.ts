/**
 * Normalización del JSONB de fire_settings recibido de la API. Cubre en concreto el eje
 * `savings_source` (v1.9.0): ausente / válido / desconocido → default `budget`.
 */

import { describe, expect, it } from "vitest";
import type { FireSettingsApi } from "../api/types";
import {
  defaultFireSettingsApi,
  normalizeInstallationFireSettings,
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

  it("valor desconocido → budget", () => {
    const raw = {
      ...defaultFireSettingsApi(),
      // Fuerza un valor fuera del enum para simular un backend futuro / dato corrupto.
      savings_source: "monte_carlo" as unknown,
    } as FireSettingsApi;
    expect(normalizeInstallationFireSettings(raw).savings_source).toBe("budget");
  });
});
