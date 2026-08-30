/**
 * PIN C7 (lado cliente) — paridad Rust↔TS de las VENTANAS DEL PROMEDIO.
 *
 * Las cotas (1–60) y los dos defaults (ingreso 3, gasto 12) viven **duplicados a mano** en cuatro
 * sitios: `MIN`/`MAX_AVG_WINDOW_MONTHS` y `default_fire_settings()` en
 * `apps/api/src/handlers/installation.rs`, `clampWindowMonths` en `lib/fire.ts`
 * («acotados a 1–60 igual que el servidor»), `defaultFireSettingsApi()` y —tercera copia de los
 * defaults— los `fallback` de `normalizeInstallationFireSettings`.
 *
 * `fire-parity.json` NO las cubre: ese fixture pinea el cálculo FIRE, no estas ventanas. Este
 * carga `apps/api/tests/fixtures/avg-window-parity.json`, el mismo fichero que lee el test Rust
 * `handlers::installation::avg_window_parity_tests`. Si un lado cambia sin el otro, UN test falla.
 *
 * Qué se rompe si divergen: el servidor `clamp`ea, el cliente **cae al fallback**. Con el techo
 * del servidor en 120 y el del cliente en 60, guardar `90` deja al servidor calculando con 90 y a
 * la SPA enseñando 12. Sin error, sin aviso: dos pantallas que no cuadran.
 *
 * Este test comprueba el comportamiento OBSERVABLE de `clampWindowMonths`, no su código: pasa los
 * valores frontera del fixture y sus vecinos de fuera, y exige que los de dentro sobrevivan y los
 * de fuera caigan al fallback. Reescribir la función a un `Math.min/max` real seguiría pasando
 * mientras la frontera no se mueva — que es exactamente lo que se quiere pinear.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { FireSettingsApi } from "../api/types";
import {
  clampWindowMonths,
  defaultFireSettingsApi,
  normalizeInstallationFireSettings,
} from "./fire";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/avg-window-parity.json",
);

type AvgWindowFixture = {
  min: number;
  max: number;
  income_default_months: number;
  expense_default_months: number;
};

const fixture = JSON.parse(
  readFileSync(FIXTURE_PATH, "utf8"),
) as AvgWindowFixture;

describe("PIN C7 — ventanas del promedio: paridad con el fixture del servidor", () => {
  it("el fixture trae los cuatro enteros que este test necesita", () => {
    // Anti-deriva silenciosa: si alguien renombra una clave, este test dice cuál falta en vez de
    // comparar contra `undefined` y pasar por accidente.
    for (const key of [
      "min",
      "max",
      "income_default_months",
      "expense_default_months",
    ] as const) {
      expect(
        Number.isInteger(fixture[key]),
        `avg-window-parity.json no trae el entero \`${key}\`. Actualiza LOS DOS consumidores: ` +
          `este fichero y apps/api/src/handlers/installation.rs (mod avg_window_parity_tests).`,
      ).toBe(true);
    }
  });

  it("defaultFireSettingsApi() usa los defaults del servidor", () => {
    const d = defaultFireSettingsApi();
    expect(
      d.income_avg_window_months,
      "El default de la ventana de INGRESO divergió del servidor. Actualiza A LA VEZ: " +
        "(1) defaultFireSettingsApi() en lib/fire.ts; (2) el fallback " +
        "clampWindowMonths(raw?.income_avg_window_months, 3) de normalizeInstallationFireSettings " +
        "(tercera copia del mismo número); (3) apps/api/tests/fixtures/avg-window-parity.json; " +
        "(4) default_fire_settings() en apps/api/src/handlers/installation.rs.",
    ).toBe(fixture.income_default_months);
    expect(
      d.expense_avg_window_months,
      "El default de la ventana de GASTO divergió del servidor. Mismos cuatro sitios que el de " +
        "ingreso, con 12 en lugar de 3.",
    ).toBe(fixture.expense_default_months);
  });

  it("normalizeInstallationFireSettings rellena con esos mismos defaults", () => {
    // Los fallbacks de `normalizeInstallationFireSettings` son una copia INDEPENDIENTE de los
    // defaults (`clampWindowMonths(raw?.x, 3)` / `(raw?.x, 12)`): `defaultFireSettingsApi()` puede
    // estar bien y estos mal. Se llega a ellos con un objeto no nulo sin las ventanas, que es lo
    // que manda un servidor viejo o una respuesta recortada.
    const fromEmptyObject = normalizeInstallationFireSettings(
      {} as FireSettingsApi,
    );
    expect(
      fromEmptyObject.income_avg_window_months,
      "El fallback de INGRESO de normalizeInstallationFireSettings no es el default del servidor.",
    ).toBe(fixture.income_default_months);
    expect(
      fromEmptyObject.expense_avg_window_months,
      "El fallback de GASTO de normalizeInstallationFireSettings no es el default del servidor.",
    ).toBe(fixture.expense_default_months);

    // Y también se llega a ellos con valores fuera de rango, que es el caso peligroso: la SPA
    // sustituye en silencio lo que el servidor sí acepta.
    const fromOutOfRange = normalizeInstallationFireSettings({
      income_avg_window_months: fixture.min - 1,
      expense_avg_window_months: fixture.max + 1,
    } as unknown as FireSettingsApi);
    expect(fromOutOfRange.income_avg_window_months).toBe(
      fixture.income_default_months,
    );
    expect(fromOutOfRange.expense_avg_window_months).toBe(
      fixture.expense_default_months,
    );
  });

  it("clampWindowMonths acota exactamente en la frontera [min, max] del servidor", () => {
    const fallback = 7; // ni min, ni max, ni ninguno de los dos defaults: distinguible

    // Dentro del rango del servidor → el valor sobrevive intacto.
    for (const inside of [fixture.min, fixture.min + 1, fixture.max - 1, fixture.max]) {
      expect(
        clampWindowMonths(inside, fallback),
        `clampWindowMonths rechazó ${inside}, que el servidor ACEPTA ` +
          `(${fixture.min}–${fixture.max}). El techo/suelo del cliente se quedó por debajo del ` +
          `del servidor: la SPA enseñaría el fallback donde el usuario guardó ${inside}. ` +
          `Actualiza el \`n < ${fixture.min} || n > ${fixture.max}\` de clampWindowMonths en ` +
          `lib/fire.ts, su comentario, y el fixture si el cambio es intencionado.`,
      ).toBe(inside);
    }

    // Fuera del rango del servidor → fallback (el cliente no manda un valor que el servidor
    // rechazaría con 400 `avg_window_out_of_range`).
    for (const outside of [fixture.min - 1, fixture.max + 1]) {
      expect(
        clampWindowMonths(outside, fallback),
        `clampWindowMonths aceptó ${outside}, que el servidor RECHAZA con 400 ` +
          `\`avg_window_out_of_range\`. El cliente acota más ancho que el servidor: el PATCH de ` +
          `Ajustes fallaría con un error que la UI no puede explicar.`,
      ).toBe(fallback);
    }
  });

  it("clampWindowMonths rechaza lo que no es un entero de meses", () => {
    // No es paridad de cotas, es la razón por la que el `fallback` existe: la respuesta de la API
    // es JSON sin tipos garantizados. Se pinea aquí porque el mismo `if` lleva las dos cosas.
    const fallback = 7;
    for (const bad of [null, undefined, "", "doce", NaN, 3.5, -1, 0]) {
      expect(clampWindowMonths(bad, fallback)).toBe(fallback);
    }
    // Pero una cadena numérica dentro de rango SÍ pasa (`Number(v)`), que es lo que llega de un
    // `<input>`: pinearlo evita que un "endurecimiento" futuro rompa Ajustes en silencio.
    expect(clampWindowMonths(String(fixture.max), fallback)).toBe(fixture.max);
  });
});
