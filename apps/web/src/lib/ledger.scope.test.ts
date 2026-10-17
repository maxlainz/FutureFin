/**
 * Ámbito de datos («Yo | Hogar») — las tres decisiones que 5.0.0 cambió, fijadas aquí porque
 * ninguna falla haciendo ruido:
 *
 * 1. **Default `mine`** (D9). Si el default se cayera otra vez a `household`, la app abriría en
 *    un agregado del hogar y el usuario leería como suyos unos números que no lo son.
 * 2. **`household` explícito en la query**. El API pasa a tener `mine` como default en 5.0.0:
 *    el `""` que la SPA mandaba antes para el hogar significaría a partir de ahora «solo yo», y
 *    la vista Hogar devolvería la mitad de los datos sin un solo error por ninguna parte.
 * 3. **Hogar = solo lectura**, que es lo que apaga los controles de escritura en toda la SPA.
 */

import { describe, expect, it } from "vitest";
import {
  isScopeReadOnly,
  ledgerViewAmp,
  ledgerViewQs,
  resolveLedgerPersonScope,
  LEDGER_PERSON_SCOPE_STORAGE_KEY,
} from "./ledger";

describe("resolveLedgerPersonScope", () => {
  it("sin valor guardado arranca en «Yo» (default 5.0.0)", () => {
    expect(resolveLedgerPersonScope(null)).toBe("mine");
    expect(resolveLedgerPersonScope(undefined)).toBe("mine");
    expect(resolveLedgerPersonScope("")).toBe("mine");
  });

  it("un valor desconocido también cae a «Yo», nunca al hogar", () => {
    expect(resolveLedgerPersonScope("HOUSEHOLD")).toBe("mine");
    expect(resolveLedgerPersonScope("todos")).toBe("mine");
    expect(resolveLedgerPersonScope("{}")).toBe("mine");
  });

  it("respeta la elección persistida del usuario", () => {
    expect(resolveLedgerPersonScope("household")).toBe("household");
    expect(resolveLedgerPersonScope(" household ")).toBe("household");
    expect(resolveLedgerPersonScope("mine")).toBe("mine");
  });

  it("la clave de almacenamiento sigue el espacio de nombres de la SPA", () => {
    expect(LEDGER_PERSON_SCOPE_STORAGE_KEY).toBe(
      "futurefin-ledger-person-scope",
    );
  });
});

describe("isScopeReadOnly", () => {
  it("el hogar es solo lectura y la vista propia no", () => {
    expect(isScopeReadOnly("household")).toBe(true);
    expect(isScopeReadOnly("mine")).toBe(false);
  });
});

describe("ledgerViewQs / ledgerViewAmp", () => {
  it("manda SIEMPRE el ámbito, también el hogar", () => {
    expect(ledgerViewQs("mine")).toBe("?view=mine");
    expect(ledgerViewQs("household")).toBe("?view=household");
  });

  it("nunca devuelve la cadena vacía (el default del API es `mine`)", () => {
    for (const scope of ["mine", "household"] as const) {
      expect(ledgerViewQs(scope)).not.toBe("");
      expect(ledgerViewQs(scope).startsWith("?view=")).toBe(true);
    }
  });

  it("la variante encadenable es el mismo parámetro con «&»", () => {
    for (const scope of ["mine", "household"] as const) {
      expect(ledgerViewAmp(scope)).toBe(`&${ledgerViewQs(scope).slice(1)}`);
      expect(ledgerViewAmp(scope).startsWith("&view=")).toBe(true);
    }
  });
});
