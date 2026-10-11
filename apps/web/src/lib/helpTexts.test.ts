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

  /**
   * El modelo v2 (5.0.0) retiró tres conceptos ENTEROS: el patrimonio objetivo como disparador de
   * la jubilación, su base (perpetuidad / puente descontado) y el colchón de caja. Los textos que
   * los describían se borraron, pero el vocabulario sobrevive en la cabeza de quien escribe la
   * siguiente entrada — y una ayuda que vuelva a prometer «el mes en que cruzas tu objetivo»
   * describiría un mecanismo que ya no existe, delante del gráfico que no lo dibuja (issue #216,
   * la mediana que se prometía y no se pintaba, es el precedente exacto).
   *
   * Se prohíben FRASES, no la palabra «objetivo»: «edad de jubilación objetivo» sigue siendo un
   * campo vivo del plan.
   */
  it("ninguna ayuda resucita el objetivo, su base ni el colchón", () => {
    const prohibidas = [
      "colchón",
      "patrimonio objetivo",
      "objetivo fire",
      "base del objetivo",
      "cruce con el objetivo",
      "cruce del objetivo",
      "descuento del puente",
      // Las tres que vigilaba `retirement-form.test.ts` sobre las frases de apartado: se mudan
      // aquí con el texto (2026-09-13), porque aquí es donde el texto vive ahora.
      "multiplica tu objetivo",
      "dimensiona el objetivo",
      "mueve el objetivo",
    ];
    const ofensas: string[] = [];
    for (const [id, t] of Object.entries(HELP_TEXTS)) {
      const texto = `${t.title} ${t.body}`.toLowerCase();
      for (const frase of prohibidas) {
        if (texto.includes(frase)) ofensas.push(`${id}: «${frase}»`);
      }
    }
    expect(ofensas, `conceptos retirados en 5.0.0: ${ofensas.join(" · ")}`).toEqual([]);
  });

  /**
   * Los tres guardas que vivían en `retirement-form.test.ts` sobre `PLAN_CARD_COPY.blurb`
   * (2026-09-13). Las frases de apartado dejaron de pintarse en prosa y pasaron al `<?>` del
   * título, así que su contenido se comprueba donde ahora vive: el catálogo. No juzgan estilo —
   * cazan que alguien reescriba una entrada y se lleve por delante el hecho que la frase
   * existía para decir.
   */
  it("«Horizonte» dice que alargarlo RETRASA tu fecha, no lo contrario", () => {
    // Era cierto mientras la fecha era un cruce de capital contra un objetivo. En v2 la fecha
    // válida es el primer mes cuyo éxito HASTA EL HORIZONTE cumple el umbral, así que alargar el
    // horizonte la retrasa. Es la mentira más cara de las que el modelo v2 dejó atrás: invita a
    // subir la edad límite «por si acaso» y a no entender por qué se va la fecha.
    const { body } = HELP_TEXTS["settings.horizon_age"];
    expect(body).not.toMatch(/no mueve tu fecha/i);
    expect(body, "y tiene que decir lo contrario, no callarse").toMatch(/retrasa/i);
  });

  /**
   * «Solo SWR» (2026-10-10): la tarjeta «Retirada» desapareció. El SWR cierra «Gasto en
   * jubilación» y el umbral abre «Horizonte y exigencia», así que cada mitad de lo que decía
   * aquella frase se comprueba ahora en la ayuda del apartado donde vive.
   */
  it("«Gasto en jubilación» explica la puerta de la tasa inicial: gasto del primer año contra la tasa", () => {
    const { body } = HELP_TEXTS["retirement.section_spending"];
    expect(body).toMatch(/tasa de retirada/i);
    expect(body).toMatch(/primer año/i);
    expect(body).toMatch(/inflación/i);
  });

  it("«Horizonte y exigencia» nombra el umbral de éxito que ahora abre el apartado", () => {
    const t = HELP_TEXTS["settings.horizon_age"];
    expect(t.title).toBe("Horizonte y exigencia");
    expect(t.body).toMatch(/umbral/i);
    expect(t.body).toMatch(/escenarios/i);
  });

  it("el SWR es una tasa INICIAL, no un tope mensual ni un objetivo de capital", () => {
    const { body } = HELP_TEXTS["settings.swr"];
    expect(body).toMatch(/inicial/i);
    expect(body).toMatch(/primer año/i);
    expect(body).toMatch(/no un tope mes a mes/i);
    expect(body).toMatch(/puente/i);
  });

  it("ninguna ayuda resucita las reglas de retirada ni el modo de gasto («solo SWR»)", () => {
    const prohibidas = [
      "regla de retirada",
      "reglas de retirada",
      "regla por saldo",
      "reglas por saldo",
      "% del saldo",
      "con bandas",
      "híbrida",
      "es mi gasto",
      "fijado por api",
    ];
    const ofensas: string[] = [];
    for (const [id, t] of Object.entries(HELP_TEXTS)) {
      const texto = `${t.title} ${t.body}`.toLowerCase();
      for (const frase of prohibidas) {
        if (texto.includes(frase)) ofensas.push(`${id}: «${frase}»`);
      }
    }
    expect(ofensas, `conceptos retirados con «solo SWR»: ${ofensas.join(" · ")}`).toEqual([]);
  });

  it("«Pensión» dice qué hace el puente, y que en ese tramo no hay aportaciones", () => {
    const { body } = HELP_TEXTS["retirement.pension"];
    expect(body).toMatch(/puente/i);
    expect(body).toMatch(/aportaciones/i);
  });

  it("cada texto tiene título corto y cuerpo con sustancia", () => {
    for (const [id, t] of Object.entries(HELP_TEXTS)) {
      expect(t.title.length, `${id}: título vacío`).toBeGreaterThan(2);
      expect(t.title.length, `${id}: título demasiado largo`).toBeLessThanOrEqual(40);
      expect(t.body.length, `${id}: cuerpo demasiado corto`).toBeGreaterThan(60);
    }
  });
});
