/**
 * Cobertura del catálogo de descripciones. Impide las dos derivas simétricas: un icono que
 * apunta a un texto que no existe (el popover saldría vacío) y un texto que ya no consume nadie
 * (queda describiendo una métrica que quizá cambió o desapareció, y nadie se entera).
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { HELP_TEXTS } from "./helpTexts";

const SRC = join(fileURLToPath(new URL(".", import.meta.url)), "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return sourceFiles(p);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [p] : [];
  });
}

const used = new Set<string>();
for (const f of sourceFiles(SRC)) {
  const src = readFileSync(f, "utf8");
  if (f.endsWith("helpTexts.ts")) continue;
  for (const m of src.matchAll(/helpId="([^"]+)"/g)) used.add(m[1]);
  // Forma de OBJETO (`helpId: "retirement.coast_month"`), no de prop JSX: desde 5.0.0 hay
  // módulos puros que deciden qué tarjetas se pintan y con qué ayuda
  // (`lib/retirement-tiles.ts`, tabla §C de #207). Sin este patrón el escáner los daba por
  // huérfanos y la cobertura bidireccional empujaba a BORRAR textos que sí se usan — el fallo
  // exacto que este test existe para impedir, del revés.
  for (const m of src.matchAll(/helpId:\s*"([^"]+)"/g)) used.add(m[1]);
  // Acepta también la forma ternaria multilínea: HELP_TEXTS[cond ? "a" : "b"].
  for (const m of src.matchAll(/HELP_TEXTS\[([\s\S]{0,220}?)\]/g)) {
    // Solo cadenas con punto: los ids del catálogo lo llevan siempre, y así la condición
    // del ternario (`side === "income"`) no se cuela como id.
    for (const q of m[1].matchAll(/"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"/g)) used.add(q[1]);
  }
}

describe("catálogo de descripciones", () => {
  it("todo id consumido existe en el catálogo", () => {
    const missing = [...used].filter((id) => !(id in HELP_TEXTS));
    expect(missing, `ids sin texto: ${missing.join(", ")}`).toEqual([]);
  });

  it("todo texto del catálogo se consume en alguna vista", () => {
    const orphans = Object.keys(HELP_TEXTS).filter((id) => !used.has(id));
    expect(orphans, `textos huérfanos: ${orphans.join(", ")}`).toEqual([]);
  });

  it("cada texto tiene título corto y cuerpo con sustancia", () => {
    for (const [id, t] of Object.entries(HELP_TEXTS)) {
      expect(t.title.length, `${id}: título vacío`).toBeGreaterThan(2);
      expect(t.title.length, `${id}: título demasiado largo`).toBeLessThanOrEqual(40);
      expect(t.body.length, `${id}: cuerpo demasiado corto`).toBeGreaterThan(60);
    }
  });
});
