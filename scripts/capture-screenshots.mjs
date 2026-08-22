/**
 * Capturas de la app para el README y la documentación.
 *
 * Salen SIEMPRE de una instalación sembrada con `scripts/seed-demo.sh`, nunca de una real: una
 * captura del Resumen es el patrimonio neto de alguien (skill `futurefin-data-hygiene`).
 *
 * Requisitos (Playwright NO es dependencia del proyecto a propósito: son ~95 MB de navegador que
 * no tiene por qué descargarse quien solo quiere compilar la app):
 *   npm i --no-save playwright
 *   npx playwright install chromium
 *   ./scripts/seed-demo.sh URL           # datos de demo en una instalación vacía
 *
 * Uso:
 *   node scripts/capture-screenshots.mjs [URL] [USUARIO] [CONTRASEÑA] [DESTINO]
 *   node scripts/capture-screenshots.mjs http://localhost:8080 demo demo-futurefin-2026 docs/img
 *
 * Genera cada vista en claro y oscuro, más una móvil. El tema se fuerza escribiendo la misma
 * preferencia que guarda la app en `localStorage`, para no depender del tema del sistema.
 */

import { mkdir } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";

const BASE = process.argv[2] ?? "http://localhost:8080";
const USER = process.argv[3] ?? "demo";
const PASS = process.argv[4] ?? "demo-futurefin-2026";
const OUT = process.argv[5] ?? "docs/img";

/** Clave de `localStorage` donde la app guarda el tema (ver `apps/web/src/lib/theme.ts`). */
const THEME_KEY = "ff-theme-pref";

const VIEWS = [
  { slug: "resumen", path: "/resumen", label: "Resumen" },
  { slug: "activos", path: "/activos", label: "Activos" },
  { slug: "pasivos", path: "/pasivos", label: "Pasivos" },
  { slug: "presupuesto", path: "/presupuesto", label: "Presupuesto" },
  { slug: "movimientos", path: "/movimientos", label: "Movimientos" },
  { slug: "proximos", path: "/proximos", label: "Próximos" },
  { slug: "jubilacion", path: "/jubilacion", label: "Jubilación" },
  { slug: "proyeccion", path: "/proyeccion", label: "Proyección" },
  { slug: "ajustes", path: "/ajustes/general", label: "Ajustes" },
];

const DESKTOP = { width: 1440, height: 950 };
const MOBILE = { width: 414, height: 896 };

/** Espera a que la vista deje de estar cargando: sin spinners y con la red tranquila. */
async function settle(page) {
  await page.waitForLoadState("networkidle").catch(() => {});
  // Los charts animan al entrar; sin esto salen a medio dibujar.
  await page.waitForTimeout(900);
}

/**
 * Inicia sesión por la API, no rellenando el formulario.
 *
 * `context.request` comparte el tarro de cookies con las páginas del contexto, así que la cookie
 * `ff_session` que devuelve el login ya viaja en la primera navegación. Conducir el formulario
 * fallaba **en silencio** —la captura salía con la pantalla de acceso vacía— y un fallo mudo en
 * un generador de capturas es peor que un error: publicas el login creyendo que es el Resumen.
 */
async function login(context) {
  const res = await context.request.post(`${BASE}/v1/auth/login`, {
    data: { username: USER, password: PASS },
  });
  if (!res.ok()) {
    throw new Error(
      `login fallido (${res.status()}): ${await res.text()}\n` +
        `¿Has sembrado la demo con ./scripts/seed-demo.sh ${BASE} ${USER} …?`,
    );
  }
}

/** Aborta si la página sigue enseñando el formulario de acceso: sin esto, una sesión que no cuaja
 *  produce ocho capturas idénticas de la pantalla de login. */
async function assertLoggedIn(page) {
  const acceder = page.getByRole("heading", { name: "Acceder" });
  if (await acceder.isVisible().catch(() => false)) {
    throw new Error("la app sigue en la pantalla de acceso: la sesión no ha cuajado");
  }
}

async function capture(browser, theme, viewport, suffix) {
  const context = await browser.newContext({
    viewport,
    deviceScaleFactor: 2,
    locale: "es-ES",
    timezoneId: "Europe/Madrid",
    colorScheme: theme === "dark" ? "dark" : "light",
  });
  // El tema se fija ANTES de que arranque la app: si se cambia después, el primer pintado sale
  // con el tema anterior y la captura lo pilla a medias.
  await context.addInitScript(
    ([key, value]) => {
      try {
        window.localStorage.setItem(key, value);
      } catch {
        /* modo privado: nos quedamos con colorScheme */
      }
    },
    [THEME_KEY, theme],
  );

  await login(context);
  const page = await context.newPage();

  for (const view of VIEWS) {
    await page.goto(`${BASE}${view.path}`, { waitUntil: "domcontentloaded" });
    await settle(page);
    await assertLoggedIn(page);
    const file = path.join(OUT, `${view.slug}-${suffix}.png`);
    await page.screenshot({ path: file });
    process.stdout.write(`  ${file}\n`);
  }

  await context.close();
}

async function main() {
  await mkdir(OUT, { recursive: true });
  const browser = await chromium.launch();
  try {
    console.log("Capturando (claro, escritorio)…");
    await capture(browser, "light", DESKTOP, "claro");
    console.log("Capturando (oscuro, escritorio)…");
    await capture(browser, "dark", DESKTOP, "oscuro");
    console.log("Capturando (claro, móvil)…");
    await capture(browser, "light", MOBILE, "movil");
  } finally {
    await browser.close();
  }
  console.log(`\nListo. Imágenes en ${OUT}/`);
}

await main();
