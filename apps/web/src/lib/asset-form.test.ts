/**
 * Cuerpo de escritura de un activo — el TRI-ESTADO de sus tres importes opcionales (5.0.0).
 *
 * Lo que estos tests protegen, por orden de gravedad:
 *  1. **Que vaciar un campo que TENÍA valor lo borre de verdad** (`null` explícito). Es el camino
 *     de vuelta a un activo determinista o sin rentabilidad declarada; hasta WP5-2 no existía y
 *     el formulario omitía la clave, así que el valor escrito por error era permanente.
 *  2. **Que vaciar un campo que ya estaba vacío NO nombre la clave.** Un `null` sobre un hueco es
 *     un PATCH que no cambia nada: invalida la cache de proyección y puede acabar en un
 *     `patch_empty` del servidor.
 *  3. Que un ALTA no mande `null` en nada: no hay valor previo que borrar.
 *  4. Que un importe ambiguo LANCE (la frase es-ES de `toApiDecimalString`) en vez de colarse
 *     como un decimal inventado.
 */

import { describe, expect, it } from "vitest";
import {
  assetOptionalDecimalPatch,
  buildAssetWriteBody,
  type AssetFormDraft,
} from "./asset-form";
import { DecimalInputError } from "./format";

const DRAFT: AssetFormDraft = {
  categoryId: "cat-1",
  name: "  Fondo índice  ",
  currentValue: "10.000",
  purchase: "",
  isLiquid: true,
  expectedReturn: "",
  volatility: "",
  notes: "  ",
};

describe("assetOptionalDecimalPatch", () => {
  it("un valor tecleado se manda siempre, normalizado a decimal de la API", () => {
    expect(assetOptionalDecimalPatch("15,5", null, true)).toBe("15.5");
    expect(assetOptionalDecimalPatch("15,5", null, false)).toBe("15.5");
  });

  it("vacío sobre un valor GUARDADO manda null: es la orden de borrar", () => {
    expect(assetOptionalDecimalPatch("", "15.0000", true)).toBeNull();
    expect(assetOptionalDecimalPatch("   ", "0.0000", true)).toBeNull();
  });

  it("vacío sobre un campo que ya estaba vacío no nombra la clave", () => {
    expect(assetOptionalDecimalPatch("", null, true)).toBeUndefined();
    expect(assetOptionalDecimalPatch("", undefined, true)).toBeUndefined();
    expect(assetOptionalDecimalPatch("", "   ", true)).toBeUndefined();
  });

  it("en un ALTA lo vacío se omite: no hay nada previo que borrar", () => {
    expect(assetOptionalDecimalPatch("", "15.0000", false)).toBeUndefined();
  });
});

describe("buildAssetWriteBody · alta", () => {
  it("manda solo los campos con contenido y ningún null", () => {
    const body = buildAssetWriteBody(DRAFT, null);
    expect(body).toEqual({
      category_id: "cat-1",
      name: "Fondo índice",
      current_value: "10000",
      is_liquid: true,
    });
    expect("expected_annual_return_percent" in body).toBe(false);
    expect("annual_volatility_percent" in body).toBe(false);
    expect("purchase_price" in body).toBe(false);
    expect("notes" in body).toBe(false);
  });

  it("incluye lo que el usuario sí escribió", () => {
    const body = buildAssetWriteBody(
      {
        ...DRAFT,
        purchase: "8.000",
        expectedReturn: "6,5",
        volatility: "16",
        notes: "  cartera larga  ",
      },
      null,
    );
    expect(body.purchase_price).toBe("8000");
    expect(body.expected_annual_return_percent).toBe("6.5");
    expect(body.annual_volatility_percent).toBe("16");
    expect(body.notes).toBe("cartera larga");
  });
});

describe("buildAssetWriteBody · edición", () => {
  const PREVIOUS = {
    expected_annual_return_percent: "6.5000",
    annual_volatility_percent: "16.0000",
  };

  it("vaciar los dos porcentajes de un activo que los tenía los BORRA", () => {
    const body = buildAssetWriteBody(DRAFT, PREVIOUS);
    expect(body.expected_annual_return_percent).toBeNull();
    expect(body.annual_volatility_percent).toBeNull();
  });

  it("vaciar solo la volatilidad deja la rentabilidad intacta (clave ausente)", () => {
    const body = buildAssetWriteBody({ ...DRAFT, expectedReturn: "6,5" }, PREVIOUS);
    expect(body.expected_annual_return_percent).toBe("6.5");
    expect(body.annual_volatility_percent).toBeNull();
  });

  it("un activo que nunca declaró volatilidad no manda la clave al guardar", () => {
    const body = buildAssetWriteBody(DRAFT, {
      expected_annual_return_percent: "6.5000",
      annual_volatility_percent: null,
    });
    expect("annual_volatility_percent" in body).toBe(false);
    expect(body.expected_annual_return_percent).toBeNull();
  });

  it("una edición sin fila previa conocida no borra nada", () => {
    const body = buildAssetWriteBody(DRAFT, {});
    expect("expected_annual_return_percent" in body).toBe(false);
    expect("annual_volatility_percent" in body).toBe(false);
  });

  it("purchase_price viaja SIEMPRE en una edición (valor o null)", () => {
    expect(buildAssetWriteBody(DRAFT, {}).purchase_price).toBeNull();
    expect(
      buildAssetWriteBody({ ...DRAFT, purchase: "8.000" }, {}).purchase_price,
    ).toBe("8000");
  });

  it("volver a escribir un valor sobre uno borrado lo fija otra vez", () => {
    const body = buildAssetWriteBody({ ...DRAFT, volatility: "0" }, PREVIOUS);
    // `0` es un valor declarado («determinista, y lo digo»), no un vacío.
    expect(body.annual_volatility_percent).toBe("0");
  });
});

describe("importes ambiguos", () => {
  it("lanzan DecimalInputError en vez de colarse como un número inventado", () => {
    expect(() =>
      buildAssetWriteBody({ ...DRAFT, volatility: "1,2,3" }, null),
    ).toThrow(DecimalInputError);
    expect(() =>
      buildAssetWriteBody({ ...DRAFT, currentValue: "diez mil" }, null),
    ).toThrow(DecimalInputError);
  });
});
