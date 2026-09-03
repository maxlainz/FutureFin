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
│   └── types.ts                  # all *Api / *Response / *Row types (mirror of Rust handler structs).
│                                 #   5.0.0 (D13, issue #207): `FireSettingsApi` (supuestos del HOGAR — impuestos,
│                                 #   ventanas del promedio, plusvalía gravable; sigue siendo owner-only) PIERDE los
│                                 #   cuatro ejes personales `fire_number_mode`/`fire_number_manual_amount`/`swr_pct`/
│                                 #   `horizon_lifespan_age`, que pasan a `RetirementProfileApi` (espejo exacto de
│                                 #   `RetirementProfile`, `apps/api/src/handlers/retirement_profile.rs`; editable por
│                                 #   CUALQUIER rol, es dato del propio usuario, no owner-only) — más
│                                 #   `RetirementStrategyApi`/`TargetBasisApi`/`BridgeDiscountBasisApi`/
│                                 #   `WithdrawalRuleKindApi`/`SpendModeApi`/`PartialExpenseBasisApi`/
│                                 #   `WithdrawalRuleApi`/`PensionPlanApi`/`PartialRetirementApi`. La respuesta de
│                                 #   `GET|PATCH /v1/auth/me/retirement-profile` es `RetirementProfileResponseApi
│                                 #   {profile, birth_date}` (mismo PATCH acepta `birth_date`); el cuerpo del PATCH es
│                                 #   `RetirementProfilePatchApi`, tri-estado (clave ausente = no cambia, `null` borra
│                                 #   lo opcional). También `AssetApiRow.annual_volatility_percent?: string | null`
│                                 #   (§A.2 — desviación típica anual; solo alimenta Monte Carlo, WP6).
│                                 #   WP5-2 añade tres piezas de contrato: `RetirementProfileResponseApi
│                                 #   .target_basis_stored` (la elección ALMACENADA; `null` = el servidor la DERIVA,
│                                 #   ausente = backend antiguo — sin ella el cliente no distingue «no lo he elegido»
│                                 #   de «he elegido esto» y congela la derivación al reenviarla);
│                                 #   `HouseholdMemberProjectionApi.series` (`MemberSeriesPointApi[]` —
│                                 #   {month_index, net_worth, net_worth_liquid}, misma rejilla y misma decimación
│                                 #   que `points[]`) + `.horizon_months` (el horizonte PROPIO del miembro, que puede
│                                 #   ser menor que `months`: ahí termina su línea); y `AssetWriteBodyApi`, el cuerpo
│                                 #   de POST/PATCH de activo con sus tres campos tri-estado, donde `undefined` y
│                                 #   `null` significan cosas DISTINTAS a propósito.
│
├── lib/                          # pure helpers, no React imports
│   ├── format.ts                 # money/percent/decimal formatting (es-ES locale), parseDisplayDecimal, METRIC_DASH,
│   │                             #   **toApiDecimalString** (normaliza es-ES → decimal de la API) y **DecimalInputError**.
│   │                             #   TODO importe tecleado pasa por ahí; ver la nota «Importes tecleados» abajo
│   ├── format.test.ts            # (cuenta: `grep -c 'it('`)
│   ├── dates.ts                  # civil-calendar arithmetic (parallel to crates/engine), TZ-aware "today", interval counts
│   ├── dates.test.ts             # 32 tests (incl. formatDateDm)
│   ├── ledger.ts                 # shared by views: ledgerViewQs, groupRowsByCategoryOrdered, asset/liability portfolio helpers,
│   │                             #   PAYMENT_FREQ_LABEL, formatProjectionMilestoneCompactLabel, budgetCategoryMap,
│   │                             #   sortBudgetEntriesMacStyle, formatAxisMoney, LedgerPersonScope, LiabilityPaymentFreq
│   │                             #   4.2.0: REPAYMENT_MODEL_LABEL/ORDER + liabilityDerivedPrincipalNum /
│   │                             #   liabilityDerivedPrincipalPreview (Σ cuotas en fixed_payments, valor actual al TIN
│   │                             #   en french; devuelve null en todo estado que el POST rechazaría con 400)
│   │                             #   5.0.0 (D9/D32, issue #207): LEDGER_PERSON_SCOPE_STORAGE_KEY (la clave de
│   │                             #   localStorage, antes vivía inline en App.tsx) + resolveLedgerPersonScope
│   │                             #   (localStorage → scope inicial; ausente/vacío/desconocido ⇒ `mine`, el nuevo
│   │                             #   default — antes caía a `household`) + isScopeReadOnly (`household` ⇒ true; es
│   │                             #   UX, no la frontera: el servidor rechaza igual con 403 `not_row_owner`) +
│   │                             #   ledgerViewAmp (mismo `?view=` para encadenar tras otro parámetro). `ledgerViewQs`
│   │                             #   cambia de contrato: los DOS scopes viajan ahora `?view=` EXPLÍCITO (antes
│   │                             #   `household` era el string vacío) — con el default del API pasando a `mine`
│   │                             #   (`LedgerViewQuery::resolve`), omitir el parámetro en Hogar devolvería solo lo
│   │                             #   propio, sin ningún error.
│   ├── ledger.repayment-model.test.ts  # 4.2.0 — paridad con apps/api/tests/fixtures/liability-derived-principal-parity.json
│   │                             #   (patrón fire-parity: mismo JSON leído por el test Rust y por Vitest) + puertas del preview
│   ├── ledger.scope.test.ts      # 5.0.0 — los cuatro helpers de scope de arriba (`grep -c 'it(' apps/web/src/lib/ledger.scope.test.ts`)
│   ├── retirement-intro.ts       # 5.0.0 (D33, issue #207): aviso de alta «Elige tu estrategia de jubilación» en
│   │                             #   Jubilación, enseñado UNA vez por navegador. RETIREMENT_INTRO_DISMISSED_STORAGE_KEY
│   │                             #   + isRetirementIntroDismissed (puro) + readRetirementIntroDismissed /
│   │                             #   persistRetirementIntroDismissed, tolerantes a un localStorage que lanza (Safari
│   │                             #   privado, el iframe del Ingress de Home Assistant): sesgo elegido, si no se puede
│   │                             #   leer el aviso SE ENSEÑA (mejor un clic de más que ocultar para siempre que la
│   │                             #   estrategia es una elección). Test: retirement-intro.test.ts
│   ├── retirementProfile.ts      # 5.0.0 (D13, issue #207): perfil de jubilación POR USUARIO en cliente. Tres
│   │                             #   responsabilidades y ninguna más — (1) defaults/clamps en LECTURA
│   │                             #   (normalizeRetirementProfile/normalizeWithdrawalRule), espejo de
│   │                             #   `resolve_retirement_profile` (`apps/api/src/handlers/retirement_profile.rs`);
│   │                             #   (2) guarda de validez en ESCRITURA (retirementProfileIssue/withdrawalRuleIssue),
│   │                             #   MISMOS códigos estables que el servidor, para que el autosave nunca prometa
│   │                             #   «Guardado automático» sobre un valor que el PATCH iba a rechazar con 400; (3)
│   │                             #   PATCH MÍNIMO tri-estado (buildRetirementProfilePatch/isEmptyRetirementProfilePatch)
│   │                             #   — mandar el perfil entero resetearía en silencio lo que el usuario no tocó.
│   │                             #   WP5-2/WP7-3b: **withStoredTargetBasis** (sustituye el `target_basis` RESUELTO
│   │                             #   que publica el servidor por la elección ALMACENADA, `target_basis_stored`;
│   │                             #   `undefined` = backend antiguo ⇒ no toca nada) y **targetBasisSource** →
│   │                             #   `stored | derived | forced_by_strategy`. Sin la sustitución, el patch mínimo
│   │                             #   no puede distinguir «el servidor derivó perpetuity» de «el usuario eligió
│   │                             #   perpetuity»: la fijación explícita no se mandaba nunca, y el radio marcado se
│   │                             #   leía como una decisión tomada. Lo que se PINTA sigue saliendo de
│   │                             #   effectiveTargetBasis, que deriva con la misma regla R6 que Rust.
│   │                             #   Además: las 5 estrategias (RETIREMENT_STRATEGIES + _LABEL/_BLURB, nombres D33) y
│   │                             #   las cotas del formulario, DUPLICADAS a propósito contra `retirement_profile.rs`
│   │                             #   §Cotas (MIN_PROFILE_AGE, MAX_WITHDRAWAL_PCT, MAX_GUARDRAIL_PCT,
│   │                             #   MAX_CASH_BUFFER_MONTHS, MIN/MAX_SUCCESS_THRESHOLD_PCT, MAX_SWR_PCT,
│   │                             #   MIN/MAX_HORIZON_LIFESPAN_AGE — la cota del SWR/horizonte no cambió al moverse de
│   │                             #   `fire.ts`, solo el dueño del dato). Test: retirementProfile.test.ts
│   │                             #   (`grep -c 'it(' apps/web/src/lib/retirementProfile.test.ts`; recorre la tabla de
│   │                             #   cotas entera para que una divergencia con Rust sea un test rojo, no un 400 en
│   │                             #   producción)
│   ├── fire.ts                   # client-side FIRE math for the live form preview (mirror of handlers/projection.rs):
│   │                             #   defaultFireSettingsApi, normalizeInstallationFireSettings, taxOnGrossCapitalAnnual,
│   │                             #   grossUpNetAnnualFire, computeFireAnnualNeedNetEur, findFirstMonthNetWorthAtLeastInflated
│   │                             #   5.0.0 (D13): `fire_number_mode`/`fire_number_manual_amount`/`swr_pct`/
│   │                             #   `horizon_lifespan_age` SALIERON de `FireSettingsApi` — son personales, viven en
│   │                             #   `RetirementProfileApi` (`lib/retirementProfile.ts` de arriba) — así que
│   │                             #   `defaultFireSettingsApi`/`normalizeInstallationFireSettings` ya no los tocan;
│   │                             #   `runwaySwrParenthetical` pasó de leer `FireSettingsApi` a leer
│   │                             #   `RetirementProfileApi` (es el SWR que enseña el paréntesis de la tarjeta
│   │                             #   Autonomía cuando el runway es indefinido)
│   ├── projection-chart.ts       # chart helpers: tick builders (startMonth param → soporta meses negativos), SVG layout,
│   │                             #   **lastPointIndexAtOrBeforeMonth** (mes → posición en `points`; OBLIGATORIO para recortar una
│   │                             #   ventana por mes: con density=hybrid la posición 13 es el mes 24 y `Math.min(mes, len-1)` no
│   │                             #   recortaba nada — ver la nota «Índice de array ≠ mes» abajo),
│   │                             #   niceYTicks, axis age/dates mode, deflationFactorAt (deflactor keyed por month_index; k<0 amplifica),
│   │                             #   PROJECTION_FOCUS_STORAGE_KEY, ASSET_LINE_COLORS (CSS vars), complementaryProjectionTickLabel,
│   │                             #   projectionHoverTitle, formatYearsEsFromMonths, formatProjectionChartHorizonLine
│   ├── cashflow-bars.ts          # geometría PURA del cash-flow mensual (4.15.0): buildCashflowColumns → Ingresos /
│   │                             #   Gastos / déficit / invertido / en cuenta / de reservas + scale; MonthlyCashflowBars solo pinta.
│   │                             #   Invariantes de alturas en cashflow-bars.test.ts
│   ├── chart-legend.ts           # modelo PURO de la leyenda de charts (ChartLegend): buildStructuralLegendItems,
│   │                             #   legendOrderByPeakDesc (leyenda peak DESC conservando el colorIndex de pintado),
│   │                             #   buildAssetLegendItems (sufijo «Nombre · owner» en duplicados de la vista hogar;
│   │                             #   las series solo-históricas ni sufijan ni vetan), assetOwnerNameById (join
│   │                             #   /v1/assets + /v1/installation/members; null = actual sin owner resoluble),
│   │                             #   collapsedAssetLegendCap / applyLegendCollapse (chip «+N más»; nunca esconde
│   │                             #   uno solo), topAssetTooltipRows (top-5 por |valor| + «Otros»). Test: chart-legend.test.ts
│   │                             #   5.0.0 (D32): **householdMemberColor(idx)** — color de un miembro del hogar por
│   │                             #   su POSICIÓN en `members[]`, ÚNICA definición del emparejamiento entre su línea
│   │                             #   fina del chart, su tick de la tira de fases y su entrada de leyenda. Reusa
│   │                             #   ASSET_LINE_COLORS. Cuando cada superficie lo calculaba por su cuenta, bastaba
│   │                             #   con que una ordenara distinto para que la leyenda nombrara la curva de otro
│   ├── member-lines.ts           # 5.0.0 (D32, issue #207): preparación PURA de las «líneas finas por miembro» del
│   │                             #   chart en vista Hogar — buildHouseholdMemberLines (members[].series → vértices
│   │                             #   {month_index, value} ya DEFLACTADOS con el factor que pasa el chart, recortados
│   │                             #   al `horizon_months` PROPIO de cada miembro y con su color) y memberValueAtMonth
│   │                             #   (el valor que lee el tooltip). **Todo en MESES**, nunca por posición: la serie
│   │                             #   viene decimada como `points[]`. Dos reglas con consecuencia visible — la línea
│   │                             #   TERMINA en el horizonte propio (nunca se extrapola: más allá esa persona no
│   │                             #   declaró vivir) y memberValueAtMonth devuelve `null` en los dos extremos, así
│   │                             #   que un miembro cuya curva ya acabó desaparece del tooltip en vez de repetir su
│   │                             #   último importe. Solo `net_worth`: el líquido viaja pero NO se dibuja.
│   │                             #   Test: member-lines.test.ts
│   ├── phase-strip.ts            # 5.0.0 (D29, issue #207): geometría PURA de la tira de fases del chart —
│   │                             #   buildPhaseSegments (phase_transitions → tramos contiguos recortados a la ventana,
│   │                             #   con transitionMonth = el mes REAL del corte aunque la ventana lo tape),
│   │                             #   phaseAtMonth, buildPhaseMarks (pensión = flecha; «Cruce» SOLO con
│   │                             #   retirement_trigger === "target_age" y cruce ≠ jubilación efectiva; un tick por
│   │                             #   miembro en Hogar, coloreado con `householdMemberColor`) y
│   │                             #   buildHouseholdMemberLegendItems (`swatch: "line"` desde WP7-3b: cada miembro
│   │                             #   tiene ya su polyline fina, así que la muestra dibuja lo que hay pintado).
│   │                             #   **Todo en MESES**: no recibe
│   │                             #   `points`, así que la densidad no puede moverla (test de invariancia
│   │                             #   monthly ≡ hybrid en phase-strip.test.ts). PHASE_LABELS trae el par largo/corto
│   │                             #   («Trabajo»/«Trab.», «Media jornada»/«½ jorn.», «Jubilado»/«Jub.»)
│   ├── plan-card.ts              # 5.0.0 (D27/D32): modelo PURO de la tarjeta «Tu plan» del Resumen —
│   │                             #   planStatusFromWarnings (PRECEDENCIA: retire_at_age_underfunded rojo >
│   │                             #   birth_date_missing > target_retirement_age_missing > «En plan»; literal
│   │                             #   desconocido ⇒ «En plan», nunca hueco) y planMilestone (jubilación EFECTIVA;
│   │                             #   `null` CON razón = no aplica, `null` SIN razón = no se jubila en el horizonte).
│   │                             #   `retire_at_age_underfunded` aún NO lo emite el motor (llega con solve.rs): el
│   │                             #   mapeo se escribe ya para que no se pinte como nada el día que llegue.
│   │                             #   Test: plan-card.test.ts
│   ├── history-merge.ts          # mergeProjectionWithHistory(series, history): une la serie histórica (month_index<0) con la
│   │                             #   proyección en el vértice mes-0; identidad byte-idéntica si history null/vacío/anchor distinto.
│   │                             #   Con net_worth null (pasivo sin fotografiar entero) cae a assets_total y marca pastIsAssetsOnly:
│   │                             #   el pasado son ACTIVOS y el chart lo etiqueta así (leyenda «Activos (histórico)» + tooltip)
│   ├── snapshot-tracker.ts       # trigger del modal: EditLog (Map<assetId, epochMs>), SNAPSHOT_EDIT_WINDOW_MS, pruneEditLog,
│   │                             #   liquidCoverageComplete (todos los activos líquidos editados dentro de la ventana rodante ~1h)
│   ├── navigation.ts             # tab ↔ URL map: TABS, TAB_PATH (incl. expenses → «Movimientos», slug canónico /movimientos + alias de lectura /gastos en tabFromPathname), SETTINGS_SUBTAB_* (incl. history → «Histórico»/historico), tabFromPathname, settingsSubTabPath
│   ├── expenses.ts               # pure helpers de la pestaña «Movimientos»: month labels (monthLabelEs/monthShortLabelEs), defaultSelectedMonth,
│   │                             #   categoriesForKind (savings→[]), ImportRowDraft + initialDraftForRow/buildConfirmDecisions/summarizeDecisions/rowMatchesFilter,
│   │                             #   ImportBatchSummary + summarizeImportBatch (agregado de la tanda multi-archivo del wizard, 4.13.0),
│   │                             #   deltaToneClass/formatDeltaCurrency (rojo/verde solo en deltas), significanceThreshold (1% del ingreso real)/trendArrow/significantDeltaTone
│   │                             #   (umbral de significancia de las flechas ↑↓), AVG_WINDOWS + avgWindowLabel (pills 3m/6m/12m/YTD/Todo), capitalizeSource, y los helpers de la tabla de
│   │                             #   movimientos: normalizeSearchText/transactionMatchesQuery (búsqueda sin acentos), compareTransactions/sortTransactions + naturalSortDir (TxnSortKey/
│   │                             #   TxnSortDir; importe por |magnitud|, tiebreak estable), groupTransactionsByCategory/sortTransactionGroups (orden fijo: kind → |subtotal| desc; el subtotal
│   │                             #   EXCLUYE las conciliadas, que sí siguen en rows — si no, divergiría de la comparativa del servidor en la misma pantalla) e isReconciled (3.5.0: fuente de
│   │                             #   verdad = transfer_counterpart_id presente). Test: expenses.test.ts
│   ├── asset-form.ts             # 5.0.0 (WP5-2): cuerpo PURO de POST/PATCH de un activo —
│   │                             #   buildAssetWriteBody + assetOptionalDecimalPatch. Existe por el **TRI-ESTADO**
│   │                             #   de sus tres importes opcionales (purchase_price,
│   │                             #   expected_annual_return_percent, annual_volatility_percent): clave ausente = no
│   │                             #   cambia, `null` = BORRA, valor = fija. La regla que hay que acertar es cuál emite
│   │                             #   un campo VACÍO en una edición — `null` solo si el activo TENÍA valor (vaciar es
│   │                             #   una orden de borrado: así se vuelve a determinista), y nada si ya estaba vacío
│   │                             #   (un `null` sobre un hueco es un PATCH que no cambia nada, invalida la cache de
│   │                             #   proyección y puede acabar en `patch_empty`). En un ALTA nunca hay `null`.
│   │                             #   Antes vivía en tres `if` dentro de `submitAssetForm` y no se podía probar.
│   │                             #   Test: asset-form.test.ts
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
│   ├── basePath.ts               # prefijo de subpath tras proxy inverso (Ingress de HA, X-Forwarded-Prefix).
│   │                             #   Lee window.__FF_BASE__ / __FF_SSO__ (los inyecta handlers/spa.rs por request) →
│   │                             #   BASE_PATH (normalizeBase: solo ruta absoluta, sin barra final; todo lo demás → "")
│   │                             #   y SSO_AVAILABLE. apiUrl(path) para fetch, appUrl(path) para pushState,
│   │                             #   stripBase(pathname) para el router. Puras testeables: apiUrlWith/stripBaseWith.
│   │                             #   Test: basePath.test.ts (11 it()). Ver la regla en la tabla de abajo
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
│   ├── icons.tsx                 # set unificado 16×16 stroke 1.5 (29 iconos)
│   └── charts/
│       ├── summary.tsx           # SummaryDonutChart + SummaryBreakdownBlock (palette fría=activos / cálida=pasivos)
│       ├── PlanningDirectionChart.tsx   # barra inflow/outflow — usada en Upcoming Y Budget
│       ├── CategoryComparisonBars.tsx   # exporta SOLO MonthlyCashflowBars (cash-flow mes a mes desde months[], tokens --cf-income/--cf-expense/--cf-savings; verde/rojo = colores
│       │                                #   FUNCIONALES de serie, excepción de charts del design system, no chrome). El chart CategoryComparisonBars (barras Budget vs Promedio) y el
│       │                                #   token --exp-average se ELIMINARON tras 2.0.0: el Real vive en la tabla/KPIs y la tendencia vs presupuesto pasó a la banda de KPIs
│       ├── MiniProjection.tsx    # SVG compacto reusado en Resumen y Jubilación. 5.0.0: prop `showPhases`
│       │                         #   (opt-in, default false) = versión REDUCIDA de la tira de fases — banda de
│       │                         #   6px sin rótulos, misma `buildPhaseSegments` que el chart grande y posicionada
│       │                         #   con `xAtMonth` (meses), nunca con `xAt` (posiciones). Encendida solo en
│       │                         #   Jubilación: en la ventana de 12 meses del Resumen la fase no cambia
│       └── ChartLegend.tsx       # leyenda compartida (chart grande + minis): HTML fuera del SVG, estructurales
│                                 #   siempre visibles + activos truncados con chip «+N más» (modelo en lib/chart-legend.ts)
│
├── views/                        # one file per tab — receives props from App.tsx, owns local UI state
│   ├── SummaryView.tsx           # KPIs → **Tu plan** → Salud financiera → Proyección 12m (zoomY) → Desglose.
│   │                             #   5.0.0 (D27/D32, issue #207): tarjeta «Plan» FIJA entre la banda de KPIs y
│   │                             #   Salud financiera (`.plan-card-grid`/`.plan-card`, helpId `summary.plan`) —
│   │                             #   estrategia · hito · estado, con el modelo en `lib/plan-card.ts`. En Hogar,
│   │                             #   una tarjeta por `members[]` («Planes del hogar»). Necesita la prop
│   │                             #   `navigate` (nueva): el estado enlaza a «Tu cuenta» o a Jubilación
│   ├── AssetsView.tsx            # 5.0.0 (§A.2, issue #207): campo «Volatilidad anual % (opc.)» en el form de alta/
│   │                             #   edición + columna «Volat. % a.a.» en la tabla — solo desktop, y solo si algún
│   │                             #   activo del grupo declara un valor > 0 (mismo patrón condicional que la columna
│   │                             #   «Rent. % a.a.»; en móvil no entra ni en la sub-línea). Vacío/`0` = activo
│   │                             #   determinista; el valor solo alimenta las futuras bandas de Monte Carlo (WP6), el
│   │                             #   camino determinista lo ignora. Help text `assets.volatility` en `helpTexts.ts`.
│   ├── LiabilitiesView.tsx       # tabla sin columna Tipo. 4.2.0: select «Modelo» + InlineHint por modelo, controles
│   │                             #   (weekly, derive) deshabilitados donde el server daría 400, preview del principal
│   │                             #   derivado, y chip del modelo en el listado SOLO cuando ≠ cuota fija (chip existente,
│   │                             #   tokens del sistema, cero CSS nuevo)
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
│   ├── ImportWizardModal.tsx     # wizard import CSV en 2 pasos (useReducer); desde 4.13.0 acepta VARIOS archivos y los procesa en COLA (estado files[]+fileIndex+confirmed[];
│   │                             #   preview/confirm POR archivo). Paso 1 = archivo(s) → select «Cuenta origen (activo)» (movido desde el footer; ahora también en el preview; cuenta y
│   │                             #   formato se aplican a toda la tanda) → formato en <details> plegado (autodetección POR archivo). Paso 2 = línea «Archivo i de N» (solo con tanda >1) +
│   │                             #   banner con fuente capitalizada + chips de conteos, bulk bar con cluster único «Asignar a visibles», footer «{X} se importarán · {Y} excluidas
│   │                             #   ({Z} duplicadas)», columna «Tipo». /import/confirm con decisions[] paralelo; «Confirmar y seguir»/«Omitir archivo» avanzan la cola (el preview fallido a
│   │                             #   mitad ofrece Omitir/Reintentar); cancelar a mitad conserva lo ya confirmado (cola NO atómica a propósito: cada CSV = su fila de transaction_imports,
│   │                             #   deshacible por separado). Stateless (sha256 por archivo).
│   │                             #   3.5.0: las «posibles transferencias» ya NO se atenúan ni se desmarcan (entran incluidas; la exclusión del gasto la hace la conciliación) — solo dup/divisa;
│   │                             #   el aviso post-tanda (agregado `summarizeImportBatch`, con `reconciled_pairs`) lo pinta GastosView en su callback `onImported`, llamado UNA vez al
│   │                             #   cerrar y solo si hubo ≥1 confirm
│   ├── ManualCashEntryModal.tsx  # alta manual de efectivo: grid multifila (magnitud + kind fija el signo) + checkbox «Repetir cada mes» por fila (→ recurrence:{}) → POST /v1/transactions/batch
│   ├── RecurringRulesModal.tsx   # modal «Recurrentes» (botón en la toolbar de Movimientos): lista GET /v1/transactions/recurring y permite «Detener» (DELETE) cada regla
│   │                             #   (conserva las instancias ya materializadas). Patrón ManualCashEntryModal: fetch al abrir, toda la lógica de presentación aquí (nada en lib/)
│   ├── UpcomingView.tsx          # Planning
│   ├── RetirementView.tsx        # KPIs + MiniProjection (zoomY, clampToMonth=jub+12, xAxis) + perfil de jubilación.
│   │                             #   5.0.0 (D26/D33, issue #207) reescribió el bloque de configuración: aviso de alta
│   │                             #   descartable (`lib/retirement-intro.ts`) → **5 tarjetas de estrategia**
│   │                             #   (`.retirement-mode-card`/`.retirement-mode-grid.retirement-strategy-grid`, reusa
│   │                             #   el patrón ya existente del modo del objetivo, no un componente nuevo) →
│   │                             #   **formulario contextual**: solo los campos de la estrategia elegida (edad
│   │                             #   objetivo, pensión con fecha e indexación, media jornada, regla de retirada +
│   │                             #   modo — aquí vive el primer/único consumidor de `.retirement-radio-stack`, antes
│   │                             #   sin uso) más los ejes movidos desde `fire_settings` (modo/importe del objetivo,
│   │                             #   SWR, edad límite del horizonte) y los de riesgo (colchón de caja, umbral de
│   │                             #   éxito — inputs YA, sin la banda de Monte Carlo de WP6 todavía). Autosave de
│   │                             #   420 ms (`queueProfileSave`) con guarda de validez (`retirementProfileIssue`,
│   │                             #   mismos códigos que el servidor) contra `onSaveRetirementProfile` (`App.tsx`).
│   │                             #   Tile «Margen disponible» presente pero PLACEHOLDER (guion + «aún no se calcula»
│   │                             #   hasta que WP5 publique el solve). En Hogar (`scopeReadOnly` prop): el bloque de
│   │                             #   perfil entero se sustituye por un panel «Solo lectura»; `canEditProfile` es
│   │                             #   propio de esta vista (`hasMembership && !scopeReadOnly`, SIN exigir
│   │                             #   `role === "owner"` — el perfil es dato personal, lo edita cualquier rol,
│   │                             #   `viewer` incluido).
│   ├── ProjectionView.tsx        # wraps ProjectionNetWorthChart
│   ├── ProjectionNetWorthChart.tsx  # gran SVG chart, drag/zoom/hover, colores vía --proj-* tokens; se extiende a meses
│   │                                #   negativos con la serie histórica (áreas + marcadores + divisor «Hoy») vía mergeProjectionWithHistory.
│   │                                #   Overlay fino de cash-flow (v1.6.0): props cashflow/cashflowDaily/onRequestDailyCashflow — pinta la curva
│   │                                #   fina (fine.grid por month_fraction real, deflactada igual) sobre la zona pasada; daily lazy al hacer zoom histórico.
│   │                                #   5.0.0 (D29/D32, issue #207): **tira de fases bajo el eje X** (modelo en
│   │                                #   lib/phase-strip.ts) con su alto sacado del PLOT, nunca de lienzo extra — el
│   │                                #   viewBox tiene que seguir casando con la caja medida; `xTickBaselineY` unifica
│   │                                #   la fila de etiquetas X (años y «Hoy»), que antes se calculaba dos veces.
│   │                                #   Sin fases ni marcas la tira mide 0 y la geometría es la de 4.15.x. Tooltip:
│   │                                #   «Retirada del mes» / «Recorte» / «Exceso» solo desde retirement_month_index,
│   │                                #   deflactados con el MISMO factor que el patrimonio. Leyenda: una entrada por
│   │                                #   miembro en Hogar (TODO nombrado en el propio fichero: la línea fina por
│   │                                #   miembro espera a que el API publique `members[].series`)
│   │                                #   Leyenda (4.0.6): HTML fuera del SVG (ChartLegend); el ResizeObserver mide .projection-chart-plot
│   │                                #   (solo el SVG) y el viewBox casa EXACTO con la caja medida (los 38px de etiquetas X rotadas salen
│   │                                #   de ph, no de lienzo extra — si no, `meet` encoge el dibujo con bandas laterales). Tooltip: top-5
│   │                                #   activos por |valor| + «Otros (k)». Prop assetOwnerNames (App.tsx) desambigua duplicados en hogar
│   ├── SettingsView.tsx          # AccountCard + sub-tabs como pills («Usuarios» owner-only, «MCP» con tokens/conexiones/toggle de escritura) + ThemeToggle en "Datos y sistema"
│   │                             #   5.0.0 (D13/D26, issue #207): Ajustes → Plan PIERDE el modo/importe del objetivo,
│   │                             #   el SWR y la edad límite del horizonte (enlace «Jubilación» en su lugar — viven
│   │                             #   ahora en el perfil por usuario). La plusvalía gravable de la retirada, que
│   │                             #   vivía ANIDADA bajo el bloque de ventanas del promedio —invisible en el modo
│   │                             #   `budget` (serie, el default), solo visible en `transactions_avg`/
│   │                             #   `budget_income_real_expense`: era un bug, no una decisión— sube a nivel de panel
│   │                             #   y es visible en los tres modos. `planEditable = isOwner && !scopeReadOnly` sigue
│   │                             #   gateando el panel entero: `fire_settings` SIGUE siendo owner-only, a diferencia
│   │                             #   del perfil de jubilación de Jubilación (ver RetirementView.tsx arriba).
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
│   │                             #   porque App.tsx nunca monta. Máquina de fases: loading → disabled (404 = kill-switch;
│   │                             #   `/v1/oauth/authorize-details` es de los pocos que el switch SÍ desmonta, así que su
│   │                             #   404 es el `not_found` del fallback de /v1 — no el `mcp_disabled` de las rutas raíz)
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

## Ámbito del hogar y candado de solo lectura (5.0.0, D9/D32, issue #207)

`ledgerPersonScope` (`"mine" | "household"`, `lib/ledger.ts`) persiste en `localStorage` bajo
`LEDGER_PERSON_SCOPE_STORAGE_KEY` y **por defecto es `mine`** — antes de 5.0.0 era `household`.
El cambio es D9: la jubilación pasa a ser una estrategia POR USUARIO, así que la vista natural al
entrar es la propia; `resolveLedgerPersonScope` decide el valor inicial (ausente, vacío o
desconocido ⇒ `mine`). El control es el segmentado «Yo | Hogar» de la TopBar (ver
[`design-system.md`](design-system.md) §Shell), no el `<select>` anterior.

`App.tsx` deriva de `ledgerPersonScope` **dos booleanos módulo-scope**, y ninguna vista repite la
regla:

- **`scopeReadOnly`** (`= isScopeReadOnly(ledgerPersonScope)`, cierto en Hogar).
- **`canEditLedger`** (`= installation?.role !== "viewer" && !scopeReadOnly`).

Hogar es un agregado informativo y de **solo lectura** (D9/D32): el servidor es quien de verdad lo
impone (403 `not_row_owner` en toda mutación de una fila ajena, D21) — lo de aquí es UX, no la
frontera de seguridad.

| Vista | Boolean consumido | Qué desaparece en solo lectura |
|---|---|---|
| Activos, Pasivos, Presupuesto, reglas de asignación, Próximos | prop `canEdit={canEditLedger}` | Alta, edición, borrado, reordenación |
| Movimientos (`GastosView`) | prop `canEdit={canEditLedger}` | Igual, **más** el materialize de recurrentes en silencio al montar (`if (!hasMembership \|\| !canEdit …) return`) |
| Histórico (`HistorySettingsPanel` vía `SettingsView`) | prop `canEditHistory={canEditLedger}` + `scopeReadOnly` (mensaje) | Alta, edición, borrado de snapshots |
| Jubilación (`RetirementView`) | prop `scopeReadOnly` → deriva su PROPIO `canEditProfile = hasMembership && !scopeReadOnly` (sin exigir `role === "owner"`: el perfil es dato personal) | Todo el bloque de perfil (tarjetas de estrategia + formulario contextual); se sustituye por un panel «Solo lectura» |
| Ajustes → Plan (`SettingsView`) | `planEditable = isOwner && !scopeReadOnly` (aquí SÍ exige owner: `fire_settings` sigue siendo del hogar) | El panel entero de supuestos del hogar |
| Resumen (`SummaryView`) | `canEditLedger` decide si `onAddFirstAsset`/`onAddFirstBudgetEntry` se pasan o quedan `undefined` | Los CTA de estado vacío (sin `onAction`, `EmptyState` se queda en texto) |
| Asistente inicial (onboarding) | `!scopeReadOnly` dentro de `showOnboarding` | El wizard completo no se ofrece en Hogar |

Un banner `.app-scope-banner` («Vista agregada del hogar · solo lectura») se pinta como **primer
hijo de `<main>`** en TODAS las pestañas cuando `scopeReadOnly` — no solo en las del ledger, porque
el ámbito es global y quien llega a Jubilación o a Resumen desde el drawer no ha pasado por ninguna
otra pantalla. Ver [`design-system.md`](design-system.md) §Shell.

## Perfil de jubilación por usuario (5.0.0, D13, issue #207)

`retirementProfile` (`RetirementProfileApi | null`, `App.tsx`) es el perfil del usuario de la
sesión — estrategia, edad objetivo, SWR, modo/importe del objetivo, edad límite del horizonte,
pensión con fecha, media jornada, regla de retirada, colchón y umbral de éxito. Es un INPUT del
motor, así que vive en `App.tsx` y no en `RetirementView`: lo consume también el Resumen (el SWR
del paréntesis de la tarjeta Autonomía, vía `runwaySwrParenthetical`). `null` mientras no ha
llegado — nunca se sustituye por el default para pintar, o la vista enseñaría un plan que no es el
del usuario.

- **Carga**: `loadRetirementProfile()` se dispara en el mismo `useEffect` que `loadInstallation()`
  al iniciar sesión — no depende de la membresía, es dato del token
  (`GET /v1/auth/me/retirement-profile`, `patch_retirement_profile_core` lo acepta de cualquier
  rol).
- **Guardado**: `saveRetirementProfilePatch(patch)`, hermana de `saveFireSettingsPatch` (que desde
  5.0.0 solo cubre los supuestos del HOGAR — impuestos, ventanas del promedio, plusvalía gravable).
  Manda el PATCH **mínimo** tri-estado (`buildRetirementProfilePatch`, `lib/retirementProfile.ts`),
  actualiza el estado con el perfil YA RESUELTO que devuelve el servidor (`target_basis` se
  DERIVA ahí — el draft local tiene que resincronizarse) y recarga la serie de proyección
  (`loadProjectionSeriesPage()`), igual que `saveFireSettingsPatch`, para que Jubilación / Resumen /
  Proyección no se queden enseñando el plan anterior hasta el siguiente cambio de pestaña.
- **A diferencia de `fire_settings`, NO es owner-only**: es el dato personal del usuario de la
  sesión y el servidor lo acepta de cualquier rol — de ahí que `canEditProfile` en `RetirementView`
  no exija `role === "owner"` (ver tabla de arriba).

## Import conventions

- **`api/`** depends only on `api/` and the DOM `fetch`. No React.
- **`lib/`** is pure: no React, no fetch. May import from other `lib/*` and from `api/types`.
- **`components/`** may import from `lib/` and `api/types`. They are dumb presentational widgets.
- **`views/`** may import from anything below (`lib/`, `api/`, `components/`, other views). They own form/UI state via `useState` and receive data + mutation callbacks from `App.tsx`.
- **`App.tsx`** owns the long-lived state (installation, user, ledgerPersonScope, lists, busy flags, `projectionSeries`, `historySeries` **and `cashflowSeries`/`cashflowDaily`**) and the API mutation handlers. `historySeries` is loaded by `loadHistorySeries()` (parallel to the projection, in the projection-tab effect and after every snapshot mutation; failure → `null`, so the chart degrades to the current future-only view). **Desde 4.4.0 pide `?window_months=1200` explícitamente**: omitir el parámetro ya NO devuelve todo el histórico —el default de la API son 120 meses, pensado para clientes que leen la serie como texto— y sin esa línea el eje pasado se cortaría en 10 años sin ningún aviso. `1200` es el máximo de la API y en la práctica significa «todo». Si algún día el chart deja de necesitar la serie entera, lo que hay que cambiar es el número, no el default del servidor. `cashflowSeries` is loaded by `loadCashflowSeries()` alongside it (weekly, `window_months=24`); `loadCashflowDaily()` fetches the daily detail lazily (`window_months=6&resolution=daily`, once per scope/reload via `cashflowDailyRequestedRef`) when the chart zooms into the recent past. Same degrade-to-`null` contract as `historySeries`. Both refresh after transaction mutations (`onCashflowMutated`) and after snapshot mutations (they anchor the fine curve). `saveSnapshotNow(kinds)` POSTs a capture and reloads both history and cash-flow. Dispatch to a view is a `<XxxView {...props} />` call.

## Where to add new code

| New thing | Goes in |
|----|----|
| New API type returned by the backend | `api/types.ts` (export it) |
| New fetch endpoint wrapper | `api/client.ts` if reusable, otherwise inline in `App.tsx` next to existing handlers |
| **Cualquier URL absoluta que resuelva el navegador** (destino de `fetch`, `pushState`) | pásala por `apiUrl`/`appUrl` de `lib/basePath.ts`; lee `window.location.pathname` con `stripBase`. Sin proxy (`window.__FF_BASE__` ausente) son la identidad. `api/client.ts` ya lo hace por ti — solo los `fetch(` directos de `App.tsx`/`views/` lo necesitan a mano |
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

## Subpath tras proxy: toda URL de API pasa por `apiUrl` (`lib/basePath.ts`)

Cuando FutureFin se sirve bajo un subpath (Ingress de Home Assistant, un `location /futurefin/` de
nginx) **el servidor no ve el prefijo**: el proxy lo quita antes de entregar la petición, y el
router de Axum sigue montado en la raíz. El prefijo solo existe para el **navegador**. `handlers/spa.rs`
lo inyecta por request en el shell (`window.__FF_BASE__`, más `window.__FF_SSO__`) y este módulo lo
convierte en tres funciones:

| Función | Dónde se usa |
|---|---|
| `apiUrl(path)` | Destino de cualquier `fetch`. `api/client.ts` lo aplica **dentro de `apiFetch`**, así que todo lo que pase por los wrappers ya está cubierto. |
| `appUrl(path)` | Destino de `pushState`/`replaceState` (la navegación de `App.tsx`). |
| `stripBase(pathname)` | Lectura de `window.location.pathname` antes de dárselo al router (`App.tsx` en el estado inicial y en `popstate`; `main.tsx` para `/oauth/authorize`). |

- **La regla**: *toda* URL absoluta que resuelva el navegador pasa por una de las tres. Los `fetch(`
  directos de `App.tsx` y de las vistas autónomas lo necesitan **a mano** —`api/client.ts` solo
  cubre lo que va por `apiFetch`—; audítalos con `grep -rn 'fetch("/v1' apps/web/src` y su gemelo
  con backtick en vez de comilla doble (los dos deben salir vacíos).
- **Contrato de no-regresión**: sin `window.__FF_BASE__` inyectado, `BASE_PATH` es `""` y las tres
  funciones son la **identidad** carácter a carácter — la app se comporta exactamente igual que
  antes de existir el módulo. `normalizeBase` degrada a `""` cualquier valor que no sea una ruta
  absoluta (vacío, `/`, `//host`, una URL completa, basura): el caso sin prefijo es siempre el
  seguro.
- `apiUrlWith` es **idempotente**: una ruta ya prefijada no se duplica. Importa porque hay valores
  (una URL que vuelve de `history`, un `url` que se recompone) que pueden pasar dos veces.
- `SSO_AVAILABLE` (= `window.__FF_SSO__ === true`) es lo que dispara el intento **único** de
  `POST /v1/auth/sso` cuando `/v1/auth/me` responde 401 (`refreshSession` en `App.tsx`). Cualquier
  fallo cae al formulario de acceso de siempre.
- Tests: `lib/basePath.test.ts`.

## Ruta `/oauth/authorize` — resuelta en `main.tsx`, no en el router de `App.tsx` (v3.1.0)

La pantalla de consentimiento OAuth es la única vista que **no** cuelga del router de `App.tsx`. La
decisión se toma en `main.tsx`, a nivel de módulo, antes de que React renderice:

```tsx
const OAuthAuthorizeView = lazy(() =>
  import("./views/OAuthAuthorizeView").then((m) => ({ default: m.OAuthAuthorizeView })),
);
const isOAuthAuthorize =
  stripBase(window.location.pathname).replace(/\/+$/, "") === "/oauth/authorize";
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
- **FIRE client-side math** (`lib/fire.ts`) duplicates the Rust engine's tax/gross-up logic. Intentional: it powers the **live preview** of the FIRE target for the household-level axes (`taxes_enabled`, `tax_brackets`, `taxable_gain_ratio`…). If you change tax brackets server-side, mirror the change here.
- **Retirement-profile client-side bounds** (`lib/retirementProfile.ts`, 5.0.0, D13, issue #207) duplicate the Rust bounds of `retirement_profile.rs` §Cotas (`MIN_PROFILE_AGE`, `MAX_WITHDRAWAL_PCT`, `MAX_CASH_BUFFER_MONTHS`, `MIN/MAX_HORIZON_LIFESPAN_AGE`…). Intentional for the same reason as `fire.ts`: the autosave guard (`retirementProfileIssue`) has to reject client-side, with the SAME stable codes the server would return, before firing a PATCH the server would 400 on. Since 5.0.0 the strategy/objective axes (`fire_number_mode`, `swr_pct`, `horizon_lifespan_age`, `target_retirement_age`, the withdrawal rule, pension, partial retirement…) are **per-user**, not installation-wide — they moved out of `FireSettingsApi`/`installation.fire_settings` into `RetirementProfileApi`/`users.retirement_profile`; a plan/settings claim that still calls the projection axes "installation-only" is stale. If you change a bound in `retirement_profile.rs`, mirror it here — `retirementProfile.test.ts` walks the whole bounds table so a divergence is a red test, not a 400 in production.

## Frontend tests

See [`tests.md`](tests.md). Setup: Vitest + `node` environment (no jsdom needed for the current test set). All tests are in `*.test.ts` files colocated with the module they test.

## Provenance and maintenance

Re-verified 2026-09-03 against `release/5.0.0` commits `b413471` (WP7 1/3 — vista «Yo» por
defecto, segmentado «Yo | Hogar», hogar de solo lectura, aviso de alta de Jubilación) y `9ae5c24`
(WP7 2/3 — tarjetas de estrategia, formulario contextual del perfil, volatilidad del activo) y las
rebanadas WP7 3a (tira de fases del chart, tarjeta «Plan» del Resumen, jubilación efectiva en el
tile de Jubilación) y 3b1 (líneas finas por miembro, `target_basis_stored` en el formulario,
vaciado tri-estado de los porcentajes del activo), issue #207. Re-verify with:

- New lib modules exist: `ls apps/web/src/lib/retirement-intro.ts apps/web/src/lib/retirementProfile.ts`
- `ledger.ts` scope helpers: `grep -n "export function resolveLedgerPersonScope\|export function isScopeReadOnly\|export function ledgerViewAmp\|export const LEDGER_PERSON_SCOPE_STORAGE_KEY" apps/web/src/lib/ledger.ts`
- Default scope is `mine`: `grep -n 'return stored?.trim() === "household" ? "household" : "mine"' apps/web/src/lib/ledger.ts`
- `App.tsx` derives exactly two module-scope booleans from `ledgerPersonScope`: `grep -n "const scopeReadOnly = isScopeReadOnly\|const canEditLedger =" apps/web/src/App.tsx`
- `canEditFire` no longer exists (it was transient in `b413471`, replaced by `scopeReadOnly` + `RetirementView`'s own `canEditProfile` in `9ae5c24`): `grep -rn canEditFire apps/web/src` should print nothing
- `FireSettingsApi` no longer carries the four moved axes: `grep -n "fire_number_mode\|swr_pct\|horizon_lifespan_age" apps/web/src/api/types.ts` — no hit should fall inside the `FireSettingsApi` type block (most are in `RetirementProfileApi`/`RetirementProfilePatchApi`; `ProjectionSeriesApi.horizon_lifespan_age` is a separate, pre-existing echo field and stays)
- `RetirementProfileApi` shape: `grep -n "export type RetirementProfileApi" -A 20 apps/web/src/api/types.ts`
- Retirement-profile save wiring: `grep -n "saveRetirementProfilePatch\|loadRetirementProfile" apps/web/src/App.tsx`
- Asset volatility field: `grep -n "annual_volatility_percent" apps/web/src/api/types.ts apps/web/src/views/AssetsView.tsx`
- `.retirement-radio-stack` has live consumers: `grep -c "retirement-radio-stack" apps/web/src/views/RetirementView.tsx` (≥1; it was defined in `App.css` with zero consumers before `9ae5c24`)
- Test counts (don't cite the raw number without re-running): `grep -c 'it(' apps/web/src/lib/retirementProfile.test.ts apps/web/src/lib/retirement-intro.test.ts apps/web/src/lib/ledger.scope.test.ts apps/web/src/lib/member-lines.test.ts apps/web/src/lib/asset-form.test.ts`
- WP7 3a — tira de fases y tarjeta «Plan»: `ls apps/web/src/lib/phase-strip.ts apps/web/src/lib/plan-card.ts`
- La tira NO indexa arrays (invariante monthly ≡ hybrid): `grep -n "los mismos tramos con la serie mensual y con la decimada" apps/web/src/lib/phase-strip.test.ts`
- El chart calcula la fila del eje X UNA vez: `grep -c "xTickBaselineY" apps/web/src/views/ProjectionNetWorthChart.tsx` (≥3: definición + años + «Hoy»)
- WP7 3b1 — líneas por miembro y builders tri-estado: `ls apps/web/src/lib/member-lines.ts apps/web/src/lib/asset-form.ts`
- El TODO de la línea fina por miembro ya no existe (WP5-2 publicó `members[].series`): `grep -rn "TODO(5.0.0, D32" apps/web/src` **debe imprimir vacío**
- Un solo reparto de color para línea, tick y leyenda: `grep -rn "householdMemberColor" apps/web/src/lib/chart-legend.ts apps/web/src/lib/phase-strip.ts apps/web/src/lib/member-lines.ts` (los tres)
- El chart pinta las líneas de miembro bajo la Σ: `grep -n "memberVisible.map" apps/web/src/views/ProjectionNetWorthChart.tsx`
- El formulario manda la elección ALMACENADA, no la resuelta: `grep -n "withStoredTargetBasis" apps/web/src/App.tsx apps/web/src/lib/retirementProfile.ts`
- El tri-estado del activo vive en un solo sitio: `grep -n "buildAssetWriteBody" apps/web/src/App.tsx apps/web/src/lib/asset-form.ts`
- GastosView's documented exception to "hide, don't disable": `grep -n "disabled={!canEdit" apps/web/src/views/GastosView.tsx` (the two inline `<select>`s — categoría/tipo — stay `disabled`, not hidden)
