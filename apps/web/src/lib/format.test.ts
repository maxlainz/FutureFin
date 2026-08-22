import { describe, expect, it } from "vitest";
import {
  METRIC_DASH,
  assetImplicitTotalReturnLabel,
  breakdownPercentOfTotal,
  formatBreakdownPct,
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatCurrencyOrDash,
  formatDebtToAssetsPct,
  formatEditableDecimalString,
  toApiDecimalString,
  formatFractionAsPercent,
  formatMoneyAmount,
  formatMonthsRough,
  formatRunwayValue,
  formatPercentAmount,
  formatPercentDisplay,
  formatPercentDisplaySigned,
  isAbsentMetric,
  isZeroFractionMetric,
  isZeroMoneyMetric,
  lastUsedLabel,
  tokenExpiryLabel,
  normalizeCurrencyIso,
  parseDisplayDecimal,
} from "./format";

describe("parseDisplayDecimal", () => {
  it("parses dot and comma decimals", () => {
    expect(parseDisplayDecimal("1.5")).toBe(1.5);
    expect(parseDisplayDecimal("1,5")).toBe(1.5);
    expect(parseDisplayDecimal("  3.14  ")).toBe(3.14);
  });
  it("returns null on empty/invalid", () => {
    expect(parseDisplayDecimal("")).toBeNull();
    expect(parseDisplayDecimal("   ")).toBeNull();
    expect(parseDisplayDecimal("abc")).toBeNull();
  });
  it("handles zero and negatives", () => {
    expect(parseDisplayDecimal("0")).toBe(0);
    expect(parseDisplayDecimal("-7.25")).toBe(-7.25);
  });
});

describe("formatEditableDecimalString", () => {
  it("recorta ceros de la API y sirve el decimal con coma española", () => {
    expect(formatEditableDecimalString("2.500000")).toBe("2,5");
    expect(formatEditableDecimalString("100.0000")).toBe("100");
  });
  it("lo que sirve vuelve a entrar: es idempotente contra parseDisplayDecimal", () => {
    // El valor precargado en un input tiene que poder reenviarse tal cual sin que el backend
    // lo rechace. Este es el ciclo que estaba roto en el formulario de activos.
    const editable = formatEditableDecimalString("1234.5000");
    expect(editable).toBe("1234,5");
    expect(toApiDecimalString(editable)).toBe("1234.5");
  });
  it("returns empty on null/undefined/empty", () => {
    expect(formatEditableDecimalString(null)).toBe("");
    expect(formatEditableDecimalString(undefined)).toBe("");
    expect(formatEditableDecimalString("")).toBe("");
  });
  it("passes through non-parseable strings", () => {
    expect(formatEditableDecimalString("abc")).toBe("abc");
  });
});

describe("formatMoneyAmount", () => {
  it("formats integers with es-ES thousand separators", () => {
    expect(formatMoneyAmount("1234.56")).toBe("1235");
    expect(formatMoneyAmount("12345.67")).toBe("12.346");
    expect(formatMoneyAmount("1000000")).toBe("1.000.000");
  });
  it("passes through unparseable input", () => {
    expect(formatMoneyAmount("not a number")).toBe("not a number");
  });
});

describe("normalizeCurrencyIso", () => {
  it("uppercases valid 3-letter codes", () => {
    expect(normalizeCurrencyIso("eur")).toBe("EUR");
    expect(normalizeCurrencyIso(" usd ")).toBe("USD");
  });
  it("rejects invalid codes", () => {
    expect(normalizeCurrencyIso("EU")).toBeNull();
    expect(normalizeCurrencyIso("EURO")).toBeNull();
    expect(normalizeCurrencyIso("123")).toBeNull();
    expect(normalizeCurrencyIso(null)).toBeNull();
  });
});

describe("formatCurrencyAmount + formatCurrencyNumber", () => {
  it("appends currency symbol (EUR) with thousand separators on big numbers", () => {
    const out = formatCurrencyAmount("12345", "EUR");
    expect(out).toMatch(/€/);
    expect(out).toMatch(/12\.345/);
  });
  it("falls back to plain money when ISO invalid", () => {
    expect(formatCurrencyAmount("12345", "EU")).toBe("12.345");
  });
  it("formatCurrencyNumber works with native numbers", () => {
    const out = formatCurrencyNumber(50000, "EUR");
    expect(out).toMatch(/50\.000/);
    expect(out).toMatch(/€/);
  });
});

describe("formatCurrencyOrDash", () => {
  it("returns dash for empty/null", () => {
    expect(formatCurrencyOrDash(null, "EUR")).toBe(METRIC_DASH);
    expect(formatCurrencyOrDash("", "EUR")).toBe(METRIC_DASH);
    expect(formatCurrencyOrDash("   ", "EUR")).toBe(METRIC_DASH);
  });
  it("formats non-empty values", () => {
    expect(formatCurrencyOrDash("100", "EUR")).toMatch(/€/);
  });
});

describe("formatPercentDisplay + formatPercentAmount + formatPercentDisplaySigned", () => {
  it("uses one decimal with ' %' suffix and comma separator", () => {
    expect(formatPercentDisplay(3.456)).toBe("3,5 %");
    expect(formatPercentDisplay(0)).toBe("0,0 %");
  });
  it("formatPercentAmount parses string input", () => {
    expect(formatPercentAmount("3.25")).toBe("3,3 %");
    expect(formatPercentAmount("not a number")).toBe("not a number");
  });
  it("signed adds explicit + for positives", () => {
    expect(formatPercentDisplaySigned(5.5)).toBe("+5,5 %");
    expect(formatPercentDisplaySigned(-2.1)).toBe("-2,1 %");
    expect(formatPercentDisplaySigned(0)).toBe("0,0 %");
  });
});

describe("assetImplicitTotalReturnLabel", () => {
  it("computes signed pct from current/purchase", () => {
    expect(assetImplicitTotalReturnLabel("1100", "1000")).toBe("+10,0 %");
    expect(assetImplicitTotalReturnLabel("900", "1000")).toBe("-10,0 %");
  });
  it("returns null when purchase missing/invalid", () => {
    expect(assetImplicitTotalReturnLabel("1100", null)).toBeNull();
    expect(assetImplicitTotalReturnLabel("1100", "")).toBeNull();
    expect(assetImplicitTotalReturnLabel("1100", "0")).toBeNull();
  });
});

describe("formatDebtToAssetsPct + formatFractionAsPercent", () => {
  it("multiplies fraction by 100 and formats", () => {
    expect(formatDebtToAssetsPct("0.25")).toBe("25,0 %");
    expect(formatFractionAsPercent("0.05")).toBe("5,0 %");
  });
  it("returns dash for null/empty/invalid", () => {
    expect(formatDebtToAssetsPct(null)).toBe(METRIC_DASH);
    expect(formatFractionAsPercent("")).toBe(METRIC_DASH);
    expect(formatDebtToAssetsPct("abc")).toBe(METRIC_DASH);
  });
});

describe("isZeroMoneyMetric + isZeroFractionMetric", () => {
  it("treats null/empty/zero as 'zero'", () => {
    expect(isZeroMoneyMetric(null)).toBe(true);
    expect(isZeroMoneyMetric("")).toBe(true);
    expect(isZeroMoneyMetric("0")).toBe(true);
    expect(isZeroMoneyMetric("0,00")).toBe(true);
  });
  it("returns false for nonzero amounts", () => {
    expect(isZeroMoneyMetric("100")).toBe(false);
    expect(isZeroFractionMetric("0.05")).toBe(false);
  });
});

describe("isAbsentMetric", () => {
  it("treats null/empty/unparseable as absent", () => {
    expect(isAbsentMetric(null)).toBe(true);
    expect(isAbsentMetric(undefined)).toBe(true);
    expect(isAbsentMetric("")).toBe(true);
    expect(isAbsentMetric("   ")).toBe(true);
  });
  it("does NOT treat an explicit zero as absent", () => {
    // El caso que motivó el helper: el runway se publica con 1 decimal desde 3.8.0, así que
    // «menos de 0,05 meses» llega como "0.0" y la tarjeta debe seguir pintándose.
    expect(isAbsentMetric("0.0")).toBe(false);
    expect(isAbsentMetric("0")).toBe(false);
    expect(isAbsentMetric("0,00")).toBe(false);
    expect(isZeroMoneyMetric("0.0")).toBe(true); // el contraste con el guard antiguo
  });
});

describe("formatMonthsRough", () => {
  it("appends ' meses' with one-decimal rounding", () => {
    expect(formatMonthsRough("3.45")).toBe("3,5 meses");
    expect(formatMonthsRough("12")).toBe("12 meses");
  });
  it("returns dash on missing data", () => {
    expect(formatMonthsRough(null)).toBe(METRIC_DASH);
    expect(formatMonthsRough("abc")).toBe(METRIC_DASH);
  });
  it("switches to years from 24 months on", () => {
    expect(formatMonthsRough("24")).toBe("2 años");
    expect(formatMonthsRough("30")).toBe("2 años y 6 meses");
    expect(formatMonthsRough("25")).toBe("2 años y 1 mes");
    expect(formatMonthsRough("1200")).toBe("100 años");
  });
  it("keeps the months format just below the threshold", () => {
    expect(formatMonthsRough("23.5")).toBe("23,5 meses");
  });
});

describe("formatRunwayValue", () => {
  it("shows 'Infinito' when the server marks it indefinite (withdrawal within SWR)", () => {
    expect(formatRunwayValue(null, true)).toBe("Infinito");
    expect(formatRunwayValue("12", true)).toBe("Infinito");
  });
  it("reports the 1200-month server cap as a floor, not an exact value", () => {
    expect(formatRunwayValue("1200", false)).toBe("+100 años");
    expect(formatRunwayValue("1200.0000", false)).toBe("+100 años");
  });
  it("delegates to formatMonthsRough otherwise", () => {
    expect(formatRunwayValue("30", false)).toBe("2 años y 6 meses");
    expect(formatRunwayValue("12", undefined)).toBe("12 meses");
    expect(formatRunwayValue("1199", false)).toBe("99 años y 11 meses");
    expect(formatRunwayValue(null, false)).toBe(METRIC_DASH);
    expect(formatRunwayValue(null, undefined)).toBe(METRIC_DASH);
  });
});

describe("breakdownPercentOfTotal + formatBreakdownPct", () => {
  it("caps pct at 100", () => {
    expect(breakdownPercentOfTotal("200", "100")).toBe(100);
    expect(formatBreakdownPct("200", "100")).toBe("100,0 %");
  });
  it("returns null when whole ≤ 0", () => {
    expect(breakdownPercentOfTotal("50", "0")).toBeNull();
    expect(formatBreakdownPct("50", "0")).toBe(METRIC_DASH);
  });
  it("computes ratio correctly", () => {
    expect(breakdownPercentOfTotal("25", "100")).toBe(25);
    expect(formatBreakdownPct("25", "100")).toBe("25,0 %");
  });
});

describe("lastUsedLabel + tokenExpiryLabel (tokens de API)", () => {
  it("lastUsedLabel: Nunca sin timestamp, fecha corta con ISO", () => {
    expect(lastUsedLabel(null)).toBe("Nunca");
    expect(lastUsedLabel(undefined)).toBe("Nunca");
    expect(lastUsedLabel("2026-08-16T10:20:30.123456Z")).toBe("16/08/2026");
  });
  it("tokenExpiryLabel: revocado gana a caducidad", () => {
    expect(
      tokenExpiryLabel("2027-01-01T00:00:00Z", "2026-08-16T09:00:00Z"),
    ).toBe("Revocado 16/08/2026");
  });
  it("tokenExpiryLabel: sin expires_at = sin caducidad", () => {
    expect(tokenExpiryLabel(null, null)).toBe("Sin caducidad");
    expect(tokenExpiryLabel(undefined, undefined)).toBe("Sin caducidad");
  });
  it("tokenExpiryLabel: con expires_at anuncia la fecha", () => {
    expect(tokenExpiryLabel("2026-11-14T12:00:00Z", null)).toBe("Caduca 14/11/2026");
  });
});

describe("toApiDecimalString", () => {
  it("convierte la coma española al punto que exige la API", () => {
    expect(toApiDecimalString("1234,5")).toBe("1234.5");
    expect(toApiDecimalString("  2,75 ")).toBe("2.75");
  });
  it("deja intacto lo que ya viene con punto", () => {
    expect(toApiDecimalString("2.5")).toBe("2.5");
    expect(toApiDecimalString("100")).toBe("100");
  });
  it("null, undefined y vacío dan cadena vacía", () => {
    expect(toApiDecimalString(null)).toBe("");
    expect(toApiDecimalString(undefined)).toBe("");
    expect(toApiDecimalString("   ")).toBe("");
  });
});
