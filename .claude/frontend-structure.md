# Frontend Structure (`apps/web/src/`)

Post-refactor (May 2026). Before: one `App.tsx` of 10.384 LOC owning everything. After: composition root + per-concern modules.

```
src/
├── App.tsx                       # composition root: auth gate + global state + route → view dispatch
├── App.css                       # global styles (consume --ff-* tokens; no hardcoded hex)
├── index.css                     # minimal reset, font-family
├── main.tsx                      # ReactDOM.createRoot entry — imports styles/theme.css before index.css.
│                                 #   ADEMÁS: resuelve la ruta /oauth/authorize aquí (NO en App.tsx) → lazy
│                                 #   OAuthAuthorizeView en vez de <App/>. Ver §Ruta /oauth/authorize abajo
│
├── styles/
│   └── theme.css                 # design tokens (--ff-*, --proj-*) con variantes claro/[data-theme=dark]
│
├── api/
│   ├── client.ts                 # fetch wrappers: apiGet/Post/Put/Patch/Delete (+ apiDeleteJson para los DELETE con
│   │                             #   cuerpo, p. ej. /v1/transactions/{id}/reconcile) + defaultFetchInit + errorMessageFromResponse
│   │                             #   + **apiFetch** (4.0.0: envuelve `fetch` y traduce el TypeError del navegador a
│   │                             #   ApiRequestError{code:"network_error", status:0} — con la API caída la UI leía «Failed to fetch»)
│   │                             #   + **setUnauthorizedHandler** (4.0.0: un 401 en CUALQUIER llamada dispara el handler que
│   │                             #   registra App.tsx → setUser(null) → login. Antes solo `refreshSession` miraba el 401, y solo al
│   │                             #   arrancar: la cookie caducada con la pestaña abierta dejaba banners acumulándose sin salida.
│   │                             #   `status: 0` NO lo dispara — un corte de red no es una sesión caducada)
│   ├── client.test.ts            # mocks `globalThis.fetch`, asserts credentials/Content-Type/204
│   └── types.ts                  # all *Api / *Response / *Row types (mirror of Rust handler structs)
│
├── lib/                          # pure helpers, no React imports
│   ├── format.ts                 # money/percent/decimal formatting (es-ES locale), parseDisplayDecimal, METRIC_DASH,
│   │                             #   **toApiDecimalString** (normaliza es-ES → decimal de la API) y **DecimalInputError**.
│   │                             #   TODO importe tecleado pasa por ahí; ver la nota «Importes tecleados» abajo
│   ├── format.test.ts            # (cuenta: `grep -c 'it('`)
│   ├── dates.ts                  # civil-calendar arithmetic (parallel to crates/engine), TZ-aware "today", interval counts
│   ├── dates.test.ts             # 29 tests (incl. formatDateDm)
│   ├── ledger.ts                 # shared by views: ledgerViewQs, groupRowsByCategoryOrdered, asset/liability portfolio helpers,
│   │                             #   PAYMENT_FREQ_LABEL, formatProjectionMilestoneCompactLabel, budgetCategoryMap,
│   │                             #   sortBudgetEntriesMacStyle, formatAxisMoney, LedgerPersonScope, LiabilityPaymentFreq
│   ├── fire.ts                   # client-side FIRE math for the live form preview (mirror of handlers/projection.rs):
│   │                             #   defaultFireSettingsApi, normalizeInstallationFireSettings, taxOnGrossCapitalAnnual,
│   │                             #   grossUpNetAnnualFire, computeFireAnnualNeedNetEur, findFirstMonthNetWorthAtLeastInflated
│   ├── projection-chart.ts       # chart helpers: tick builders (startMonth param → soporta meses negativos), SVG layout,
│   │                             #   **lastPointIndexAtOrBeforeMonth** (mes → posición en `points`; OBLIGATORIO para recortar una
│   │                             #   ventana por mes: con density=hybrid la posición 13 es el mes 24 y `Math.min(mes, len-1)` no
│   │                             #   recortaba nada — ver la nota «Índice de array ≠ mes» abajo),
│   │                             #   niceYTicks, axis age/dates mode, deflationFactorAt (deflactor keyed por month_index; k<0 amplifica),
│   │                             #   PROJECTION_FOCUS_STORAGE_KEY, ASSET_LINE_COLORS (CSS vars), complementaryProjectionTickLabel,
│   │                             #   projectionHoverTitle, formatYearsEsFromMonths, formatProjectionChartHorizonLine
│   ├── chart-legend.ts           # modelo PURO de la leyenda de charts (ChartLegend): buildStructuralLegendItems,
│   │                             #   legendOrderByPeakDesc (leyenda peak DESC conservando el colorIndex de pintado),
│   │                             #   buildAssetLegendItems (sufijo «Nombre · owner» en duplicados de la vista hogar;
│   │                             #   las series solo-históricas ni sufijan ni vetan), assetOwnerNameById (join
│   │                             #   /v1/assets + /v1/installation/members; null = actual sin owner resoluble),
│   │                             #   collapsedAssetLegendCap / applyLegendCollapse (chip «+N más»; nunca esconde
│   │                             #   uno solo), topAssetTooltipRows (top-5 por |valor| + «Otros»). Test: chart-legend.test.ts
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
│   │                             #   TxnSortDir; importe por |magnitud|, tiebreak estable), groupTransactionsByCategory/sortTransactionGroups (orden fijo: kind → |subtotal| desc; el subtotal
│   │                             #   EXCLUYE las conciliadas, que sí siguen en rows — si no, divergiría de la comparativa del servidor en la misma pantalla) e isReconciled (3.5.0: fuente de
│   │                             #   verdad = transfer_counterpart_id presente). Test: expenses.test.ts
│   ├── files.ts                  # readFileAsBase64(File): base64 en trozos de 32 KiB. Compartido por el import .ffbackup (App.tsx) y el wizard de CSV
│   ├── responsive.ts             # MOBILE_MAX_WIDTH (640 = bp:mobile), isMobileWidth (puro, test en node) y useIsMobile()
│   │                             #   (matchMedia, lectura síncrona inicial). Gatea el patrón «columnas esenciales» de TODAS las
│   │                             #   tablas en móvil: mismo boolean para th y td (desincronización imposible), fila tappable →
│   │                             #   modal de edición. Desktop byte-idéntico con isMobile=false. Test: responsive.test.ts
│   ├── chart-gestures.ts         # aritmética PURA de los gestos táctiles del chart grande: clampWindowStart, panWindow,
│   │                             #   pinchWindow (+ ChartDomain). Espejo exacto de los clamps/ancla del onWheel — test de
│   │                             #   equivalencia en chart-gestures.test.ts. ProjectionNetWorthChart la consume desde su
│   │                             #   máquina de gestos Pointer Events (touch-action: pan-y; vertical = scroll de página)
│   ├── theme.ts                  # ThemePref ("auto"|"light"|"dark") + apply/load/save + subscribeSystemThemeChanges
│   └── oauth.ts                  # helpers PUROS de la pantalla de consentimiento (v3.1.0): AuthorizeParams,
│                                 #   parseAuthorizeParams (null si falta cualquiera de los 5 params obligatorios;
│                                 #   `code_challenge_method=plain` SÍ parsea — rechazarlo es del servidor),
│                                 #   redirectHostLabel, authorizeErrorMessage (8 códigos → copy es-ES).
│                                 #   Test: oauth.test.ts (8 casos)
│
├── components/                   # generic UI primitives (no domain knowledge)
│   ├── TopBar.tsx                # cabecera única: marca + nav pills + extras + hamburguesa
│   ├── MobileNavDrawer.tsx       # drawer derecho ≤720px
│   ├── AccountCard.tsx           # tarjeta de cuenta en Ajustes (sustituye user-chip + Salir del header antiguo)
│   ├── ThemeToggle.tsx           # segmented Auto/Claro/Oscuro (usado en Ajustes → General → Apariencia)
│   ├── Switch.tsx                # switch accesible track+thumb (`.ff-switch*`); variant="chart" = label small-caps (Proyección); usado también en Ajustes → Integraciones
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
│       ├── MiniProjection.tsx    # SVG compacto reusado en Resumen y Jubilación
│       └── ChartLegend.tsx       # leyenda compartida (chart grande + minis): HTML fuera del SVG, estructurales
│                                 #   siempre visibles + activos truncados con chip «+N más» (modelo en lib/chart-legend.ts)
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
│   │                             #   borrado con dos opciones (solo este / y detener repetición) + **conciliación de transferencias** (3.5.0): badge «conciliada» (`.exp-reconciled-tag`,
│   │                             #   tooltip con la contrapartida) y fila atenuada (`.exp-row-reconciled`) cuando `isReconciled`, y «Desconciliar» en el modal
│   │                             #   de edición (DELETE /v1/transactions/{id}/reconcile) → `handleMutated`. **NO hay botón «Conciliar ahora»**: se retiró al
│   │                             #   añadir el barrido periódico del servidor (`FUTUREFIN_RECONCILE_SWEEP_HOURS`) + el pase post-import; `POST /v1/transactions/reconcile`
│   │                             #   sigue existiendo en la API y como tool MCP, pero la SPA ya no lo llama.
│   │                             #   Materializa recurrentes en silencio al montar (solo canEdit). `onCashflowMutated` avisa a App.
│   ├── ImportWizardModal.tsx     # wizard import CSV en 2 pasos (useReducer). Paso 1 = archivo → select «Cuenta origen (activo)» (movido desde el footer; ahora también en el preview) →
│   │                             #   formato en <details> plegado (autodetección por defecto). Paso 2 = banner con fuente capitalizada + chips de conteos, bulk bar con cluster único
│   │                             #   «Asignar a visibles», footer «{X} se importarán · {Y} excluidas ({Z} duplicadas)», columna «Tipo». /import/confirm con decisions[] paralelo. Stateless (sha256).
│   │                             #   3.5.0: las «posibles transferencias» ya NO se atenúan ni se desmarcan (entran incluidas; la exclusión del gasto la hace la conciliación) — solo dup/divisa;
│   │                             #   el aviso post-confirm (con `reconciled_pairs`) lo pinta GastosView en su callback `onImported`
│   ├── ManualCashEntryModal.tsx  # alta manual de efectivo: grid multifila (magnitud + kind fija el signo) + checkbox «Repetir cada mes» por fila (→ recurrence:{}) → POST /v1/transactions/batch
│   ├── RecurringRulesModal.tsx   # modal «Recurrentes» (botón en la toolbar de Movimientos): lista GET /v1/transactions/recurring y permite «Detener» (DELETE) cada regla
│   │                             #   (conserva las instancias ya materializadas). Patrón ManualCashEntryModal: fetch al abrir, toda la lógica de presentación aquí (nada en lib/)
│   ├── UpcomingView.tsx          # Planning
│   ├── RetirementView.tsx        # KPIs + MiniProjection (zoomY, clampToMonth=jub+12, xAxis) + FIRE config
│   ├── ProjectionView.tsx        # wraps ProjectionNetWorthChart
│   ├── ProjectionNetWorthChart.tsx  # gran SVG chart, drag/zoom/hover, colores vía --proj-* tokens; se extiende a meses
│   │                                #   negativos con la serie histórica (áreas + marcadores + divisor «Hoy») vía mergeProjectionWithHistory.
│   │                                #   Overlay fino de cash-flow (v1.6.0): props cashflow/cashflowDaily/onRequestDailyCashflow — pinta la curva
│   │                                #   fina (fine.grid por month_fraction real, deflactada igual) sobre la zona pasada; daily lazy al hacer zoom histórico.
│   │                                #   Leyenda (4.0.6): HTML fuera del SVG (ChartLegend); el ResizeObserver mide .projection-chart-plot
│   │                                #   (solo el SVG) y el viewBox casa EXACTO con la caja medida (los 38px de etiquetas X rotadas salen
│   │                                #   de ph, no de lienzo extra — si no, `meet` encoge el dibujo con bandas laterales). Tooltip: top-5
│   │                                #   activos por |valor| + «Otros (k)». Prop assetOwnerNames (App.tsx) desambigua duplicados en hogar
│   ├── SettingsView.tsx          # AccountCard + sub-tabs como pills («Usuarios» owner-only, «MCP» con tokens/conexiones/toggle de escritura) + ThemeToggle en "Datos y sistema"
│   ├── ApiTokensPanel.tsx        # Ajustes → Integraciones: tokens de API (MCP). Self-fetch (patrón HistorySettingsPanel); crear (modal
│   │                             #   label + caducidad), secreto mostrado UNA vez con copiar, tabla (prefix/último uso/vigencia),
│   │                             #   revocar con modal de confirmación. Visible para cualquier miembro (v3.0.0).
│   ├── OAuthConnectionsPanel.tsx # Ajustes → Integraciones, sección «Conexiones», justo debajo de ApiTokensPanel (v3.1.0). Calco del
│   │                             #   patrón ApiTokensPanel: sin props, self-fetch GET /v1/oauth/connections; tabla
│   │                             #   Aplicación (client_name + host verificado) / Conectada / Último uso; revocar =
│   │                             #   DELETE /v1/oauth/connections/{id} tras Modal de confirmación → corte inmediato.
│   │                             #   Sigue disponible con FUTUREFIN_MCP_ENABLED=0 (el endpoint se monta siempre)
│   ├── OAuthAuthorizeView.tsx    # pantalla de consentimiento OAuth (v3.1.0), montada desde main.tsx, NO desde App.tsx.
│   │                             #   Autónoma: aplica el tema ella misma (applyTheme/loadThemePref) e importa App.css,
│   │                             #   porque App.tsx nunca monta. Máquina de fases: loading → disabled (404 = kill-switch)
│   │                             #   | invalid (error FATAL: pinta y muere, JAMÁS redirige) | redirecting (error
│   │                             #   redirigible → location.replace) | login (401 → LoginPanel) | consent → submitting
│   │                             #   → pending (403) | error. Endpoints: GET /v1/oauth/authorize-details, GET /v1/auth/me,
│   │                             #   POST /v1/oauth/authorize, POST /v1/auth/logout («Cambiar de usuario»).
│   │                             #   **El redirect final lo construye el SERVIDOR** (`redirect_to`); el cliente nunca
│   │                             #   concatena code/state. Aprobar y cancelar usan el mismo POST (`approve: bool`)
│   ├── HistorySettingsPanel.tsx  # Ajustes → Histórico: filtros año/kind, tabla de snapshots, modal añadir/editar, borrar (backfill).
│   │                             #   Prefill: el modal crear autocompleta el grid vía GET /v1/history/snapshots/prefill (repuebla en
│   │                             #   silencio al cambiar fecha/kind si el grid no está «dirty»; «Recalcular» si lo está); editar ofrece
│   │                             #   «Añadir items que faltan» (append por item_id). Fallo de red → fila en blanco, modal usable.
│   └── AllocationRulesPanel.tsx  # used embedded inside BudgetView modal
│
└── auth/
    ├── BootstrapInstallationPanel.tsx  # first-user setup form (currency + IANA tz)
    └── LoginPanel.tsx                  # panel de login AUTOCONTENIDO (v3.1.0): props {intro?, onAuthenticated},
                                        #   POST /v1/auth/login y ya. Sin modo registro (en una instalación ya
                                        #   creada el owner aprueba desde Ajustes), sin logout ni refresh.
                                        #   Lo consume solo OAuthAuthorizeView
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
| **Campo de formulario que envía un importe/porcentaje** | `toApiDecimalString(raw)` de `lib/format.ts`, DENTRO del `try` del submit. Ver §Importes tecleados |
| **Recortar una serie de proyección por un mes** | `lastPointIndexAtOrBeforeMonth(points, mes)` de `lib/projection-chart.ts`. Ver §Índice de array ≠ mes |
| New design token (color/radius/shadow) | `styles/theme.css` con variantes claro **y** `[data-theme="dark"]`. Nunca hardcoded en App.css/componentes. |
| New icon | extender el set en `components/icons.tsx` (viewBox 16×16, stroke 1.5). No crear SVG sueltos en views. |
| New shared chart/SVG widget | `components/charts/` — si es una proyección compacta, considera reusar `MiniProjection` con props |
| New full tab/page | `views/NewView.tsx` + add to `TABS` / `TAB_PATH` in `lib/navigation.ts` + render branch in `App.tsx` + add pill al `TopBar` (automático vía `TABS`) |
| New Settings sub-tab | add to `SettingsSubTabId` + `SETTINGS_SUBTAB_SLUG`/`_LABEL` in `lib/navigation.ts` (con test en `navigation.test.ts`), visibilidad en `visibleSettingsSubTabs` (App.tsx) + render branch inside `SettingsView` (sub-tabs son `ff-nav-pill` ya, no tab-bar). Precedente completo: la sub-tab `integrations` (tokens + conexiones + toggle de escritura; «access» quedó owner-only renombrada «Usuarios», slug `acceso` intacto) |

> **Los nombres de las sub-pestañas de Ajustes cambiaron en 3.10.0** y la fuente de verdad es
> `SETTINGS_SUBTAB_LABEL` (`lib/navigation.ts`): hoy son **General, Plan, Categorías, Histórico,
> Usuarios, Integraciones, Copias de seguridad**. Al citarlas en un doc o en copy, cítalas de ahí.
> Los slugs viejos siguen resolviendo (`/ajustes/mcp` → `integrations`, `/ajustes/proyeccion` y
> `/ajustes/jubilacion` → `plan`, `/ajustes/acceso` → `access`), fijado en `navigation.test.ts`.
| Tabla nueva (o columnas nuevas en una existente) | seguir el patrón móvil «columnas esenciales»: gatear th/td con `useIsMobile()` (`lib/responsive.ts`), datos secundarios a `.cell-subline`, fila tappable → modal. Doctrina completa en design-system.md «Responsive / móvil». Controles densos dentro de la tabla → añadirlos al carve-out táctil de App.css (sección A2) |
| New auth/setup flow | `auth/` |
| New **standalone page outside the tab router** (like `/oauth/authorize`) | `main.tsx`: rama lazy antes de `<App/>`. Ver §Ruta `/oauth/authorize` — el router de `App.tsx` canonicaliza cualquier path desconocido |

## Why this layout

- **`App.tsx` shrinks** to coordination only. Easy to reason about routing + global state.
- **Pure helpers in `lib/`** are testable in `node` (no DOM, no jsdom). Vitest runs them in ~30 ms.
- **Views are self-contained**: each one can be opened and understood without scrolling 10K lines.
- **Tests live next to code**: `format.test.ts` sits beside `format.ts`. The pattern scales — add helpers + tests together.
- **No circular deps**: `views/` import `lib/`, `lib/` doesn't import `views/`. Linter would catch it.

## Importes tecleados: `toApiDecimalString` es obligatorio (4.0.0)

Todo campo que mande un importe o un porcentaje al backend pasa por
`toApiDecimalString(raw)` (`lib/format.ts`). No conviertas a mano.

- **El incidente**: la conversión era `raw.replace(",", ".")` — solo la primera coma, el punto sin
  tocar. `250.000` (doscientos cincuenta mil, escritura española normal) llegaba tal cual y
  `Decimal::from_str` lo lee como **250**. Sin error: el modal se cerraba y el patrimonio, la
  proyección, el número FIRE y el runway quedaban mal en silencio, tres órdenes de magnitud. El
  asistente de primera vez llegaba a invitar a hacerlo: su placeholder era literalmente `1.500`.
- **Reglas** (en orden): con coma, la coma es el decimal y los puntos son miles; sin coma, puntos
  que separan grupos de exactamente 3 dígitos son miles; un punto suelto que no forma grupo es el
  decimal (así se teclean los porcentajes); **cualquier otra cosa lanza `DecimalInputError`**.
  Rechazar lo ambiguo en vez de adivinar es el punto — adivinar fue el fallo.
- **Llámala DENTRO del `try` del submit.** Cuatro submits convertían antes de su `try`, así que la
  excepción se les escapaba como promesa rechazada y no pintaba nada. El patrón es capturar
  `DecimalInputError` y traducirla al error de la vista.

## Índice de array ≠ mes: `lastPointIndexAtOrBeforeMonth` (4.0.0)

Con `?density=hybrid` el servidor **decima** la serie (meses 0..12, 24, 36, … y el último del
horizonte), así que la posición 13 de `points` es el mes 24 y `points.length` (~82) **no** es el
número de meses. Todo lo que recorte una ventana por un MES tiene que traducir mes → posición con
`lastPointIndexAtOrBeforeMonth(points, mes)`, nunca con `Math.min(mes, len-1)` — que en `hybrid` no
recortaba nada.

Lo que se rompió por no hacerlo: `AssetsView` calculaba «objetivo alcanzado en dic 2027» donde la
proyección lo alcanza en 2031, y `MiniProjection` rotulaba el eje con años que no correspondían a
su serie. Con `density=monthly` la salida es idéntica, que es justo por lo que pasa desapercibido.
Es la misma clase de fallo que el incidente v1.4.2 de la deflactación del chart.

## Ruta `/oauth/authorize` — resuelta en `main.tsx`, no en el router de `App.tsx` (v3.1.0)

La pantalla de consentimiento OAuth es la única vista que **no** cuelga del router de `App.tsx`. La
decisión se toma en `main.tsx`, a nivel de módulo, antes de que React renderice:

```tsx
const OAuthAuthorizeView = lazy(() =>
  import("./views/OAuthAuthorizeView").then((m) => ({ default: m.OAuthAuthorizeView })),
);
const isOAuthAuthorize =
  window.location.pathname.replace(/\/+$/, "") === "/oauth/authorize";
// …
{isOAuthAuthorize ? (
  <Suspense fallback={null}><OAuthAuthorizeView /></Suspense>
) : (
  <App />
)}
```

- **Match exacto** (tras quitar barras finales), no por prefijo. En esa ruta `<App/>` **no se monta
  en absoluto** — de ahí que la vista aplique el tema por su cuenta e importe `App.css`.
- **Chunk lazy**: `React.lazy` + `import()`, así que el bundle principal no carga la pantalla de
  consentimiento para el 99,9 % de las visitas. `Suspense fallback={null}` (pantalla en blanco
  mientras baja el chunk, que es diminuto). Nada de esto pasa por `prefetchOtherViews`: no es una
  pestaña y no se llega a ella navegando.
- **Por qué NO puede vivir en el router de `App.tsx`**: su `useLayoutEffect` de canonicalización
  reescribe cualquier path que no reconozca —
  ```tsx
  if (tabFromPathname(pathname) === null) { navigate("/resumen", true); return; }
  ```
  y `navigate` hace `window.history.replaceState(null, "", "/resumen")`, **una URL sin query
  string**. `tabFromPathname` (`lib/navigation.ts`) devuelve `null` para `/oauth/authorize`, así que
  el efecto destruiría `client_id`, `redirect_uri`, `code_challenge` y `state` de forma
  irrecuperable — y al ser un `useLayoutEffect`, antes del primer paint. Registrar la ruta en `TABS`
  tampoco sirve: no es una pestaña. **`App.tsx` queda literalmente intacto** (cero menciones a
  `oauth`), que era el objetivo.
- Simetría con el backend: la ruta **tampoco** existe en el API — la sirve el fallback SPA. Ver
  [`api-routes.md`](api-routes.md) §OAuth 2.1 y la prohibición del proxy `"/oauth"` a secas en
  [`env-and-config.md`](env-and-config.md) §Vite config.
- `auth/LoginPanel.tsx` es un panel de login **duplicado a propósito** (plan B autorizado), no una
  extracción del formulario de `App.tsx`: el estado de auth de `App.tsx` está entrelazado con
  logout/refresh y moverlo arriesgaba regresión sin cambiar nada observable. Si algún día `App.tsx`
  adelgaza, ese panel es el punto de aterrizaje natural.

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
- **Auth gate flow** (login/register/pending screens) is inline in `App.tsx`. `BootstrapInstallationPanel` is extracted but the login/register form is small enough that splitting it adds ceremony. v3.1.0 needed a login form **outside** `App.tsx` (the OAuth consent screen) and deliberately **duplicated** it as `auth/LoginPanel.tsx` instead of extracting the original — see §Ruta `/oauth/authorize`.
- **FIRE client-side math** (`lib/fire.ts`) duplicates the Rust engine's tax/gross-up logic. Intentional: it powers the **live preview** of the FIRE settings form (user types `swr_pct`, sees the target update without a round-trip). If you change tax brackets server-side, mirror the change here.

## Frontend tests

See [`tests.md`](tests.md). Setup: Vitest + `node` environment (no jsdom needed for the current test set). All tests are in `*.test.ts` files colocated with the module they test.
