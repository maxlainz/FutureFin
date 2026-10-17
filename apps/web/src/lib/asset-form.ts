/**
 * Construcción PURA del cuerpo de `POST /v1/assets` y `PATCH /v1/assets/{id}` desde el formulario
 * de Activos (5.0.0, §A.2 + tri-estado de WP5-2).
 *
 * Existe por UN motivo concreto: los tres campos opcionales de importe del activo
 * —`purchase_price`, `expected_annual_return_percent` y `annual_volatility_percent`— son
 * **tri-estado** en el servidor, y el tri-estado no se puede improvisar en el submit.
 *
 *   - clave **ausente** → el servidor conserva lo que hay,
 *   - `null` → lo **borra**,
 *   - decimal-string → lo fija.
 *
 * La regla que hace falta acertar es cuál de los dos primeros emite un campo VACÍO en una edición:
 * si el activo **tenía** valor, vaciarlo es una orden de borrado y viaja `null`; si no tenía nada,
 * mandar `null` sería escribir «bórralo» sobre un hueco — un PATCH que no cambia nada, una
 * invalidación de la cache de proyección y, con `is_empty` de por medio, un 400 `patch_empty`
 * evitable. En un ALTA no hay valor previo que borrar: lo vacío simplemente no se manda.
 *
 * Hasta 4.15.x el servidor recibía los dos porcentajes como opción SIMPLE y `null` era
 * indistinguible de omitir: vaciar el campo NO devolvía el activo a determinista y el formulario
 * lo omitía a propósito. Con el tri-estado del servidor (WP5-2, `merge_optional_decimal_patch_with`)
 * el camino de vuelta existe, y esta función es donde se toma.
 *
 * El parseo de cada importe pasa por `toApiDecimalString`, que **lanza** `DecimalInputError` con
 * una frase en es-ES ante un texto que no es un número: el submit ya corre dentro de su try/catch
 * y esa frase es la que ve el usuario. Validar aquí las cotas (`> −100`, `[0, 100]`) sería
 * duplicar el contrato del servidor en un tercer sitio; el 400 con su código estable es la única
 * fuente.
 */

import type { AssetApiRow, AssetWriteBodyApi } from "../api/types";
import { toApiDecimalString } from "./format";

/** Lo que el formulario tiene tecleado, tal cual (sin normalizar). */
export type AssetFormDraft = {
  categoryId: string;
  name: string;
  currentValue: string;
  purchase: string;
  isLiquid: boolean;
  expectedReturn: string;
  volatility: string;
  notes: string;
};

/**
 * Los valores que el activo tiene AHORA en el servidor. Solo hacen falta los tri-estado: son los
 * únicos donde «vacío» significa cosas distintas según lo que hubiera antes.
 */
export type AssetPreviousValues = Pick<
  AssetApiRow,
  "expected_annual_return_percent" | "annual_volatility_percent"
>;

/** `true` cuando el activo tiene hoy un valor declarado en ese campo (no `null`, no vacío). */
function hasStoredValue(raw: string | null | undefined): boolean {
  return raw != null && String(raw).trim() !== "";
}

/**
 * Un campo tri-estado de importe: qué mandar (si es que hay que mandar algo).
 *
 * Devuelve `undefined` para «no nombres la clave». `null` **solo** cuando el usuario ha vaciado un
 * campo que tenía valor guardado.
 */
export function assetOptionalDecimalPatch(
  typed: string,
  previous: string | null | undefined,
  isEdit: boolean,
): string | null | undefined {
  const value = toApiDecimalString(typed);
  if (value !== "") return value;
  if (!isEdit) return undefined;
  return hasStoredValue(previous) ? null : undefined;
}

/**
 * Cuerpo del alta o de la edición.
 *
 * `previous` es la fila actual en una edición (`null` en un alta). En el alta se mandan solo los
 * campos con contenido; en la edición se manda además el `null` de lo que el usuario ha vaciado.
 *
 * `purchase_price` es la excepción histórica: en una edición viaja SIEMPRE (valor o `null`). Era
 * ya tri-estado en el servidor mucho antes que los otros dos y omitirlo dejaba ambigüedad; se
 * conserva tal cual para no cambiar un comportamiento que no toca esta ola.
 */
export function buildAssetWriteBody(
  draft: AssetFormDraft,
  previous: AssetPreviousValues | null,
): AssetWriteBodyApi {
  const isEdit = previous !== null;
  const body: AssetWriteBodyApi = {
    category_id: draft.categoryId,
    name: draft.name.trim(),
    current_value: toApiDecimalString(draft.currentValue),
    is_liquid: draft.isLiquid,
  };

  const expected = assetOptionalDecimalPatch(
    draft.expectedReturn,
    previous?.expected_annual_return_percent,
    isEdit,
  );
  if (expected !== undefined) body.expected_annual_return_percent = expected;

  const volatility = assetOptionalDecimalPatch(
    draft.volatility,
    previous?.annual_volatility_percent,
    isEdit,
  );
  if (volatility !== undefined) body.annual_volatility_percent = volatility;

  const purchase = toApiDecimalString(draft.purchase);
  if (isEdit) {
    body.purchase_price = purchase === "" ? null : purchase;
  } else if (purchase !== "") {
    body.purchase_price = purchase;
  }

  const notes = draft.notes.trim();
  if (notes) body.notes = notes;

  return body;
}
