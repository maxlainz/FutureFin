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
> del plan» tiene TRES valores, no dos: verde en el umbral o por encima, ámbar hasta diez puntos
> porcentuales por debajo, rojo el resto. Con solo `--ff-pos`/`--ff-neg` el peldaño intermedio se
> pintaba de rojo (alarma donde no la hay) o de nada (indistinguible de «va bien»), y el semáforo
> dejaba de ser un semáforo. El token es el **hermano de `--ff-neg` en la escala de estado**: mismo
> croma y misma luminancia en cada rama, hue 75. No es un acento nuevo y **no decora**: la
> restricción es la de pos/neg — solo marca estado (hoy: la piel de `.metric-card--warn`), nunca
> chrome, iconos ni texto decorativo. El veredicto lo decide el SERVIDOR (`success_verdict`); la SPA
> solo lo traduce a tono.

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
- **Banner de alta `.retirement-intro-banner`** (5.0.0, D33, issue #207, solo en `RetirementView.tsx`): «Elige tu estrategia de jubilación» (+ «Añade tu fecha de nacimiento», enlace a «Tu cuenta», si falta), descartable UNA vez por navegador (`lib/retirement-intro.ts`). También `.banner.info-banner` por debajo — cero color propio — con dos clases de layout: `.retirement-intro-banner` (`flex; justify-content: space-between; flex-wrap: wrap`, texto y botón de descarte en la misma línea, apilados si no caben) y `.retirement-intro-banner-text` (columna, `min-width: 0` para que el texto largo no reviente el flex). Nunca se muestra junto al de Hogar: solo aparece cuando `!scopeReadOnly`.
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

### `RiskFanChart` — [`components/charts/RiskFanChart.tsx`](../apps/web/src/components/charts/RiskFanChart.tsx) (5.0.0, D28, issue #207)

Abanico de percentiles de la sección «Riesgo» de Jubilación: **área entre p10 y p90 + mediana p50
discontinua + línea determinista sólida**, las tres sobre el mismo eje X.

- **Por qué NO es una prop más de `MiniProjection`** (la regla de «usa MiniProjection para
  cualquier chart pequeño» tiene aquí su excepción declarada): aquel dibuja UNA serie sobre la
  rejilla de `points[]`; este tiene **dos fuentes con rejillas distintas** —la banda viaja siempre
  a densidad `hybrid` y la serie que el cliente tiene cargada suele ser `monthly`—, un área entre
  percentiles y un eje Y que no puede anclarse a 0. Meterlo dentro habría añadido cinco props y una
  segunda máquina de escalas al componente que ya usan Resumen y Jubilación.
- **Todo por MES** (`xAtMonth`), nunca por posición de array: con dos densidades a la vez,
  emparejar por índice desplaza el abanico décadas y el chart resultante **sigue pareciendo
  correcto**. La aritmética (alineación, deflactación, rango) vive pura en
  [`lib/risk-bands.ts`](../apps/web/src/lib/risk-bands.ts) con su test; el componente solo pinta.
- **Color: dos tokens.** Banda y mediana en `--ff-accent` (es la lectura del plan, familia del
  objetivo FIRE); la determinista en `--proj-nw`, el color con el que el patrimonio se dibuja en
  toda la app, para que se reconozca la curva que ya se conoce. Se separan **además por patrón**
  (relleno 16 % / guion `5 3` / sólida), así que la identidad nunca es solo color. La opacidad va
  en `fillOpacity`/`strokeOpacity` y no dentro del color: el mismo token resuelve claro y oscuro.
- El marcador de jubilación usa `--proj-fire`, el mismo que en el chart grande.
- Leyenda obligatoria (3 series) vía `ChartLegend` con swatches `area` / `dashed` / `line`, y bajo
  el chart la nota **«Bandas puntuales: la mediana no es un camino»** (`.risk-fan-note`, con el
  `HelpPopover` de `retirement.bands` en la misma línea): la curva central se calcula ordenando los
  valores de CADA mes, así que no corresponde a ninguna simulación. Es una instrucción de lectura,
  por eso va pegada al chart y no en el pie — y por eso la ayuda cuelga de ella y no del título del
  panel.
- **La ayuda de las filas cuelga del RÓTULO de cada fila** (`.risk-extra-head`), no del panel:
  `RiskExtraRow` lleva un `helpId?` opcional y la vista envuelve el rótulo en
  `label-with-help risk-extra-label` — el mismo patrón que ya usaba la tabla de agotamiento. Una
  sola ayuda en el título explicaría la fila que el usuario no está mirando. Sin `helpId` la fila
  conserva su `<span class="risk-extra-label">` pelado y no gasta el icono.
- **La cifra de «Éxito del plan» es una FRASE, no un número corto** (5.0.0, pase de correcciones
  §G): «87 de cada 100 escenarios se jubilan y no agotan el capital». `.metric-value` es mono a
  1.25rem dentro de un `.metric-value-row` con `flex-wrap: wrap`, así que envuelve a dos o tres
  renglones y la tarjeta crece — **es deliberado y no se recorta**: el rótulo corto anterior
  describía la definición vieja del éxito y sobrevivió a ella. El primer slot sigue siendo el
  umbral y el segundo, en el Resumen, los escenarios que no llegan a jubilarse (`nowrap` +
  ellipsis, como todo `.metric-value-detail`). Es el único KPI de la app cuyo valor es una
  oración; si algún día hay dos, ese es el momento de darle una variante propia a `.metric-value`,
  no de acortar la frase.

Clases de la sección (todas en `App.css`, cero color propio salvo `--ff-warn`):
`.metric-card--warn` (tono ámbar del KPI, misma construcción que `--danger`), `.risk-fan-note`,
`.risk-depletion-grid` + `.risk-depletion-cell`/`-age`/`-value` (la tabla «Probabilidad de agotar
el capital» — `auto-fit` con `minmax(min(50%, 7rem), 1fr)`, que da **dos columnas en móvil sin una
media query nueva**), `.risk-extra-rows`/`.risk-extra-row`/`-head`/`-label`/`-value`/`-detail` y
`.risk-footnote` (procedencia: ms, caminos y semilla; `overflow-wrap: anywhere` porque la semilla
es un entero de 20 dígitos que no cabe a 360px).

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
> **Tarjeta de plan (`.plan-card-grid`/`.plan-card`)** (5.0.0, D27/D32, issue #207, `SummaryView.tsx`): tarjeta de ESTADO, no un KPI — reusa la piel de `.metric-card` (papel + filete suave + `--ff-radius-kpi` + `--ff-shadow-flat`) pero su cifra grande no es dinero, así que solo el hito va en monoespaciada; cuando no hay hito, el texto cae a la tipografía del cuerpo atenuada (`.plan-card-milestone--absent`) para que una explicación no se lea como un dato exacto. `.plan-card-detail` es un **slot siempre reservado** (`&nbsp;` sin edad), misma disciplina que el paréntesis de `MetricCard`: sin él, dos tarjetas del hogar con y sin edad dejan la línea de estado a alturas distintas. El estado rojo (`.plan-card--danger` + `.plan-card-status--danger`) usa **el mismo tinte que `.error-banner`** (`--ff-neg` al 8 % sobre `--ff-paper`) — un solo vocabulario de «esto va mal» en toda la app. En Hogar la rejilla lleva una tarjeta por miembro y colapsa a una columna ≤640px.
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
> **Cifras del plan (`.plan-card-figures`)** (5.0.0 WP7-3b2, `SummaryView.tsx`): fila propia entre
> el hito y el estado de `.plan-card`, con «Ahorro necesario X/mes · Margen Y/mes». Va en su renglón
> y no junto al hito porque son magnitudes AL MES y el hito es una fecha — en la misma línea se leen
> como una sola frase. `flex-wrap` para que en móvil apilen alineadas a la izquierda.
>
> **Segunda rejilla de Jubilación (`.metric-grid.retirement-solve-grid`)** y **avisos
> (`.retirement-strategy-notices`)** (5.0.0 WP7-3b2, `RetirementView.tsx`): las tarjetas que dependen
> de la estrategia elegida NO entran en la banda superior (`.workspace-kpi-strip`, que es `nowrap`
> con scroll horizontal): con media jornada más una pensión serían siete tarjetas en una sola tira
> desplazable. Van en una `.metric-grid` que **envuelve** (`minmax(min(100%, 12rem), 1fr)`), y bajo
> ella los avisos que solo matizan el cálculo, en `.muted` y sin color propio — el rojo vive arriba,
> en un `.error-banner`, porque cambia cómo se leen TODAS las cifras y no solo una.
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

1. **Usa los tokens**. Nunca hardcoded hex. Si necesitas un color que no está, primero pregúntate si puedes vivir con `color-mix(in oklch, var(--ff-accent) X%, var(--ff-paper))`. Si no, añade un token nuevo en `theme.css` con variantes claro/oscuro. El enforcement automático es el freezer [`styles/no-hex-outside-theme.test.ts`](../apps/web/src/styles/no-hex-outside-theme.test.ts): sus contadas excepciones sancionadas (p. ej. la sombra del tooltip de Proyección, issue #105) se registran en `RGBA_ZERO_EXCEPTIONS` por **`file:línea` exacta, no por patrón** — cualquier edición de `App.css` que inserte líneas por encima desplaza el anclaje y rompe el test aunque el CSS no haya cambiado de verdad. 5.0.0 lo movió (2399/2400 → 2425/2426, cuando el segmentado «Yo | Hogar» y los banners de ámbito/alta se insertaron más arriba en el archivo). Si el freezer falla así: `grep -n "rgba(0, 0, 0," apps/web/src/App.css` para encontrar las líneas reales de hoy y actualiza los literales de `RGBA_ZERO_EXCEPTIONS` a esos números — no borres la excepción, muévela.
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
- Banner de alta de Jubilación: `grep -n "retirement-intro-banner" apps/web/src/App.css apps/web/src/views/RetirementView.tsx`
- `.retirement-radio-stack` tiene consumidores (cero antes de `9ae5c24`): `grep -c "retirement-radio-stack" apps/web/src/views/RetirementView.tsx` (≥1)
- Excepción de solo lectura en Movimientos: `grep -n "disabled={!canEdit" apps/web/src/views/GastosView.tsx`
- Anclas actuales del freezer: `grep -n 'App.css:' apps/web/src/styles/no-hex-outside-theme.test.ts` — deben casar con `grep -n "rgba(0, 0, 0," apps/web/src/App.css`
- Tira de fases (clases y tokens): `grep -n "projection-phase-band\|projection-phase-label\|projection-phase-mark-label" apps/web/src/App.css apps/web/src/views/ProjectionNetWorthChart.tsx apps/web/src/components/charts/MiniProjection.tsx`
- Su alto sale del plot, no de lienzo extra: `grep -n "layoutDims.ph - xAxisExtraBottom - phaseStripH" apps/web/src/views/ProjectionNetWorthChart.tsx`
- Tarjeta de plan: `grep -n "plan-card" apps/web/src/App.css apps/web/src/views/SummaryView.tsx`
- El viejo `<select>` de vista desapareció: `grep -n "ledger-view-select" apps/web/src/App.css apps/web/src/App.tsx` (debe imprimir vacío)
- Líneas de miembro y su color compartido: `grep -n "householdMemberColor" apps/web/src/lib/*.ts` (≥3 ficheros: definición, tira de fases y líneas)
- La leyenda del miembro dibuja línea, no discontinua: `grep -n 'swatch: "line" as const' apps/web/src/lib/phase-strip.ts`
- «(derivada)» y su salida en el formulario: `grep -n "targetBasisSource\|Volver a la derivada" apps/web/src/views/RetirementView.tsx`
- Tokens de las auxiliares, en las DOS ramas del tema: `grep -c -- "--proj-required" apps/web/src/styles/theme.css` (2) y `grep -c -- "--proj-coast" apps/web/src/styles/theme.css` (2)
- Y sin hex propio: `grep -n -- "--proj-required\|--proj-coast" apps/web/src/styles/theme.css` — las cuatro líneas son `color-mix`, ninguna un literal
- Tono rojo de KPI: `grep -n "metric-card--danger" apps/web/src/App.css apps/web/src/components/MetricCard.tsx` y su único consumidor `grep -n 'tone={t.tone === "danger"' apps/web/src/views/RetirementView.tsx`
- Segunda rejilla y avisos de Jubilación: `grep -n "retirement-solve-grid\|retirement-strategy-notices" apps/web/src/App.css apps/web/src/views/RetirementView.tsx` (el nombre NO es `retirement-strategy-grid`: esa clase ya existe **sin envolver** para las 5 radio-cards de estrategia y se habría aplicado también aquí)
- Cifras del plan: `grep -n "plan-card-figures" apps/web/src/App.css apps/web/src/views/SummaryView.tsx`
- **WP7 3c** — el abanico existe y no vive dentro de `MiniProjection`: `ls apps/web/src/components/charts/RiskFanChart.tsx` y `grep -c "p10\|p90" apps/web/src/components/charts/MiniProjection.tsx` (**0**)
- `--ff-warn` está en las DOS ramas del tema y sin más consumidores que el tono de KPI: `grep -c -- "--ff-warn" apps/web/src/styles/theme.css` (2) y `grep -rn -- "var(--ff-warn)" apps/web/src/App.css` (solo `.metric-card--warn`)
- Clases de la sección «Riesgo»: `grep -n "risk-fan-note\|risk-depletion-grid\|risk-extra-rows\|risk-footnote\|summary-success-grid" apps/web/src/App.css apps/web/src/views/RetirementView.tsx apps/web/src/views/SummaryView.tsx`
- La ayuda por fila reusa `label-with-help` y NO añadió CSS: `grep -n "label-with-help risk-extra-label" apps/web/src/views/RetirementView.tsx` (2: la tabla de agotamiento y las filas extra) y `grep -c "risk-extra-help" apps/web/src/App.css` (**0** — no hay clase nueva)
- El valor del KPI de éxito es una oración, y envuelve sin CSS nuevo: `grep -n -A 6 '^\.metric-value-row' apps/web/src/App.css` (el `flex-wrap: wrap` que ya estaba es lo único que hace falta) y `grep -n "de cada 100 escenarios se jubilan" apps/web/src/lib/risk-bands.ts`
- La aritmética del abanico vive PURA y testeada: `grep -c 'it(' apps/web/src/lib/risk-bands.test.ts`
