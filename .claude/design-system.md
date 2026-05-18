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
  - Variantes: `tone="hero" | "accent" | "accent-2"` con tinte progresivo del acento.

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
