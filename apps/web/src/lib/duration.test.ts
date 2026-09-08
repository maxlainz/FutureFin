/**
 * `formatMonthSpanEs` frente a su vecina `formatYearsEsFromMonths`: las dos formatean meses y
 * dicen cosas distintas en los extremos, que es justo donde S8 se equivocó.
 */

import { describe, expect, it } from "vitest";
import { formatMonthSpanEs } from "./duration";
import { formatYearsEsFromMonths } from "./projection-chart";

describe("formatMonthSpanEs — un TRAMO, no una cuenta atrás", () => {
  it("un tramo vacío son «0 meses», nunca «Ya alcanzado»", () => {
    expect(formatMonthSpanEs(0)).toBe("0 meses");
    // La diferencia con la cuenta atrás, explícita: son funciones distintas a propósito.
    expect(formatYearsEsFromMonths(0)).toBe("Ya alcanzado");
  });

  it("un tramo negativo también son «0 meses» (una pensión anterior a la jubilación)", () => {
    expect(formatMonthSpanEs(-7)).toBe("0 meses");
  });

  it("por debajo del año cuenta en meses, con singular", () => {
    expect(formatMonthSpanEs(1)).toBe("1 mes");
    expect(formatMonthSpanEs(5)).toBe("5 meses");
    expect(formatMonthSpanEs(11)).toBe("11 meses");
  });

  it("a partir de 12 cuenta en años y NUNCA dice «y 0 meses»", () => {
    expect(formatMonthSpanEs(12)).toBe("1 año");
    expect(formatMonthSpanEs(24)).toBe("2 años");
    expect(formatMonthSpanEs(144)).toBe("12 años");
  });

  it("los meses sobrantes se dicen solo si los hay", () => {
    expect(formatMonthSpanEs(13)).toBe("1 año y 1 mes");
    expect(formatMonthSpanEs(17)).toBe("1 año y 5 meses");
    expect(formatMonthSpanEs(199)).toBe("16 años y 7 meses");
  });

  it("redondea al mes entero", () => {
    expect(formatMonthSpanEs(11.6)).toBe("1 año");
    expect(formatMonthSpanEs(12.4)).toBe("1 año");
  });

  it("un valor no finito no se estima: son «0 meses»", () => {
    expect(formatMonthSpanEs(Number.NaN)).toBe("0 meses");
    expect(formatMonthSpanEs(Number.POSITIVE_INFINITY)).toBe("0 meses");
  });

  it("coincide con la cuenta atrás en todo el rango positivo (solo difieren en ≤ 0)", () => {
    for (let m = 1; m <= 400; m += 1) {
      expect(formatMonthSpanEs(m), `mes ${m}`).toBe(formatYearsEsFromMonths(m));
    }
  });
});
