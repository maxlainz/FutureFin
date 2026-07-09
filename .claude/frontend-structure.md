# Frontend Structure (`apps/web/src/`)

Post-refactor (May 2026). Before: one `App.tsx` of 10.384 LOC owning everything. After: composition root + per-concern modules.

```
src/
├── App.tsx                       # composition root: auth gate + global state + route → view dispatch
├── App.css                       # global styles (consume --ff-* tokens; no hardcoded hex)
├── index.css                     # minimal reset, font-family
├── main.tsx                      # ReactDOM.createRoot entry — imports styles/theme.css before index.css
│
├── styles/
│   └── theme.css                 # design tokens (--ff-*, --proj-*) con variantes claro/[data-theme=dark]
│
├── api/
│   ├── client.ts                 # fetch wrappers: apiGet/Post/Put/Patch/Delete + defaultFetchInit + errorMessageFromResponse
│   ├── client.test.ts            # mocks `globalThis.fetch`, asserts credentials/Content-Type/204
│   └── types.ts                  # all *Api / *Response / *Row types (mirror of Rust handler structs)
│
├── lib/                          # pure helpers, no React imports
│   ├── format.ts                 # money/percent/decimal formatting (es-ES locale), parseDisplayDecimal, METRIC_DASH
│   ├── format.test.ts            # 29 tests
│   ├── dates.ts                  # civil-calendar arithmetic (parallel to crates/engine), TZ-aware "today", interval counts
│   ├── dates.test.ts             # 29 tests (incl. formatDateDm)
│   ├── ledger.ts                 # shared by views: ledgerViewQs, groupRowsByCategoryOrdered, asset/liability portfolio helpers,
│   │                             #   PAYMENT_FREQ_LABEL, formatProjectionMilestoneCompactLabel, budgetCategoryMap,
│   │                             #   sortBudgetEntriesMacStyle, formatAxisMoney, LedgerPersonScope, LiabilityPaymentFreq
│   ├── fire.ts                   # client-side FIRE math for the live form preview (mirror of handlers/projection.rs):
│   │                             #   defaultFireSettingsApi, normalizeInstallationFireSettings, taxOnGrossCapitalAnnual,
│   │                             #   grossUpNetAnnualFire, computeFireAnnualNeedNetEur, findFirstMonthNetWorthAtLeastInflated
│   ├── projection-chart.ts       # chart helpers: tick builders (startMonth param → soporta meses negativos), SVG layout,
│   │                             #   niceYTicks, axis age/dates mode, deflationFactorAt (deflactor keyed por month_index; k<0 amplifica),
│   │                             #   PROJECTION_FOCUS_STORAGE_KEY, ASSET_LINE_COLORS (CSS vars), complementaryProjectionTickLabel,
│   │                             #   projectionHoverTitle, formatYearsEsFromMonths, formatProjectionChartHorizonLine
│   ├── history-merge.ts          # mergeProjectionWithHistory(series, history): une la serie histórica (month_index<0) con la
│   │                             #   proyección en el vértice mes-0; identidad byte-idéntica si history null/vacío/anchor distinto
│   ├── snapshot-tracker.ts       # trigger del modal: EditLog (Map<assetId, epochMs>), SNAPSHOT_EDIT_WINDOW_MS, pruneEditLog,
│   │                             #   liquidCoverageComplete (todos los activos líquidos editados dentro de la ventana rodante ~1h)
│   ├── navigation.ts             # tab ↔ URL map: TABS, TAB_PATH (incl. expenses → «Movimientos», slug canónico /movimientos + alias de lectura /gastos en tabFromPathname), SETTINGS_SUBTAB_* (incl. history → «Histórico»/historico), tabFromPathname, settingsSubTabPath
│   ├── expenses.ts               # pure helpers de la pestaña «Movimientos»: month labels (monthLabelEs/monthShortLabelEs), defaultSelectedMonth,
│   │                             #   categoriesForKind (savings→[]), ImportRowDraft + initialDraftForRow/buildConfirmDecisions/summarizeDecisions/rowMatchesFilter,
│   │                             #   deltaToneClass/formatDeltaCurrency (rojo/verde solo en deltas), significanceThreshold (1% del ingreso real)/trendArrow/significantDeltaTone
│   │                             #   (umbral de significancia de las flechas ↑↓), AVG_WINDOWS + avgWindowLabel (pills 3m/6m/12m/YTD/Todo), capitalizeSource, y los helpers de la tabla de
│   │                             #   movimientos: normalizeSearchText/transactionMatchesQuery (búsqueda sin acentos), compareTransactions/sortTransactions + naturalSortDir (TxnSortKey/
│   │                             #   TxnSortDir; importe por |magnitud|, tiebreak estable), groupTransactionsByCategory/sortTransactionGroups (orden fijo: kind → |subtotal| desc). Test: expenses.test.ts
│   ├── files.ts                  # readFileAsBase64(File): base64 en trozos de 32 KiB. Compartido por el import .ffbackup (App.tsx) y el wizard de CSV
│   ├── responsive.ts             # MOBILE_MAX_WIDTH (640 = bp:mobile), isMobileWidth (puro, test en node) y useIsMobile()
│   │                             #   (matchMedia, lectura síncrona inicial). Gatea el patrón «columnas esenciales» de TODAS las
│   │                             #   tablas en móvil: mismo boolean para th y td (desincronización imposible), fila tappable →
│   │                             #   modal de edición. Desktop byte-idéntico con isMobile=false. Test: responsive.test.ts
│   ├── chart-gestures.ts         # aritmética PURA de los gestos táctiles del chart grande: clampWindowStart, panWindow,
│   │                             #   pinchWindow (+ ChartDomain). Espejo exacto de los clamps/ancla del onWheel — test de
│   │                             #   equivalencia en chart-gestures.test.ts. ProjectionNetWorthChart la consume desde su
│   │                             #   máquina de gestos Pointer Events (touch-action: pan-y; vertical = scroll de página)
│   └── theme.ts                  # ThemePref ("auto"|"light"|"dark") + apply/load/save + subscribeSystemThemeChanges
│
├── components/                   # generic UI primitives (no domain knowledge)
│   ├── TopBar.tsx                # cabecera única: marca + nav pills + extras + hamburguesa
│   ├── MobileNavDrawer.tsx       # drawer derecho ≤720px
│   ├── AccountCard.tsx           # tarjeta de cuenta en Ajustes (sustituye user-chip + Salir del header antiguo)
│   ├── ThemeToggle.tsx           # segmented Auto/Claro/Oscuro (usado en Ajustes → Datos)
│   ├── Modal.tsx                 # Modal + ModalFormError + InlineHint
│   ├── MetricCard.tsx            # KPI con paren-slot siempre presente (prop `trend?` ocupa el mismo slot, prioridad sobre `parenthetical`) + tone hero/accent/accent-2
│   ├── SnapshotButton.tsx        # botón «Guardar snapshot» (idle→busy→success/error) en panel-head de Activos y Pasivos
│   ├── SnapshotPromptModal.tsx   # modal «¿Guardar snapshot?» (paso assets → paso liabilities); tonto, lógica en App.tsx
│   ├── icons.tsx                 # set unificado 16×16 stroke 1.5 (~25 iconos)
│   └── charts/
│       ├── summary.tsx           # SummaryDonutChart + SummaryBreakdownBlock (palette fría=activos / cálida=pasivos)
│       ├── PlanningDirectionChart.tsx   # barra inflow/outflow — usada en Upcoming Y Budget
│       ├── CategoryComparisonBars.tsx   # exporta SOLO MonthlyCashflowBars (cash-flow mes a mes desde months[], tokens --cf-income/--cf-expense/--cf-savings; verde/rojo = colores
│       │                                #   FUNCIONALES de serie, excepción de charts del design system, no chrome). El chart CategoryComparisonBars (barras Budget vs Promedio) y el
│       │                                #   token --exp-average se ELIMINARON tras 2.0.0: el Real vive en la tabla/KPIs y la tendencia vs presupuesto pasó a la banda de KPIs
│       └── MiniProjection.tsx    # SVG compacto reusado en Resumen y Jubilación
│
├── views/                        # one file per tab — receives props from App.tsx, owns local UI state
│   ├── SummaryView.tsx           # KPIs → Salud financiera → Proyección 12m (zoomY) → Desglose
│   ├── AssetsView.tsx
│   ├── LiabilitiesView.tsx       # tabla sin columna Tipo
│   ├── BudgetView.tsx            # KPIs + Distribución (PlanningDirectionChart) + columnas Ingresos/Gastos
│   ├── GastosView.tsx            # pestaña «Movimientos» (título «Movimientos» desde v1.8.0; TabId interno sigue siendo "expenses"). Vista AUTÓNOMA (self-fetch,
│   │                             #   patrón HistorySettingsPanel): KPIs cuya cifra principal es el PROMEDIO de la ventana (etiqueta «… promedio (3m/6m/12m/YTD/total)», «—» sin datos);
│   │                             #   Gastos e Ingresos añaden bajo la cifra una línea `trend` (flecha + delta avg−budget «vs presupuesto», helper puro kpiBudgetTrend); Ahorro y Tasa sin
│   │                             #   delta (no hay budget de ahorro). Tasa de ahorro aquí = savings/income de la ventana (≠ la del Resumen, que es net/income — distinto a propósito).
│   │                             #   Selector de mes + pills de ventana (3m/6m/12m/YTD/Todo),
│   │                             #   comparativa por categoría con **fila TOTAL** y **flechas de tendencia** ↑↓/= (real vs promedio, umbral de significancia = 1% del ingreso real;
│   │                             #   «=» atenuado si hay promedio pero |Δ| ≤ umbral, slot vacío sin promedio; el glifo va en un **slot de ancho fijo** siempre presente para no
│   │                             #   desalinear las cifras — la comparativa por barras CategoryComparisonBars se eliminó tras 2.0.0, queda solo MonthlyCashflowBars como chart de apoyo),
│   │                             #   tabla de movimientos SIN scroll interno (se quitó table-scroll--sticky →
│   │                             #   la página crece; sin thead sticky) con **búsqueda** en vivo (concepto+categoría, insensible a acentos), **agrupación por categoría conmutable**
│   │                             #   (subtotal firmado por grupo; orden de grupos FIJO: secciones por kind ingresos → ahorro → gastos y |subtotal| desc dentro de cada sección,
│   │                             #   «Sin categoría» va con su kind) y **cabeceras ordenables** (fecha/concepto/importe; importe por magnitud; la clave activa solo ordena las filas
│   │                             #   dentro de cada grupo) — helpers puros en lib/expenses.ts —, edición inline optimista + modal (fecha/importe/concepto editables también en importadas: el backend ancla la huella al CSV) + tag «recurrente» +
│   │                             #   borrado con dos opciones (solo este / y detener repetición). Materializa recurrentes en silencio al montar (solo canEdit). `onCashflowMutated` avisa a App.
│   ├── ImportWizardModal.tsx     # wizard import CSV en 2 pasos (useReducer). Paso 1 = archivo → select «Cuenta origen (activo)» (movido desde el footer; ahora también en el preview) →
│   │                             #   formato en <details> plegado (autodetección por defecto). Paso 2 = banner con fuente capitalizada + chips de conteos, bulk bar con cluster único
│   │                             #   «Asignar a visibles», footer «{X} se importarán · {Y} excluidas ({Z} duplicadas)», columna «Tipo». /import/confirm con decisions[] paralelo. Stateless (sha256)
│   ├── ManualCashEntryModal.tsx  # alta manual de efectivo: grid multifila (magnitud + kind fija el signo) + checkbox «Repetir cada mes» por fila (→ recurrence:{}) → POST /v1/transactions/batch
│   ├── RecurringRulesModal.tsx   # modal «Recurrentes» (botón en la toolbar de Movimientos): lista GET /v1/transactions/recurring y permite «Detener» (DELETE) cada regla
│   │                             #   (conserva las instancias ya materializadas). Patrón ManualCashEntryModal: fetch al abrir, toda la lógica de presentación aquí (nada en lib/)
│   ├── UpcomingView.tsx          # Planning
│   ├── RetirementView.tsx        # KPIs + MiniProjection (zoomY, clampToMonth=jub+12, xAxis) + FIRE config
│   ├── ProjectionView.tsx        # wraps ProjectionNetWorthChart
│   ├── ProjectionNetWorthChart.tsx  # gran SVG chart, drag/zoom/hover, colores vía --proj-* tokens; se extiende a meses
│   │                                #   negativos con la serie histórica (áreas + marcadores + divisor «Hoy») vía mergeProjectionWithHistory.
│   │                                #   Overlay fino de cash-flow (v1.6.0): props cashflow/cashflowDaily/onRequestDailyCashflow — pinta la curva
│   │                                #   fina (fine.grid por month_fraction real, deflactada igual) sobre la zona pasada; daily lazy al hacer zoom histórico
│   ├── SettingsView.tsx          # AccountCard + sub-tabs como pills + ThemeToggle en "Datos y sistema"
│   ├── HistorySettingsPanel.tsx  # Ajustes → Histórico: filtros año/kind, tabla de snapshots, modal añadir/editar, borrar (backfill).
│   │                             #   Prefill: el modal crear autocompleta el grid vía GET /v1/history/snapshots/prefill (repuebla en
│   │                             #   silencio al cambiar fecha/kind si el grid no está «dirty»; «Recalcular» si lo está); editar ofrece
│   │                             #   «Añadir items que faltan» (append por item_id). Fallo de red → fila en blanco, modal usable.
│   └── AllocationRulesPanel.tsx  # used embedded inside BudgetView modal
│
└── auth/
    └── BootstrapInstallationPanel.tsx  # first-user setup form (currency + IANA tz)
```

> Para los **tokens, paleta y reglas visuales** del rediseño V1 consulta [`design-system.md`](design-system.md).

## Import conventions

- **`api/`** depends only on `api/` and the DOM `fetch`. No React.
- **`lib/`** is pure: no React, no fetch. May import from other `lib/*` and from `api/types`.
- **`components/`** may import from `lib/` and `api/types`. They are dumb presentational widgets.
- **`views/`** may import from anything below (`lib/`, `api/`, `components/`, other views). They own form/UI state via `useState` and receive data + mutation callbacks from `App.tsx`.
- **`App.tsx`** owns the long-lived state (installation, user, ledgerPersonScope, lists, busy flags, `projectionSeries`, `historySeries` **and `cashflowSeries`/`cashflowDaily`**) and the API mutation handlers. `historySeries` is loaded by `loadHistorySeries()` (parallel to the projection, in the projection-tab effect and after every snapshot mutation; failure → `null`, so the chart degrades to the current future-only view). `cashflowSeries` is loaded by `loadCashflowSeries()` alongside it (weekly, `window_months=24`); `loadCashflowDaily()` fetches the daily detail lazily (`window_months=6&resolution=daily`, once per scope/reload via `cashflowDailyRequestedRef`) when the chart zooms into the recent past. Same degrade-to-`null` contract as `historySeries`. Both refresh after transaction mutations (`onCashflowMutated`) and after snapshot mutations (they anchor the fine curve). `saveSnapshotNow(kinds)` POSTs a capture and reloads both history and cash-flow. Dispatch to a view is a `<XxxView {...props} />` call.

## Where to add new code

| New thing | Goes in |
|----|----|
| New API type returned by the backend | `api/types.ts` (export it) |
| New fetch endpoint wrapper | `api/client.ts` if reusable, otherwise inline in `App.tsx` next to existing handlers |
| New pure formatter / parser | `lib/format.ts` (with a Vitest in `lib/format.test.ts`) |
| New design token (color/radius/shadow) | `styles/theme.css` con variantes claro **y** `[data-theme="dark"]`. Nunca hardcoded en App.css/componentes. |
| New icon | extender el set en `components/icons.tsx` (viewBox 16×16, stroke 1.5). No crear SVG sueltos en views. |
| New shared chart/SVG widget | `components/charts/` — si es una proyección compacta, considera reusar `MiniProjection` con props |
| New full tab/page | `views/NewView.tsx` + add to `TABS` / `TAB_PATH` in `lib/navigation.ts` + render branch in `App.tsx` + add pill al `TopBar` (automático vía `TABS`) |
| New Settings sub-tab | add to `SETTINGS_SUBTAB_SLUG`/`_LABEL` in `lib/navigation.ts` + render branch inside `SettingsView` (sub-tabs son `ff-nav-pill` ya, no tab-bar) |
| Tabla nueva (o columnas nuevas en una existente) | seguir el patrón móvil «columnas esenciales»: gatear th/td con `useIsMobile()` (`lib/responsive.ts`), datos secundarios a `.cell-subline`, fila tappable → modal. Doctrina completa en design-system.md «Responsive / móvil». Controles densos dentro de la tabla → añadirlos al carve-out táctil de App.css (sección A2) |
| New auth/setup flow | `auth/` |

## Why this layout

- **`App.tsx` shrinks** to coordination only. Easy to reason about routing + global state.
- **Pure helpers in `lib/`** are testable in `node` (no DOM, no jsdom). Vitest runs them in ~30 ms.
- **Views are self-contained**: each one can be opened and understood without scrolling 10K lines.
- **Tests live next to code**: `format.test.ts` sits beside `format.ts`. The pattern scales — add helpers + tests together.
- **No circular deps**: `views/` import `lib/`, `lib/` doesn't import `views/`. Linter would catch it.

## Prefetching de views lazy

Tras autenticarse, confirmar `hasMembership` y **esperar a que termine la pestaña actual** (vía `currentTabBusy` derivado del `*Busy` correspondiente al `activeTab`), `App.tsx` ejecuta `prefetchOtherViews` dentro de un `requestIdleCallback` (con `setTimeout` de fallback). La función:

1. Itera una lista ordenada de tareas (`projection > assets > liabilities > budget > retirement > upcoming > settings`) **en serie con `await`**, no en paralelo, para no saturar ancho de banda ni CPU del API al inicio.
2. Por cada tarea: `await t.importChunk()` (calienta el chunk de Vite) → `await t.loadData?.()` (hidrata estado).
3. Excluye la pestaña actual (sus datos ya están en estado) y `summary` (su loader ya pre-fetcha `/v1/projection/series` en su propio `Promise.all`).
4. Recibe un `AbortSignal`: si el usuario hace logout durante el prefetch, se cancela. Un `prefetchedRef` evita que se vuelva a disparar tras navegar entre pestañas.

Los `useEffect[activeTab === "xxx"]` se mantienen como refresh-on-navigation tras mutaciones (no se eliminan). Si el prefetch ya pobló el estado, la navegación es instantánea y el refresh subsiguiente ocurre en background.

> No usamos `<link rel="modulepreload">` en `index.html` porque queremos que el prefetch ocurra **solo post-login**, no en la landing pre-auth.

### Chart grande aislado en su propio chunk

[ProjectionNetWorthChart](views/ProjectionNetWorthChart.tsx) está cargado con `React.lazy` **dentro** de [ProjectionView](views/ProjectionView.tsx). El `<Suspense fallback>` muestra `.ff-chart-skeleton` (placeholder con la altura del chart, sin animación) mientras se descarga el chunk y se calcula el `useMemo` inicial. El shell (subtítulo + milestones) aparece antes que el chart, sin layout shift.

`prefetchOtherViews` calienta ambos chunks (`ProjectionView` + `ProjectionNetWorthChart`) tras login, así que la primera entrada a la pestaña es instantánea.

En `App.tsx`, los tres setters que reciben `ProjectionSeriesApi` (`loadSummaryPage`, `loadProjectionSeriesPage`, `loadRetirementPage`) envuelven `setProjectionSeries(data)` en `startTransition()`. React marca el re-render del chart como de baja prioridad, dejando la UI responsiva a clics e inputs mientras reconcilia el SVG pesado.

## Debug del chart de Proyección (perf)

[apps/web/src/lib/perf.ts](apps/web/src/lib/perf.ts) expone `chartPerf` con `mark`/`measure`/`report`. Está apagado por defecto (early-return) y se activa de dos formas:

- **Por URL**: añadir `?perf=1` (p. ej. `http://127.0.0.1:8080/proyeccion?perf=1`). Solo dura mientras esté la query.
- **Persistente**: en la consola del navegador, `localStorage.setItem("debug:chart-perf","1")` + recarga. Para desactivar, `localStorage.removeItem("debug:chart-perf")`.

Cuando activo:
1. `App.tsx` marca `fetch-start` / `fetch-response` / `fetch-end` en cada loader que dispara `/v1/projection/series`.
2. `ProjectionNetWorthChart.tsx` marca `render-start`, los tres sub-memos (`baseSeries`, `xTicks`, `model`) y `first-commit` (post-render).
3. Tras el commit, un `useEffect` vuelca a la consola un `console.table` con las measures y `[chart:perf] total ≈ Xms`. Limpia las marks/measures después.
4. `main.tsx` registra un `PerformanceObserver({entryTypes:["longtask"]})` que avisa si algún task >50 ms bloquea el main thread.

Útil para responder "¿el cuello es el fetch, el JSON.parse, el memo o el paint?" sin tener que abrir el flame chart de Performance. Se mantiene en código como herramienta de diagnóstico — no añadir telemetría externa.

## What is NOT extracted (intentional)

- **API mutation handlers** (`submitAssetForm`, `deleteLiabilityRow`, etc.) stay in `App.tsx`. They close over `setAssets`, `setLiabilities`, etc. Moving them out requires a state library (Redux / Zustand / TanStack Query) — out of scope.
- **Auth gate flow** (login/register/pending screens) is inline in `App.tsx`. `BootstrapInstallationPanel` is extracted but the login/register form is small enough that splitting it adds ceremony.
- **FIRE client-side math** (`lib/fire.ts`) duplicates the Rust engine's tax/gross-up logic. Intentional: it powers the **live preview** of the FIRE settings form (user types `swr_pct`, sees the target update without a round-trip). If you change tax brackets server-side, mirror the change here.

## Frontend tests

See [`tests.md`](tests.md). Setup: Vitest + `node` environment (no jsdom needed for the current test set). All tests are in `*.test.ts` files colocated with the module they test.
