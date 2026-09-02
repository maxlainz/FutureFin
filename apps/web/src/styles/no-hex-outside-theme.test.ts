/**
 * FREEZER hex — «cero hex fuera de `theme.css`» deja de ser prosa y pasa a ser un test.
 *
 * La regla es del design system (`.claude/design-system.md` §Identidad y §Reglas para añadir UI
 * nueva; resumen en CLAUDE.md §UI conventions): **nunca uses hex hardcoded en `App.css` o en componentes; consume
 * `var(--ff-*)`**. Los tokens viven en un único sitio, `src/styles/theme.css`, porque de ahí sale
 * el tema claro/oscuro: un `#71717a` suelto en un componente sobrevive al cambio de tema y produce
 * el fallo típico —texto invisible en oscuro, borde que no encaja en claro— que solo se ve mirando
 * la pantalla, y solo en uno de los dos temas.
 *
 * **Por qué un test y no stylelint**: añadir stylelint es una dependencia nueva, una config nueva
 * y un job nuevo para vigilar una sola regla. Este fichero corre dentro de la suite Vitest que ya
 * es gate bloqueante en CI (job `web`), no añade nada al `package.json`, y falla con el `path:line`
 * del hex y con qué hacer.
 *
 * ### Qué se escanea
 * `src/**` con extensión `.css`, `.tsx` y `.ts`, excluyendo:
 *
 * - `src/styles/theme.css` — es LA excepción: el único sitio donde un hex es correcto.
 * - los propios ficheros de test (`*.test.ts` / `*.test.tsx`). Dos razones: un test puede
 *   legítimamente afirmar sobre un color, y —sobre todo— **este fichero se escanearía a sí mismo**.
 *   Es el «comando que se cuenta a sí mismo» que la norma de la casa tiene fichado.
 *
 * Se incluye `.ts` aunque los componentes sean `.tsx` porque la propia convención del repo empuja
 * la lógica fuera de los componentes hacia `lib/*.ts`: limitar el barrido a `.tsx` dejaría abierta
 * justo la puerta por la que saldría el color.
 *
 * ### Falsos positivos, y por qué el filtro es estrecho
 * Hoy el conteo real es **CERO** en todo `src/` con el patrón desnudo `#[0-9a-fA-F]{3,8}\b`
 * (verificado con `grep -rnE`), así que cualquier exclusión de más solo puede debilitar el test.
 * Se excluyen únicamente formas que **no pueden ser un color**:
 *
 * 1. Referencias a fragmentos SVG: `url(#gradId)` — hoy `url(#nwFill-…)` y `url(#…Clip-…)` en
 *    `ProjectionNetWorthChart.tsx`, que no casan por no ser hex, pero un id futuro como `#fade`
 *    sí casaría.
 * 2. Anclas y `href`/`xlink:href` que apuntan a un fragmento.
 * 3. Comentarios: bloques `/* … *\/` en ambos tipos de fichero, y líneas cuyo contenido EMPIEZA
 *    por `//` o por `*` (continuación de JSDoc). Un `// … #fff` al final de una línea de código
 *    NO se excluye a propósito: quitar comentarios de final de línea sin romper `https://` o un
 *    literal de cadena requiere un tokenizador, y el sesgo correcto del error aquí es el ruido, no
 *    el silencio.
 *
 * Si este test falla por un falso positivo genuino, **no relajes el patrón**: mueve el color a
 * `theme.css` como token `--ff-*`, o —si de verdad no es un color— añade aquí una exclusión
 * documentada tan estrecha como las tres de arriba.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SRC_ROOT = path.resolve(__dirname, "..");

/** El único fichero donde un hex es correcto. */
const THEME_FILE = path.join(SRC_ROOT, "styles", "theme.css");

const SCANNED_EXTENSIONS = new Set([".css", ".tsx", ".ts"]);

/**
 * Un `#` seguido de 3–8 dígitos hexadecimales terminados en frontera de palabra: cubre `#fff`,
 * `#ffff`, `#71717a`, `#71717a80` y `#71717aff`. `{3,8}` y no `{3,}` porque un identificador largo
 * sin frontera no es un color.
 */
const HEX_PATTERN = /#[0-9a-fA-F]{3,8}\b/g;

/** Referencias a fragmentos (SVG `url(#id)`, anclas, `xlink:href`). Nunca son colores. */
const FRAGMENT_REFERENCE = /(?:url\(\s*['"]?|(?:xlink:)?href\s*=\s*\{?\s*['"`])#[\w-]+/g;

function collectFiles(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) {
      collectFiles(full, out);
      continue;
    }
    if (!SCANNED_EXTENSIONS.has(path.extname(full))) continue;
    if (full === THEME_FILE) continue;
    if (/\.test\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
}

/**
 * Neutraliza (sustituyendo por espacios, para no mover columnas ni números de línea) lo que no
 * puede contener un color: comentarios de bloque, líneas que empiezan por `//` o `*`, y las
 * referencias a fragmentos.
 */
function blankNonColorContexts(source: string): string {
  const blanks = (s: string) => " ".repeat(s.length);

  // Comentarios de bloque, preservando los saltos de línea para no desalinear el conteo.
  let out = source.replace(/\/\*[\s\S]*?\*\//g, (m) =>
    m.replace(/[^\n]/g, " "),
  );

  out = out
    .split("\n")
    .map((line) => {
      const trimmed = line.trimStart();
      // Línea de comentario completa: `//`, `///`, `//!`, o continuación de JSDoc (`*`).
      // `*/` ya quedó blanqueado arriba, así que aquí solo llegan continuaciones reales.
      if (trimmed.startsWith("//") || trimmed.startsWith("*")) return blanks(line);
      return line.replace(FRAGMENT_REFERENCE, blanks);
    })
    .join("\n");

  return out;
}

type Hit = { file: string; line: number; text: string; match: string };

function hexHits(): Hit[] {
  const files: string[] = [];
  collectFiles(SRC_ROOT, files);
  files.sort();

  const hits: Hit[] = [];
  for (const file of files) {
    const cleaned = blankNonColorContexts(readFileSync(file, "utf8"));
    cleaned.split("\n").forEach((line, i) => {
      for (const m of line.matchAll(HEX_PATTERN)) {
        hits.push({
          file: path.relative(SRC_ROOT, file),
          line: i + 1,
          text: line.trim(),
          match: m[0],
        });
      }
    });
  }
  return hits;
}

/**
 * `rgba(0, 0, 0, …)` — o cualquier variante de espaciado — fuera de `theme.css`. Mismo espíritu
 * que HEX_PATTERN: un negro-con-alfa suelto en un componente no cambia con el tema. Cubre
 * `rgba(0,0,0,` y `rgba(0, 0, 0,` (y cualquier cantidad de espacios entre comas).
 */
const RGBA_ZERO_PATTERN = /rgba\(\s*0\s*,\s*0\s*,\s*0\s*,/g;

/**
 * Excepción puntual documentada (issue #105): el box-shadow del tooltip del chart de proyección
 * se queda como `rgba(0,0,0,…)` a propósito — ver el comentario en el propio App.css junto a
 * `.projection-chart-tooltip`. Cerrada por file+línea, no por patrón, para que cualquier OTRO
 * `rgba(0,0,0,…)` que aparezca (incluida una futura línea 2356/2357 movida) siga cazándose: si
 * el tooltip se mueve, este test empieza a fallar en la línea nueva hasta que se actualice aquí,
 * en vez de dejar pasar en silencio un `rgba(0,0,0,…)` distinto que caiga en las mismas líneas.
 */
const RGBA_ZERO_EXCEPTIONS: ReadonlySet<string> = new Set([
  // La sombra del tooltip del chart (excepción documentada junto al CSS). Ola 7 (#126/#148)
  // insertó reglas más arriba en App.css y las líneas se desplazaron 2356/2357 → 2382/2383;
  // 4.15.0 (`.category-default-tag`, la marca de la categoría por defecto) las movió a
  // 2399/2400, y 5.0.0 (segmentado «Yo | Hogar» del TopBar + los dos banners de ámbito y de
  // alta) a 2425/2426. Que haya que tocar esto en cada movimiento es el precio deliberado de
  // cerrar la excepción por file+línea en vez de por patrón.
  "App.css:2425",
  "App.css:2426",
]);

function rgbaZeroHits(): Hit[] {
  const files: string[] = [];
  collectFiles(SRC_ROOT, files);
  files.sort();

  const hits: Hit[] = [];
  for (const file of files) {
    const relFile = path.relative(SRC_ROOT, file);
    const cleaned = blankNonColorContexts(readFileSync(file, "utf8"));
    cleaned.split("\n").forEach((line, i) => {
      const lineNumber = i + 1;
      if (RGBA_ZERO_EXCEPTIONS.has(`${relFile}:${lineNumber}`)) return;
      for (const m of line.matchAll(RGBA_ZERO_PATTERN)) {
        hits.push({
          file: relFile,
          line: lineNumber,
          text: line.trim(),
          match: m[0],
        });
      }
    });
  }
  return hits;
}

describe("FREEZER hex — la paleta vive solo en theme.css", () => {
  it("el barrido encuentra ficheros que mirar", () => {
    // Anti-deriva silenciosa: un barrido que no barre nada pasaría siempre. Si alguien mueve
    // `src/` o cambia las extensiones, esto lo dice en vez de dar un verde falso.
    const files: string[] = [];
    collectFiles(SRC_ROOT, files);
    expect(
      files.length,
      `no se encontró ningún .css/.tsx/.ts bajo ${SRC_ROOT}: el freezer estaría pasando por vacío`,
    ).toBeGreaterThan(20);
    expect(
      files.some((f) => f.endsWith("App.css")),
      "App.css no entró en el barrido, y es precisamente el fichero que la norma nombra",
    ).toBe(true);
    expect(
      files.includes(THEME_FILE),
      "theme.css debe quedar EXCLUIDO: es el único sitio donde un hex es correcto",
    ).toBe(false);
  });

  it("el detector reconoce un hex y no confunde una referencia a fragmento", () => {
    // Prueba de vida: sin esto, un `blankNonColorContexts` demasiado agresivo convierte el freezer
    // en un test que siempre pasa.
    const asColor = blankNonColorContexts("  color: #71717a;");
    expect(asColor.match(HEX_PATTERN)).not.toBeNull();
    expect(
      blankNonColorContexts("  --ff-fg: #fff;").match(HEX_PATTERN),
    ).not.toBeNull();

    // …y no marca lo que no es color.
    expect(
      blankNonColorContexts('  <path fill={`url(#fade)`} />').match(HEX_PATTERN),
    ).toBeNull();
    expect(
      blankNonColorContexts('  <a href="#abc">x</a>').match(HEX_PATTERN),
    ).toBeNull();
    expect(
      blankNonColorContexts("  // antes era #71717a").match(HEX_PATTERN),
    ).toBeNull();
    expect(
      blankNonColorContexts("/* legacy: #ffffff */").match(HEX_PATTERN),
    ).toBeNull();
  });

  it("no hay ni un color hex fuera de styles/theme.css", () => {
    const hits = hexHits();
    const detail = hits
      .map((h) => `  ${h.file}:${h.line}  ${h.match}   →   ${h.text}`)
      .join("\n");

    expect(
      hits,
      hits.length === 0
        ? ""
        : `Color(es) hex hardcoded fuera de src/styles/theme.css (${hits.length}):\n${detail}\n\n` +
            "La paleta vive en UN sitio: src/styles/theme.css. Los componentes y App.css consumen " +
            "`var(--ff-*)`, nunca un hex.\n" +
            "Arreglo: define (o reutiliza) el token en theme.css —con su valor para claro y para " +
            "oscuro— y sustituye el hex por `var(--ff-loquesea)`. Un hex suelto no cambia con el " +
            "tema: se ve bien en el tema en el que lo escribiste y mal en el otro, y no hay tests " +
            "de render que lo detecten (Vitest corre en `environment: \"node\"`).\n" +
            "Si de verdad NO es un color (un id de fragmento SVG, un ancla), NO relajes el patrón: " +
            "añade una exclusión estrecha y documentada en `blankNonColorContexts`, como las tres " +
            "que ya hay.",
    ).toEqual([]);
  });

  it("no hay rgba(0,0,0,…) hardcoded fuera de styles/theme.css (barrido issue #105)", () => {
    const hits = rgbaZeroHits();
    const detail = hits
      .map((h) => `  ${h.file}:${h.line}  ${h.match}   →   ${h.text}`)
      .join("\n");

    expect(
      hits,
      hits.length === 0
        ? ""
        : `rgba(0, 0, 0, …) hardcoded fuera de src/styles/theme.css (${hits.length}):\n${detail}\n\n` +
            "Las sombras/negros con alfa viven en tokens de theme.css (--ff-shadow-*). Define (o " +
            "reutiliza) el token con el mismo valor rgba y sustituye por `var(--ff-shadow-loquesea)`.\n" +
            "Si es una excepción deliberada de verdad (como el tooltip del chart de proyección), " +
            "documéntala junto al CSS y añade su file:línea a RGBA_ZERO_EXCEPTIONS en este fichero " +
            "— no relajes RGBA_ZERO_PATTERN.",
    ).toEqual([]);
  });
});
