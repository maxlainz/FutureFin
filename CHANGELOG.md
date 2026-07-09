# Changelog

All notable changes to FutureFin will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

## [2.0.1] - 2026-07-09

Ronda de feedback tras 2.0.0: UX de Ajustes y de la banda de KPIs de Movimientos, edición de movimientos
importados, backfill inmediato de recurrentes con fecha pasada y detección de ahorro insensible a acentos.
Incluye dos cambios de **contrato de API** (el PATCH de una transacción importada ya no bloquea campos; nuevo
**422 `recurrence_too_old`** en el alta con recurrencia). Sin migración.

### Ajustes → Proyección — «fuente del ahorro» pasa a `<select>` estándar
- **De segmented a desplegable nativo**: «Fuente del ahorro de la simulación» deja de ser el segmented
  `.ff-segmented` y pasa a un `<select>` estándar con las mismas dos opciones (**Presupuesto** /
  **Promedio 12 meses**). El bloque de ayuda sale **fuera** del `<label>` (como hermano, asociado con
  `aria-describedby="savings-source-help"`) para que el nombre accesible del control sea solo su título y
  un clic en la ayuda no despliegue el select. Tres `<small>` explican Presupuesto, Promedio 12 meses y que
  Resumen/proyección/target FIRE siguen el modo elegido. **`.ff-segmented` se elimina de `App.css`** (el
  bloque de tokens queda ya solo para `.ff-theme-toggle`): no queda ningún segmented de 2–3 opciones en la app.

### Movimientos — KPIs muestran el promedio de la ventana + tendencia vs presupuesto
- **Valor principal = promedio de la ventana**: las cuatro KPIs de la banda pasan a mostrar como cifra
  principal el **promedio** de la ventana del selector (`expense_avg` / `income_avg` / `savings_avg` /
  tasa promedio = `savings_avg / income_avg`), no el valor real del mes. Las etiquetas lo reflejan:
  «Gasto promedio (3m/6m/12m/YTD/total)», «Ingreso promedio …», «Ahorro promedio …», «Tasa de ahorro …».
  Sin promedio (`months_with_data == 0`) → `—`.
- **Línea de tendencia bajo Gastos e Ingresos**: nueva línea de tendencia (flecha + delta `avg − budget` +
  «vs presupuesto») bajo la cifra principal, con el color **solo** en la flecha y la cifra
  (`num-pos`/`num-neg`); gastar menos / ingresar más que el presupuesto es favorable, `|Δ| ≤ umbral` → «=»
  neutro. Helper puro `kpiBudgetTrend` en `lib/expenses.ts` (devuelve `null` — slot reservado pero vacío — si
  no hay promedio o `budget <= 0`, porque comparar contra 0 no informa). **Ahorro y Tasa de ahorro no llevan
  delta** (no existe presupuesto de ahorro). Desaparecen los parentheticals «media …».
- **Frontend**: nuevo prop `trend?: ReactNode` en `MetricCard`, que ocupa el **mismo** slot reservado que
  `parenthetical` (baseline de fila intacta) y tiene prioridad sobre él. CSS `.metric-trend` +
  `.metric-trend-arrow` / `.metric-trend-delta` / `.metric-trend-label` (una sola línea; flecha y delta
  nunca se truncan, «vs presupuesto» hace ellipsis en tarjetas estrechas).
- **Definición deliberadamente distinta**: la «Tasa de ahorro» de Movimientos es `savings/income` (de la
  ventana); la del **Resumen** es `net/income`. Son magnitudes distintas a propósito.

### Movimientos — eliminada la comparativa de barras por categoría
- **`CategoryComparisonBars` fuera**: se elimina el componente de barras horizontales Budget vs Promedio por
  categoría (el valor Real ya vivía en la tabla y las KPIs). Con él se van el bloque CSS `.cmp-*` y el token
  de color `--exp-average` (zinc-500/400 claro/oscuro). **`MonthlyCashflowBars`** (cash-flow mensual
  divergente) permanece en el mismo archivo `charts/CategoryComparisonBars.tsx`, ahora su único export.

### API — PATCH de movimientos importados ya no bloquea campos (huella anclada al CSV)
- **`op_date`/`amount`/`concept` ahora editables también en importadas** (`import_id NOT NULL`). Hasta ahora
  → **400 `immutable_field`**; ese código y esa rama **desaparecen del crate**. La diferencia de
  comportamiento se traslada a la **huella de dedup**: en manuales se recomputa al cambiar esos campos
  (tomando un ordinal libre, liberando el anterior); en importadas la huella queda **anclada** a la del CSV
  original y **nunca** se recomputa, de modo que un re-import del mismo archivo sigue detectando el duplicado
  aunque el usuario haya reubicado la fecha o corregido importe/concepto. El modal de edición deja de
  deshabilitar esos inputs en importadas (el aviso pasa a «editarlo no afecta a la detección de duplicados»).
  Tests: `patch_imported_op_date_is_immutable` → **`patch_imported_fields_editable_fingerprint_anchored`**,
  y nuevo `patch_manual_op_date_recomputes_and_allows_reuse`.

### Recurrentes — el alta con fecha pasada backfillea en la misma transacción (bugfix)
- **Síntoma → causa → fix**: al crear un movimiento con `recurrence` y `op_date` en el pasado, las instancias
  de los meses intermedios no aparecían hasta **recargar** la vista de Movimientos — porque era el frontend,
  al montar, quien llamaba a `/recurring/materialize`. El create solo insertaba la instancia de origen y
  creaba la regla; el relleno dependía de esa llamada posterior. Ahora el create (y `/batch`) backfillea
  **todas** las instancias intermedias hasta hoy **dentro del mismo commit** del alta, vía el loop compartido
  `materialize_rule` / `backfill_new_rule` (extraído de `materialize_recurring`) y el helper
  `insert_manual_with_recurrence`. `POST /recurring/materialize` **sigue existiendo** para el avance de mes.
- **API — nueva cota `recurrence_too_old` (422)**: una recurrencia con `op_date` a más de **10 años** en el
  pasado generaría cientos de instancias en la transacción del alta → se rechaza con **422
  `recurrence_too_old`** (`assert_recurrence_not_too_old`). Es la **primera** variante
  `ApiError::Unprocessable` / `ErrorCode::Unprocessable` del crate (aparte de los 422 de deserialización de
  serde). Tests: `create_with_past_date_backfills_instances`, `recurrence_op_date_too_old_*`,
  `recurrence_op_date_within_bound_created`.

### Import — clasificación de ahorro y reglas aprendidas insensibles a acentos
- **Fold de diacríticos solo en comparaciones**: `is_savings_hint` (heurística de ahorro del preview) y el
  matching de reglas aprendidas (`rule_matches`) pliegan los diacríticos del español (`ÁÉÍÓÚÜÑ`→`AEIOUUN`,
  con minúsculas) antes de comparar, mediante el nuevo helper puro `fold_diacritics_upper` (en `schema.rs`).
  Así «Aportación…» con tilde se detecta como ahorro y una regla acentuada matchea un concepto sin tilde y
  viceversa. **Los patrones almacenados, `normalize_concept` y las huellas quedan intactos** (conservan sus
  acentos): el fold es exclusivamente de comparación, nunca toca datos persistidos ni fingerprints. Tests
  nuevos en `transactions_import.rs` (`savings_hint_accent_insensitive_*`, `learned_rule_matches_accent_insensitive*`).

## [2.0.0] - 2026-07-09

Toggle **«fuente del ahorro»** de la simulación FIRE: la proyección puede alimentarse del
**presupuesto** (comportamiento histórico) o del **promedio real de los últimos 12 meses de
transacciones**. Aditivo, sin migración. Cambio de clase **engine-input** (los errores son
silenciosos: las cifras siguen pareciendo plausibles) → se incluyen números worked before/after.

### Proyección — fuente del ahorro configurable (`savings_source`)
- **Nuevo eje `savings_source` en `fire_settings`**: `"budget"` (default, modo A) | `"transactions_avg"`
  (modo B). Se elige en **Ajustes → Proyección** con un segmented **«Presupuesto» / «Promedio 12
  meses»** (owner-only, autosave vía `saveFireSettingsPatch`). Deserialización **estricta** como
  `FireNumberMode`: valor desconocido → **422**; campo ausente → `budget` (backups viejos siguen
  cargando; `#[serde(default)]` a nivel de struct `FireSettings`).
- **Modo B — de dónde sale el ahorro**: el engine toma income/expense del **promedio ponderado** de
  las transacciones en la ventana `[primer día del mes actual − 12 meses, primer día del mes actual)`
  (12 meses calendario **completos**; el mes en curso queda fuera). Denominador = `months_with_data`
  (meses del tramo con ≥1 transacción de cualquier `kind`), misma semántica que la comparativa de
  Movimientos → un historial corto no diluye la media. Helper único
  `transactions/summary.rs::transactions_12m_avg`.
- **Resta híbrida de cuotas**: a `expense_avg` se le resta, por cada **liability activa** (filtrada
  por `payment_end_date`), el **promedio real** de sus transacciones con `linked_liability_id` si
  existen, y si no su **cuota nominal** del ledger (`liability_monthly_payment`, weekly ×52/12). Clamp
  global `expense_eff = max(0, expense_avg − Σ resta)`. Fórmula en un **único punto de verdad**
  (`effective_avg_income_expense`) consumido por `projection.rs` y `summary.rs` para que no diverjan.
  El engine sigue modelando las liabilities como `debt_service` → el ahorro **sube automáticamente al
  terminar cada préstamo** (step-up, verificado por test).
- **Target FIRE en modo B**: `annual_expense` usa `expense_eff` como base (antes `expense_retirement`
  del presupuesto) y `current_income` usa `income_eff`; `manual` sin cambios. **Cambio de base
  semántico e intencional**. La **fase de jubilación** (income/expense_retirement) sigue viniendo del
  **presupuesto** en ambos modos — desajuste target-vs-drawdown documentado en
  `futurefin-fire-domain-reference`. `end_adj` (ajustes por end-date de partidas de presupuesto) se
  **anula** en modo B (el gasto ya no es del presupuesto); los `planning_flows` (`flow_adj`) se
  mantienen (ortogonales).
- **Fallback silencioso**: `months_with_data == 0` en modo B → se usan los escalares del presupuesto
  (modo A efectivo). La respuesta señaliza el modo **efectivo** tras el fallback.
- **`GET /v1/summary` sigue el toggle**: en modo B con datos, `income_monthly_equivalent = income_eff`,
  `expense_regular_monthly_equivalent = expense_eff`, `net_monthly_equivalent = income_eff − expense_eff
  − Σ cuotas nominales activas` (casa con la pendiente del chart, que resta el debt service, y con el
  modo A, que incluye las cuotas derivadas) y `savings_rate` derivado. Campos nuevos en
  `financial_health`: **`savings_source`** (modo efectivo tras fallback) y
  **`savings_source_months_with_data`** (0 en modo A/fallback). `GET /v1/assets`
  (`contribution_nominal_monthly`) también respeta el modo.
- **Preview FIRE de Jubilación (frontend)**: `RetirementView` consume los equivalentes efectivos de
  `/v1/summary` en modo B (fetch gateado al modo) en vez de recalcular el need desde el presupuesto —
  elimina la clase de divergencia cliente/servidor. KPIs de Resumen etiquetados con parenthetical
  «promedio de N meses» en modo B.

### Contrato de cache — invalidación ahora **condicionada al modo**
- **`transactions` pasa a ser input del engine solo en modo B**: hasta ahora las mutaciones de
  transacciones **nunca** invalidaban la cache de proyección (contrato «transactions no son inputs
  del engine»). Con `savings_source = transactions_avg` **sí lo son**, así que create/batch/patch/
  delete, delete de import, import confirm y `recurring/materialize` invalidan la cache **solo cuando
  el modo efectivo es B** (gating en `invalidate_projection_if_transactions_avg`, best-effort
  post-commit: lee `savings_source`, y un fallo del SELECT **jamás** convierte una mutación exitosa en
  5xx). `rules.rs`, los previews y el borrado de una regla recurrente **nunca** invalidan. Sin warm-up
  tras mutación (rechazado históricamente). Test `transactions_projection_cache.rs` reescrito con el
  contrato condicional (modo A = ninguna mutación invalida; modo B = cada mutación invalida; flip
  A↔B vía PATCH installation invalida).

### Números worked before/after (fixture `summary_savings_source.rs`, cambio engine-input)
Misma instalación, un único mes con datos (el último completo): income real 3.000, gasto total 1.500
(de los cuales 400 vinculados a L1); presupuesto distinto adrede (income 9.000, gasto 8.000). Dos
liabilities activas: L1 (cuota nominal 500, con txn vinculada avg 400) y L2 (cuota nominal 300, sin
vincular).

| KPI (`financial_health`) | Modo A (`budget`) — antes | Modo B (`transactions_avg`) — después |
|---|---|---|
| `income_monthly_equivalent` | 9.000 (presupuesto) | 3.000 (`income_avg`) |
| `expense_regular_monthly_equivalent` | 8.000 (presupuesto) | 800 (`expense_eff` = 1.500 − [400 real L1 + 300 nominal L2]) |
| `net_monthly_equivalent` | budget − cuotas derivadas | 1.400 (= 3.000 − 800 `expense_eff` − 800 debt_service nominal) |
| `savings_source` | `budget`, months 0 | `transactions_avg`, months 1 |

Proyección (fixture `savings_source.rs`, `monthly_delta_assumption`): con budget income 5.000 / gasto
3.000 → **delta 2.000**; en modo B con income_avg 1.800 y expense_avg 600 (sin cuotas) → **delta
1.200**. `months_with_data == 0` en modo B → delta = 3.000 (idéntico al presupuesto, sin regresión).

### Migración / compatibilidad
- **Sin migración**: `savings_source` es aditivo en el JSONB `fire_settings` con `#[serde(default)]`;
  un `fire_settings` sin el campo → `budget`.
- **Backups `.ffbackup`**: sin cambio de `CURRENT_SCHEMA_VERSION` (sigue en **6**); el campo viaja
  dentro del snapshot informativo de settings con default en deserialización.
- **Rollback**: volver a una imagen anterior ignora el campo (lo deserializa a `budget`); ningún dato
  se pierde.

## [1.8.0] - 2026-07-08

Rediseño de la pestaña **Gastos → «Movimientos»** (frontend + backend, desplegados juntos), promedio
**ponderado**, movimientos **recurrentes** y backup `.ffbackup` **v6**.

### Movimientos — promedio ponderado (fix del «promedio 6m sale a 0»)
- **El promedio de la comparativa salía 0 (o ridículamente bajo) con poco historial** — síntoma:
  «Promedio 6m» a 0 aunque hubiera meses con gasto real. **Causa raíz**: el denominador del promedio
  era el **ancho fijo** de la ventana (p. ej. 6), de modo que los meses **sin ninguna transacción**
  contaban como 0 y diluían la media (3 meses reales ÷ 6 = mitad; 1 mes ÷ 6 ≈ ruido). **Fix**: el
  promedio pasa a ser **ponderado** — el denominador es `months_with_data` (nº de meses del tramo con
  ≥1 transacción del scope), nunca el ancho de la ventana; un mes vacío ya no diluye. Cuando
  `months_with_data = 0`, promedios y KPIs muestran «—» en vez de un 0 engañoso. **Lección**: un
  promedio sobre una ventana temporal debe dividir por los periodos con dato, no por el tamaño nominal
  de la ventana.
- **Ventanas nuevas del promedio**: al selector `3m · 6m · 12m` se añaden **`YTD`** (meses del año del
  mes seleccionado estrictamente anteriores a él; enero → tramo vacío) y **`Todo`** (desde el primer
  movimiento). El query param es ahora `avg_window` ∈ {`3`,`6`,`12`,`ytd`,`all`} (default `6`; trim +
  case-insensitive; inválido → 400 `avg_window must be one of 3, 6, 12, ytd, all`). El antiguo
  `avg_months` (1..24) se conserva como **alias legado**; `avg_window` gana si vienen ambos.

### Movimientos — rediseño de la pestaña
- **La pestaña «Gastos» pasa a llamarse «Movimientos»** (título y pill de navegación). La ruta
  canónica es `/movimientos`; `/gastos` sigue resolviendo como **alias de lectura** en
  `tabFromPathname` (los bookmarks viejos no se rompen). El `TabId` interno (`"expenses"`) y el
  archivo `views/GastosView.tsx` no cambian.
- **Fila TOTAL** en las tablas de gasto e ingreso (Real + flecha, Budget, Δ, Promedio) desde
  `summary.totals`.
- **Flechas de tendencia ↑/↓/=** en la celda «Real» (real vs promedio, `delta_vs_avg`), coloreadas
  `num-pos`/`num-neg` **solo** si `|Δ|` supera el **umbral de significancia = 1 % del ingreso real del
  mes** (fallback `income_budget`); con promedio pero por debajo del umbral la desviación se considera
  ruido → glifo **«=» atenuado** (`EqualsIcon` nuevo en `icons.tsx`; también el Δ vs budget va en
  gris); sin promedio el slot queda vacío (sin datos ≠ sin cambio). El glifo se pinta en un **slot de
  ancho fijo siempre reservado** (`.exp-trend-slot`, aunque esté vacío) para no desalinear las cifras
  de la columna Real — mismo principio que el paren-slot de `MetricCard`. Helpers puros nuevos con
  Vitest en `lib/expenses.ts` (`significanceThreshold`, `trendArrow` — direcciones
  `up`/`down`/`flat`/`null` —, `significantDeltaTone`, `AVG_WINDOWS`, `avgWindowLabel`,
  `capitalizeSource`, y los de búsqueda/orden/agrupación de la tabla — ver abajo); `expenses.test.ts`
  pasa de 32 a 75 tests.
- **Tabla de movimientos: búsqueda + agrupación + ordenación**. Barra de controles bajo la cabecera:
  **búsqueda** en vivo (concepto + nombre de categoría, insensible a mayúsculas y acentos, sin fetch) y
  toggle **«Por categoría»** (activo por defecto) que conmuta agrupado ↔ lista plana. Las cabeceras
  **Fecha / Concepto / Importe** son ordenables (click alterna asc/desc; cambiar de columna arranca en
  su orden natural — fecha/importe desc, concepto asc; `aria-sort` + indicador ↑/↓). **Importe ordena
  por magnitud** (`|amount|`, para ver los movimientos más grandes). En modo agrupado, cada grupo es una
  categoría (savings → «Ahorro / Inversión»; sin categoría → «Sin categoría» **por kind**) con contador
  y **subtotal firmado**, y el orden de los grupos es **FIJO**, ajeno a la clave activa: **secciones por
  kind — ingresos → ahorro → gastos — y, dentro de cada sección, de mayor a menor cantidad
  (`|subtotal|` desc)**; la clave activa solo ordena las filas DENTRO de cada grupo. Filtro sin
  resultados → «Sin resultados.». Helpers puros nuevos en `lib/expenses.ts`: `normalizeSearchText`,
  `transactionMatchesQuery`, `compareTransactions`/`sortTransactions` (`TxnSortKey`/`TxnSortDir`),
  `naturalSortDir`, `groupTransactionsByCategory`/`sortTransactionGroups`.
- Se retira el contador **«N meses con datos»** del toolbar (ruido); el «—» de promedios/KPIs sin
  histórico se conserva.
- **Tabla de movimientos sin scroll interno**: se retira `table-scroll--sticky` de la tabla principal
  (la página crece en vez de anidar un scroll; se pierde deliberadamente el `thead` sticky). La clase
  sigue existiendo para el preview del import.

### Movimientos — gráficas (excepción de color sancionada)
- La comparativa por categoría (`CategoryComparisonBars`) pasa de **3 series a 2**: **Budget**
  (`--ff-accent`) y **Promedio** (`--exp-average`). La serie **Real** se elimina de las barras — vive ya
  en la tabla y las KPIs.
- El cash-flow mensual (`MonthlyCashflowBars`) estrena tokens de tema `--cf-income` (verde sobrio,
  `oklch(0.58 0.10 165)` claro / `oklch(0.72 0.10 165)` oscuro), `--cf-expense` (rojo sobrio,
  `oklch(0.58 0.13 25)` / `oklch(0.70 0.13 25)`) y `--cf-savings` (= `--ff-accent`). **Excepción
  explícita** a la regla «sin rojo/verde en el chrome»: son colores **funcionales de serie** del
  gráfico, dentro de la única zona (charts) donde el design system acepta varios colores. Sancionado en
  `design-system.md`.

### Movimientos — cuotas de pasivo fuera de la comparativa (API interno breaking)
- Se elimina la línea derivada **«Cuotas de pasivos»** de la comparativa. Antes, el summary añadía una
  línea derivada (`derived_debt_line`, solo lado budget) con el equivalente mensual de las cuotas de
  pasivo; como las cuotas reales ya entran como movimientos en su categoría de gasto ordinaria, la
  comparativa las **contaba dos veces**. Ahora `totals.expense_budget` = **Σ del presupuesto de las
  categorías de gasto**, sin la línea derivada (el endpoint `/v1/budget` de la pestaña Presupuesto no
  cambia; solo la comparativa de Movimientos).
- **API breaking (interno)**: `GET /v1/transactions/summary` **elimina** del response
  `derived_debt_line` y `avg_months`, y **añade** `avg_window`, `window_months`, `months_with_data`.
  Frontend y backend se despliegan **juntos** en la misma imagen, así que no hay ventana de
  incompatibilidad para clientes; se marca como breaking del contrato interno para dejar constancia.

### Movimientos — recurrentes (nuevo)
- **Movimientos recurrentes** per-user (nómina, alquiler, aportación mensual…). Una **regla-plantilla**
  (`recurring_transaction_rules`) guarda concepto, importe firmado, `kind`, categoría, enlaces y día del
  mes; `POST /v1/transactions/recurring/materialize` genera las **copias mensuales** pendientes en
  `transactions` (`source='manual'`, enlazadas por `recurring_rule_id`), una por mes civil vencido.
- **Idempotencia por cursor**: `last_materialized_month` (primer día de mes) es la **única** fuente de
  idempotencia — re-materializar no duplica ni recrea instancias borradas (el cursor ya pasó ese mes);
  a propósito **sin** `UNIQUE(regla, mes)`. **Nunca crea `op_date` futuro**: el mes en curso solo se
  materializa cuando su día del mes ya ha llegado; el día se clampa a fin de mes en meses cortos.
- **UI**: checkbox «Repetir cada mes» por fila en el alta de efectivo (`ManualCashEntryModal`); tag
  «recurrente» en la tabla; borrar una instancia recurrente ofrece «Eliminar solo este» / «Eliminar y
  detener repetición»; modal nuevo «Recurrentes» (`views/RecurringRulesModal.tsx`, botón en la toolbar)
  para listar y detener reglas; materialización **silenciosa** al montar la vista (solo con permiso de
  escritura, refresca si generó algo). Sin `PATCH` de plantilla (borrar y recrear). Como el resto del
  módulo, **no invalida la cache de proyección** (las transacciones no son inputs del engine;
  regresión ampliada en `transactions_projection_cache.rs`).

### Import wizard — reorganización
- **Paso 1**: el archivo primero; el select **«Cuenta origen (activo)»** sube desde el footer (y ahora
  se envía también en el preview); el formato/preset va en un `<details>` plegado (autodetección por
  defecto). **Paso 2**: banner con la fuente **capitalizada** (`MyInvestor`) + chips de conteos, bulk
  bar con un único cluster «Asignar a visibles», footer «{X} se importarán · {Y} excluidas ({Z}
  duplicadas ya guardadas)», y la columna «Kind» renombrada a «Tipo».

### Migración / compatibilidad
- **Migración `20260708090000_recurring_transaction_rules.sql`**: crea la tabla
  `recurring_transaction_rules` (per-user; `amount` firmado `NUMERIC(18,4)` CHECK <> 0; `category_id`
  FK `ON DELETE RESTRICT`; `linked_asset_id`/`linked_liability_id` FK `ON DELETE SET NULL`;
  `day_of_month` 1..31; cursor `last_materialized_month DATE`) y añade la columna
  `transactions.recurring_rule_id` (FK `ON DELETE SET NULL`) + índices. Sin pérdida de datos.
- **Borrado de categorías**: `categories.rs` ahora cuenta (`category_reference_count`) y **remapea**
  también las `recurring_transaction_rules` al borrar una categoría, junto a las `transactions` (ambas
  con `category_id` `RESTRICT`).
- **Backups `.ffbackup`**: `CURRENT_SCHEMA_VERSION` sube de **5 a 6**. `BackupPayloadV6` = V5 +
  `recurring_transaction_rules: Vec<BackupRecurringRule>` + `BackupTransaction.recurring_rule_index`.
  Los backups **v1..v5 siguen importando** (cadena `migrate_to_current` extendida con
  `payload_v5_to_v6`, que arranca la colección nueva vacía). `last_materialized_month` se lleva verbatim
  para no re-materializar duplicados al importar.
- **Rollback**: volver a una imagen anterior con la migración ya aplicada deja la tabla/columna
  huérfanas (inertes para el código viejo); un backup v6 no importa en un servidor ≤v5 (lo rechaza
  `parse_payload` con 409 «newer than this server supports»).

## [1.7.1] — 2026-07-07

Fix visual de la pestaña **Gastos** (solo frontend): espaciados verticales que se tocaban en
móvil y en escritorio.

### Fixed
- El toolbar de Gastos (mes · ventana · acciones) tocaba directamente el borde del panel
  «Comparativa» (gap 0 verificado con Playwright en 390 y 1280 px): los botones «Importar CSV» /
  «Añadir efectivo» se apoyaban sobre el panel. Ahora `expenses-toolbar` lleva `margin-bottom`
  de 1rem, el mismo ritmo que separa los paneles entre sí.
- Las barras de la comparativa (`CategoryComparisonBars`) dibujaban una **doble línea
  separadora** (el borde inferior de la última fila de la tabla + su propio `bordered-top`),
  que en móvil leía como una fila vacía. Se elimina el `bordered-top` (la tabla ya aporta el
  separador) y el bloque pasa a `margin-top` propio; además las filas de barras ganan aire
  (gap 0.5rem → 0.75rem) para que los ticks de una categoría no se fundan visualmente con la
  barra de la siguiente.

### Verificación
- Barrido programático de gaps verticales entre hermanos del DOM (Playwright): el par
  toolbar→panel con gap 0 desaparece; cero intersecciones reales de elementos en 360/390/1280 px.
- Regla de oro re-verificada: sin scroll-X de página en 360/390/430/639/641/719/721/1280 px ×
  12 rutas; tema claro y oscuro revisados; `typecheck` + `lint` + 220 tests Vitest en verde.

## [1.7.0] — 2026-07-07

Revisión profunda de la **interfaz móvil** (solo frontend; sin cambios de API ni de esquema). Se
adopta una regla de diseño global: **la página solo scrollea hacia abajo** — cero scroll horizontal
de página; el scroll lateral queda confinado al interior de tablas como válvula residual.

### Sistema responsive (App.css / theme.css)
- Dos breakpoints canónicos etiquetados greppables (`/* bp:struct 720 */` estructura,
  `/* bp:mobile 640 */` densidad phone), documentados en la cabecera de `App.css` y en la nueva
  sección «Responsive / móvil» de `design-system.md`. Excepciones sancionadas por componente:
  `bp:edge 340` (título del TopBar) y `bp:topbar 1000` (ver abajo).
- Las franjas de KPIs abandonan el scroll horizontal deliberado: en ≤720px pasan a grid `auto-fit`
  (2×2 en iPhone; los milestones N-variables de Proyección forman filas de 2).
- Áreas táctiles: token `--ff-touch-min` (44px) aplicado a controles primarios en ≤640px, con
  carve-out explícito para los controles densos de tabla.
- Toolbars apiladas full-width en móvil (la de Gastos en 3 filas limpias), TopBar estrecha,
  modales con acciones apiladas (primario al alcance del pulgar) y paddings reducidos.
- **Fix estructural**: entre 721 y ~980px las 9 pills de navegación desbordaban la página entera;
  el colapso a hamburguesa sube a 1000px (solo TopBar).

### Tablas: columnas esenciales en móvil
- Las 12 tablas muestran en ≤640px solo columnas esenciales (p. ej. Movimientos: fecha `dd/mm` ·
  concepto · importe) con los datos secundarios en una sub-línea muted; tap en la fila (con
  chevron, foco y teclado) abre el modal de edición existente, que gana botón «Eliminar» solo-móvil.
- Mecanismo: hook `useIsMobile()` (`lib/responsive.ts`, matchMedia 640px) con render condicional —
  th/td no pueden desincronizarse; los selects inline de Movimientos se omiten en móvil (edición
  vía modal) y en el preview del import los selects y vínculos migran a la fila expandible.
- Desktop byte-idéntico: con `isMobile=false` el JSX es exactamente el anterior.

### Chart de patrimonio: gestos táctiles completos
- Arrastrar = pan, pellizcar = zoom (ancla en el punto medio, mismos límites que la rueda — la
  aritmética vive en `lib/chart-gestures.ts` con tests de equivalencia exacta contra el wheel),
  tocar = tooltip con auto-cierre; `touch-action: pan-y` para que el arrastre vertical siga
  scrolleando la página (el gesto aborta vía `pointercancel`).
- En móvil la pestaña Proyección deja de ser un viewport bloqueado: scrollea como el resto, con el
  chart a altura acotada (`min(72dvh, 30rem)`; `100dvh` para las barras dinámicas de iOS Safari).
- La leyenda del chart baja a su propia banda bajo la cabecera en anchos <560px (se solapaban).
- Ruta de escritorio intacta: `onWheel` y hover sin cambios (guards por `pointerType`).
- Cash-flow mensual de Gastos: 12 meses en móvil (24 columnas eran ilegibles).

### Verificación
- QA automatizado (Playwright): `scrollWidth <= innerWidth` en 8 viewports (360-1280) × 12 rutas,
  KPIs 2×2, tablas esenciales, táctil ≥44px, regresión desktop (columnas y selects inline
  intactos a 1280px), capturas revisadas en tema claro y oscuro. 220 tests Vitest.

## [1.6.0] — 2026-07-07

Histórico de **gasto mensual**: una nueva pestaña «Gastos» que importa el histórico REAL de gasto
(CSV bancarios o efectivo a mano), lo categoriza y lo compara mes a mes contra el presupuesto y el
promedio. Nada de esto existía (el modelo solo tenía flujos recurrentes de `budget_entries` y
snapshots de patrimonio; no había ninguna transacción datada). Además, ese cash-flow **moldea** la
curva histórica fina del chart de patrimonio sin contradecir los snapshots (tier-2). Detalle de
diseño: [`.claude/data-model.md`](.claude/data-model.md), [`.claude/api-routes.md`](.claude/api-routes.md),
[`.claude/engine.md`](.claude/engine.md).

### Gastos — Import CSV, categorización y comparativa mensual

- **Nueva pestaña «Gastos»** (`/gastos`): vista autónoma con KPIs del mes, selector de mes (default
  último mes **completo**, badge para el parcial en curso), comparativa por categoría Real \| Budget
  \| Δ \| Promedio (ventana 3/6/12 meses) y tabla de movimientos con edición inline y modal completo.
- **Import de CSV bancario** (`POST /v1/transactions/import/preview`→`/confirm`, stateless): presets
  hardcoded MyInvestor y N26 con **autodetección por cabecera** (`source=auto`), decodificación UTF-8
  con **fallback Windows-1252** para exports antiguos. El preview no escribe nada y devuelve un
  `file_sha256`; el confirm reenvía el mismo archivo + sha (anti file-swap) más un `decisions[]`
  paralelo por índice → 400 `preview_confirm_mismatch` si el sha o el nº de filas no cuadran.
- **Dedup por huella**: `UNIQUE (installation, owner, fingerprint, fingerprint_ordinal)`; la huella se
  computa en Rust (`source · op_date ISO · importe canónico 4dp · concepto normalizado`) y **nunca se
  almacena** en el CSV/backup. El `ordinal` (`MAX+1` por huella) distingue ocurrencias repetidas del
  mismo movimiento; forzar una fila `already_imported` incrementa el ordinal en vez de dar 409. Los
  duplicados, las transferencias internas (heurística) y los movimientos en divisa ≠ EUR llegan al
  preview **desmarcados** para que el usuario los revise.
- **Categorización con reglas aprendidas**: al confirmar un import con categorías, se hace upsert de
  una `categorization_rule` por patrón (derivado del concepto sin sufijos de referencia numérica);
  el siguiente preview PRE-asigna kind+categoría. Precedencia: source-específica > agnóstica → exact
  > prefix > substring → patrón más largo → `updated_at`. CRUD completo en `/v1/transactions/rules`.
- **Efectivo manual**: alta individual (`POST /v1/transactions`) y multifila (`/batch`, ≤1000). El
  usuario teclea una **magnitud** y el kind fija el signo (ingreso → +, gasto/ahorro → −, la
  convención firmada del backend). `savings` no admite categoría (`savings_no_category`).
- **Comparativa** (`GET /v1/transactions/summary`): mes real vs presupuesto vs promedio de N meses,
  con magnitudes ≥0 para comparar (gasto = `−Σ`, ingreso = `+Σ`, ahorro = `−Σ`, con bloque propio y
  excluido del consumo). Las cuotas de pasivo aparecen **solo en el lado budget** (`derived_debt_line`,
  reutilizando `budget.rs`) — sus actuals ya viven en su categoría de gasto → **sin doble conteo**.
- **Campos inmutables en importadas**: en una transacción con `import_id`, `op_date`/`amount`/`concept`
  son inmutables por PATCH (protegen la huella) → 400 `immutable_field`; en manuales la huella se
  recomputa. Borrar un lote (`DELETE /v1/transactions/imports/{id}?confirm=true`) deshace el import en
  cascada.

### Histórico — Cash-flow tier-2 y overlay fino del chart

- **Nuevo endpoint** `GET /v1/history/cashflow`: dos capas independientes. (1) `months[]` — agregado
  mensual **firmado** por kind (`expense`/`savings` ≤0, `income` ≥0, `net` = suma), Decimal-string,
  contiguo `-window_months..=0`. (2) `fine` (opcional) — la curva fina de patrimonio (`weekly` default,
  `daily` solo con `window_months ≤ 6` → si no, 400 `daily_window_too_large`) donde los deltas de las
  transacciones vinculadas a un asset (pata cuenta del batch = `+amount`; pata destino de un ahorro =
  `−amount`) **moldean** la curva **sin contradecir los snapshots**: pasa exacta por ambos extremos
  (`v(t) = Va + C(a→t) + f·(Vb − Va − C_total)`, intervalo semiabierto `(a,t]`). Presente solo si hay
  transacciones vinculadas Y snapshots que anclar. Sin cache; `spawn_blocking` solo en `daily`.
- **Refactor puro de `GET /v1/history/series`**: el pipeline común (`fetch_history_scope` +
  `accumulate_series`) se comparte; con un mapa de cash-flow vacío, la serie mensual de snapshots
  queda **byte a byte idéntica** (test de regresión compara el JSON completo con y sin transacciones
  sembradas; el engine garantiza P3: `cashflow` vacío ⇒ interpolación lineal textual).
- **Overlay fino en el chart de patrimonio**: `ProjectionNetWorthChart` pinta la curva histórica fina
  (`fine.grid` posicionado por `month_fraction` real, deflactado con el mismo deflator fraccional)
  sobre la zona pasada; en la zona cubierta recorta la polilínea mensual y las une sin hueco. `daily`
  se fetchea **lazy** al hacer zoom histórico reciente. Sin cash-flow o ante cualquier fallo de fetch,
  el pasado queda exactamente como antes. La recarga está cableada a mutaciones de transacciones,
  snapshots y cambio de scope.
- **Sin impacto en la proyección**: ningún handler de `transactions` ni de `cashflow` llama a
  `refresh_projection_after_mutation` — las transacciones no son inputs del engine (arranca en el mes
  0 con el ledger vivo), así que invalidar la cache aquí solo tiraría una entrada caliente sin cambiar
  ni un número. Regresión: `transactions_projection_cache.rs`.

### Migración / compatibilidad

- **Migración `20260707120000_transactions_and_rules.sql`**: crea tres tablas per-user —
  `transaction_imports` (cabecera de un lote de CSV), `transactions` (movimiento datado y firmado) y
  `categorization_rules` (reglas aprendidas). Semántica de FK deliberada: `import_id` ON DELETE
  CASCADE (deshacer un import borra sus movimientos), `category_id` ON DELETE RESTRICT (categoría en
  uso no se borra sin remap — `categories.rs` la incluye en el reference-count), `linked_asset_id`/
  `linked_liability_id`/`account_asset_id`/`assign_category_id` ON DELETE SET NULL (el movimiento/regla
  sobrevive al borrado de la fila de ledger/categoría).
- **Datos**: sin pérdida de datos (tablas nuevas, aditivas). El histórico de gasto arranca vacío.
- **Backups `.ffbackup`**: `schema_version` sube a **5** (`BackupPayloadV5` = V4 + `transaction_imports`
  + `transactions` + `categorization_rules`). Refs por índice a los vecs del payload; la **huella se
  recomputa al importar** (nunca se exporta), solo se lleva `fingerprint_ordinal`. Importar un backup
  ≤v4 rellena las tres colecciones vacías (`payload_v4_to_v5`); la cadena v1→…→v5 sigue intacta.
- **Dependencias nuevas** (`apps/api/Cargo.toml`): `csv` (parseo de los CSV bancarios), `encoding_rs`
  (fallback Windows-1252) y `sha2` (el `file_sha256` del flujo preview→confirm).
- **Sin breaking**: endpoints y tablas nuevos, backup retrocompatible; ningún payload ni ruta previa
  cambia de forma.

### Tests

- **Integración (local)**: el módulo de transacciones añade 27 tests — `transactions_crud.rs`,
  `transactions_import.rs`, `transactions_summary.rs`, `transactions_projection_cache.rs` (regresión
  no-cache) — más el roundtrip v5 en `backup_user_roundtrip.rs` y fixtures anonimizados de ambos
  bancos; el endpoint de cash-flow añade `history_cashflow.rs` (incl. el diff byte a byte de
  `/history/series` con y sin transacciones). **Engine**: propiedades P1–P5 del anclaje de cash-flow
  en `crates/engine/src/history.rs`. **Frontend**: Vitest de `lib/expenses.ts`.

## [1.5.1] — 2026-07-07

Pequeña mejora sobre el histórico de v1.5.0: el modal de backfill deja de arrancar vacío. Ahora
propone los items del usuario con sus valores **interpolados a la fecha elegida** con la misma
matemática de la serie histórica.

### Histórico — Prefill del backfill

- **Nuevo endpoint** `GET /v1/history/snapshots/prefill?kind=&date=`: devuelve, para el `kind`
  (`asset` \| `liability`) y la fecha civil pedidos, la lista de items del propio usuario con un
  valor sugerido y un `basis` ∈ `interpolated` \| `first_snapshot` \| `live` \| `not_owned`.
  Interpolación **idéntica a `/v1/history/series`** — lineal en días civiles para activos, curva de
  amortización francesa (corregida por residuo) para pasivos — reutilizando el engine puro; sin
  redondeo intermedio.
- **Items posteriores o ya vendidos**: un item que aún no existía en esa fecha (o una fila ya
  borrada/expirada) llega con `value: "0"` y `existed: false`; el modal lo marca con una pista
  visual para que el usuario decida si incluirlo. `date` en el futuro / `kind` inválido → 400 con
  los códigos estables ya usados por el backfill (`snapshot_date_in_future`, `invalid_kind`).
- **Auto-relleno del modal de creación**: al abrir «Añadir snapshot» los valores se prerrellenan y
  se **refrescan** al cambiar fecha o kind mientras el usuario no haya tocado nada; en cuanto edita
  (dirty) el refetch automático se detiene y aparece «Recalcular sugerencias» para pedirlo a mano.
- **Edición**: el modal de editar snapshot gana «Añadir items que faltan», que **solo** anexa los
  items ausentes (nunca reescribe valores ya introducidos), útil cuando el ledger creció después de
  guardar el snapshot.

### Migración / compatibilidad

- **Sin migración de base de datos**; endpoint puramente aditivo (GET de solo lectura, misma
  matemática que la serie ya existente). **Sin breaking**: no cambia payloads existentes ni el
  esquema `.ffbackup`.

### Tests

- **Integración (local)**: ~7 tests nuevos en `history_snapshots.rs` para el prefill
  (interpolación, `first_snapshot`, `live`, `not_owned`, validaciones 400, viewer).

## [1.5.0] — 2026-07-06

Perspectiva histórica del patrimonio: como los valores del ledger se actualizan a mano cada
cierto tiempo (no en tiempo real), la app guarda **snapshots manuales** por usuario e
**interpola** entre ellos para reconstruir la serie histórica de patrimonio neto, mostrada unida a
la proyección en un único chart temporal (pasado + presente + futuro). Nada parecido existía antes
(no había tabla de historial). Detalle de diseño: [`.claude/data-model.md`](.claude/data-model.md),
[`.claude/api-routes.md`](.claude/api-routes.md), [`.claude/engine.md`](.claude/engine.md).

### Histórico — Snapshots de patrimonio por usuario

- **Captura manual** (botón «Guardar snapshot» en Activos y Pasivos): `POST /v1/history/snapshots/capture`
  copia los items vivos del propio usuario (assets: valor actual; liabilities no expiradas: además
  `payment_amount`/`apr_percent`/`payment_frequency`, para sobrevivir borrados). **Upsert por día
  civil** en `calendar_tz` — capturar dos veces el mismo día reescribe el snapshot silenciosamente.
  0 filas propias → snapshot válido con 0 items. Nuevas tablas `history_snapshots` /
  `history_snapshot_items` (migración `20260706203746_history_snapshots.sql`).
- **Backfill editable** en `Ajustes → Histórico` (nueva sub-pestaña): `GET /v1/history/snapshots?year=&kind=`,
  `POST` (crear, `source='backfill'`), `PUT /{id}` (reemplazo completo de items, `kind` inmutable),
  `DELETE /{id}`. Guardia `id+installation+owner` → 404 si no es tuyo (no revela existencia);
  fecha ocupada → 409 (constraint de unicidad, mapeado por el `From<sqlx::Error>` central);
  validaciones 400 con códigos estables (`snapshot_date_in_future`, `duplicate_item_id`,
  `terms_only_for_liabilities`, `invalid_kind`…), con bounds copiados de `assets.rs`/`liabilities.rs`.
- **Serie interpolada server-side** (`GET /v1/history/series`): la matemática vive en el engine puro
  (`crates/engine/src/history.rs`, `evaluate_timeline`) — **lineal en días civiles** para activos y
  **curva de amortización francesa** para pasivos, corregida por residuo para pasar **exacta por
  ambos extremos** (`P(g)=max(theo(x)+f·(P_b−theo(N)),0)`; fallback lineal si el pago no cubre el
  interés o faltan términos). Todo `Decimal` sin redondeo intermedio; el total suma exactamente lo
  observado en cada fecha de snapshot. El cliente **no** interpola — recibe la serie lista para
  pintar (no hace falta fixture de paridad; ver `.claude/skills/futurefin-validation-and-qa`).
- **Chart unificado** (`ProjectionNetWorthChart`): se extiende a la izquierda con `month_index`
  negativos — línea NW histórica (token `--proj-nw-past`), áreas apiladas por activo también en el
  pasado (mismo rescale I6, `Σáreas = max(0,NW)`), marcadores de snapshot (círculo relleno = asset,
  hueco = liability) y divisor vertical «Hoy». Zoom/pan alcanzan meses negativos; el modo focus
  sigue arrancando en mes 0. El estado vacío (sin snapshots) renderiza **idéntico píxel a píxel** al
  chart anterior, garantizado por la identidad por referencia de `mergeProjectionWithHistory`
  (`apps/web/src/lib/history-merge.ts`).
- **Inflación hacia atrás**: el toggle «ajustado a inflación» deflacta también el pasado, con el
  mismo deflactor keyed por `month_index` real (`deflationFactorAt`); con k negativo **amplifica**
  (`×(1+inf/100)^(−k/12)`). Nunca por posición de array (raíz del incidente v1.4.2).
- **Modal «¿Guardar snapshot?»**: salta una vez cuando el usuario ha editado el valor de **todos**
  sus activos líquidos propios dentro de una ventana rodante de ~1 h (tracking en memoria por
  sesión, `lib/snapshot-tracker.ts`); tras guardar activos ofrece snapshot de pasivos si hubo
  cambios. Componentes `SnapshotButton.tsx` + `SnapshotPromptModal.tsx` (tontos; la lógica vive en
  `App.tsx`).
- **Scoping**: `GET /v1/history/series?view=mine` = serie propia; `household` (default) = **suma
  server-side** de las series interpoladas de cada usuario (agregación en Rust vía los helpers
  `LedgerView`). Las filas compartidas (`owner_user_id IS NULL`) no se capturan — limitación
  documentada. `AssetResponse` (`GET /v1/assets`) gana `owner_user_id: Option<Uuid>` (dato de
  display, no frontera de seguridad) para que el trigger del modal funcione en vista household.
- **Excepción f64 extendida y documentada**: los arrays por punto de `/v1/history/series`
  (`net_worth`/`assets_total`/`liabilities_total`, `asset_series[].values`, `markers[].total`) se
  serializan como `f64` (misma justificación chart-only que `/v1/projection/series`; una sola
  definición `serialize_decimal_as_f64`, ahora `pub(crate)`). Los CRUD de snapshots siguen
  Decimal-as-string. Actualizados D4/I3 en `futurefin-architecture-contract` y `api-routes.md`.
- **Sin invalidación de cache por diseño**: los snapshots **no son inputs del engine** de
  proyección, así que sus mutaciones **no** llaman a `refresh_projection_after_mutation` — la cache
  de proyección nunca se invalida por escribir historial. Comentario explícito en el handler + test
  de regresión `snapshot_mutations_do_not_touch_projection_cache`. La serie no tiene cache propia
  (cómputo sub-ms).

### Backups — `.ffbackup` schema v4

- **`CURRENT_SCHEMA_VERSION` 3 → 4**: el export incluye ahora los snapshots del usuario
  (`BackupPayloadV4` = V3 + `snapshots`; cadena `payload_v3_to_v4` encadenada en `migrate_to_current`).
  v1/v2/v3 **siguen importando** (v3→v4 rellena una lista de snapshots vacía). El rechazo de
  versiones futuras se mantiene: un `.ffbackup` v4 **no** se puede importar en un servidor ≤1.4.x
  (rechazo limpio con «update FutureFin to import this backup», no corrupción).
- **Mecanismo de re-enlace**: cada item de snapshot exporta `ledger_index` (posición en el array
  assets/liabilities del propio payload) **e** `item_key` (= `source_item_id` original). Al importar,
  si `ledger_index` está presente se reescribe `source_item_id` al UUID fresco de la fila re-creada
  (mantiene el enlace entre snapshots y el empalme con hoy); si es null se conserva `item_key`
  verbatim (items de filas borradas / backfill libre siguen enlazados entre sí). `ledger_index`
  fuera de rango → 400 con rollback de la transacción. El preview reporta counts de `snapshots` y
  `snapshot_items`.
- **FIX (bug preexistente)**: `import_user_backup_apply` no llamaba a
  `refresh_projection_after_mutation` tras `tx.commit()` → la proyección quedaba **stale hasta
  60 min** después de un import. Ahora invalida la cache al terminar.

### Correcciones del chart (bugs preexistentes con densidad `hybrid`)

- **FIX — fecha errónea en el tooltip**: el hover pasaba el **índice de array** a
  `projectionHoverTitle` en lugar del `month_index` real del punto. Con `density=hybrid` (puntos no
  equidistantes) el título mostraba una fecha equivocada a partir del mes 12. Ahora usa
  `pts[hover].month_index`.
- **FIX — valor erróneo en los marcadores de planning**: se indexaba `nw[m.mi]` por índice de mes
  sobre el array de puntos (que bajo `hybrid` no es 1 punto/mes), leyendo el patrimonio de otro
  punto. Ahora resuelve el valor con `valueAtMonth` y excluye `mi < 0`. Con `density=monthly` ambos
  fixes son idénticos al comportamiento previo (sin regresión).

### Migración / compatibilidad

- **Migración aditiva** `20260706203746_history_snapshots.sql`: solo crea dos tablas nuevas
  (`history_snapshots`, `history_snapshot_items`) + índice; **sin pérdida de datos** y sin tocar
  columnas existentes. El rollback de la app es inocuo mientras las tablas queden huérfanas (nada
  más las lee); un downgrade real de imagen sigue las reglas de `_sqlx_migrations` (roll-forward).
- **Sin nuevas variables de entorno ni ajustes de instalación** — el histórico es superficie
  per-user de request/datos.
- No breaking: endpoints nuevos, campo de respuesta opcional (`AssetResponse.owner_user_id`),
  arrays f64 adicionales y `.ffbackup` v4 aditivo (importa v1–v3). Único límite de compatibilidad:
  un backup v4 no es importable en versiones ≤1.4.x (rechazo limpio, por diseño).

### Tests

- **Engine (CI)**: `crates/engine/src/history.rs` — 14 tests (lineal, amortización con corrección
  residual, reglas de timeline, `month_index`/`add_months_signed` negativos). Engine total 22 → 36.
- **Integración (local)**: `history_snapshots.rs` (12), `history_series.rs` (7, números predichos
  antes de ejecutar), `backup_user_roundtrip.rs` (8) + 4 unit tests nuevos en
  `backup_user/schema.rs` (migración v3→v4, roundtrip v4, rechazo versión futura, cadena v1→v4).
  Nuevo helper `register_and_approve_member` en `tests/common/mod.rs`.
- **Vitest**: `history-merge.test.ts` (11), `projection-chart.test.ts` (10), `snapshot-tracker.test.ts`
  (8) + casos negativos en `dates.test.ts`. Total 72 → 104.

## [1.4.4] — 2026-07-02

### Documentación — biblioteca de skills + CLAUDE.md como punto de entrada único

- **Nueva biblioteca de 15 skills en `.claude/skills/`** para que cualquier sesión de IA (o dev) sin contexto previo pueda mantener el proyecto: runbooks core (change-control, debugging, build/run/config, validation, diagnostics con scripts, docs), packs de conocimiento (architecture-contract, fire-domain-reference, failure-archaeology) y capa avanzada (projection-realism-campaign, proof-toolkit, research-frontier, research-methodology). Todo verificado contra el código; revisión a tres bandas (factual, doctrina, usabilidad) con fixes aplicados.
- **`CLAUDE.md` reorganizado como entry point único**: sección "Start here" con tabla de enrutado tarea→skill, las tres capas de documentación y la regla de mantenimiento (Provenance por skill; erratas en `futurefin-docs-and-writing` §7).
- **Ocho erratas de documentación corregidas** (docs decían una cosa, el código otra): `.claude/tests.md` afirmaba "no hay CI" (existe `ci.yml`; lo que NO corre son los tests de integración Postgres ni Vitest) y "33 migraciones" (son 31; ahora se referencia el comando en vez del número); `.claude/data-model.md`, `.claude/engine.md` y `.claude/api-routes.md` aún describían `projection_target_age` (eliminada en v1.0.6) y los valores viejos `mac_*` de `horizon_basis` (reales: `lifespan_90 | fallback_no_demographics | months_override`); `.claude/auth-and-membership.md` apuntaba a un `docs/spec/AUTH_MODEL.md` inexistente; `README.md` documentaba el endpoint eliminado `GET /v1/backup/export.zip` (sustituido por los endpoints `.ffbackup` en v1.0.9; la sección Backups ahora describe las dos capas reales); y el comando de dev de CLAUDE.md/README para levantar solo Postgres omitía el override split-dev (sin él, `cargo run` no puede conectar porque la DB no expone puerto al host). `.claude/env-and-config.md` además presentaba un "default" para `DATABASE_URL` (es obligatoria; panic al arrancar) y describía mal `SESSION_TTL_DAYS` (fuera de rango cae al default 30, no se clampa).
- **Comentarios de código desactualizados corregidos** (sin cambio de comportamiento): doc-comment de `horizon_basis` en `handlers/projection.rs` (listaba los `mac_*`) y el header de `apps/api/tests/common/mod.rs` (referenciaba un `make clean-test-schemas`/script inexistentes; ahora da el one-liner psql real).
- `.claude/tests.md` documenta ahora el job-por-job de CI y añade `projection_cache.rs` al inventario de tests de integración.

## [1.4.3] — 2026-06-24

### Resumen — Mini-gráfica de proyección

- **Leyenda desglosada por activo**: la leyenda de la mini-gráfica ("Proyección · 12 meses") ya no muestra un genérico "Composición por activo", sino una entrada por cada activo (color del área + nombre), con los mismos colores y orden que las áreas apiladas del chart.
- **Valor al final de la serie**: la cabecera del panel muestra ahora el patrimonio neto de inicio → fin de la ventana de 12 meses, en un span discreto alineado a la derecha del título (reutiliza el patrón ya existente en Jubilación).

### Frontend — Limpieza de lint

- Resueltos 10 problemas de lint preexistentes (`npm run lint:web` queda en 0): `prefer-const` y dos violaciones de `rules-of-hooks` en `ProjectionNetWorthChart` (los `useEffect` de animación del eje Y se movieron antes del early return, sin cambio de comportamiento), directivas `eslint-disable` muertas en `perf.ts`/`main.tsx`, y supresión documentada de `exhaustive-deps` en los efectos de re-init del draft FIRE (`RetirementView`, `SettingsView`).

## [1.4.2] — 2026-06-19

### Proyección — Milestones ajustados a inflación

- **Milestones en euros de hoy**: los hitos de patrimonio (1M, 2.5M, 5M…) ahora respetan el toggle "Inflation Adjusted" del chart. Con el toggle activo se cruzan sobre el patrimonio **deflactado**, es decir, el hito de 1.000.000 € se alcanza cuando el patrimonio nominal vale 1.000.000 € *en poder adquisitivo de hoy* — más tarde que en términos nominales, y algunos umbrales altos dejan de ser alcanzables dentro del horizonte. Con el toggle apagado siguen siendo nominales (comportamiento anterior). Las KPIs y los marcadores del chart se actualizan al cambiar el toggle.
- **Backend**: nuevo campo `milestones_real` en `ProjectionSeriesResponse` (mismos umbrales sobre el patrimonio deflactado; vacío cuando la inflación es 0 — la web reusa `milestones`). Helper `deflate_points_to_today` que deflacta a resolución mensual completa para no perder precisión del mes de cruce con densidad `hybrid`. La jubilación no cambia: su mes de cruce es invariante a la inflación.
- **Fix de deflactación del chart**: `ProjectionNetWorthChart` deflactaba cada punto usando su índice de array en vez de su `month_index` real. Con densidad `hybrid` (los puntos no son equidistantes) esto subestimaba los años transcurridos y deflactaba de menos a partir del mes 12, hasta que llegaba la serie `monthly`. Ahora usa `month_index`, lo que además alinea la curva con los `milestones_real` del backend. Para densidad `monthly` el resultado es idéntico (sin regresión).

## [1.4.1] — 2026-06-18

### Frontend — Hover de la gráfica de proyección

- **Unidad complementaria en el tooltip**: el título del hover muestra ahora siempre la otra unidad entre paréntesis — en modo edad `NN años (MM/AAAA)`, en modo fecha `MM/AAAA (NN años)` (la edad solo si hay fecha de nacimiento configurada). Solo afecta al hover; los ticks del eje X no cambian.
- **Hover respeta el ajuste por inflación**: las cifras del tooltip (patrimonio neto, capital aportado, activos) usan ahora las series deflactadas, coincidiendo con el eje Y cuando el toggle "ajustado a inflación" está activo. Antes mostraban valores nominales aunque el resto del chart estuviera en "dinero de hoy".

## [1.4.0] — 2026-05-19

Refresca de UI completa (V1 redesign) + iteración de rendimiento end-to-end sobre `/v1/projection/series` (server cache + compresión + formato más liviano + densidad híbrida + two-phase loading + skeletons). Reglas y tokens completos en [`.claude/design-system.md`](.claude/design-system.md).

### Backend — Rendimiento de proyección

- **Cache in-memory de proyección**: `AppState` mantiene un `RwLock<HashMap<(installation_id, view, owner_user_id), Arc<ProjectionSeriesResponse>>>` con sliding TTL de 60 min. Hits sub-ms; misses delegan al cómputo full (extraído en `compute_projection_series_response`).
- **Invalidación por mutación**: cualquier handler que toca assets, liabilities, budget entries, planning flows, allocation rules, installation (inflation/FIRE/show_age_mode) o `user.birth_date` llama `refresh_projection_after_mutation(state, installation_id, user_id)`. Borra todas las entries del installation en background. Próximo GET recomputa una vez.
- **Invalidación por logout**: `POST /v1/auth/logout` borra las entries `view=mine` del usuario; las `view=household` siguen disponibles para otros miembros.
- **Warm-up post-login**: tras `POST /v1/auth/login` exitoso, `tokio::spawn` recomputa `view=household` y guarda en cache. El primer GET tras login es hit. Si el usuario no es miembro de ningún installation (caso pending), skip silencioso. Sin warm-up tras mutación: evita una race condition donde dos warm-ups concurrentes podían dejar el cache stale.
- **Compresión gzip** vía `tower_http::compression::CompressionLayer`. Reduce el response de `/v1/projection/series` de ~260 KB a ~30 KB y aplica a todos los endpoints >1 KB.
- **Arrays grandes serializados como `f64`** en `ProjectionSeriesResponse`: `points[].net_worth`, `points[].contributed_capital`, `fire_target_series`, `asset_series[].values`. Reduce ~30 KB extra el JSON y elimina ~5.000 llamadas a `parseDisplayDecimal` en el cliente. Los KPIs escalares y totales (`starting_net_worth`, `jubilacion_target_net_worth`, milestones) siguen como Decimal-as-string — la precisión decimal se mantiene donde importa.
- **`?density=hybrid` + two-phase loading**: `/v1/projection/series?density=hybrid` decima los arrays grandes a un patrón mixto (mes 0..12 mensual + mes 24, 36, ..., months anual) → ~82 puntos en lugar de ~841, JSON ~5 KB. El cliente lanza `hybrid` + `monthly` en paralelo y reemplaza con `startTransition` cuando llega el full. Warm-up post-login calienta ambas densidades. El cómputo interno del engine no cambia (840 meses); milestones y FIRE crossover siguen calculados sobre el array completo para no perder precisión.
- **Refactor del chart a `monthIndex`**: `ProjectionNetWorthChart` ahora calcula coordenadas X a partir del `month_index` real de cada punto (no del índice de array), lo que soporta densidades mixtas sin distorsión. `viewWindow` opera en meses (`startMonth`, `monthSpan`); pan/zoom es invariante respecto a la densidad servida.
- **Skeleton frames** en los 3 sitios donde había layout shift al cargar (Proyección, Resumen, Jubilación). Tres variantes en `App.css`: `.ff-chart-skeleton` (480 px chart grande), `--mini` (170 px MiniProjection) y `--donut` (220 px desglose Resumen). Los paneles siempre se renderizan con el placeholder y se reemplazan in-place cuando llega la data.

### Frontend — Adaptación al nuevo formato

- `ProjectionPointApi`, `AssetSeriesApi` y `ProjectionSeriesApi.fire_target_series` usan `number`/`number[]` en lugar de `string`/`string[]`. `MiniProjection` y `ProjectionNetWorthChart` consumen los valores directamente sin parsear.
- Nuevo helper `formatCurrencyOrDashNumber` en `lib/format.ts` para los hover labels del chart grande que ya reciben `number | undefined`.

### Frontend — Identidad visual

- **Paleta nueva**: base monocromática zinc (blanco roto `#f4f4f5` en claro / casi-negro `#0a0a0a` en oscuro) + único acento periwinkle (`oklch(0.56 0.13 250)` / `oklch(0.74 0.11 250)`). Verde/rojo se reservan exclusivamente para texto de cifras delta (deltas, saldos, `−€640`); fuera del chrome decorativo.
- **Modo oscuro**: `<html data-theme="dark|light">` controlado desde `Ajustes → Datos y sistema → Apariencia`. Preferencia `auto` (sigue `prefers-color-scheme` y reacciona en vivo) / `light` / `dark`, persistida en `localStorage`. Helpers en `apps/web/src/lib/theme.ts`.
- **Tokens centralizados**: `apps/web/src/styles/theme.css` define todos los colores, radii y sombras como CSS vars (`--ff-*`, `--proj-*`). `App.css` ya no contiene hex hardcoded.
- **Iconografía unificada**: set único en `components/icons.tsx` (viewBox 16×16, stroke 1.5, `currentColor`). ~25 iconos consistentes.

### Frontend — Shell

- **TopBar única** sustituye al header + tab-bar. Marca a la izquierda, pills de navegación derecha, selector de vista (mío/hogar) anclado en esquina superior derecha vía slot `extras`, botón hamburguesa visible solo en `≤720px`.
- **Cuenta movida a Ajustes**: nueva tarjeta destacada `AccountCard` con avatar + badge de rol + botones Editar cuenta / Cerrar sesión. La cabecera queda limpia.
- **Móvil**: drawer lateral derecho (`MobileNavDrawer`) con todas las secciones, sin bottom-nav.
- **Ancho del contenido**: 66rem centrado en escritorio (`.app-main`). Proyección sigue siendo full-bleed.

### Frontend — Componentes

- **`MetricCard`**: reserva siempre el slot del paréntesis (con `&nbsp;` cuando vacío) para que dos KPIs en la misma fila tengan baseline alineada. Soporta `tone="hero|accent|accent-2"`.
- **`MiniProjection`**: nuevo SVG compacto reutilizable con el lenguaje visual de la proyección grande. Usado en Resumen (12 m, zoomY) y Jubilación (`clampToMonth=jub+12`, zoomY, `xAxis` con edad/fecha). Las áreas se escalan proporcionalmente a `NW(t)` — replica la lógica del chart grande — por lo que **la suma de áreas == NW** y nunca exceden la línea NW.
- **`PlanningDirectionChart`** ahora también se usa en Presupuesto (panel "Distribución" con ingresos/gastos), no solo en Próximos.

### Frontend — Vistas

- **Resumen**: orden `KPIs → Salud financiera → Proyección 12 m → Desglose`. El chart de 12 m usa `zoomY` para que la línea NW vaya de esquina a esquina.
- **Jubilación**: el chart se reconecta al motor (recarga `/v1/projection/series` tras guardar FIRE), ahora muestra eje X con edad/fecha según config, recorta a `jub + 12 meses` cuando hay cruce y zoom Y entre NW(hoy) y NW(fin). Marcadores circulares (antes salían ovalados por `preserveAspectRatio="none"`; ahora el viewBox se mide con `ResizeObserver`).
- **Pasivos**: oculta la columna "Tipo" de la tabla.
- **Presupuesto**: nuevo panel "Distribución" con barra inflow/outflow (mismo widget que Próximos).
- **Ajustes**: account card arriba (todas las sub-tabs), sub-tabs como pills (no tab-bar), nueva sección "Apariencia" en "Datos y sistema" con toggle de tema.

### Frontend — Proyección (chart grande)

- **Tokens de color**: hex hardcoded (`#047857`, `#b45309`, `#7c3aed`, etc.) sustituidos por `var(--proj-*)`. La composición, hover, zoom, leyenda y tooltips quedan idénticos en claro.
- **Modo oscuro funcional**: paleta de áreas (`--proj-area-1..10`) con tonos medios en claro y pasteles más claros en oscuro para mantener contraste.
- **Tooltip independiente del tema**: forzado a `color: #fafafa` + bg `rgba(10,10,10,0.92)`. El bug previo causaba texto oscuro sobre fondo oscuro en modo oscuro.
- **Leyenda con espaciado dinámico mejorado**: `legendCharPx 6.5 → 7.6`, budget `0.6 → 0.66` del plot. Antes subestimaba anchos y los items adyacentes se pisaban.
- **Milestones con anti-colisión**: si dos milestones quedan cerca horizontalmente, el segundo sube al siguiente carril (14 px arriba) y la línea punteada se estira automáticamente hasta la nueva `y2`, manteniendo continuidad visual.

### Frontend — Infraestructura

- Nuevo `apps/web/src/styles/` con `theme.css` (tokens). Importado primero en `main.tsx`.
- Nuevo `lib/theme.ts` con `ThemePref`, `applyTheme`, `loadThemePref`, `saveThemePref`, `subscribeSystemThemeChanges`.
- Nuevos componentes: `TopBar`, `MobileNavDrawer`, `AccountCard`, `ThemeToggle`, `MiniProjection`.
- `loadSummaryPage` ahora carga la serie de proyección en paralelo con el summary (para alimentar el MiniProjection del Resumen).
- `saveFireSettingsPatch` recarga la serie de proyección tras guardar (para que el chart de Jubilación reaccione sin cambiar de pestaña).
- **Prefetch secuencial de chunks lazy y datos tras login**: `prefetchOtherViews` en `App.tsx` espera a que termine la pestaña actual (`currentTabBusy` derivado del `*Busy` correspondiente) y luego, dentro de un `requestIdleCallback`, encadena en serie los `import("./views/XxxView")` y `loadXxxPage()` del resto (`projection → assets → liabilities → budget → retirement → upcoming → settings`). Sin saturación inicial. `AbortSignal` cancela el prefetch en logout; `prefetchedRef` evita re-dispararlo al cambiar de pestaña. La pestaña Proyección (chunk pesado: `ProjectionNetWorthChart` 1.032 LOC + `lib/projection-chart.ts` 442 LOC) abre instantánea tras la primera pestaña.
- **`ProjectionNetWorthChart` aislado en su propio chunk**: dentro de `ProjectionView` se carga con `React.lazy`. El `<Suspense>` muestra `.ff-chart-skeleton` (placeholder con altura del chart) mientras se descarga el chunk y se calcula la geometría. Sin layout shift.
- **`startTransition` al setear `projectionSeries`**: los 3 setters (`loadSummaryPage`, `loadProjectionSeriesPage`, `loadRetirementPage`) envuelven `setProjectionSeries(data)` en `startTransition()` para que React priorice inputs/clics mientras reconcilia el SVG pesado.
- **`useMemo` del chart partido en sub-memos**: `ProjectionNetWorthChart` divide el `model` monolítico en `baseSeries` (deflactación + stacking, sin viewWindow), `xTicksAll` (ticks del horizonte completo) y `model` (slicing + yTicks + markers, lo único que cambia con pan/zoom). Pan/zoom dejan de recalcular deflactación y stacking, ~85% del compute previo.
- **Memoización en charts livianos**: `MiniProjection` envuelve todo el compute (parseo, escalas, stacks, jubMonth) en un `useMemo`; antes recomputaba O(assets × months) en cada render del padre. `SummaryDonutChart` memoiza el `conic-gradient` y el filtrado de filas.

### Dev tooling

- Nuevo `docker-compose.split-dev.yml`: override que expone Postgres en `127.0.0.1:5432`, necesario cuando se usa `cargo run` local en lugar del contenedor. Ver [`.claude/env-and-config.md`](.claude/env-and-config.md).

### Documentación

- Nuevo doc [`.claude/design-system.md`](.claude/design-system.md) con tokens, paleta y reglas para añadir UI nueva.
- `.claude/frontend-structure.md` y `CLAUDE.md` actualizados con los nuevos componentes y convenciones.

## [1.3.0] — 2026-05-18

Refactor profundo de base interna sin cambios funcionales visibles para el usuario. Mismas cifras en pantalla, código más sano, +134 tests añadidos, frontend partido en módulos.

### Backend — Operaciones limpias
- **Los GET ya no mutan la base de datos**: `GET /v1/liabilities`, `/summary`, `/budget`, `/assets`, `/projection` filtran los pasivos vencidos (`payment_end_date < today`) en vez de borrarlos físicamente. La función `purge_expired_liabilities` y su llamada desde los 6 handlers fue eliminada. Los datos vencidos persisten en BD (útil para auditoría) pero no aparecen en las consultas.
- **Reparación automática de migraciones eliminada**: el bucle `IDEMPOTENT_MIGRATION_REPAIR_VERSIONS` (12 rondas con checksum-repair) desaparece. `sqlx::migrate!().run()` corre directo. Drift real ahora falla en lugar de quedar enmascarado.
- **Pool de Postgres con tuning real**: `idle_timeout=10min`, `max_lifetime=30min`, `min_connections=1`. Antes las conexiones flotaban indefinidamente.
- **Límites de cuerpo de request**: 1 MB global, 16 MB en `/v1/backup/user-import` (donde se descomprime gzip). Devuelve 413 si se excede.

### Backend — Rendimiento
- **`spawn_blocking` en proyección**: los ~70 años × 12 meses × N activos × cascada con `Decimal::powd` ya no bloquean el reactor Tokio. `GET /v1/projection/series` sigue dando el mismo output bit-exact.
- **Doble simulación en paralelo**: el marker `compound_outpaces_true_savings_month_index` (que necesita una segunda simulación neutralizando planning + liabilities) ahora corre con `tokio::join!` junto a la principal. ~50% menos latencia al usuario.
- **Queries del handler de proyección consolidadas**: 7 fetch secuenciales (assets, allocation_rules, liabilities, planning_flows, installation, user, asset_names) → 2 `tokio::try_join!` paralelos.
- **Gross-up FIRE por forma cerrada**: la búsqueda binaria de 90 iteraciones sobre tramos fiscales se sustituye por la fórmula cerrada por tramos (la función `después-de-tax(gross)` es lineal por tramo, despejas el tramo correcto). Resultado idéntico ±0.01 €.

### Backend — Refactor
- **Helper `LedgerView` con fragmento SQL**: `scope_where(table_alias)`, `next_arg_index()`, `bind_scope_as`, `bind_scope_scalar`. Los 6 handlers que tenían `match view { Household => "WHERE installation_id = $1", Mine => "WHERE installation_id = $1 AND owner_user_id = $2" }` ahora consumen el helper. ~500 LOC menos y elimina la clase de bug de "binds invertidos entre ramas" (ya había un caso vivo en `budget.rs` con el orden de placeholders del derived-from-liabilities).
- **`impl From<sqlx::Error> for ApiError`**: detecta SQLSTATE 23505 (`unique_violation`) → `ApiError::Conflict` (409) y 23503 (`foreign_key_violation`) → `ApiError::BadRequest`. Los `map_unique_violation` / `insert_conflict` ad-hoc en `auth.rs` y `pending_users.rs` desaparecen.
- **`FireNumberMode::Deserialize` estricto**: enviar `fire_number_mode: "foobar"` ahora devuelve 422 (antes silenciaba a default).
- **Código zombie eliminado**: `bump_contributed_series_with_purchase_basis` (parche para "binarios antiguos") y campo `fire_number_expense_adjustment_pct` (sin consumidor).
- **`fire_target_at_month_index` público en el crate engine**: el handler ya no duplica la fórmula `base × (1+r)^(years)`, la llama. Off-by-one entre handler y motor resuelto.

### Frontend — Split de `App.tsx`
De **10.384 LOC en un solo componente con 151 useState** a **~3.079 LOC de composición**, repartido en:

```
apps/web/src/
├── api/{client.ts, types.ts}         # wrapper fetch + tipos *Api
├── lib/{format,dates,ledger,fire,navigation,projection-chart}.ts
├── components/{Modal,MetricCard,icons}.tsx + components/charts/
├── views/{Summary,Assets,Liabilities,Budget,Upcoming,Retirement,Projection,Settings,AllocationRulesPanel}View.tsx
└── auth/BootstrapInstallationPanel.tsx
```

- **Code-splitting con `React.lazy` + `<Suspense>`**: las 7 vistas se cargan bajo demanda. Bundle inicial **351 kB → 263 kB** (gzip 105 → 84 kB, -20%).
- **Bug encontrado por la propia migración**: `RetirementView` pasaba `expense_regular_monthly_equivalent` al cálculo FIRE mientras el servidor usa `expense_retirement_monthly_equivalent`. Si el usuario marcaba gastos como `ends_at_retirement = true`, la previa del formulario y el target real del servidor podían diferir 2-3×. Corregido en los 4 sitios.

### Tests — De 22 a 156
Antes: 22 tests unitarios en `crates/engine`. Ahora: **156 tests** (84 backend + 72 frontend).

- **Backend integration (`apps/api/tests/`)**: nuevo crate de integración con `TestApp::spawn()` que arranca el router Axum completo sobre un esquema Postgres aislado por test. Helpers para `register_and_login_owner`, `post_json_with_cookie`, etc. 7 ficheros, 18 tests: smoke, liabilities_purge, body_limits, installation_patch, unique_violation, projection_marker, fire_parity.
- **Frontend Vitest**: 72 tests en `lib/format.test.ts` (29), `lib/dates.test.ts` (26), `api/client.test.ts` (10), `lib/fire.test.ts` (7).
- **Fixture compartida cliente↔servidor**: `apps/api/tests/fixtures/fire-parity.json` con 6 casos canónicos. Tanto `fire_parity.rs` (Rust) como `fire.test.ts` (TS) consumen el mismo JSON y verifican que llegan al mismo `target_nw` ±1 €. Si alguien toca tramos fiscales en un solo lado, uno de los dos suites falla.

### Otros
- Nuevo `apps/api/src/lib.rs` que expone `db`, `error`, `routes`, `state`, `auth`, `handlers` para que los tests de integración monten el router. `main.rs` pasa a usar la librería.
- **No hay cambios de API que rompan clientes existentes** salvo la eliminación de `fire_number_expense_adjustment_pct` (campo sin consumidor) y el rechazo estricto de `fire_number_mode` desconocido. El resto es bit-exact compatible.

## [1.2.0] — 2026-05-17

### Motor de proyección — Target FIRE móvil con inflación (breaking)
- **Target FIRE crece con la inflación cada mes** para preservar el poder adquisitivo del usuario en la jubilación. El motor compara el patrimonio (en euros nominales) contra `base × (1 + inflación%)^(meses/12)` mes a mes. Antes el target era plano (un escalar fijo), lo que hacía que activar/desactivar la inflación apenas moviera la edad de jubilación.
- **Modelo coherentemente nominal**: ingresos, gastos, aportaciones y rendimiento de activos se mantienen constantes en euros nominales — refleja la filosofía «haciendo lo que hago ahora, ¿qué tal voy?». El motor ya no deflacta el rendimiento (antes la serie estaba a medio camino entre real y nominal, lo que generaba comportamiento incoherente con un target plano).
- **Toggle `projection_includes_inflation` eliminado** (UI y DB). Ahora solo se introduce el % anual: `0` desactiva el target móvil (target plano en euros de hoy), `>0` activa la inflación que mueve el target.
- **Nuevo campo `fire_target_series`** en `GET /v1/projection/series`: serie del target FIRE ajustado por inflación, paralela a `points`. La UI dibuja una segunda curva (línea discontinua morada) sobre el gráfico de patrimonio para hacer visible el movimiento del target.
- **Migración `20260520120000_inflation_always_on.sql`**: `DROP COLUMN projection_includes_inflation`, `annual_inflation_assumption_percent NOT NULL DEFAULT 0`.
- **API breaking**: `PATCH /v1/installation` ya no acepta `projection_includes_inflation`. `annual_inflation_assumption_percent` pasa de nullable opcional a string requerida cuando se envía. El response `InstallationSnapshot` ya no incluye `projection_includes_inflation` y `annual_inflation_assumption_percent` es siempre string decimal (default `"0"`).
- **Engine breaking**: `ProjectionInput.inflation_annual_percent` y `fire_target_net_worth: Option<Decimal>` se reemplazan por `fire_target: Option<FireTarget { base_amount, annual_inflation_percent }>`. El struct `FireTarget` se re-exporta desde `futurefin_engine`.

### UI — Jubilación
- **Curva del target FIRE móvil en el gráfico de proyección**: nueva línea discontinua morada que muestra cómo crece tu objetivo con la inflación. La leyenda añade una entrada «Target FIRE».
- **Etiqueta de inflación reescrita**: `Patrimonio nominal · target FIRE +X% anual` (en lugar de `Dinero de hoy …`). Refleja con precisión que la serie ya no se deflacta.
- **Banner `Inflación a 0%`**: sustituye al antiguo «Inflación desactivada». Avisa que con 0% el target queda plano y la fecha objetivo puede ser optimista en términos de poder adquisitivo real.
- **Formulario de proyección simplificado** (Ajustes): desaparece el checkbox; solo queda el input `Inflación anual %` con copy explicativa.

## [1.1.1] — 2026-05-16

### UI — Proyección
- **Leyenda del gráfico de proyección rediseñada**: La leyenda pasa a ocupar la franja superior del gráfico justificada a la derecha, en lugar de apilarse a un lado robando espacio al área de trazado. Los items se reparten en filas con wrapping automático en función del ancho disponible y del número de activos visibles. Los headlines (scope, horizonte, inflación, Δ presupuesto) se mantienen anclados a la izquierda. `buildProjectionChartLayout` ahora acepta los labels de la leyenda y calcula el espacio vertical necesario para no solapar con los headlines.
- **Activos en la leyenda — orden y paleta**: Las series por activo se ordenan ascendentemente por su valor pico en la proyección (el activo más pequeño aparece primero, el más grande último). Nueva paleta menos saturada (azul/teal/verde) que favorece la lectura de las áreas apiladas. Las áreas de relleno bajan a `fillOpacity 0.14` con borde más marcado para mejorar contraste.
- **Milestone "Interés > ahorro"**: La tarjeta KPI "Interés compuesto · Supera al ahorro" desaparece del panel de Trayectoria proyectada. En su lugar, el momento se representa como una línea vertical en el gráfico con etiqueta, anclada al eje X y alcanzando la curva de patrimonio neto, igual que el resto de milestones (Jubilación, hitos de Planning). Es información in-situ sobre el cruce, en vez de un tile separado que repetía la fecha.

### UI — Activos
- **Target visible antes del valor con tooltip**: La celda Valor pasa de `1.234 € (Obj. 4,5K)` a `(Obj. 4,5K) 1.234 €`. Anteponer el objetivo deja claro de un vistazo qué cifra es la meta y cuál el actual. Cuando el activo ya supera el objetivo, el tag desaparece (el objetivo se considera cumplido). Si la proyección alcanza el objetivo en algún mes futuro, el tag muestra al hacer hover un tooltip `Objetivo alcanzado en MMM YYYY`. La fecha se computa a partir de `asset_series` (serie por activo del `GET /v1/projection/series`) cruzando con `anchor_date_ymd`.

### UI — Jubilación
- **Objetivo FIRE muestra anual y mensual equivalente**: Las tres tarjetas de modo (manual, gasto anual, ingreso actual) muestran ahora `12.000 € (1.000 €/mes)` en lugar de solo el anual. El equivalente mensual va en un span más pequeño y atenuado para no competir con el dato principal. Aplica para los tres modos.

### UI — Presupuesto y Próximos
- **Columna "Fin" eliminada del listado de Gastos**: La columna que mostraba `Jub.` / `2027-05` / `—` desaparece (ya solo quedaba para mostrar info redundante con el toggle del modal). El toggle de fin de gasto sigue editable desde el modal de edición de línea.
- **Próximos — "Panorama" → "Distribución"**: El panel inferior cambia de título para describir mejor lo que muestra (distribución de flujos pendientes por categoría/tipo, no un panorama temporal).

### CSS
- Drop de selectores muertos: `.projection-chart-legend--stacked`, `.projection-chart-compound-marker`, `.projection-chart-compound-label` (la leyenda ya no tiene modo stacked y el marker compound usa la clase genérica de milestones).
- Nueva clase `.retirement-mode-monthly` (gris claro, ~78% size, weight normal) para el equivalente mensual entre paréntesis.
- `.planning-dir-svg` fija altura a 14px (antes `max-width: 28rem; height: auto`).

## [1.1.0] — 2026-05-16

Versión consolidada que agrupa los cambios incrementales 1.0.13–1.0.20 publicados durante el día. Resumen para usuarios:

### Added
- **Asignación del sobrante mediante reglas en cascada**: nuevo concepto que reemplaza la configuración de aportaciones por activo. Las reglas viven a nivel de **Presupuesto** (accesibles vía el engranaje en el tile **Neto** de la KPI strip) y se evalúan en orden ascendente sobre el sobrante mensual (ingresos − gastos − cuotas de deuda + flujos puntuales de Próximos). Tipos: `fixed` (€/mes), `percent` (% del sobrante restante) y `remainder` (todo lo que quede). Cada regla puede llevar un tope opcional resoluble a euros:
  - `amount` — tope absoluto en €.
  - `months_expense` — N × (gasto mensual + cuotas de deuda).
  - `income_multiple` — N × ingreso mensual.
  El backend impone que exista exactamente una regla `remainder` sin tope (el sumidero) y que sea siempre la última; permite múltiples `remainder` con tope intercaladas (caso típico: "fondo de emergencia hasta 3 meses de gasto", que se salta cuando se llena).
- **API**: nuevos endpoints `/v1/allocation-rules/` (`GET`, `POST`, `PATCH`, `DELETE`, `POST /reorder`). El schema de backup `.ffbackup` sube a `schema_version 3` (v1 y v2 se migran descartando los campos heredados de contribución; el usuario reconfigura sus reglas tras importar).
- **Activos — objetivo visible en la columna Valor**: cuando una regla con tope apunta a un activo, la celda Valor muestra `Actual € (Obj. 4,5K)` con el target redondeado al centenar superior y abreviado igual que los milestones de la proyección. Funciona para los tres tipos de tope (`amount`, `months_expense`, `income_multiple`).

### Changed
- **Modelo de proyección**: el motor (`crates/engine`) deja de almacenar la configuración de aportación en `SimAsset` y la consume desde la cascada (`allocation_rules`). 20 tests del engine cubren los nuevos casos.
- **Esquema de base de datos**:
  - Nueva tabla `allocation_rules` (`20260519120000_allocation_rules.sql`).
  - **Drop limpio** de las columnas `monthly_contribution_fixed`, `contribution_remainder_weight`, `contribution_frequency`, `contribution_cap_kind`, `contribution_cap_value` en `assets` (`20260519120100_drop_asset_contribution_columns.sql`). La configuración previa de aportación automática **se pierde** en la migración; el usuario debe rehacerla como reglas en Presupuesto.
- **Presupuesto — UI**:
  - El acceso a "Asignación del sobrante" se mueve al **engranaje** del tile Neto (Modal). Antes era un panel inline que robaba espacio.
  - La columna **Tras jub.** desaparece del listado de Ingresos (el toggle sigue editable desde el modal de edición de línea).

### Fixed
- **Tablas — solape de botones de acción**: los botones de editar/eliminar ya no se solapan visualmente con el contenido de la columna anterior. Causa raíz: `.budget-row-actions { display: inline-flex }` se aplicaba directamente al `<td>` y rompía el modelo de tabla. Solución: envolver los botones en un `<div>` interno y dejar el `<td>` con `display: table-cell` por defecto. Afecta a 6 tablas (Activos, Pasivos, Ingresos, Gastos, Planning y Reglas).
- **Activos — columnas vacías por categoría**: las columnas **Compra**, **Δ compra**, **Rent. % a.a.** y **Aporte** se ocultan automáticamente por categoría cuando ningún activo tiene el dato. La columna **Líquido** desaparece de la tabla (sigue usándose internamente para drenaje).

### Migración / compatibilidad
- Backups `.ffbackup` v1 y v2 siguen siendo importables; los campos heredados de contribución por activo se descartan (no migran a reglas; el usuario reconfigura).
- Tras actualizar la imagen, **el primer arranque ejecuta las dos migraciones nuevas y deja los activos sin reglas de asignación configuradas**. Crea las reglas desde Presupuesto → engranaje del tile Neto.

## [1.0.20] — 2026-05-16

### Fixed
- **Tablas — fix definitivo del solape en celdas de acciones**: La causa raíz no era ni `display: flex` vs `inline-flex` ni la falta de sticky: era que `.budget-row-actions` (con `display: inline-flex`) se aplicaba **directamente al `<td>`**, sobreescribiendo el `display: table-cell` natural y sacando la celda del modelo de tabla. El navegador la renderizaba fuera de su columna, tapando contenido adyacente (visible especialmente en la tabla de Ingresos donde la columna **Importe mensual** quedaba completamente oculta tras los botones). Solución: los botones se envuelven ahora en un `<div className="budget-row-actions">` interno y el `<td>` se queda solo con `.asset-actions-cell` (display: table-cell por defecto). Se revierten los hacks de v1.0.18–v1.0.19 (sticky, ::before sombra, hover-bg). Aplica en 6 tablas (Activos, Pasivos, Ingresos, Gastos, Planning y Reglas).

## [1.0.19] — 2026-05-16

### Fixed
- **Tablas — columna de acciones ahora sticky**: El fix anterior (`inline-flex` + `padding-left` + `background-color`) no era suficiente. Ahora `.asset-actions-cell` usa `position: sticky; right: 0` para anclarse al borde derecho del wrapper scrollable; el `background-color` blanco (con hover coherente) garantiza que ningún texto desbordado de la columna anterior queda visible bajo los botones. Sutil sombra `::before` indica el corte cuando la tabla tiene overflow horizontal. Aplica a Activos, Reglas, Ingresos, Gastos, Planning y Categorías.

## [1.0.18] — 2026-05-16

### Fixed
- **Tablas — texto oculto bajo los botones de acción**: La regla `.budget-row-actions { display: flex }` aplicada directamente al `<td>` sacaba la celda del modelo de tabla en algunos navegadores y provocaba que el contenido de la columna anterior (cuando era largo + `white-space: nowrap`) se renderizara por debajo de los botones. Cambiado a `display: inline-flex`, que mantiene la alineación pero respeta el flujo de table cell. Adicionalmente, `.asset-actions-cell` recibe `padding-left: 1rem` y `background-color: #fff` (con hover coherente) para crear separación visual y evitar cualquier solape residual.

### UI
- **Activos — etiqueta del target**: `(≈ 4,5K)` cambia a `(Obj. 4,5K)`. El prefijo "Obj." es más claro como "objetivo" y deja inequívoco que el valor entre paréntesis es el target, no una aproximación del actual.

## [1.0.17] — 2026-05-16

### UI
- **Presupuesto — Asignación del sobrante en engranaje del tile Neto**: El botón al pie de Ingresos desaparece. En su lugar, el tile **Neto** de la KPI strip muestra un **engranaje** en su esquina superior derecha que abre directamente el Modal de Asignación del sobrante. Es un acceso secundario y discreto que ya no roba espacio visual.
- **Activos — Target compacto entre paréntesis**: La celda Valor pasa de `Actual / Target` a `Actual € (≈ 4,5K)`. El target se redondea **hacia arriba al siguiente centenar** y se abrevia con el mismo formato que los milestones de la proyección (K/M/B/T). Aplica para reglas con cap_kind `amount`, `months_expense` o `income_multiple`.
- **Presupuesto — sin columna "Tras jub." en Ingresos**: La columna desaparece del listado de líneas de Ingreso. El toggle `persists_after_retirement` sigue editable desde el modal de edición.
- **Tablas — botones de acción al borde derecho**: `.budget-row-actions` ahora usa `justify-content: flex-end`, así los iconos editar/eliminar quedan pegados al borde derecho de la celda (que ya estaba a `width: 1%; text-align: right`) en todas las tablas de Presupuesto, Activos y Reglas.

### Componentes
- `MetricCard` acepta nuevo prop opcional `action?: ReactNode` para mostrar un botón/icono en la esquina superior derecha. Sin breaking change para los usos existentes.
- Nuevo icono inline `GearIcon`. Nuevo helper `roundUpToHundred(n)`.

## [1.0.16] — 2026-05-16

### Changed
- **Activos — Target visible para todos los tipos de tope**: La celda **Valor** muestra `Actual / Target` también cuando la regla de asignación usa `cap_kind = 'months_expense'` (N × gasto + cuotas deuda) o `cap_kind = 'income_multiple'` (N × ingreso), no solo `'amount'`. El target se resuelve a euros en cada GET usando el presupuesto del scope. Cuando hay varias reglas con tope apuntando al mismo activo, se muestra el de la regla con **mayor prioridad** (la primera de la cascada).
- **Tablas — botones de acción al borde derecho**: La celda `.asset-actions-cell` ahora toma ancho mínimo y se alinea a la derecha. Los botones de editar/eliminar quedan pegados al borde derecho de la tabla en activos, pasivos, presupuesto y reglas de asignación.
- **Presupuesto — Asignación del sobrante como Modal**: El panel deja de ocupar el header de la página. En su lugar aparece un botón discreto `Asignación del sobrante · N reglas ↗` al pie de la columna de Ingresos. Al pulsar abre un Modal ancho con la misma tabla, banners de validación y modal anidado de crear/editar regla.

### API
- `GET /v1/assets`, `POST /v1/assets`, `PATCH /v1/assets/:id`: `contribution_target_amount` ahora se calcula desde la primera regla con tope (cualquier `cap_kind`), resolviendo `months_expense` y `income_multiple` a € con el ingreso/gasto/cuota de deuda mensual del scope.
- Nuevo helper interno `projection::monthly_income_expense_debt_for_view` reutilizable por otros handlers.

## [1.0.15] — 2026-05-16

### UI
- **Activos — tabla compactada**: Eliminada la columna **Líquido** (el dato sigue vivo en el modal y se usa internamente para drenaje y proyecciones, pero no aporta en la vista). Las columnas **Compra**, **Δ compra**, **Rent. % a.a.** y **Aporte** se ocultan por categoría cuando ningún activo de esa categoría tiene el dato, para que las tarjetas no muestren columnas en blanco.
- **Activos — Valor muestra objetivo**: Cuando una regla de asignación apunta al activo con `cap_kind = 'amount'` (tope en € concreto), la celda **Valor** pasa a mostrar `Actual / Target`. Los topes relativos (`months_expense`, `income_multiple`) no se muestran porque varían con el presupuesto. Si varias reglas amount-cap apuntan al mismo activo, se usa la más restrictiva.

### API
- `GET /v1/assets` y `POST/PATCH /v1/assets` devuelven nuevo campo `contribution_target_amount` (string decimal o ausente). Calculado como `MIN(cap_value)` de las reglas activas del scope con `cap_kind='amount'` y `target_asset_id = id`.

## [1.0.14] — 2026-05-16

### Changed
- **Reglas de asignación — invariante "regla resto sin tope al final"**: La regla `remainder` sin tope actúa como sumidero del sobrante y debe ser única por scope y siempre la última en la cascada. El backend ahora:
  - Al crear cualquier regla cuando ya existe el sumidero, la inserta automáticamente **antes** de él (sin tener que reordenar a mano).
  - Rechaza crear/editar una segunda regla `remainder` sin tope (`uncapped_remainder_exists`).
  - Rechaza un `reorder` que deje al sumidero en cualquier posición que no sea la última (`sink_must_be_last`).
  - Sigue exigiendo que haya exactamente un sumidero activo en el scope.
  - Las reglas `remainder` **con tope** siguen permitidas en cualquier posición previa (caso típico: "fondo de emergencia hasta 3 meses de gasto", que se salta cuando se llena).

### UI
- Sección "Asignación del sobrante" mejora copy: explica la cascada, los tres tipos de regla y el rol del sumidero. Banner amarillo cuando el sumidero no es la última regla (avisa de que las reglas posteriores recibirán 0 €). El modal de creación muestra una ayuda contextual según el tipo de regla seleccionado. La columna **Aporte** de Activos clarifica en tooltip que incluye los flujos de la pestaña Próximos.

## [1.0.13] — 2026-05-16

### Changed
- **Aportaciones a activos — reglas de cascada en Presupuesto**: La configuración de aportación automática deja de vivir en cada activo y pasa a ser una cascada de reglas globales (por usuario) gestionada desde la pestaña **Presupuesto**. Cada regla apunta a un activo destino, tiene un tipo (`fixed` €/mes, `percent` del sobrante restante, `remainder` para lo que quede) y un tope opcional (`amount` €, `months_expense` N×gasto+deuda, `income_multiple` N×ingreso). El motor evalúa las reglas en orden ascendente de prioridad sobre el sobrante mensual (ingresos − gastos − cuotas de deuda); si una regla alcanza su tope, se salta y el sobrante baja a la siguiente. Permite expresar prioridades naturales como "fondo de emergencia primero (hasta 3 meses de gasto), luego pensiones, resto a ETF". Reemplaza por completo el modelo anterior basado en `monthly_contribution_fixed` + `contribution_remainder_weight` + `contribution_cap` por activo, que se solapaba mal con casos reales (suma de fijas mayor que el sobrante, pesos confusos al sumar >100 %, falta de orden explícito).
- **Backup `.ffbackup` → `schema_version 3`**: nuevo formato que separa `assets` (sin campos de contribución) de `allocation_rules`. Backups v1/v2 se migran a v3 dropeando los campos heredados (el usuario reconfigura sus reglas tras importar).

### Removed
- **Columnas `monthly_contribution_fixed`, `contribution_remainder_weight`, `contribution_frequency`, `contribution_cap_kind`, `contribution_cap_value` en `assets`**: migradas a `allocation_rules` con migración `20260519120100_drop_asset_contribution_columns.sql`. Migración hermana `20260519120000_allocation_rules.sql` crea la nueva tabla. **No hay migración de datos** (drop limpio): la configuración previa de aportación automática se pierde y debe reintroducirse como reglas. UI relacionada (sección "Aportación automática" del modal de activo, columna "Aporte" recibida del backend, tarjeta KPI "Aporte mensual (est.)") se reorganiza en Presupuesto → Asignación del sobrante.

### API
- Nuevos endpoints `/v1/allocation-rules/` (GET/POST/PATCH/DELETE) y `POST /v1/allocation-rules/reorder`. Validación servidor: cada scope (hogar o por usuario) debe mantener al menos una regla `remainder` activa; intentar borrar la última devuelve `400 remainder_required`. Endpoints `/v1/assets/*` simplificados (sin los 5 campos eliminados).

## [1.0.12] — 2026-05-16

### Fixed
- **Motor de proyección — inflación unificada a modelo real puro**: Antes el motor mezclaba lógicas (deflactaba series al final, inflaba retiro en jubilación, inflaba FIRE target, dejaba ingresos/gastos/aportaciones nominales fijos). Esto causaba inconsistencias visibles (p.ej. drenaje de activos antes de la jubilación con inflación activa). Ahora toda la simulación opera en € de `ref_date`: la única aplicación de inflación es descontarla al rendimiento de cada activo (`r_real = (1+r_nominal)/(1+inf) − 1`). El `expected_annual_return_percent` que introduce el usuario se sigue interpretando como **nominal**. Comportamiento sin inflación inalterado. Las series devueltas por `GET /v1/projection/series` ya no requieren transformación cliente. Implica proyecciones más conservadoras (y realistas) para usuarios con inflación activa, porque el rendimiento real es menor que el nominal usado antes.

## [1.0.11] — 2026-05-16

### Added
- **Activos — tope de aportación automática**: Cada activo puede limitar su aportación recurrente a una cantidad fija (€) o a N meses de gasto (gasto mensual + servicio de deuda activo). Cuando el activo llega al tope, el motor de proyección redistribuye el flujo de ese mes al resto de activos según su cuota fija y peso sobre remanente; si todos están topados, el sobrante se acumula como caja. Migración `20260518120000_assets_contribution_cap.sql`. Backup `.ffbackup` sube a `schema_version 2` (v1 se migra a v2 con tope `None`).

### Changed
- **Motor de proyección — fallback del remanente sin pesos**: Si ningún activo elegible tiene `weight > 0` (todos solo cuota fija), el remanente del mes ya no se queda como caja: se aporta al activo **líquido** con mayor rentabilidad esperada (empate → reparto equitativo). Antes este caso enviaba el sobrante a `surplus_cash`. Aplica también cuando un activo topado libera flujo y los demás no tienen peso configurado.

## [1.0.10] — 2026-05-15

### Fixed
- **Backup `.ffbackup` — export rompía con 500**: La query SQL del export pedía `b.label` y `b.frequency` de `budget_entries`, pero esas columnas se eliminaron en la migración `20260505180000_budget_entries_monthly_only` (el presupuesto pasó a ser solo-mensual sin etiqueta libre). Ahora export e import omiten ambos campos; el schema `BackupBudgetEntry` ya no los incluye.

## [1.0.9] — 2026-05-14

### Added
- **Backup `.ffbackup` cifrado por usuario**: Sustituye al export ZIP/CSV. Cada usuario exporta solo sus filas (`assets`, `liabilities`, `budget_entries`, `planning_flows` con `owner_user_id = self`) en un contenedor binario versionado cifrado con AES-256-GCM. La clave se deriva de la contraseña de cuenta vía Argon2id (m=19456, t=2, p=1) con sal aleatoria por export; AAD incluye `schema_version`, `user_id` y `exported_at`. Endpoints: `POST /v1/backup/user-export`, `POST /v1/backup/user-import/preview`, `POST /v1/backup/user-import` (replace-only, transaccional). El manifest queda en claro para que el servidor pueda rechazar `schema_version` futuras sin intentar descifrar.
- **Planning — mostrar hitos en el gráfico**: Cada planning flow tiene un nuevo flag `show_in_chart` (solo activable si hay `due_date`). Los hitos marcados se renderizan como líneas verticales en el gráfico de proyección. Migración `20260517120000_planning_flows_show_in_chart.sql`.
- **Per-asset projection series**: `GET /v1/projection/series` ahora devuelve `asset_series[]` (un array por activo con su valor mes a mes, paralelo a `points`). Permite renderizar el desglose por activo sin recalcular en el cliente. El engine deflacta cada serie con el mismo factor que `net_worth` cuando hay inflación activa.

## [1.0.8] — 2026-05-14

### Added
- **Presupuesto — fin de gasto**: Las entradas de gasto recurrente ahora admiten una fecha de fin opcional. Dos modos: "Al jubilarse" (el gasto deja de computarse en la proyección a partir del mes de jubilación) o "Hasta la fecha" (el gasto se cancela a partir del mes indicado). Los gastos que terminan al jubilarse también reducen el objetivo FIRE calculado por el modo `AnnualExpense`.

## [1.0.7] — 2026-05-14

### Changed
- **Docker build**: Node.js build stage upgraded from 22.14 to 24.15 (Active LTS, EOL April 2028). Aligns the production image with CI, which already ran on Node 24.

## [1.0.6] — 2026-05-14

### Improved
- **Projection API**: `GET /v1/projection/series` now returns `jubilacion_month_index` and `jubilacion_target_net_worth` — the FIRE milestone is computed server-side (gross-up + SWR division already run by the engine layer) instead of being duplicated in the browser.

### Fixed
- **Projection engine — FIRE is the sole retirement trigger**: `projection_target_age` has been removed entirely. The engine no longer enters retirement due to age; only reaching the FIRE target net worth triggers the retirement phase. This eliminates the visual gap where the "contributed capital" line stopped growing years before the Jubilación milestone marker.
- **Projection horizon — fixed 90-year lifespan**: The chart horizon is now computed as 90 years from the oldest household member's birth date (clamped 5–70 years, 30-year fallback when no birth date is set), replacing the manual "target age" setting that has been removed.

## [1.0.4] — 2026-05-13

### Added
- **Projection — Jubilación milestone**: The projection chart now marks the month when net worth reaches the FIRE target with an amber vertical line labelled "Jubilación". The "Trayectoria proyectada" panel shows it as a metric card with the target net worth and the estimated date.

### Fixed
- **Projection engine — contributions stop at retirement**: New contributions to `contributed_capital` now stop as soon as the portfolio reaches the FIRE target net worth (or `retirement_start_month`, whichever comes first). Previously, any budget surplus in retirement (e.g. persistent pension income exceeding expenses) was still being invested and counted as new contributed capital. The API computes the FIRE target from `fire_settings` (same SWR + tax gross-up logic as the frontend) and passes it to the engine as `fire_target_net_worth`.

## [1.0.3] — 2026-05-13

### Added
- **Budget — Persiste tras jubilación**: Each income entry now has a "Persists after retirement" toggle (default off). Income items marked as persisting continue to contribute to cash flow after the retirement age; all others stop. This drives a more realistic FIRE projection and a lower FIRE wealth target when passive/pension income is present.
- **Projection engine**: `income_retirement_monthly` field in `ProjectionInput`; simulation loop switches income at `retirement_start_month` instead of keeping it flat for the full horizon. `retirement_monthly_withdrawal` is always 0 — the income drop alone drives the portfolio drain.
- **FIRE target**: Annual need now subtracts persistent retirement income (`max(0, expense − income_retirement) × 12 / SWR` in annual-expense mode; `max(0, income − income_retirement) × 12 / SWR` in current-income mode).
- **Registration**: `birth_date` is now a required field at sign-up (was optional and had to be set separately).
- **Dev workflow**: `docker-compose.local.yml` + CLAUDE.md instructions for full-stack local testing without publishing to Docker Hub.

## [1.0.2] — 2026-05-12

### Fixed
- Docker healthcheck changed from `CMD` (exec form) to `CMD-SHELL` so `curl` resolves correctly via shell PATH; bash `/dev/tcp` fallback for images without `curl`
- `RUST_LOG` added to `docker-compose.yml` so container logs are visible by default

### Improved
- Startup log milestones: version, database connected, migrations applied, server config (port, session TTL, cookie_secure)

## [1.0.1] — 2026-05-12

### Infrastructure
- Single `docker-compose.yml` for production (Docker Hub image, no TLS overlay)
- Only `POSTGRES_PASSWORD` is required; all other vars have sane defaults
- `apps/api/Dockerfile` runtime stage now includes `curl` (required for healthcheck)
- Dev tooling (`CLAUDE.md`, `.claude/`, `.github/`) removed from `main` branch

## [1.0.0] — 2026-05-12

### First public release

**Auth & multi-user**
- Username + password authentication (Argon2id), session cookie `ff_session`
- Singleton installation per deployment; first user becomes owner automatically
- Owner approves pending registrations; roles: `owner`, `member`, `viewer`
- Household view (`default`) and personal view (`?view=mine`) scoped by `owner_user_id`

**Financial ledger**
- Assets: value, purchase price (cost basis Δ), liquidity flag, expected annual return, fixed + weighted contributions, weekly/monthly frequency
- Liabilities: principal (manual or derived from payment plan), APR, weekly/monthly payment schedule, auto-expiry
- Budget: persisted monthly income/expense lines; liability-derived debt payments included in snapshot
- Planning flows: upcoming one-off inflows and outflows with optional due dates
- Categories: CRUD per scope (asset, liability, income, expense)

**Analytics**
- Summary: net worth, total assets/liabilities, debt-to-assets ratio, financial health metrics (savings rate, runway, upcoming coverage), category and type-tag breakdowns
- Projection: monthly net-worth series via `futurefin-engine` (compound growth, debt service, asset contributions, planning cash adjustments, optional inflation deflation)
- FIRE / Jubilación tab: FIRE number modes (manual, annual expense × SWR, current income), capital-gains tax brackets, gap to target

**Infrastructure**
- Axum API, all routes under `/v1/`, OpenAPI at `/openapi.json`
- PostgreSQL + SQLx migrations (auto-run on startup)
- React + TypeScript + Vite SPA embedded in the Docker image
- Docker image: multi-arch (`linux/amd64`, `linux/arm64`), published to GHCR on `vX.Y.Z` tags
- NAS deploy: `docker-compose.yml`, imagen desde Docker Hub
- Backup: `GET /v1/backup/export.zip` (CSV ZIP, owner only)
