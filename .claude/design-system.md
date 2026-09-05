# Design System — V1 redesign (May 2026)

Refresca de UI completa, **solo frontend**: no toca handlers, datos ni endpoints. Origen: bundle de Claude Design exportado (`futurefin/`). El diseño se llamó "V1" en el chat de iteración — el resto de variantes (V2–V4) se descartaron.

> **Si haces cambios visuales, lee este doc primero.** Los tokens y reglas viven aquí; no improvises colores ni clases nuevas en App.css.

## Identidad

- **Base monocromática**: blanco roto (zinc-100 `#f4f4f5`) en claro / casi-negro (zinc-950 `#0a0a0a`) en oscuro.
- **Único color de marca**: acento periwinkle (`oklch(0.56 0.13 250)` claro, `oklch(0.74 0.11 250)` oscuro). Cualquier "destacado" del UI usa solo este color.
- **Sin tonos cálidos**: no hay crema, beige, etc. Si encuentras alguno en el código, restitúyelo a un zinc.
- **Semánticos pos/neg**: verde/rojo **únicamente para texto de cifras delta** (deltas, saldos, "−€640"). Nunca en fondos, bordes, iconos o chrome decorativo.
- **Escala de ESTADO — la excepción acotada** (`--ff-neg`, `--ff-warn`; 5.0.0, D17/D28): tres tarjetas y un banner tiñen su PIEL con un semáforo — `.error-banner`, `.plan-card--danger`, `.metric-card--danger` y `.metric-card--warn`. No contradice la regla de arriba: no es color de cifra ni decoración, es el estado de un plan («llega» / «al límite» / «no llega»), y el tinte es siempre el mismo (8 % sobre `--ff-paper`, borde al 45 %) para que se lea como **un solo vocabulario**. Fuera de esa lista, pos/neg/warn siguen prohibidos en fondos, bordes, iconos y chrome. **Verde no tiene piel**: «va bien» es el estado normal, y teñir también el caso bueno convierte el color en ruido.
- **Gráficas — única excepción**: las charts (proyección, mini-projection, donut del Resumen) pueden usar varios colores funcionales para distinguir series. Sigue siendo una paleta sobria.

## Tokens

Definidos en [`apps/web/src/styles/theme.css`](../apps/web/src/styles/theme.css). Todos los componentes y `App.css` deben consumirlos vía `var(--ff-*)`; nada de hex hardcoded fuera de `theme.css`.

| Token | Light | Dark | Uso |
|---|---|---|---|
| `--ff-bg` | `#f4f4f5` | `#0a0a0a` | Fondo de la app |
| `--ff-paper` | `#fafafa` | `#18181b` | Paneles, KPIs, botones secundarios |
| `--ff-frame` | `#ffffff` | `#27272a` | Cabecera, modales |
| `--ff-ink` | `#0a0a0a` | `#fafafa` | Texto principal |
| `--ff-ink-soft` | `#404040` | `#d4d4d8` | Labels, meta-info |
| `--ff-muted` | `#737373` | `#a1a1aa` | Hints, ticks, placeholders |
| `--ff-line-soft` | `#e4e4e7` | `#3f3f46` | Bordes habituales |
| `--ff-accent` | `oklch(0.56 0.13 250)` | `oklch(0.74 0.11 250)` | Único color de marca |
| `--ff-accent-fg` | `#fff` | tinta deep | Texto sobre acento (raro) |
| `--ff-pos` / `--ff-neg` | oklch deep | oklch pastel | **Solo cifras delta** |
| `--ff-warn` | `oklch(0.60 0.14 75)` | `oklch(0.80 0.13 75)` | **Ámbar de ESTADO** (5.0.0, D28) — el peldaño intermedio del semáforo. Mismas restricciones que pos/neg |

> **`--ff-warn` — por qué hay un tercer color de estado (5.0.0, D28, issue #207).** El KPI «Éxito
> del plan» tiene TRES valores, no dos: **verde SOLO al 100 %** (cero escenarios agotados), ámbar
> hasta diez puntos porcentuales por debajo, rojo el resto. El corte lo fijó V7 de la tercera
> vuelta de UX y **ya no es configurable**: el umbral de éxito del perfil desapareció. Con solo
> `--ff-pos`/`--ff-neg` el peldaño intermedio se pintaba de rojo (alarma donde no la hay) o de nada
> (indistinguible de «va bien»), y el semáforo dejaba de ser un semáforo. El token es el **hermano
> de `--ff-neg` en la escala de estado**: mismo croma y misma luminancia en cada rama, hue 75. No es
> un acento nuevo y **no decora**: la restricción es la de pos/neg — solo marca estado, nunca
> chrome, iconos ni texto decorativo. El veredicto lo decide el SERVIDOR (`success_verdict`); la SPA
> solo lo traduce a tono.
>
> **Segundo consumidor desde V5: el degradado de la banda de riesgo.** `lib/risk-gradient.ts` mezcla
> `--ff-pos` → `--ff-warn` → `--ff-neg` por probabilidad de agotar el capital, con cortes ABSOLUTOS
> (0 % · 5 % · 10 %) que son el mismo semáforo leído del lado de la ruina: el 10 % de escenarios
> agotados ES el 90 % de éxito, el corte por debajo del cual el servidor da el plan por rojo. Es
> color de DATO dentro de un chart (excepción de gráficas), no chrome. Si alguien mueve un extremo
> del veredicto tiene que mover el otro: los dos salen de la misma tabla.

Radii: `--ff-radius-{frame=12, panel=14, kpi=12, pill=999, input=10}px`.

### Tokens del chart de proyección

`--proj-nw`, `--proj-cc`, `--proj-fire`, `--proj-grid`, `--proj-plot-bg`, `--proj-tick`, `--proj-meta`, `--proj-axis`, `--proj-milestone`, `--proj-crosshair`.

> **`--proj-jub` está definido pero MUERTO** (verificado 2026-08-22): existe en las dos ramas de
> [`theme.css`](../apps/web/src/styles/theme.css) como alias de `--ff-accent` y **nadie lo consume**
> (`grep -rn 'proj-jub' apps/web/src` solo devuelve sus dos definiciones). La línea de jubilación
> del chart usa `--proj-fire`. No lo cites como token vivo ni construyas nada nuevo sobre él: o se
> conecta a un consumidor real, o se borra. Se documenta aquí en vez de callarlo porque un token
> huérfano que parece vivo se acaba usando y arrastra una intención que nadie decidió.

Áreas de activos: `--proj-area-1` a `--proj-area-10` (paleta polícroma — azul/teal/violeta/... en claro, pasteles más claros en oscuro). Consumidos por [`ASSET_LINE_COLORS`](../apps/web/src/lib/projection-chart.ts).

Series auxiliares del plan (5.0.0, D29, issue #207): **`--proj-required`** («Capital necesario») y
**`--proj-coast`** («Si dejas de aportar en el mes coast»), las dos discontinuas y consumidas solo
desde [`lib/plan-series.ts`](../apps/web/src/lib/plan-series.ts). Son **familia del acento** —lo
que el plan exige, igual que el objetivo FIRE— y se separan entre sí por **luminancia y patrón de
guion**, no por un hue nuevo: `color-mix(in oklch, var(--ff-accent) 80%|45%, var(--proj-nw))`, con
dash `6 4` y `2 5`. Al derivarse de dos tokens que ya tienen rama clara y oscura, el mismo texto
resuelve distinto en cada tema (mismo patrón que `--cf-savings-cash`); aun así se **repiten** en
los dos bloques de `theme.css`, como sus vecinos. La paleta polícroma de `--proj-area-*` estaba
descartada a propósito: esas diez son ÁREAS de activo y una línea fina del mismo hue se leería como
un activo más.

Direccionales: **`--proj-pos` / `--proj-neg`** (alias de `--ff-pos`/`--ff-neg` en las dos ramas de
[`theme.css`](../apps/web/src/styles/theme.css)). Consumidores: las líneas/labels de planning inflow–outflow del
chart de proyección (`App.css`, `.projection-chart-planning-inflow-line` y hermanas) y las barras de
`PlanningDirectionChart` (`.planning-dir-bar-in/-out`; ver
§Componentes nuevos (charts)). Son colores **funcionales de serie**, amparados por la excepción de
charts — mismas restricciones que `--cf-income`/`--cf-expense`: prohibidos en chrome, texto o iconos.

#### Región histórica (pasado) — v1.5.0

El chart se extiende a la izquierda con la serie histórica de patrimonio (meses `< 0`). Cuatro tokens nuevos, todos grises deliberados (el pasado es "atenuado", nunca acento):

| Token | Light | Dark | Uso |
|---|---|---|---|
| `--proj-nw-past` | `#525252` | `#a1a1aa` | Línea de patrimonio neto en el tramo pasado (2,25px; el futuro sigue en `--proj-nw`) |
| `--proj-past-bg` | `rgba(10, 10, 10, 0.03)` | `rgba(250, 250, 250, 0.04)` | Rect de fondo sobre toda la región `< 0` (velo tenue que separa pasado de futuro) |
| `--proj-today-divider` | `#a3a3a3` | `#52525b` | Divisor vertical «Hoy» en `x = 0`, con etiqueta |
| `--proj-snapshot` | `#404040` | `#d4d4d8` | Marcadores de snapshot (círculo relleno = asset, hueco = liability) |

Las áreas apiladas por activo del pasado **no** llevan token propio: reutilizan `--proj-area-1..10` (misma regla de rescale I6 que en el futuro). Verifica claro **y** oscuro: el velo `--proj-past-bg` debe leerse sin oscurecer las áreas.

### Tokens de las gráficas de la pestaña «Movimientos» — v1.8.0

Una gráfica de apoyo (`components/charts/CategoryComparisonBars.tsx` → export único `MonthlyCashflowBars`), **amparada por la excepción de charts** (varios colores funcionales para distinguir series). Fuera de esa gráfica, estos tokens están **prohibidos en chrome, texto o iconos** — como cualquier color de serie.

| Token | Light | Dark | Uso |
|---|---|---|---|
| `--cf-income` | `oklch(0.58 0.10 165)` | `oklch(0.72 0.10 165)` | Serie **Ingresos** del cash-flow mensual (`MonthlyCashflowBars`) — verde sobrio |
| `--cf-expense` | `oklch(0.58 0.13 25)` | `oklch(0.70 0.13 25)` | Serie **Gastos** del cash-flow — rojo sobrio |
| `--cf-savings` | `= var(--ff-accent)` | `= var(--ff-accent)` | Serie **Invertido** del cash-flow (la clase `savings`, rotulada «Inversión» desde 4.15.0) |
| `--cf-savings-cash` | `color-mix(in oklch, var(--cf-savings) 40%, var(--ff-paper))` | ídem (resuelve por rama) | Sub-segmento **En cuenta** del ahorro (4.15.0): la parte de ingresos − gastos que no se invirtió. Derivado del token de inversión para que se lea como «el mismo ahorro, más claro» |

> **Comparativa por categoría eliminada**: el chart `CategoryComparisonBars` (barras horizontales Budget vs Promedio) se retiró tras 2.0.0 — con él se fueron el token **`--exp-average`** y el bloque CSS `.cmp-*`. El valor Real ya vivía en la tabla y las KPIs, y la tendencia vs presupuesto pasó a la banda de KPIs (ver §KPIs). El único chart que queda en ese archivo es `MonthlyCashflowBars`.
>
> **Composición de la barra (4.15.0)**: hacia arriba Ingresos; hacia abajo Gastos + Ahorro(neto), con el ahorro
> partido en «invertido» (`--cf-savings`) y «en cuenta» (`--cf-savings-cash`), de modo que la parte sólida
> inferior mide exactamente lo que la superior cuando ingresos ≥ gastos. Los dos casos que rompen esa igualdad
> llevan **trama** y nombre: «déficit» (gastos > ingresos, color de gasto) y «de reservas» (inversión > ahorro,
> color de inversión). La trama es `.cf-bar--hatched` = `repeating-linear-gradient(45deg, var(--ff-paper) 0 2px,
> transparent 2px 5px)` sobre el color de serie — cero hex y resuelve por tema sola. **No** se usa un
> `<pattern>` SVG: el chart es `div`+CSS y convertirlo a SVG no compraría nada. La aritmética vive pura en
> `lib/cashflow-bars.ts` (testeada), el componente solo pinta.
>
> **Excepción explícita a la regla "sin rojo/verde en el chrome"**: el cash-flow (`MonthlyCashflowBars`) introduce **verde/rojo** (`--cf-income`/`--cf-expense`) para ingresos vs gastos: son colores **funcionales de serie del gráfico**, no chrome decorativo, y por tanto quedan dentro de la única zona (charts) donde el design system acepta varios colores. Verifica claro **y** oscuro.

## Tema (auto / claro / oscuro)

- Estado vive en `App.tsx` (`themePref: ThemePref`), persistido en `localStorage["ff-theme-pref"]`.
- Default: `"auto"` → sigue `prefers-color-scheme` y se suscribe a cambios en vivo.
- Aplicación: `applyTheme(pref)` pone `data-theme="dark"|"light"` en `<html>`.
- Toggle UI: segmented Auto/Claro/Oscuro en `Ajustes → General → Apariencia` (la sub-pestaña se
  reorganizó en 3.10.0; los nombres vivos están en `SETTINGS_SUBTAB_LABEL`, `lib/navigation.ts`).
- Helpers en [`apps/web/src/lib/theme.ts`](../apps/web/src/lib/theme.ts).

## Formato de cifras y copy

Reglas de presentación de toda cifra y texto del UI (los helpers viven en
[`apps/web/src/lib/format.ts`](../apps/web/src/lib/format.ts)):

- **Importes**: sin decimales, símbolo de moneda detrás del número (`1.234 €`). Usa
  `formatCurrencyAmount` / `formatCurrencyNumber` — nunca `toString()` ni concatenación manual.
  Las únicas excepciones sancionadas son dos, ambas de chart — ver §Formateo de importes en charts.
- **Porcentajes**: exactamente un decimal, sufijo ` %` (`3,5 %`). Usa `formatPercentAmount` /
  `formatPercentDisplay`. La función ya incluye el sufijo — no lo añadas encima.
- **Copy**: mínimo — labels cortos, estados vacíos en pocas palabras (`Sin datos.`). Los estados
  canónicos de panel están en §Paneles de Ajustes.

## Shell

- **TopBar única** ([`components/TopBar.tsx`](../apps/web/src/components/TopBar.tsx)): marca `FF · FutureFin` izquierda, pills de navegación derecha, slot `extras` anclado en esquina superior derecha, botón hamburguesa solo `≤720px`.
- **Slot `extras` — control segmentado «Yo | Hogar»** (5.0.0, D9/D32, issue #207): sustituye al `<select>` de vista mío/hogar. Reusa la piel de `.ff-theme-toggle` (mismo lenguaje visual que el toggle de tema, ver §Componentes) con la clase modificadora `.ff-topbar-scope` — solo compacta padding/tipografía (`button { padding: 0.3rem 0.72rem; font-size: 0.78rem }`) para que quepa junto al título, el indicador de salud y la hamburguesa sin romper la regla de oro (cero scroll horizontal a 360px). `role="group"` + `aria-pressed` por botón (no `role="radiogroup"`: son dos botones independientes con `is-active`, patrón idéntico a `ThemeToggle`), **más estrecho que el `<select>` anterior en todos los breakpoints** (verificable: `git show b413471 --stat -- apps/web/src/App.css` — el bloque `.ledger-view-select` se borra entero, incluida su variante `≤720px`).
- **Banner de ámbito `.app-scope-banner`** (5.0.0, D9/D32): cuando la vista es Hogar (`scopeReadOnly`), «Vista agregada del hogar · solo lectura» se pinta como **primer hijo de `<main>`, en TODAS las pestañas** — no solo las del ledger, porque el ámbito es global y quien navega directo a Jubilación desde el drawer no ha visto ningún otro banner. Reusa `.banner.info-banner` (cero color propio, resuelve claro/oscuro solo); la clase propia (`display:flex; flex-direction:column`) solo existe porque en Proyección `.app-main` es un flex-column con `overflow:hidden` y sin `flex: 0 0 auto` el banner cedería altura al chart.
- **Banner de alta de Jubilación — RETIRADO** (nació en 5.0.0 D33 como `.retirement-intro-banner`, muere en la tercera vuelta de UX, F5): «Elige tu estrategia de jubilación» sobre una pantalla que YA tiene una estrategia elegida es un cartel que sobra, y su descarte vivía en un flag de `localStorage` (`lib/retirement-intro.ts`) que **nunca miraba el perfil**, así que reaparecía en cada navegador nuevo por muy configurado que estuviera el plan. Con él se fueron el módulo, su test y su bloque CSS. La lección, si alguien propone otro banner de alta: **un aviso de onboarding se apaga con el ESTADO que lo hace innecesario, no con un flag por navegador.**
- **Sin tab-bar separada**: la `.tab-bar` legacy está `display:none`. Toda la navegación vive en TopBar.
- **Móvil**: hamburguesa abre `MobileNavDrawer` ([`components/MobileNavDrawer.tsx`](../apps/web/src/components/MobileNavDrawer.tsx)) — drawer lateral derecho con todas las secciones. Sin bottom-nav.
- **Cuenta**: el chip de usuario + Salir vivían en la cabecera; ahora la cuenta vive en `Ajustes` como `AccountCard` ([`components/AccountCard.tsx`](../apps/web/src/components/AccountCard.tsx)) con avatar, badge de rol, botones Editar cuenta + Cerrar sesión. La cabecera queda limpia.

## Layout

- **Ancho de contenido**: `max-width: 66rem` (`app-main`). Antes era ancho completo; ahora el contenido se centra. Proyección sigue siendo full-bleed (`.app-main--projection-fullbleed`).
- **KPIs**:
  - Tile con borde + paper, radius `--ff-radius-kpi`, `align-self: stretch` para alinear en altura.
  - **Slot del paréntesis siempre presente** — `MetricCard` renderiza un `<div>` con `&nbsp;` cuando no hay valor, para que dos KPIs en la misma fila tengan baseline alineada. La info adicional de una KPI va **siempre en la prop `parenthetical`, nunca en `suffix`**.
  - **Slot compartido `trend`** — prop `trend?: ReactNode` de `MetricCard` que ocupa **ese mismo slot reservado** (baseline intacta) y tiene **prioridad** sobre `parenthetical`. Se usa en la banda de Movimientos para la tendencia «promedio vs presupuesto» (flecha + delta + «vs presupuesto»). CSS `.metric-trend` (una sola línea, `white-space: nowrap`) con hijos `.metric-trend-arrow` / `.metric-trend-delta` (flecha y cifra: prioritarios, nunca truncados, color solo aquí vía `num-pos`/`num-neg`) y `.metric-trend-label` (cede espacio con ellipsis en tarjetas estrechas, hereda muted).
  - Variantes: `tone="hero" | "accent" | "accent-2"` con tinte progresivo del acento.
- **Adornos en celdas numéricas alineadas a la derecha**: cualquier adorno variable (flechas de tendencia, badges) va en un **slot de ancho fijo siempre reservado** tras la cifra (mismo principio que el paren-slot de `MetricCard`) — nunca condicional. Si el adorno solo aparece en algunas filas, las cifras se desalinean; el slot vacío mantiene la columna. Precedente: `.exp-trend-slot` en la comparativa de Movimientos.

## Responsive / móvil

Sistema responsive base introducido en v1.7.0 (Workstream A). Toda la doctrina vive en la cabecera de [`App.css`](../apps/web/src/App.css) y en la sección `MOBILE` al final del mismo archivo.

### Regla de oro

**La página solo scrollea en vertical.** Cero scroll horizontal a nivel de página en cualquier ancho. El único scroll lateral permitido es **dentro** de una tabla (`.table-scroll { overflow-x: auto }`), como válvula residual — nunca la página entera. Antes de mergear cualquier cambio visual, verifica en 360 / 390 / 430 / 639 / 641 / 719 / 721 px que `document.scrollingElement.scrollWidth <= innerWidth`.

**A ESCRITORIO la válvula no debe dispararse tampoco (5.0.0, F3/P7).** `.app-main { max-width: 66rem }` fija el contenido a ~1.024–1.056 px en CUALQUIER viewport de escritorio — un breakpoint solo «arregla» una franja y reabre el bug en un monitor más ancho. Patrón reutilizable para una tabla con columnas de contenido intrínseco (nowrap, `<select>`s, chips) que no caben en ese ancho: `table-layout: fixed` en la tabla + `<colgroup>` con `<col className="col-…">` por columna (nunca `style=`; anchos en `rem` para columnas de contenido fijo — fecha, acciones — y en `%` para las proporcionales; como mucho una columna sin ancho, `.col-remainder`, que se lleva el resto). Aplicado en `.assets-table--budget-lines` (Presupuesto, ambas tablas) y `.exp-movements-table--fixed` (Movimientos, solo escritorio — móvil sigue en 3 columnas y `table-layout: auto`, sin colgroup). Texto largo en una celda de ancho fijo va en un `<span>` con ellipsis de una línea y `title={…}` para el texto completo (`.exp-concept-text`), nunca en la celda directamente — así los hermanos (chips, tags) siguen pudiendo saltar de línea.

### Breakpoints canónicos (solo dos)

CSS no admite `var()` dentro de `@media`, así que la convención es un **comentario greppable `bp:`** etiquetando cada media query:

| Breakpoint | Etiqueta | Gobierna |
|---|---|---|
| `720px` | `/* bp:struct 720 */` | Estructura: nav, colapsos de columnas, paddings de shell, **KPI strip** |
| `640px` | `/* bp:mobile 640 */` | Densidad phone: táctil, toolbars, paddings de modal/panel, forms |

**Prohibido** introducir un tercer breakpoint estructural sin actualizar este documento. Auditoría: `grep -n "bp:" apps/web/src/App.css` lista todas las media queries responsive.

Excepciones sancionadas (por componente, no ejes estructurales):
- `340px` `/* bp:edge 340 */` — guarda de borde ultra-estrecho que oculta solo el título del TopBar (queda el logo). No cuelgues más reglas de este ancho.
- `1000px` `/* bp:topbar 1000 */` — colapso de la nav del TopBar a hamburguesa: las 9 pills necesitan ~982px y entre 721-980px desbordaban la página entera (violación de la regla de oro detectada en QA v1.7.0). Solo reglas del TopBar pueden usar este ancho; el resto de la estructura sigue en 720.

### Táctil mínimo + carve-out

- Token `--ff-touch-min: 2.75rem` (≈44px, HIG / WCAG 2.5.5) en [`theme.css`](../apps/web/src/styles/theme.css). Es **inerte**: solo lo consume la sección `MOBILE` dentro de `@media (max-width: 640px)`.
- Controles primarios a ≥ `--ff-touch-min` en `≤640`: `.btn`, `.btn.icon-btn`, `.field input/select` (checkbox/radio excluidos), `.modal-close`, hamburguesa (`.ff-topbar-mobile-toggle`), items del drawer (`.ff-mobile-drawer-item`), switch `.ff-switch` (`2.6×1.5rem`; Proyección y Ajustes → Integraciones).
- **Carve-out obligatorio**: los controles densos dentro de tablas quedan **excluidos** del bump (`min-height: 0`, y `min-width: 0` en los icon-btn) — su densidad la gobierna el trabajo de tablas móviles, no el sistema base. Selectores exentos: `.assets-table .btn`, `.asset-actions-cell .btn`, `.budget-row-actions .btn`, `.exp-inline-select`, `.import-preview-table select`. Si añades un control táctil nuevo dentro de una tabla, añádelo al carve-out.

### Patrón KPI strip

La banda de KPIs (`.metric-grid.workspace-kpi-strip`) es `flex nowrap + overflow-x:auto` en desktop (una fila, scroll deliberado). En `≤720` pasa a **grid `auto-fit`** (`minmax(min(100%, 9.5rem), 1fr)`, `overflow: visible`) → filas de 2 (2×2 a 390px), sin scroll-X. `min-width: 0` en las cards es **crítico** (el `10rem` de desktop desbordaría). El `metric-value` usa `clamp(1rem, 4.4vw, 1.25rem)`. El slot del paréntesis se mantiene (baseline por fila).

### Patrón toolbar

Toolbars horizontales con `margin-left: auto` se apilan en `≤640` a **columna full-width** matando el `margin-left`: fila por grupo, controles a `flex: 1` o `width: 100%`. Aplicado a `.expenses-toolbar` (3 filas: mes · ventana · acciones), `.panel-head-row`, `.import-bulk-bar`, `.import-footer`.

### Modales

En `≤640`: backdrop `0.5rem`, header/body reducidos, y `.asset-form-actions` → `column-reverse` full-width (botones apilados, orden DOM conservado para tab/a11y).

### Invariante de regresión (desktop cero)

Ninguna declaración nueva fuera de `@media (max-width: …)` salvo **tokens inertes** en `theme.css` y los `minmax(min(100%, X), 1fr)` (no-op en desktop: el contenedor siempre supera `X`). Screenshot a 1280px antes/después debe ser pixel-idéntico. La sección `MOBILE` vive al final de `App.css` para ganar por cascada a las reglas grid inertes de las KPI strips.

## Componentes nuevos (charts)

### `MiniProjection` — [`components/charts/MiniProjection.tsx`](../apps/web/src/components/charts/MiniProjection.tsx)

Chart compacto reutilizable, mismo lenguaje visual que la Proyección. Usado en Resumen (12 m) y Jubilación. **Para cualquier chart pequeño nuevo, usa este componente en lugar de SVG custom** — comparte tokens con el chart grande y soporta `zoomY`, `clampToMonth`, `xAxis`, áreas escaladas al NW.

Props clave:
- `series: ProjectionSeriesApi | null` — la serie ya cargada por `App.tsx` desde `GET /v1/projection/series` (no hace fetch propio).
- `months?` — recorta la ventana visible.
- `clampToMonth?` — última posición (mes) visible; tiene prioridad sobre `months`. Usado en Jubilación para mostrar `jub + 12`.
- `zoomY?` — eje Y entre min/max de los valores visibles. Combinable con áreas (el stack se ancla al suelo del rango).
- `showAreas?` — apila los `asset_series`.
- `showFire?`, `showJub?` — overlays del target FIRE y marcador del primer cruce.
- `xAxis?: { ageUiMode, birthDateIso, anchorDateYmd, calendarTz }` — cuando se pasa, dibuja ~5 ticks de edad/fecha en la base.

**Invariante crítica**: las áreas se escalan proporcionalmente a NW(t), idéntico a [`ProjectionNetWorthChart`](../apps/web/src/views/ProjectionNetWorthChart.tsx#L213-L223): `area_i(t) = NW(t) × (asset_i(t) / Σ asset_j(t))`. Por construcción la suma de áreas == NW, así que **las áreas nunca pueden exceder la línea NW** geométricamente.

**Marcador circular**: el SVG usa `viewBox=containerW × height` medido con `ResizeObserver`, por lo que las unidades del viewBox = píxeles reales y los `<circle>` salen circulares aun con `preserveAspectRatio="none"` para las polylines.

#### Props opcionales `band` / `markers` / `deflator` (5.0.0, U1b, decisión U5 de #207)

Jubilación pasaba de DOS charts —el determinista de arriba y el abanico `RiskFanChart` de la
sección «Riesgo», con ejes X distintos— a UNO: `MiniProjection` absorbió el abanico. Las tres son
**opcionales y no-op** cuando no se pasan (el Resumen no las usa y su chart sigue byte a byte el
de 4.15.x):

- **`band?: {month, p10, p90}[]`** — la banda 10–90 % de los escenarios con volatilidad, en euros
  NOMINALES y **por MES** (nunca por posición: la banda viaja SIEMPRE a densidad `hybrid` y
  `points[]` puede ser `monthly`, la segunda fase del two-phase). Se recorta a la ventana visible
  por `month`, se ordena y se pinta como UN `path` cerrado (p90 de ida, p10 de vuelta) — nunca dos
  polígonos, que dejarían una costura de 1 px en oscuro. Relleno `var(--ff-accent)` al 16 % +
  trazo al 30 % (opacidad en el atributo, nunca en el color, para que el mismo token resuelva
  claro/oscuro): es la lectura del plan, familia del objetivo FIRE. **Sin trazo de mediana** — el
  componente no recibe `p50` en absoluto (el tipo `MiniProjectionBandPoint` no tiene ese campo);
  la mediana sigue viva en otras lecturas (`lib/risk-bands.ts`, filas de detalle), pero ya no se
  dibuja como línea. Menos de dos puntos dibujables no es media banda: es ninguna.
- **`markers?: RetirementChartMarker[]`** (`lib/retirement-chart.ts`) — hasta cuatro hitos del
  plan (jubilación/coast/media jornada/pensión), cada uno una línea vertical SIEMPRE pintada +
  rótulo que se cede por prioridad si colisiona con uno ya puesto (`placeMarkerLabels`, `minGapPx`
  = 46 por defecto; la jubilación nunca cede el suyo). La jubilación usa `--ff-accent` sólido
  (1.5px); las secundarias, `--proj-meta` discontinuo (`3 3`, 1px) — son contexto, no el hito que
  la página contesta.
- **`deflator?: (monthIndex: number) => number`** — UN factor aplicado a patrimonio, objetivo FIRE
  y banda **por igual**; las áreas de activo lo heredan al escalarse al patrimonio. Deflactar solo
  una serie separaría el abanico de la línea que dice contener, y el chart seguiría pareciendo
  correcto.

#### Props opcionales `yAxis` / `bandGradient` / `bandEdgeLabels` / `hoverLabel` (5.0.0, V2/V5)

La tercera vuelta de UX arrancó del veredicto del owner sobre el chart de riesgo: «no deja nada
claro qué representa cada cosa» (F6), «los tiles no muestran nada que la gráfica no muestre» (F7),
«añadir riesgo en color rojo/verde» (F8). Las cuatro props responden a eso y **todas son opt-in**:

- **`yAxis?: { currencyIso }`** — importes en el eje. Reserva una canaleta a la izquierda
  (`padLeft = W < 420 ? 34 : 46`, los dos anchos del chart grande) y pinta ~4 líneas de rejilla
  (`niceYTicks` filtrado a `[vmin, vmax]`, `--proj-grid`) rotuladas con `formatAxisMoney`, que es
  una de las **dos excepciones sancionadas** a los helpers canónicos (ver §Formateo de importes en
  charts). Los valores ya vienen deflactados, así que «En dinero de hoy» mueve el eje entero.
- **`bandGradient?: RiskGradientStop[]`** — el relleno de la banda por probabilidad de agotar el
  capital ([`lib/risk-gradient.ts`](../apps/web/src/lib/risk-gradient.ts)). `<linearGradient>` con
  **`gradientUnits="userSpaceOnUse"` obligatorio**: con el default (`objectBoundingBox`) el
  degradado se estiraría a la caja del `path` —que no empieza en `monthStart` ni acaba en
  `monthEnd`— y el mapeo mes→color se desplazaría sin que nada fallara. Los `<stop>` llevan
  `style={{ stopColor }}` porque el valor es un `color-mix()` y el atributo de presentación de
  SVG 1.1 no lo acepta: **es la única excepción sancionada a «cero estilos inline»**, y el `id`
  va prefijado `ff-risk-` para que un `#` en `url(#…)` nunca se confunda con un hex. Opacidades
  0,28 / 0,55 con degradado; 0,16 / 0,30 sin él (el acento plano de siempre).
- **`bandEdgeLabels?: { p10, p90 }`** — qué es cada borde, en el extremo derecho y dentro del
  plot, con halo (`.proj-mini-band-label`, `paint-order: stroke`). Se omiten **los dos** si los
  bordes distan < 14 px: media etiqueta rotularía el borde equivocado.
- **`hoverLabel?: (month) => string | null`** — el porcentaje exacto por edad. Rect captor
  `fill="none" pointerEvents="all"` el ÚLTIMO del SVG, crosshair `--proj-crosshair` y texto con
  halo. Lo construye el llamante desde `depletionProbabilityAtMonth`, **la misma función que
  colorea**: un tooltip alimentado por otro cálculo podría contradecir al tinte y nadie lo notaría.

> **Invariante: sin estas props, la geometría es BYTE A BYTE la de antes.** El Resumen usa este
> mismo componente y su chart no puede moverse un píxel por un cambio que solo pedía Jubilación.
> Lo que la sostiene son dos cosas: `padLeft` degrada a `padX` (4) cuando no hay `yAxis`, y cada
> bloque nuevo vive dentro de un `prop ? … : null` — sin prop no se emite ni un nodo. Si tocas la
> geometría, la verificación es una captura a 1280 px del Resumen antes y después.

**`RiskFanChart.tsx` se retiró en el mismo commit** (su única consumidora desaparece): dos fuentes
con rejillas distintas —la objeción que justificaba un componente aparte— siguen siendo dos
fuentes distintas, pero ya no exigen un componente propio: `band` entra ya emparejado por mes y el
propio `MiniProjection` hace la intersección de ventana. La aritmética de alineación/deflactación
que SÍ sigue viva por si hiciera falta un abanico con mediana en otro contexto vive en
`buildRiskFan` (`lib/risk-bands.ts`), marcada `@deprecated` y sin consumidor de UI.

### `ChartLegend` — [`components/charts/ChartLegend.tsx`](../apps/web/src/components/charts/ChartLegend.tsx)

Leyenda compartida de charts (4.0.6): la consumen el chart grande de Proyección, el
MiniProjection del Resumen y el de Jubilación (la antigua `MiniProjectionLegend` se eliminó).
HTML normal —nunca dentro de un `<svg>`— con `flex-wrap` real.

- Props: `{ structural, assets?, collapsedCap?, size? ("sm"|"md"), className?, ariaLabel? }`.
  Los items son `ChartLegendItem` (`lib/chart-legend.ts`): `{ key, label, color, swatch, title? }`
  con `swatch ∈ line | dashed | area` y `color` SIEMPRE un token `var(--…)` (entra por la
  custom property `--ff-legend-color`; el swatch de área usa `color-mix` 14 %/40 %, el mismo
  tinte que las áreas del chart).
- **Colapso**: las entradas `structural` se ven siempre; los `assets` se truncan a
  `collapsedCap` con un chip «+N más» / «Ver menos» (`applyLegendCollapse` no esconde nunca
  uno solo). Caps por ancho en `collapsedAssetLegendCap` (≤640 → 3, ≤720 → 4, resto → 6;
  mini charts: `DEFAULT_LEGEND_ASSET_CAP` = 4). El estado expandido es efímero (se resetea al
  montar, sin localStorage). Expandida, la lista scrollea dentro de `max-height: 8.5rem`.
- **No interactiva** para series (decisión de producto): el único control es el chip.
- El chip es un control denso de chart: queda **fuera del bump `--ff-touch-min`** a propósito
  (mismo criterio que el carve-out de tablas), con `min-height: 2.25rem` propio en `≤640`.
- Toda la lógica de modelo (orden por peak desc conservando el color de pintado, sufijo de
  owner en duplicados, colapso, top-N del tooltip) vive PURA en
  [`lib/chart-legend.ts`](../apps/web/src/lib/chart-legend.ts) y está testeada en Vitest.

### Riesgo compacto y «Detalle del cálculo» (Jubilación) — sin chart propio (5.0.0, D28; reescrito por V1/V5/V6/V7)

Desde U1b la sección «Riesgo» de Jubilación ya no es un panel con su propio chart: es un bloque
**compacto** dentro del panel «Resultado», con todo lo de segundo orden plegado en «Detalle del
cálculo» (el abanico vive en el chart único, ver §`MiniProjection` arriba). La tercera vuelta de UX
lo dejó en TRES cosas y ni una más:

1. **El KPI «Éxito del plan»**, cuyo valor es «87,0 %» —un porcentaje con un decimal, como todo
   porcentaje de la casa— y cuyo subtítulo dice el SUJETO de la cifra, no un umbral: «de los
   escenarios no agotan el capital», o «0 de 500 escenarios agotan el capital» cuando es verde.
2. **La línea informativa del colchón de caja** (`.retirement-buffer-line`, V6): de dónde sale, su
   equivalente en meses y su coste en puntos de éxito, con el enlace a Reglas de ahorro o —si
   alguien lo fijó por API— la salida «Volver al tope de tu regla» (`PATCH null`, tri-estado).
3. **El aviso «sin volatilidad declarada»**, intacto.

**Lo que se fue, y por qué no vuelve:**

- **La tabla «Probabilidad de agotar el capital»** (`.risk-depletion-grid` y familia, retiradas de
  `App.css`). El owner: «los tiles de riesgo son inútiles: no muestran nada que la gráfica no
  muestre. Si la gráfica estuviera bien hecha no harían falta» (F7). Se cumplió la condición: el
  degradado de la banda dice lo mismo edad a edad y con más resolución, y el hover da el número
  exacto. Lo único que el color NO puede rotular —el total acumulado al final del horizonte, porque
  su última parada cae en el borde del plot— bajó a «Detalle del cálculo» como la fila
  `depletion_total`. **Si alguien propone volver a tabular el agotamiento, la pregunta es qué dice
  esa tabla que el color y el hover no digan ya.**
- **Los dos campos de la tarjeta «Riesgo»** del formulario: el colchón se DERIVA (V6) y el umbral
  de éxito dejó de existir (V7). Por eso no hay tarjeta «Riesgo» en `PLAN_CARD_ORDER`.

**El valor del KPI ya NO es una oración.** Entre U1b y esta vuelta, «Éxito del plan» fue el único
KPI de la app cuyo valor era una frase entera («87 de cada 100 escenarios se jubilan y no agotan el
capital»), envolviendo a dos o tres renglones dentro de `.metric-value-row`. El owner lo leyó como
«demasiado texto para caber en una caja» (F2) y tenía razón: `.metric-value` es mono, 1,25 rem y
`tabular-nums` — tipografía para «87,0 %», no para once palabras. La condición **no se perdió**:
bajó al subtítulo, que es el slot que sí envuelve (`.summary-success-grid
.metric-value-parenthetical` y `.plan-card-wide-kpi .metric-value-parenthetical`, `white-space:
normal`, mismo precedente que `.retirement-tiles-grid`). La lección general: **si una cifra necesita
once palabras para no mentir, las palabras van en el subtítulo; el valor sigue siendo un valor.**

Otras reglas del bloque que siguen vigentes:

- **La ayuda de las filas cuelga del RÓTULO de cada fila** (`.risk-extra-head`), no del panel:
  `RiskExtraRow` lleva un `helpId?` opcional y la vista envuelve el rótulo en
  `label-with-help risk-extra-label`. Una sola ayuda en el título explicaría la fila que el usuario
  no está mirando. Sin `helpId` la fila conserva su `<span class="risk-extra-label">` pelado.
- **La escala de color va bajo el chart, no en la leyenda** (`.retirement-risk-scale` / `-step` /
  `-swatch`): `ChartLegend` nombra SERIES, y esto es una escala continua — meterla ahí la haría
  parecer una cuarta línea del gráfico. El color de cada muestra entra por la custom property
  `--ff-risk-swatch`, el mismo patrón que `--ff-legend-color`. Cuando hay degradado, la entrada
  «Banda 10–90 %» **sale** de `ChartLegend`: su swatch tendría que enseñar UN color y la banda ya
  no tiene uno.

Clases que sobreviven (todas en `App.css`, cero color propio salvo `--ff-warn`):
`.metric-card--warn` (tono ámbar del KPI, misma construcción que `--danger`; consumidor dinámico vía
`successVerdictTone`, no un literal `tone="warn"` en el JSX — no lo busques así),
`.risk-extra-rows`/`.risk-extra-row`/`-head`/`-label`/`-value`/`-detail` y `.risk-footnote`
(procedencia: ms, caminos y semilla; `overflow-wrap: anywhere` porque la semilla es un entero de 20
dígitos que no cabe a 360px). **`.risk-fan-note` se retiró** con `RiskFanChart.tsx` — la nota
«Bandas puntuales» vive ahora como fila de `retirementDetailRows`, con el `HelpPopover` de
`retirement.bands`.

> **Variante DESCARTADA del color de riesgo (V5, documentada para no volver a proponerla)**: una
> tira de 6 px bajo el eje X, coloreada por probabilidad de agotar el capital, en vez de teñir la
> banda. Se descartó por dos razones: competiría con la tira de fases (D29), que ya vive exactamente
> ahí y usa el tinte progresivo del acento, y dejaría la banda azul **sin significado** — el
> problema que F6 denunciaba. Es el plan B si el contraste del degradado al 28 % no aguantara en
> oscuro; en ese caso, la tira de fases tendría que moverse o desaparecer, no compartir carril.

### `PlanningDirectionChart` — [`components/charts/PlanningDirectionChart.tsx`](../apps/web/src/components/charts/PlanningDirectionChart.tsx)

Barra apilada inflow/outflow de la pestaña Próximos (`<svg viewBox="0 0 100 12">`). SVG inline
**amparado por la excepción de charts** (renderizado de datos, no icono — no puede vivir en
`icons.tsx`); colores por clase CSS a `var(--proj-pos)`/`var(--proj-neg)`, cero hex.

### Formateo de importes en charts — las DOS excepciones sancionadas a los 4 helpers canónicos

La regla de §Formato de cifras y copy («usa `formatCurrencyAmount`/`formatCurrencyNumber`, nunca
concatenación manual») tiene exactamente dos excepciones, ambas en [`lib/ledger.ts`](../apps/web/src/lib/ledger.ts)
y ambas de **chart**, donde el espacio manda:

- **`formatAxisMoney`** (etiquetas del eje Y del chart grande): construye su propio
  `Intl.NumberFormat` de moneda porque necesita `notation: "compact"` («1,2 M€»), que los helpers
  canónicos no exponen.
- **`formatProjectionMilestoneCompactLabel`** (etiquetas de hito del chart): sufijos `K/M/B/T`
  manuales; por debajo de 1.000 delega en `formatMoneyAmount`.

No añadas una tercera vía: si un caso nuevo necesita compactación, reutiliza estas dos o extiende
los helpers canónicos. Fuera de charts, la regla no tiene excepciones.
## Componentes nuevos (UI)

| Componente | Propósito |
|---|---|
| `TopBar` | Cabecera única (marca + pills + extras + hamburguesa) |
| `MobileNavDrawer` | Drawer derecho con todas las secciones (≤720px) |
| `AccountCard` | Cuenta destacada en Ajustes (avatar + rol + acciones) |
| `ThemeToggle` | Segmented Auto/Claro/Oscuro |
| `MiniProjection` | Chart compacto reutilizable (ver arriba) |

> **Switch (`components/Switch.tsx`, clases `.ff-switch*`)**: el toggle track+thumb accesible (`role="switch"`, focus-visible con outline `--ff-accent`, disabled al 55 %). Nació en la barra de Proyección y se extrajo a componente al añadir el toggle «Permitir escritura vía MCP» de Ajustes → Integraciones; `variant="chart"` conserva el label small-caps compacto de las barras de chart. Para booleanos de formulario clásicos sigue existiendo `label.field.checkbox-field`.

> **Segmented control (`.ff-theme-toggle`)**: dos consumidores desde 5.0.0 — el toggle de tema Auto/Claro/Oscuro (`ThemeToggle`, clase base `.ff-theme-toggle`) y el segmentado «Yo | Hogar» de la TopBar (`.ff-theme-toggle.ff-topbar-scope`, ver §Shell), que reusa la misma piel con una modificadora que solo compacta padding/tipografía — cero declaraciones nuevas de color. **Ya no es cierto que sea el único que queda**; si actualizas este párrafo por un tercer consumidor, dilo aquí y no lo dejes desactualizado otra vez. La antigua clase compartida **`.ff-segmented` se eliminó** tras 2.0.0: la «fuente del ahorro» de la entonces `Ajustes → Proyección` (hoy `Ajustes → Plan`) pasó a un `<select>` nativo estándar (con `<small>` de ayuda asociada por `aria-describedby`, fuera del `<label>`). Si necesitas un nuevo control inline de 2–3 opciones, valora primero un `<select>`; si de verdad hace falta un segmented con botones, extiende `.ff-theme-toggle` con una modificadora (precedente: `.ff-topbar-scope`), no reintroduzcas `.ff-segmented`. Verifica claro **y** oscuro.

> **Radio-cards de configuración (`.retirement-mode-card`/`.retirement-mode-grid`)**: NO son un segmented — son `<label>` con un `<input type="radio" className="sr-only">` dentro, estilados como tarjeta (borde + `is-active` con tinte de acento). Nacieron para el modo del objetivo anual de Jubilación y 5.0.0 (D26, issue #207) los reusa tal cual para las **5 tarjetas de estrategia** (`RetirementView.tsx`, modificadora `.retirement-strategy-grid`: mismo `grid-template-columns` pero `repeat(auto-fit, minmax(min(100%, 15rem), 1fr))` porque cinco no caben en la rejilla fija de 3 del modo del objetivo — no-op en escritorio, colapsa solo en móvil, sin breakpoint nuevo).
>
> **Las dos rejillas de radio-cards viven en tarjetas ANCHAS.** Desde V3 las dos (estrategia y modo
> del objetivo) caen dentro de una `.retirement-card`, y la columna de la rejilla de tarjetas mide
> 21 rem: una radio-card de ~7 rem no cabe con su nombre en una línea. Por eso «Estrategia» y
> «Gasto en jubilación» llevan `.retirement-card--wide` (`grid-column: 1 / -1`). Si añades una
> tercera rejilla de radio-cards, su tarjeta va ancha también.

### Tarjetas de configuración (`.retirement-card*`) — 5.0.0, V3 de la tercera vuelta de UX

Sustituyen al acordeón «Avanzado» de U12, que el owner leyó como «un cajón de sastre mal explicado»
(F10) y al que pidió «no tener miedo a hacer cuadros para cada cosa (pensión, gasto tras
jubilación…) en vez de macrobloques confusos» (F9). El formulario de Jubilación es ahora **una
tarjeta por TEMA, todo a la vista**: Estrategia · Edades · Pensión · Gasto en jubilación · Retirada ·
Horizonte.

- `.retirement-card-grid`: `repeat(auto-fit, minmax(min(100%, 21rem), 1fr))`, `gap: 1rem`. **Dos
  columnas en escritorio y no una**: seis tarjetas apiladas dejarían el panel «Resultado» a dos
  pantallas de scroll, y U1 dice que el resultado va DEBAJO del plan — debajo, no lejos. Sin
  breakpoint nuevo.
- `.retirement-card`: borde `--ff-line-soft`, radio `--ff-radius-kpi`, fondo
  `color-mix(in oklch, var(--ff-paper) 60%, var(--ff-bg))` — un escalón por debajo del panel que las
  contiene, para que se lean como subdivisiones y no como paneles hermanos.
- `.retirement-card--wide`: `grid-column: 1 / -1` (las dos rejillas de radio-cards, ver arriba).
- `.retirement-card-blurb`: la frase de qué hace la tarjeta, 0,78 rem `--ff-ink-soft`. **No es
  decoración**: el copy de cada una (`PLAN_CARD_COPY`, `lib/retirement-form.ts`) dice qué cambia y
  qué implica cambiarlo, nunca «aquí van las edades» — un título más no habría arreglado nada.

**Qué desapareció con el acordeón** y por qué no vuelve: la línea «Supuestos: retirada 3,5 % · … ·
umbral 95,0 %» (`.retirement-advanced-summary-text`, `lib/assumptions-line.ts`) existía como
contrapeso U12 —enunciar lo que el acordeón escondía— y **sin acordeón no hay nada escondido que
enunciar**: todo supuesto en vigor está en su tarjeta, a la vista. Era además prosa compuesta desde
una tabla viva: sin consumidor solo podía envejecer. Registrado en `futurefin-failure-archaeology`.
>
> **Tarjeta ancha «Tu plan» (`.plan-card-wide*`)** (5.0.0, D27/D32 → U9, issue #207, `SummaryView.tsx`):
> sustituye a la rejilla de tarjetas D27 original (`.plan-card-grid`/`.plan-card`/`.plan-card-figures`,
> **retirada de `App.css` en el pase de documentación U5b** tras confirmar cero consumidores en
> `apps/web/src` — `grep -n "^\.plan-card {\|^\.plan-card-grid {\|^\.plan-card-figures {" apps/web/src/App.css`
> vacío). `.plan-card-wide` es un `flex` en fila:
> `.plan-card-wide-main` (título = la frase-hito coloreada por tono con `.plan-card-wide-title--danger`,
> subtítulo = estrategia + hito secundario) y `.plan-card-wide-kpi` con el KPI «Éxito del plan» a la
> derecha (`.metric-card` sin CSS propio). `.plan-card-wide-warning` debajo, con su propia variante
> `--danger`. En ≤640px el KPI cae bajo el título (`flex-direction: column`). Mismo vocabulario de
> «esto va mal» que `.error-banner` (`--ff-neg` al 8 % sobre `--ff-paper`, borde al 45 %).
>
> **Frases del hogar (`.plan-sentence-list`/`.plan-sentence-item`/`.plan-sentence-dot`)** (5.0.0,
> D32 → U10, issue #207, `SummaryView.tsx`): en Hogar, «Planes del hogar» no es una rejilla de
> tarjetas — es una `<ul>` de frases, una por miembro, con `.plan-sentence-dot` pintado con
> `householdMemberColor(idx)` (el MISMO color que la línea fina de ese miembro en el chart y su tick
> de la tira de fases). Sin cifras por persona: el hogar no tiene plan propio.
>
> **Tono rojo de KPI (`.metric-card--danger`, prop `tone="danger"` de `MetricCard`)** (5.0.0, D17,
> issue #207): mismo tinte que `.error-banner` y que `.plan-card--danger` (`--ff-neg` al 8 % sobre
> `--ff-paper`, borde al 45 %) — **un solo vocabulario de «esto va mal»**. Solo tiñe la piel y el
> segundo slot (`.metric-value-detail`): la cifra sigue en tinta normal porque el número no está
> mal; lo que está mal es lo que significa. Único consumidor hoy: «Ahorro necesario» de Jubilación
> cuando `underfunded === true`.
>
> **Tono ámbar de KPI (`.metric-card--warn`, prop `tone="warn"` de `MetricCard`)** (5.0.0, D28,
> issue #207): el peldaño INTERMEDIO del semáforo «Éxito del plan». Misma construcción que
> `--danger` —tinte al 8 %, borde al 45 %— pero con `--ff-warn`, para que verde/ámbar/rojo se lean
> como una escala y no como tres decoraciones sin relación. Consumidores: «Éxito del plan» en
> Jubilación (§Riesgo) y en el Resumen. El verde NO tiene piel propia: «va bien» es el estado
> normal y teñir también el caso bueno convierte el color en ruido.
>
> **KPI de éxito del Resumen (`.metric-grid.summary-success-grid`)** (5.0.0, D28): una sola tarjeta
> dentro del panel «Tu plan». `auto-fill` y **no** `auto-fit` a propósito: con `auto-fit` la única
> tarjeta absorbería las pistas vacías y se estiraría a toda la fila.
>
> **`.plan-card-figures` (5.0.0 WP7-3b2) queda retirado de `SummaryView.tsx` desde U9**: la fila
> «Ahorro necesario X/mes · Margen Y/mes» no sobrevivió al rediseño — `planCardV2` no tiene esas dos
> cifras sueltas, la ORACIÓN ya las integra. La clase **se retiró de `App.css`** en el pase de
> documentación U5b (mismo caso que `.plan-card-grid`/`.plan-card` arriba).
>
> **Frase-hito con filete lateral de tono (`.retirement-sentence`)** (5.0.0, U1b → U7, issue #207,
> `RetirementView.tsx`): la cabecera de «Resultado» es UNA oración (`lib/plan-sentence.ts`), y su
> tono se marca con un `border-left: 3px solid` — `.retirement-sentence--ok` = `--ff-accent`,
> `--warn` = `--ff-warn`, `--danger` = `--ff-neg` — **nunca con el color del TEXTO**, que se queda
> en `--ff-ink` los tres casos. Es la misma regla que gobierna toda la escala de estado (D17/D28):
> verde/rojo son para cifras delta, y la piel/filete es donde vive el semáforo. Un filete lateral
> en vez de teñir el texto entero deja la frase larga legible sin que el tono compita con la
> lectura — la misma razón por la que `.plan-card-wide-title--danger` sí tiñe texto (es un título
> corto, no una oración) pero nunca un párrafo largo.
>
> **Tiles v2 con subtítulo que ENVUELVE (`.retirement-tiles-grid .metric-value-parenthetical`)**
> (5.0.0, U1b → U7, issue #207, `RetirementView.tsx`): sustituye a la «segunda rejilla»
> `.metric-grid.retirement-solve-grid` + avisos `.retirement-strategy-notices` de WP7-3b2 —
> **las dos clases se retiraron, ni una tiene ya consumidor** (`grep -c "retirement-solve-grid\|retirement-strategy-notices" apps/web/src/App.css apps/web/src/views/RetirementView.tsx`
> → 0 en los dos ficheros). U7 topa la cabecera a **3 tarjetas** (`RETIREMENT_TILES_V2_CAP`,
> `lib/retirement-tiles.ts`), así que ya no hace falta una segunda rejilla aparte de la banda
> superior: son las MISMAS `.metric-grid.retirement-tiles-grid` (`auto-fit`,
> `minmax(min(100%, 15rem), 1fr)`). La variante existe solo para que el subtítulo pueda ENVOLVER
> (`white-space: normal`, sin recorte): ahí vive la BASE de la cifra («de 658 €/mes de sobrante · es
> TODO tu sobrante y no basta»), y U7 prohíbe truncarla — media base es peor que ninguna. Los
> avisos que antes vivían en `.retirement-strategy-notices` se repartieron por tono: el rojo
> (D17, `underfunded`) sube a un `.error-banner` sobre las tarjetas; el resto baja a
> «Detalle del cálculo» como filas de `retirementDetailRows`.
>
> **Acordeón «Avanzado» y su línea «Supuestos» (`.retirement-advanced*`) — RETIRADOS** (nacieron en
> 5.0.0 U1b → U12, mueren en la tercera vuelta de UX, F10). Eran un `<details class="panel
> retirement-advanced">` cuyo `<summary>` pintaba `assumptionsLine(profile, ctx)` siempre, plegado o
> abierto, como contrapeso de U2: si escondes un campo por estrategia, tienes que enunciar su valor
> o lo estás forzando en silencio. **La solución era correcta para el problema equivocado**: el
> owner no quería que le enunciaran lo escondido, quería que no se escondiera nada («idealmente
> Avanzado desaparece si la información se organiza y jerarquiza bien»). Con las tarjetas por tema
> (§Tarjetas de configuración) los 13 campos están a la vista, así que no hay nada que enunciar y la
> línea se quedó sin sujeto. Se fueron el `<details>`, `.retirement-advanced*`,
> `.retirement-advanced-link` y el módulo `lib/assumptions-line.ts` con su test. **Antes de proponer
> otro acordeón de configuración, lee `futurefin-failure-archaeology` §3.**
>
> **Indicador de guardado único (`.retirement-save-state`)** (5.0.0, U1b → S6, issue #207,
> `RetirementView.tsx`): sustituye a los seis pies «Guardado automático.» de WP7, uno por panel, que
> podían contradecirse entre sí. Vive en la cabecera (`role="status" aria-live="polite"`), texto
> `muted` salvo el estado de error (`.retirement-save-state--danger`, `--ff-neg`) — precedencia fija
> en `saveIndicatorLabel` (`lib/retirement-form.ts`): error > guardando > bloqueado > guardado. El
> caso «guardado» se queda en tinta muted a propósito (misma regla que el verde sin piel: el estado
> normal no se tiñe).
>
> **`.retirement-radio-stack`** (radios nativos en línea, `<label class="field checkbox-field">` por opción, `role="radiogroup"` en el contenedor): existía en `App.css` **sin ningún consumidor** antes de 5.0.0. `9ae5c24` le da los dos primeros — la base del objetivo (`perpetuity`/`bridge_to_pension`) y el `kind` de la regla de retirada, ambos en `RetirementView.tsx` (`grep -c "retirement-radio-stack" apps/web/src/views/RetirementView.tsx` → 2). No lo confundas con el segmented: aquí el foco/tabulación son los `<input>` nativos, no un `role="group"` de botones.

## Iconografía

`apps/web/src/components/icons.tsx` — set unificado:

- viewBox `16×16`, `stroke="currentColor"`, `strokeWidth=1.5`, `linecap/linejoin="round"`.
- El color lo da el padre; el tamaño se controla por CSS (sin width/height fijos en los SVG).
- **No introduzcas SVG nuevo fuera de `icons.tsx`.** La única excepción sancionada son los charts
  (render de datos, no iconos — precedente: `PlanningDirectionChart`, §Componentes nuevos (charts)).
- Iconos disponibles: `PlusIcon`, `RowEditIcon`, `RowTrashIcon`, `GearIcon`, `XIcon`, `CheckIcon`, `MoreIcon`, `ChevronIcon`, `ChevronLeftIcon`, `ChevronDownIcon`, `MenuIcon`, `UserIcon`, `DragIcon`, `DownloadIcon`, `CalendarIcon`, `FilterIcon`, `SortIcon`, `LinkIcon`, `RefreshIcon`, `EyeIcon`, `SearchIcon`, `ArrowUpIcon`, `ArrowDownIcon`, `DuplicateIcon`.

## Reglas para el chart de Proyección grande

- **Composición del rediseño V1 intacta salvo la leyenda**: hover, zoom, tooltips, dimensiones, planning markers, jubilación pill — idénticos a antes del rediseño (solo se sustituyeron hex hardcoded por `var(--proj-*)`). La leyenda se rediseñó en 4.0.6 (ver siguiente bullet).
- **Leyenda FUERA del SVG** (4.0.6): vive en HTML (`ChartLegend`, ver §Componentes) debajo del plot, dentro de `.projection-chart-root` (columna flex: `.projection-chart-plot` con `flex:1` + leyenda). Motivos: (1) `flex-wrap` real del navegador en lugar de anchos estimados a mano (el viejo `legendCharPx=7.6` de `buildProjectionChartLayout` se parcheó una vez por solaparse y aun así dimensionaba con un orden de activos distinto al renderizado); (2) el plot deja de ceder altura a la leyenda — `mt` es constante y con N activos la leyenda colapsada cuesta ~2,2rem fijos (antes, 8 filas de leyenda dejaban el plot en ~12px); (3) el chip «+N más» es un `<button>` nativo fuera de la máquina de gestos pan/zoom del `<svg>`. El `ResizeObserver` del chart mide `.projection-chart-plot`, NO la raíz (si midiera la raíz, el viewBox incluiría la leyenda y el hover se desalinearía); `wrapRef` (la raíz) sigue siendo la base del tooltip absoluto.
- **Etiquetas de activo con owner** (solo vista hogar): los nombres duplicados se desambiguan «Nombre · owner» (`buildAssetLegendItems`); las series solo-históricas (de snapshots, sin fila en `/v1/assets`) ni se sufijan ni vetan el sufijo de las actuales. El tooltip lista el top-5 por |valor| del mes + «Otros (k) — suma» (`topAssetTooltipRows`).
- **Ticks X diezmados por ancho** (4.0.6): los builders de año/edad emiten un tick por año (55 en un horizonte de 90); `thinTicksFromEnd` recorta los ticks **visibles** a `projectionMaxXTicks(pw, mode)` (≥52px/etiqueta en plots <560, ≥34 en anchos) desde el final — el fin de la ventana siempre etiquetado, huecos uniformes. NUNCA diezmar sobre el horizonte completo: la ventana filtraría los supervivientes y un zoom se quedaría sin etiquetas.
- **«Hoy» vive en la fila del eje X** (4.0.6): la etiqueta del divisor de hoy se alinea con las etiquetas de año (misma baseline y rotación), no flota sobre el plot pegada al subtítulo; cuando está visible, se apartan las etiquetas de año a <40px del divisor. El divisor vertical no cambia.
- **Comportamiento móvil (≤640, `useIsMobile`)** (4.0.6): estilo MiniProjection pero navegable — (1) «Vista cercana» activada POR DEFECTO vía override efímero en `ProjectionView` (estado, no storage: el toggle funciona pero nunca pisa `PROJECTION_FOCUS_STORAGE_KEY` → escritorio conserva su memoria); (2) sin etiquetas del eje Y ni caption EUR (`hideYAxisLabels` → `ml=16`, el plot gana todo el margen; el valor exacto vive en el tooltip); (3) la 2.ª línea de meta se parte en dos (`compactHeader` reserva la altura). La vista cercana (todas las plataformas) deja **margen tras el último hito** (~12 %) para que la etiqueta del marcador «Jubilación» no quede pisada por el borde, y el suelo de los milestones es `mt+22` para que un hito pegado al techo del plot no pierda la mitad superior de su etiqueta por el clip.
- **Tooltip independiente del tema**: forzado a `color: #fafafa !important` + bg `rgba(10,10,10,0.92)`. Antes usaba `var(--ff-frame)` que en oscuro daba texto oscuro sobre fondo oscuro.
- **Tira de fases bajo el eje X** (5.0.0, D29, issue #207): banda de 12px (+12 para la fila de
  rótulos de las marcas) entre el suelo del plot y la fila de etiquetas del eje X, con un tramo por
  fase («Trabajo» / «Media jornada» / «Jubilado»; en móvil «Trab.» / «½ jorn.» / «Jub.»), la
  pensión como **flecha** y el cruce del objetivo como **tick discontinuo rotulado «Cruce»** —
  este último SOLO cuando `retirement_trigger === "target_age"` y el cruce cae en un mes distinto
  del de la jubilación efectiva; cuando coinciden se rotula una sola vez, porque es un solo hecho.
  Los **marcadores verticales** siguen siendo exclusivos de la jubilación efectiva (carriles y
  `isJubilacion` intactos): la tira es **aditiva**. Su alto sale del **plot** (`ph`), jamás de
  lienzo extra — la misma regla que los 38px de las etiquetas rotadas, y por el mismo motivo (un
  viewBox más alto que la caja medida haría que `meet` encogiera el dibujo con bandas laterales).
  Sin fases ni marcas la tira mide 0 y la geometría es byte a byte la de 4.15.x. Color: **tinte
  progresivo del acento** (`--ff-accent` al 7 / 18 / 32 % contra `--ff-paper`), la misma escala
  ordinal que las variantes de `.metric-card`, no color decorativo. Modelo puro en
  [`lib/phase-strip.ts`](../apps/web/src/lib/phase-strip.ts) — **todo por `month_index`**, nunca
  por posición del array. `MiniProjection` tiene una versión reducida opt-in (`showPhases`): banda
  de 6px sin rótulos.
- **Líneas finas por miembro** (5.0.0, D32, issue #207, solo vista Hogar): la curva de la Σ sigue
  siendo la gruesa (`--proj-nw`, 2,85px) y **debajo** va una polyline de 1,3px al 80 % de opacidad
  por cada miembro, con su patrimonio neto. El color sale de `householdMemberColor(idx)`
  ([`lib/chart-legend.ts`](../apps/web/src/lib/chart-legend.ts)) — la paleta `--proj-area-1..10`
  otra vez, y **una sola definición** para las tres superficies de la misma persona: su línea, su
  tick en la tira de fases y su entrada de leyenda (que por eso pasa de `dashed` a `line`).
  Emparejarlas por su cuenta era pedir que la leyenda acabara nombrando la curva de otro. Solo se
  dibuja el patrimonio: el líquido viaja en `members[].series` pero **no se pinta** (dos líneas por
  persona convierten el chart en una maraña, y el líquido solo significa algo contra el objetivo de
  esa persona, que el agregado no publica). Los valores entran en el dominio Y —un miembro puede
  caer por debajo de la Σ— porque una línea recortada por el clip se lee como una línea que se
  ACABA, y aquí eso significa otra cosa: **una línea que termina antes del borde derecho es un
  horizonte propio más corto** (`members[].horizon_months`), nunca un dato que falte. Modelo puro y
  por `month_index` en [`lib/member-lines.ts`](../apps/web/src/lib/member-lines.ts); `MiniProjection`
  sigue siendo **solo Σ**. El tooltip añade una fila por miembro con el valor del mes, y omite a
  quien ya no tiene línea ahí.
- **Milestones con collision-avoidance**: si dos milestones quedan cerca horizontalmente, el segundo sube al siguiente "carril" (14px arriba) y la línea punteada se estira hasta la nueva `y2`. Ver [`ProjectionNetWorthChart.tsx`](../apps/web/src/views/ProjectionNetWorthChart.tsx) en el bloque `(() => { … lanes … })()`.

## Paneles de Ajustes — norma de texto (4.0.6)

Armonización pedida por el owner («no todos encajan, hay párrafos diferentes, iconografía que
sobra»). TODO panel de Ajustes sigue esta plantilla; si añades uno nuevo, cópiala:

1. **Título**: `h3.panel-title`, **sin icono** (los tres `panel-title-icon` de Integraciones se
   retiraron junto con la clase; el patrón dominante siempre fue sin icono). `.panel-head-row`
   SOLO cuando hay una acción a la derecha (botón); sin acción, `h3` a secas.
2. **Descripción** (opcional): UNA `<p className="muted">` de 1–2 frases directamente bajo el
   título, sin separador. El detalle largo vive en el `HelpPopover` del campo, no en prosa
   suelta (precedente: los tres modos de «Fuente del ahorro» — en pantalla solo la descripción
   del modo ELEGIDO; la comparativa completa está en `settings.savings_source`).
3. **Controles** después, separados con `bordered-top`. La ayuda de un campo concreto va en
   `<small className="muted">` dentro de su `label.field`.
4. **Estados**: `Cargando…` / `Sin datos.` / `Solo lectura.` (cortos, canónicos). Valor
   read-only para no-owners: `<strong>{valor}</strong> · solo el propietario puede cambiarlo.`
   Estado de guardado SIEMPRE al pie del panel: `Guardando…` / `Guardado automático.`
   **En Ajustes no hay botones de guardar**: todo ajuste autosalva con debounce y una guarda
   de validez client-side (el PATCH no se lanza con un valor a medio teclear — precedentes:
   IANA válida para la zona horaria, 0–50 para la inflación; el pie lo dice: «… — sin
   guardar»). Los modales de CRUD (categorías, snapshots, backups) sí llevan botón: son
   acciones, no ajustes.
5. **`<strong>` solo para valores de dato**, nunca énfasis retórico. La clase `compact` no
   existe (era un no-op y se purgó); `.hint` y `.health-dl` se retiraron (usos únicos —
   `muted tight` y `settings-meta-dl` los sustituyen). Listas en modales: `.muted-list`, sin
   estilos inline.

### Una sola escala de títulos (5.0.0, P6 — F12 del owner)

El owner volvió a ver «títulos y tamaños de fuente distintos entre paneles». La auditoría encontró
que Ajustes estaba homogéneo y que la desigualdad vivía **una pantalla más allá**, en Jubilación,
que mezclaba cuatro cabeceras. La norma resultante, que vale para toda la app:

| Qué | Clase |
|---|---|
| Título de PANEL | `h3.panel-title` |
| Título de sub-sección o de TARJETA | `h4.panel-title` — misma clase; la jerarquía la da el contenedor, no el tamaño |
| Texto de ayuda de un campo | `p.muted.tight` (nunca `<small>`: `.muted` solo fija color, y `<small>` cae a ~11 px) |
| Disparador de un `<details>` de segundo orden | `.details-trigger` (0,82 rem / 600 / `--ff-ink-soft`) |

**Ningún `<summary>` ni `<h4>` sin clase decide su propio tamaño**: si un elemento va más pequeño a
propósito —como «Detalle del cálculo», que es la puerta a lo que se puede no leer— ese tamaño está
declarado en una clase con nombre y documentado aquí, no heredado del user-agent.

> **`.subsection-title` se ha retirado (WP-G, mismo pase que esta nota).** Los seis consumidores que
> quedaban tras `RetirementView` (ya migrado a `h4.panel-title` en WP-D) migraron también: los dos
> `<h4>` de `SummaryBreakdownBlock` y los dos de `views/GastosView.tsx` («Gastos»/«Ingresos») a
> `h4.panel-title` sin cambio visual (la base de `.subsection-title` era idéntica a `.panel-title`
> salvo el margen inferior, que `.panel-title` sí lleva); los dos `<h4>` de `SummaryDonutChart` —el
> único caso que de verdad necesitaba la talla menor— a una clase propia con nombre,
> `.donut-card-title` (0,82 rem / 600 / `--ff-ink-soft`, la misma declaración que tenía
> `.summary-donut-card .subsection-title`, ahora sin depender de dónde cae un título genérico).
> Verificación: `grep -c "subsection-title" apps/web/src/App.css apps/web/src/components/charts/summary.tsx apps/web/src/views/GastosView.tsx` → **0** en los tres.

## Vista de solo lectura (Hogar) — norma de renderizado (5.0.0, D9/D32, issue #207)

La vista Hogar es un agregado informativo y de **solo lectura** (para la tabla completa de qué
vista consume qué boolean, ver [`frontend-structure.md`](frontend-structure.md) §Ámbito del hogar
y candado de solo lectura). La norma de renderizado es **ocultar el control, no deshabilitarlo**:
un botón «Añadir» o «Editar» desaparece del DOM en vez de quedarse gris y sin acción — un control
deshabilitado sigue anunciando una acción que no va a pasar nada, y en móvil sigue ocupando espacio
táctil para nada.

**Única excepción documentada**: los dos `<select>` inline de la tabla de Movimientos
(`GastosView.tsx` — categoría y tipo de cada fila, clase `.exp-inline-select`) se quedan
`disabled={!canEdit}` en vez de ocultarse (`grep -n "disabled={!canEdit" apps/web/src/views/GastosView.tsx`
→ 2 hits). Ocultarlos colapsaría dos columnas enteras de la tabla en Hogar, desalineando el resto
de columnas frente a la vista «Yo» — el mismo principio de «slot siempre reservado» que gobierna
los adornos numéricos (§Layout). Si añades una segunda excepción, anótala aquí; si no, la norma es
ocultar.

## Reglas para añadir UI nueva

1. **Usa los tokens**. Nunca hardcoded hex. Si necesitas un color que no está, primero pregúntate si puedes vivir con `color-mix(in oklch, var(--ff-accent) X%, var(--ff-paper))`. Si no, añade un token nuevo en `theme.css` con variantes claro/oscuro. El enforcement automático es el freezer [`styles/no-hex-outside-theme.test.ts`](../apps/web/src/styles/no-hex-outside-theme.test.ts): sus contadas excepciones sancionadas (p. ej. la sombra del tooltip de Proyección, issue #105) se registran en `RGBA_ZERO_EXCEPTIONS` por **`file:línea` exacta, no por patrón** — cualquier edición de `App.css` que inserte líneas por encima desplaza el anclaje y rompe el test aunque el CSS no haya cambiado de verdad. 5.0.0 lo movió varias veces: a 2425/2426 cuando el segmentado «Yo | Hogar» y los banners de ámbito/alta se insertaron más arriba; a 2404/2405 en la tercera vuelta de UX, al retirar F5 el banner de alta de Jubilación y sus 21 líneas de CSS; y **a 2396/2397** en WP-C/WP-G (F3/F12), que borraron por encima `.subsection-title` y su override `.summary-donut-card .subsection-title` (retiradas, ver §Una sola escala de títulos) y reescribieron `.assets-table--budget-lines` a `table-layout: fixed` — un neto de 8 líneas menos. Las anclas se mueven en los dos sentidos: un borrado por encima las desplaza igual que una inserción. Si el freezer falla así: `grep -n "rgba(0, 0, 0," apps/web/src/App.css` para encontrar las líneas reales de hoy y actualiza los literales de `RGBA_ZERO_EXCEPTIONS` a esos números — no borres la excepción, muévela.
2. **Verifica claro y oscuro antes de mergear**. Toggle desde Ajustes y revisa: KPIs, modales, tooltips, hover states, focus rings.
3. **No mezcles tab-bar legacy con TopBar**. La nav es responsabilidad exclusiva de `TopBar`. Sub-tabs (como las de Ajustes) van como pills con clase `ff-nav-pill`.
4. **No introduzcas color decorativo**. Pos/neg = cifras delta. Acento = destacar UN ítem (botón primario, KPI hero, marker de jubilación, slice principal de un donut). El resto vive en grayscale.
5. **Un valor que el servidor DERIVA se rotula como derivado.** Patrón fijado en 5.0.0 por «Base del
   objetivo» de Jubilación: el radio marca la opción que se está aplicando y, mientras nadie la haya
   elegido, el rótulo del grupo lleva un ` (derivada)` atenuado (`.muted` inline) y aparece la salida
   «Volver a la derivada» (`btn ghost text` + `.retirement-basis-reset`, **fuera** del `radiogroup`:
   un botón enfocable entre radios rompe la navegación con flechas) en cuanto sí está fijada, para
   poder soltarla otra vez con el `null` del tri-estado. Sin el rótulo, una opción
   marcada se lee como una decisión tomada — y el formulario la reenvía, congelando una derivación
   que debía seguir moviéndose. Si añades otro campo derivado por el servidor, cópialo.
6. **Una vista no repite la divisa en el subtítulo.** «Moneda EUR» / «Mensual · EUR» /
   «Importes · EUR» se retiraron de Activos, Pasivos, Presupuesto, Movimientos, Próximos y Resumen
   en el barrido de copys de 5.0.0 (issue #207): la divisa vive en `Ajustes → General` y en cada
   cifra formateada (`formatCurrencyAmount` ya lleva el símbolo detrás) — un subtítulo que solo
   repite eso no informa, ocupa una línea. Si una pantalla nueva necesita decir la divisa, es que
   le falta un formateador, no un subtítulo.
7. **El chip de scope no existe: el segmentado manda.** El chip «Mío · sin titular en Hogar» (y
   variantes) se retiró de las mismas seis pantallas por el mismo barrido: el segmentado «Yo |
   Hogar» de la TopBar (`.ff-topbar-scope`, ver §Shell) ya dice en qué vista está el usuario, y no
   hace falta que cada pantalla lo repita con su propio chip. Una UI nueva que necesite anunciar el
   scope se apoya en el segmentado — no reintroduzcas un chip de ámbito por pantalla.

## Provenance and maintenance

Re-verificado 2026-09-03 contra los commits `b413471` (WP7 1/3 — vista «Yo» por defecto,
segmentado «Yo | Hogar», hogar de solo lectura, aviso de alta de Jubilación) y `9ae5c24` (WP7 2/3
— tarjetas de estrategia, formulario contextual del perfil, volatilidad del activo), más las
rebanadas WP7 3a (tira de fases del chart de Proyección y tarjeta «Plan» del Resumen), **3b1**
(líneas finas por miembro, «(derivada)» de la base del objetivo, vaciado de los porcentajes del
activo) y **3b2** (tarjetas por estrategia y su rojo, series auxiliares discontinuas del chart,
cifras del plan en el Resumen), de la rama `release/5.0.0`, issue #207. Re-verificar con:

- Segmentado de la TopBar: `grep -n "ff-topbar-scope" apps/web/src/App.css apps/web/src/App.tsx`
- Banner de ámbito, primer hijo de `<main>`: `grep -n "app-scope-banner" apps/web/src/App.css apps/web/src/App.tsx`
- Banner de alta de Jubilación **retirado** (F5): `grep -c "retirement-intro-banner" apps/web/src/App.css apps/web/src/views/RetirementView.tsx` (**0** en los dos) y `ls apps/web/src/lib/retirement-intro.ts` debe FALLAR
- `.retirement-radio-stack` tiene consumidores (cero antes de `9ae5c24`): `grep -c "retirement-radio-stack" apps/web/src/views/RetirementView.tsx` (≥1)
- Excepción de solo lectura en Movimientos: `grep -n "disabled={!canEdit" apps/web/src/views/GastosView.tsx`
- Anclas actuales del freezer: `grep -n 'App.css:' apps/web/src/styles/no-hex-outside-theme.test.ts` — deben casar con `grep -n "rgba(0, 0, 0," apps/web/src/App.css`
- Tira de fases (clases y tokens): `grep -n "projection-phase-band\|projection-phase-label\|projection-phase-mark-label" apps/web/src/App.css apps/web/src/views/ProjectionNetWorthChart.tsx apps/web/src/components/charts/MiniProjection.tsx`
- Su alto sale del plot, no de lienzo extra: `grep -n "layoutDims.ph - xAxisExtraBottom - phaseStripH" apps/web/src/views/ProjectionNetWorthChart.tsx`
- Tarjeta de plan (D27 original) sin consumidor en `SummaryView.tsx`: `git grep -n "plan-card-grid\|className=\"plan-card\"" -- 'apps/web/src/**/*.tsx'` vacío — la tarjeta viva es `.plan-card-wide*` (ver addendum U9/U10 abajo)
- El viejo `<select>` de vista desapareció: `grep -n "ledger-view-select" apps/web/src/App.css apps/web/src/App.tsx` (debe imprimir vacío)
- Líneas de miembro y su color compartido: `grep -n "householdMemberColor" apps/web/src/lib/*.ts` (≥3 ficheros: definición, tira de fases y líneas)
- La leyenda del miembro dibuja línea, no discontinua: `grep -n 'swatch: "line" as const' apps/web/src/lib/phase-strip.ts`
- «(derivada)» y su salida en el formulario: `grep -n "targetBasisSource\|Volver a la derivada" apps/web/src/views/RetirementView.tsx`
- Tokens de las auxiliares, en las DOS ramas del tema: `grep -c -- "--proj-required" apps/web/src/styles/theme.css` (2) y `grep -c -- "--proj-coast" apps/web/src/styles/theme.css` (2)
- Y sin hex propio: `grep -n -- "--proj-required\|--proj-coast" apps/web/src/styles/theme.css` — las cuatro líneas son `color-mix`, ninguna un literal
- Tono rojo de KPI: `grep -n "metric-card--danger" apps/web/src/App.css apps/web/src/components/MetricCard.tsx` y su único consumidor `grep -n 'tone={t.tone === "danger"' apps/web/src/views/RetirementView.tsx`
- Segunda rejilla y avisos de Jubilación (retirados en U1b, ver el addendum U5b más abajo): `grep -c "retirement-solve-grid\|retirement-strategy-notices" apps/web/src/App.css apps/web/src/views/RetirementView.tsx` (**0** en los dos)
- Cifras del plan (D27 original, retiradas en U9): `grep -c "plan-card-figures" apps/web/src/views/SummaryView.tsx apps/web/src/App.css` (**0** en los dos desde U5b — la regla se borró de `App.css`, no solo su consumidor)
- **El abanico vive dentro de `MiniProjection` desde U1b** (ya NO en un componente propio): `ls apps/web/src/components/charts/RiskFanChart.tsx` debe FALLAR (no existe) y `grep -c "p10\|p90" apps/web/src/components/charts/MiniProjection.tsx` da un número **NO-CERO** (13 el 2026-09-03) — antes de U1b daba 0 y esa cifra en 0 era la prueba de que el abanico vivía fuera; el mismo comando invertido es ahora la prueba de que vive dentro.
- `--ff-warn` está en las DOS ramas del tema y sin más consumidores que el tono de KPI: `grep -c -- "--ff-warn" apps/web/src/styles/theme.css` (2) y `grep -rn -- "var(--ff-warn)" apps/web/src/App.css` (solo `.metric-card--warn`)
- Clases de la sección «Riesgo compacto» (`.risk-fan-note` retirada junto con `RiskFanChart.tsx`, ver addendum U5b): `grep -c "risk-fan-note" apps/web/src/App.css apps/web/src/views/RetirementView.tsx` (**0** en los dos) y `grep -n "risk-depletion-grid\|risk-extra-rows\|risk-footnote\|summary-success-grid" apps/web/src/App.css apps/web/src/views/RetirementView.tsx apps/web/src/views/SummaryView.tsx` (estas cuatro SIGUEN vivas)
- La ayuda por fila reusa `label-with-help` y NO añadió CSS: `grep -n "label-with-help risk-extra-label" apps/web/src/views/RetirementView.tsx` (2: la tabla de agotamiento y las filas extra) y `grep -c "risk-extra-help" apps/web/src/App.css` (**0** — no hay clase nueva)
- El valor del KPI de éxito es una oración, y envuelve sin CSS nuevo: `grep -n -A 6 '^\.metric-value-row' apps/web/src/App.css` (el `flex-wrap: wrap` que ya estaba es lo único que hace falta) y `grep -n "de cada 100 escenarios se jubilan" apps/web/src/lib/risk-bands.ts`
- La aritmética del abanico vive PURA y testeada: `grep -c 'it(' apps/web/src/lib/risk-bands.test.ts`

**Añadido 2026-09-03 para la segunda revisión de UX de Jubilación (U0–U12, issue #207), rama
`release/5.0.0`**: la tarjeta ancha `.plan-card-wide*` y las frases del hogar `.plan-sentence-*`
que sustituyen al trío D27 original en `SummaryView.tsx` (U9/U10), y las dos reglas nuevas de
§Reglas para añadir UI nueva (divisa fuera del subtítulo, scope solo por el segmentado). Ese pase
NO documentaba `RetirementView.tsx`/`components/charts/*` porque U1b los estaba reescribiendo en
paralelo. Re-verify with:

- La tarjeta ancha y sus piezas: `grep -n "plan-card-wide" apps/web/src/App.css apps/web/src/views/SummaryView.tsx`
- Las frases del hogar: `grep -n "plan-sentence-list\|plan-sentence-item\|plan-sentence-dot" apps/web/src/App.css apps/web/src/views/SummaryView.tsx`
- Las dos reglas de copy nuevas, sin regresión: `grep -rn "Moneda EUR\|Mensual · EUR\|Importes · EUR\|sin titular en Hogar" apps/web/src/views/*.tsx` — un único acierto (`SummaryView.tsx`, el comentario «S7» que explica la retirada), cero en JSX renderizado

**Completado 2026-09-03 en el pase de documentación U5b (mismo issue #207), tras aterrizar
`debc52d` (U1b)**: la sección `MiniProjection` gana las props `band`/`markers`/`deflator`; la
sección `RiskFanChart` se sustituye por «Riesgo compacto y «Detalle del cálculo»» (el componente se
retiró, y con él `.risk-fan-note`); las tres nuevas notas de patrón (frase-hito con filete de tono,
tiles v2 con subtítulo que envuelve, acordeón «Avanzado»/indicador de guardado único); y el trío
D27 original (`.plan-card-grid`/`.plan-card`/`.plan-card-figures`) **se retiró de verdad de
`App.css`** — ya no es «sin consumidor, retiro pendiente». Re-verify with:

- `RiskFanChart.tsx` no existe: `ls apps/web/src/components/charts/RiskFanChart.tsx` (falla)
- `MiniProjection` tiene las tres props: `grep -n "band?:\|markers?:\|deflator?:" apps/web/src/components/charts/MiniProjection.tsx`
- El trío D27 y `.risk-fan-note` fuera de `App.css` (regla, no solo consumidor): `grep -c "^\.plan-card {\|^\.plan-card-grid {\|^\.plan-card-figures {\|^\.risk-fan-note {" apps/web/src/App.css` (**0**)
- La frase-hito usa filete, no color de texto: `grep -n -A 4 "^\.retirement-sentence {" apps/web/src/App.css` (sin `color` propio en el bloque base; el tono está en `border-left-color`, tres reglas después)
- El acordeón «Avanzado» pinta `assumptions` en el `<summary>`: `grep -n "retirement-advanced-summary-text" apps/web/src/App.css apps/web/src/views/RetirementView.tsx`
- El acordeón «Avanzado» y su línea «Supuestos» **ya no existen**: `grep -c "^export const ADVANCED_SECTION_LABEL" apps/web/src/lib/retirement-form.ts` (**0**) y `grep -c "^\.retirement-advanced" apps/web/src/App.css` (**0**) — anclados a la DECLARACIÓN, porque los dos nombres siguen citados en los comentarios que explican su retirada; y `ls apps/web/src/lib/assumptions-line.ts` debe FALLAR

**Añadido en la tercera vuelta de UX de Jubilación (V1–V7; feedback F2 y F5–F10 del owner,
2026-09-05, rama `release/5.0.0`)**: §Tarjetas de configuración, las cuatro props nuevas de
`MiniProjection`, la §Riesgo compacto reescrita, la norma de títulos P6 y la variante descartada del
color. Re-verify with:

- Las seis tarjetas y su copy: `grep -n "PLAN_CARD_ORDER" -A 8 apps/web/src/lib/plan-fields.ts` (seis ids, `risk` NO está) y `grep -c "^    blurb:" apps/web/src/lib/retirement-form.ts` (**6** — anclado a la indentación de las entradas, para no contar también la declaración del tipo)
- Las cuatro props nuevas del chart: `grep -n "yAxis?:\|bandGradient?:\|bandEdgeLabels?:\|hoverLabel?:" apps/web/src/components/charts/MiniProjection.tsx` (4 aciertos)
- La invariante «sin ellas, byte a byte»: `grep -n "const padLeft = yAxis" apps/web/src/components/charts/MiniProjection.tsx` (degrada a `padX`)
- El degradado usa `userSpaceOnUse` y el id prefijado: `grep -n "gradientUnits=\"userSpaceOnUse\"\|ff-risk-" apps/web/src/components/charts/MiniProjection.tsx`
- Cortes absolutos del color, en un módulo puro y testeado: `grep -n "RISK_AMBER_AT\|RISK_RED_AT" apps/web/src/lib/risk-gradient.ts` y `grep -c "it(" apps/web/src/lib/risk-gradient.test.ts`
- La tabla de agotamiento por edad **ya no existe**: `grep -c "risk-depletion" apps/web/src/App.css apps/web/src/views/RetirementView.tsx` (**0** en los dos)
- El valor del KPI de éxito es un porcentaje, no una oración: `grep -n "formatSuccessPercent\|successParenthetical" apps/web/src/lib/risk-bands.ts` y `grep -c "export function formatSuccessScenarios\|export function formatSuccessThreshold" apps/web/src/lib/risk-bands.ts` (**0**; el literal de la oración vieja NO sirve como grep: sobrevive citado en el docblock que explica por qué se retiró, y un comando que se cuenta a sí mismo es deriva silenciosa)
- La escala del color NO es un ítem de leyenda: `grep -n "retirement-risk-scale" apps/web/src/App.css apps/web/src/views/RetirementView.tsx`
- `.subsection-title` sigue teniendo consumidores (deuda declarada arriba): `grep -rl "subsection-title" apps/web/src/views apps/web/src/components` (hoy `charts/summary.tsx` y `GastosView.tsx`; el día que imprima vacío, retira la clase de `App.css`)
