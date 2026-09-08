/**
 * Paridad backend↔frontend del catálogo de errores.
 *
 * Lee el mismo `apps/api/tests/fixtures/error-codes.json` que genera y verifica
 * `apps/api/tests/error_codes_parity.rs`, y exige que TODO código que la API puede devolver
 * tenga su frase en español. Sin este test el fallo sería silencioso: un código sin traducir no
 * rompe nada, solo degrada el mensaje a uno genérico, y nadie lo nota hasta que un usuario
 * pregunta por qué la app «no dice qué ha pasado».
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ERROR_MESSAGES, messageForError } from "./errorMessages";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../../api/tests/fixtures/error-codes.json",
);

type Fixture = { http_classes: string[]; codes: string[] };
const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as Fixture;

/** Códigos que nacen en el cliente y por tanto no están en el fixture del backend. */
const CLIENT_ONLY = new Set(["empty_json_body", "network_error"]);

/**
 * Códigos que el backend entrega por REDIRECCIÓN (`?ha_error=…` en la vuelta del login por
 * Home Assistant), no en el cuerpo de una respuesta de error. El fixture solo enumera los
 * `ApiError`, así que estos no tienen por qué aparecer ahí — pero su frase sí vive en el
 * catálogo, que es el único sitio donde se traduce lo que ve el usuario.
 */
const REDIRECT_ONLY = new Set([
  "ha_sso_disabled",
  "ha_state_mismatch",
  "ha_exchange_failed",
  "ha_identity_failed",
]);

describe("catálogo de errores", () => {
  it("el fixture trae códigos y clases", () => {
    expect(fixture.http_classes.length).toBeGreaterThan(0);
    expect(fixture.codes.length).toBeGreaterThan(0);
  });

  it("toda clase HTTP tiene frase en español", () => {
    const faltan = fixture.http_classes.filter((c) => !ERROR_MESSAGES[c]);
    expect(faltan, `sin traducir: ${faltan.join(", ")}`).toEqual([]);
  });

  it("todo código estable de la API tiene frase en español", () => {
    const faltan = fixture.codes.filter((c) => !ERROR_MESSAGES[c]);
    expect(
      faltan,
      `Códigos sin frase en errorMessages.ts: ${faltan.join(", ")}. ` +
        "Añádelos o el usuario verá un mensaje genérico.",
    ).toEqual([]);
  });

  it("no hay entradas huérfanas en el catálogo", () => {
    const conocidos = new Set([
      ...fixture.http_classes,
      ...fixture.codes,
      ...CLIENT_ONLY,
      ...REDIRECT_ONLY,
    ]);
    const sobran = Object.keys(ERROR_MESSAGES).filter((c) => !conocidos.has(c));
    expect(
      sobran,
      `Frases para códigos que la API ya no devuelve: ${sobran.join(", ")}. ` +
        "Bórralas o el catálogo se convierte en un cementerio.",
    ).toEqual([]);
  });

  it("las frases están en español y no filtran jerga técnica", () => {
    for (const [code, frase] of Object.entries(ERROR_MESSAGES)) {
      expect(frase.length, `${code}: frase vacía`).toBeGreaterThan(10);
      expect(frase, `${code}: acaba sin punto`).toMatch(/[.:]$|[.]\s*$|[a-zA-Z-]$/);
      expect(frase, `${code}: filtra el código al usuario`).not.toContain(code);
      expect(frase, `${code}: jerga HTTP`).not.toMatch(/\bHTTP\b|\b4\d\d\b|\b5\d\d\b/);
    }
  });
});

/**
 * Vista Hogar de solo lectura (5.0.0, D9/D21/D32). Los dos códigos que nacen de esa regla se
 * fijan aparte del barrido genérico: son 403/400, y sin frase propia el usuario leería el
 * mensaje de clase HTTP («No tienes permiso…»), que no dice ni de quién es la fila ni qué hacer
 * — que es exactamente lo que la SPA tiene que explicar cuando esconde un botón.
 */
describe("hogar de solo lectura", () => {
  it("«fila de otro miembro» tiene frase propia, no la genérica de 403", () => {
    const frase = ERROR_MESSAGES.not_row_owner;
    expect(frase).toBeTruthy();
    expect(frase).not.toBe(ERROR_MESSAGES.forbidden);
    expect(messageForError("not_row_owner", 403)).toBe(frase);
  });

  it("«la vista del hogar no escribe» tiene frase propia y menciona la salida", () => {
    const frase = ERROR_MESSAGES.household_read_only;
    expect(frase).toBeTruthy();
    expect(frase).not.toBe(ERROR_MESSAGES.bad_request);
    expect(frase).toContain("Yo");
    expect(messageForError("household_read_only", 400)).toBe(frase);
  });
});

/**
 * Fases y estrategias (5.0.0, WP5-1). Los dos códigos nuevos de la superficie de proyección se
 * fijan aparte porque los dos son 400 y su mensaje genérico («Los datos enviados no son
 * válidos… revisa el formulario») mandaría al usuario a corregir unos datos que están BIEN: uno
 * es un scope equivocado y el otro una capacidad del motor que todavía no existe.
 */
describe("simulación por estrategias", () => {
  it("«el hogar no se simula» dice cuál es la salida (la vista «Yo»)", () => {
    const frase = ERROR_MESSAGES.household_not_simulable;
    expect(frase).toBeTruthy();
    expect(frase).not.toBe(ERROR_MESSAGES.bad_request);
    expect(frase).toContain("Yo");
    expect(messageForError("household_not_simulable", 400)).toBe(frase);
  });

  it("«el motor aún no hace eso» no culpa a los datos del usuario", () => {
    const frase = ERROR_MESSAGES.engine_feature_unavailable;
    expect(frase).toBeTruthy();
    expect(frase).not.toBe(ERROR_MESSAGES.engine_rejected_input);
    expect(frase).not.toBe(ERROR_MESSAGES.bad_request);
    // La distinción es el valor del código: `engine_rejected_input` pide revisar los datos.
    expect(frase).not.toMatch(/[Rr]evisa/);
    expect(messageForError("engine_feature_unavailable", 400)).toBe(frase);
  });
});

describe("messageForError", () => {
  it("prefiere el código estable al status", () => {
    expect(messageForError("username_taken", 500)).toBe(ERROR_MESSAGES.username_taken);
  });
  it("cae al status cuando el código no está en el catálogo", () => {
    expect(messageForError("no_existe", 403)).toBe(ERROR_MESSAGES.forbidden);
  });
  it("cae al genérico sin código ni status conocidos", () => {
    expect(messageForError(null, 418)).toBe("Algo ha fallado. Inténtalo de nuevo en unos segundos.");
  });
});
