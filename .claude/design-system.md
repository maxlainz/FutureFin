# Design System — V1 redesign (May 2026)

Refresca de UI completa, **solo frontend**: no toca handlers, datos ni endpoints. Origen: bundle de Claude Design exportado (`futurefin/`). El diseño se llamó "V1" en el chat de iteración — el resto de variantes (V2–V4) se descartaron.

> **Si haces cambios visuales, lee este doc primero.** Los tokens y reglas viven aquí; no improvises colores ni clases nuevas en App.css.

## Identidad

- **Base monocromática**: blanco roto (zinc-100 `#f4f4f5`) en claro / casi-negro (zinc-950 `#0a0a0a`) en oscuro.
- **Único color de marca**: acento periwinkle (`oklch(0.56 0.13 250)` claro, `oklch(0.74 0.11 250)` oscuro). Cualquier "destacado" del UI usa solo este color.
- **Sin tonos cálidos**: no hay crema, beige, etc. Si encuentras alguno en el código, restitúyelo a un zinc.
- **Semánticos pos/neg**: verde/rojo **únicamente para texto de cifras delta** (deltas, saldos, "−€640"). Nunca en fondos, bordes, iconos o chrome decorativo.
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

Radii: `--ff-radius-{frame=12, panel=14, kpi=12, pill=999, input=10}px`.

### Tokens del chart de proyección

`--proj-nw`, `--proj-cc`, `--proj-fire`, `--proj-jub`, `--proj-grid`, `--proj-plot-bg`, `--proj-tick`, `--proj-meta`, `--proj-axis`, `--proj-milestone`, `--proj-crosshair`.

Áreas de activos: `--proj-area-1` a `--proj-area-10` (paleta polícroma — azul/teal/violeta/... en claro, pasteles más claros en oscuro). Consumidos por [`ASSET_LINE_COLORS`](../apps/web/src/lib/projection-chart.ts).

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
| `--cf-savings` | `= var(--ff-accent)` | `= var(--ff-accent)` | Serie **Ahorro** del cash-flow |

> **Comparativa por categoría eliminada**: el chart `CategoryComparisonBars` (barras horizontales Budget vs Promedio) se retiró tras 2.0.0 — con él se fueron el token **`--exp-average`** y el bloque CSS `.cmp-*`. El valor Real ya vivía en la tabla y las KPIs, y la tendencia vs presupuesto pasó a la banda de KPIs (ver §KPIs). El único chart que queda en ese archivo es `MonthlyCashflowBars`.
>
> **Excepción explícita a la regla "sin rojo/verde en el chrome"**: el cash-flow (`MonthlyCashflowBars`) introduce **verde/rojo** (`--cf-income`/`--cf-expense`) para ingresos vs gastos: son colores **funcionales de serie del gráfico**, no chrome decorativo, y por tanto quedan dentro de la única zona (charts) donde el design system acepta varios colores. Verifica claro **y** oscuro.

## Tema (auto / claro / oscuro)

- Estado vive en `App.tsx` (`themePref: ThemePref`), persistido en `localStorage["ff-theme-pref"]`.
- Default: `"auto"` → sigue `prefers-color-scheme` y se suscribe a cambios en vivo.
- Aplicación: `applyTheme(pref)` pone `data-theme="dark"|"light"` en `<html>`.
- Toggle UI: segmented Auto/Claro/Oscuro en `Ajustes → Datos y sistema → Apariencia`.
- Helpers en [`apps/web/src/lib/theme.ts`](../apps/web/src/lib/theme.ts).

## Shell

- **TopBar única** ([`components/TopBar.tsx`](../apps/web/src/components/TopBar.tsx)): marca `FF · FutureFin` izquierda, pills de navegación derecha, slot `extras` (selector de vista mío/hogar) anclado en esquina superior derecha, botón hamburguesa solo `≤720px`.
- **Sin tab-bar separada**: la `.tab-bar` legacy está `display:none`. Toda la navegación vive en TopBar.
- **Móvil**: hamburguesa abre `MobileNavDrawer` ([`components/MobileNavDrawer.tsx`](../apps/web/src/components/MobileNavDrawer.tsx)) — drawer lateral derecho con todas las secciones. Sin bottom-nav.
- **Cuenta**: el chip de usuario + Salir vivían en la cabecera; ahora la cuenta vive en `Ajustes` como `AccountCard` ([`components/AccountCard.tsx`](../apps/web/src/components/AccountCard.tsx)) con avatar, badge de rol, botones Editar cuenta + Cerrar sesión. La cabecera queda limpia.

## Layout

- **Ancho de contenido**: `max-width: 66rem` (`app-main`). Antes era ancho completo; ahora el contenido se centra. Proyección sigue siendo full-bleed (`.app-main--projection-fullbleed`).
- **KPIs**:
  - Tile con borde + paper, radius `--ff-radius-kpi`, `align-self: stretch` para alinear en altura.
  - **Slot del paréntesis siempre presente** — `MetricCard` renderiza un `<div>` con `&nbsp;` cuando no hay valor, para que dos KPIs en la misma fila tengan baseline alineada.
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
- Controles primarios a ≥ `--ff-touch-min` en `≤640`: `.btn`, `.btn.icon-btn`, `.field input/select` (checkbox/radio excluidos), `.modal-close`, hamburguesa (`.ff-topbar-mobile-toggle`), items del drawer (`.ff-mobile-drawer-item`), switch de Proyección (`2.6×1.5rem`).
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

Chart compacto reutilizable, mismo lenguaje visual que la Proyección. Usado en Resumen (12 m) y Jubilación.

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

### `MiniProjectionLegend`

Leyenda discreta que acompaña al MiniProjection. Cada item: `{ label, color, dash? }`.

## Componentes nuevos (UI)

| Componente | Propósito |
|---|---|
| `TopBar` | Cabecera única (marca + pills + extras + hamburguesa) |
| `MobileNavDrawer` | Drawer derecho con todas las secciones (≤720px) |
| `AccountCard` | Cuenta destacada en Ajustes (avatar + rol + acciones) |
| `ThemeToggle` | Segmented Auto/Claro/Oscuro |
| `MiniProjection` | Chart compacto reutilizable (ver arriba) |

> **Segmented control (`.ff-theme-toggle`)**: el único segmented que queda es el toggle de tema Auto/Claro/Oscuro (`ThemeToggle`, clase `.ff-theme-toggle`). La antigua clase compartida **`.ff-segmented` se eliminó** tras 2.0.0: la «fuente del ahorro» de `Ajustes → Proyección` pasó a un `<select>` nativo estándar (con `<small>` de ayuda asociada por `aria-describedby`, fuera del `<label>`). Si necesitas un nuevo control inline de 2–3 opciones, valora primero un `<select>`; si de verdad hace falta un segmented, reintroduce la variante en el bloque de `.ff-theme-toggle` en `App.css`. Verifica claro **y** oscuro.

## Iconografía

`apps/web/src/components/icons.tsx` — set unificado:

- viewBox `16×16`, `stroke="currentColor"`, `strokeWidth=1.5`, `linecap/linejoin="round"`.
- El color lo da el padre; el tamaño se controla por CSS (sin width/height fijos en los SVG).
- Iconos disponibles: `PlusIcon`, `RowEditIcon`, `RowTrashIcon`, `GearIcon`, `XIcon`, `CheckIcon`, `MoreIcon`, `ChevronIcon`, `ChevronLeftIcon`, `ChevronDownIcon`, `MenuIcon`, `UserIcon`, `DragIcon`, `DownloadIcon`, `CalendarIcon`, `FilterIcon`, `SortIcon`, `LinkIcon`, `RefreshIcon`, `EyeIcon`, `SearchIcon`, `ArrowUpIcon`, `ArrowDownIcon`, `DuplicateIcon`.

## Reglas para el chart de Proyección grande

- **Composición intacta**: hover, zoom, leyenda, tooltips, dimensiones, planning markers, jubilación pill — todo idéntico a antes del rediseño. Solo se sustituyeron hex hardcoded por `var(--proj-*)`.
- **Tooltip independiente del tema**: forzado a `color: #fafafa !important` + bg `rgba(10,10,10,0.92)`. Antes usaba `var(--ff-frame)` que en oscuro daba texto oscuro sobre fondo oscuro.
- **Leyenda con espaciado dinámico**: `legendCharPx=7.6` (era 6.5, subestimaba) y `legendBudgetWidth=0.66*pw` (era 0.6). Ver [`buildProjectionChartLayout`](../apps/web/src/lib/projection-chart.ts).
- **Milestones con collision-avoidance**: si dos milestones quedan cerca horizontalmente, el segundo sube al siguiente "carril" (14px arriba) y la línea punteada se estira hasta la nueva `y2`. Ver [`ProjectionNetWorthChart.tsx`](../apps/web/src/views/ProjectionNetWorthChart.tsx) en el bloque `(() => { … lanes … })()`.

## Reglas para añadir UI nueva

1. **Usa los tokens**. Nunca hardcoded hex. Si necesitas un color que no está, primero pregúntate si puedes vivir con `color-mix(in oklch, var(--ff-accent) X%, var(--ff-paper))`. Si no, añade un token nuevo en `theme.css` con variantes claro/oscuro.
2. **Verifica claro y oscuro antes de mergear**. Toggle desde Ajustes y revisa: KPIs, modales, tooltips, hover states, focus rings.
3. **No mezcles tab-bar legacy con TopBar**. La nav es responsabilidad exclusiva de `TopBar`. Sub-tabs (como las de Ajustes) van como pills con clase `ff-nav-pill`.
4. **No introduzcas color decorativo**. Pos/neg = cifras delta. Acento = destacar UN ítem (botón primario, KPI hero, marker de jubilación, slice principal de un donut). El resto vive en grayscale.
