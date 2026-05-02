---
name: FinFuture product dossier
overview: "FutureFin self-hosted: paridad de capacidades con el Mac salvo artefactos desktop y salvo primer arranque (sin demo, sin hogar/categorías por defecto). Implementación libre. Backup monofichero + CSV como capacidad. Sin migración legacy."
todos:
  - id: spec-auth-model
    content: "Documentar modelo multi-usuario: hogares, membresías, roles y visibilidad por persona/dataset"
    status: completed
  - id: parity-checklist
    content: "Inventario capacidades vs Mac; exclusiones: arquitectura desktop + sin demo/sin seed/sin categorías por defecto (estado vacío + errores explícitos); tests/oráculos; implementación libre"
    status: completed
  - id: backup-archive-spec
    content: Especificar backup monofichero (cifrado, versión de esquema, alcance hogar/tenant) + contrato de ZIP/import CSV alineado a CSVService — formatos nuevos; sin lectura de export Mac legacy
    status: completed
  - id: oracle-tests
    content: Definir conjunto de casos/regresión (fixtures numéricos) tomados de SummaryService y tests Swift existentes
    status: completed
  - id: repo-split
    content: Repo nuevo como codebase principal; política de versionado y comunicación de sunset del binario macOS (deprecación)
    status: completed
  - id: doc-sync-readme
    content: "Cuando toque documentar: alinear README con código (pestañas, FIRE, reglas micro) o sustituir por spec generada desde tests"
    status: completed
  - id: mac-deprecation-comms
    content: "Comunicación de sunset del cliente macOS: sin soporte, migración ni compatibilidad con el producto obsoleto"
    status: completed
isProject: false
---

# Dossier de producto: FutureFin / FinFuture

> Copia archivada del plan Cursor (`finfuture_product_dossier_122c7357`). Rutas `file:///…` apuntan al repo Swift local **FinFuture** en la máquina de desarrollo (ajustar si clonas en otro sitio).

## Estrategia de línea de producto: refactor completa, independiente, futuro único

La versión **self-hosted en Docker** (nuevo repositorio / nuevo stack) **no es un “port” menor**: es una **refactorización completa y autónoma** del producto — la **línea que cuenta de aquí en adelante**. La aplicación **macOS actual** (`~/Documents/GitHub/FinFuture` o `FinFuture`) quedará **deprecada**: sin evolución funcional prevista más allá de lo imprescindible; el esfuerzo de ingeniería y el roadmap se concentran en la nueva base.

- **Independencia:** código, despliegue, persistencia y multi-usuario se diseñan en el nuevo proyecto sin arrastrar SwiftPM, SwiftUI ni SQLite local del escritorio como runtime del producto final.
- **Rol del código Mac durante la transición:** sirve como **especificación ejecutable** y **oráculo de paridad** (reglas micro, tests, capturas de comportamiento) hasta que el MVP nuevo cubra el uso real. No hay compromiso de **importar, convertir ni soportar** datos del cliente macOS: ese producto queda **obsoleto** y **fuera de compatibilidad**.

---

## Fuente de verdad del comportamiento legado: código Swift y tests, no el README

La documentación en repo macOS (p. ej. `README.md`) está **desactualizada** frente al comportamiento real. Para definir **qué debe igualar** la nueva app, este dossier prioriza el **código Swift** como contrato: `src/core` (dominio + motores), `renderer` (órdenes de datos y UX), y `tests`. La utilidad frente a Excel está en las **reglas micro**; la nueva implementación debe reproducirlas con precisión.

El **README del nuevo repo** (cuando exista) debería describir el producto vigente (servidor, Docker, multi-usuario), no depender del texto legacy del Mac.

Archivos de referencia recurrentes: `Domain.swift`, `SummaryService.swift`, `AppState.swift`, `FinFutureApp.swift`.

**Ejemplo de divergencia doc ↔ código:** el README habla de “seis pestañas”; la app registra **siete** (`Summary`, `Assets`, `Liabilities`, `Budget`, `Upcoming`, **Retirement/FIRE**, `Projection` en `FinFutureApp.swift`). El Summary en UI ya incluye una **fila FIRE** (`SummaryFireRow` en `SummaryTabView.swift`), no reflejada en la descripción antigua del README.

---

## MVP Docker: paridad de capacidades (referencia Mac) y libertad de implementación

**Must-have por defecto:** **todas** las **funcionalidades orientadas al usuario** que existan en el cliente macOS según el **código** (pestañas principales, Settings en tres ejes Hogar/miembros · Backups · Categorías, filtros por persona, formularios, gráficos, FIRE completo, import diagnostics cuando aplique, etc.) entran en el **MVP** del producto Docker/web.

**Excepción — necesidades de arquitectura Mac o primer arranque legacy:** no es obligatorio replicar **mecanismos que solo existen por el desktop Apple**. Ejemplos típicos: migración automática **FinFuture → FutureFin** bajo `~/Library/Application Support`, rutas locales SQLite del usuario, `FolderDialog` / panel del Finder, ventana con **autosave de frame** / tabbing de `NSWindow`, script `build_macos_app.sh`, `About` como `NSAboutPanel`. **También quedan fuera de paridad** el hogar de muestra, `demo()` ante fallo de BD y las **categorías por defecto** insertadas automáticamente al estar vacío — véase el apartado “Sin datos de demo…” más abajo. Donde haya **valor de usuario equivalente** (p. ej. “acerca de” con versión y créditos), se entrega con patrones web normales; donde no aporte en servidor, se omite.

**Regla de ingeniería:** hay que implementar **hacia el resultado funcional** (qué puede hacer el usuario y qué números ve), **no** hacia “copiar el Swift línea a línea”. Si una alternativa es **más eficiente, mantenible y adecuada** al entorno (batch en servidor, event sourcing, cálculo incremental, otro formato de serialización, UX sin sheets nativos, etc.) siempre que **preserve los mismos contratos micro** validados por tests/oráculos, **debe preferirse**.

**Backups e intercambio de datos (capacidad, no forma Mac):**

- **Backup/restauración en archivo único** con contraseña (equivalente de producto al `.ffbackup`): **MVP**; formato binario **libre** y adaptado al stack; contenido al menos equivalente al snapshot + FIRE settings como en `BackupArchiveService`.
- **Export/import CSV multi-dataset** como en `AppState.importExportCSVFilenames` / pestaña Backups de `SettingsView`: **MVP como capacidad** (mismos conjuntos de datos, misma tolerancia de parsing que `CSVService`, informes de diagnóstico donde existan). La **UX** no tiene que ser “elegir carpeta”: puede ser **descarga/subida de ZIP**, endpoints API, o flujo web equivalente.

**Obsolescencia:** **ninguna** migración ni compatibilidad con datos/export del **cliente Mac deprecado**. Los formatos nuevos (ZIP CSV, backup monofichero propio) son los únicos soportados en la nueva línea.

**Persistencia en runtime:** motor de BBDD / almacenamiento **implementación libre** (no se exige SQLite ni rutas del Mac).

**Sin datos de demo ni categorías por defecto (decisión de producto explícita):** el cliente Mac hoy puede arrancar con **hogar de muestra**, **`AppState.demo()`** si falla SQLite y **`defaultCategories`** / siembra cuando la BD está vacía (`seedDefaultData`, `defaultCategories`). **El MVP nuevo no debe replicar ninguno de esos comportamientos.** No existe modo demo en memoria; si la persistencia falla, el servicio debe **fallar de forma explícita** (error, salud degradada, sin datos ficticios). El **primer uso** es **estado vacío** hasta que el usuario **cree** hogar/miembros/categorías o **importe** backup/CSV válido — la UI debe guiar esa creación sin inventar filas ni categorías predefinidas. Esto es una **desviación intencionada** respecto al primer arranque del Mac, no un bug de paridad.

---

## Principio de diseño: “Excel con esteroides”, sin botón de calcular

La experiencia que debe preservarse es la de una **hoja de cálculo avanzada**: **no hay pasos de “Calcular”, “Recalcular” o “Aplicar”** para que los totales, gráficos, FIRE o la serie proyectada se actualicen. En el Mac esto emerge de `@Observable` / vistas que leen propiedades derivadas (`summarySnapshot`, `netWorthProjectionPoints`, `fireMilestone`, etc.) que se **recomputan al cambiar cualquier input**.

Para el MVP Docker/web:

- Cualquier cambio en un campo **debe propagarse de inmediato** a KPIs, tablas y gráficos afectados (misma sensación que Excel con fórmulas encadenadas).
- Evitar modales que “congelen” el estado hasta confirmar, salvo donde el Mac ya lo haga por persistencia explícita (p. ej. guardar formulario de ítem equivale a cerrar hoja en SwiftUI).
- La sensación de latencia debe ser mínima en LAN; los motores pesados pueden optimizarse después sin cambiar este principio de producto.

---

## Contratos “micro” que solo están garantizados en código (muestra representativa)

Para cualquier implementación, estos **resultados numéricos y reglas** deben coincidir con los **oráculos** (código Swift + tests); no se deducen del README. **Cómo** se calcula en servidor (sincrónico vs job, caché, tipo Decimal en otro lenguaje) es secundario si el **contrato observable** es el mismo.

- **Presupuesto vs proyección:** la serie de patrimonio (`netWorthProjectionPoints`) usa `filteredRegularBudgetEntries`: ingresos/gastos **persistidos**, **sin** las filas derivadas de planes de pago de deuda. El Budget tab muestra `filteredBudgetEntries` = regulares **más** derivadas (`derivedBudgetEntriesFromPaymentPlans`). Es una bifurcación deliberada: la UI de presupuesto refleja cash-flow con cuotas; el motor de proyección evita doble contar pagos que ya restan vía `projectedMonthlySavings`.

- **Pagos de deuda en simulación:** `projectedMonthlySavings` resta solo cuotas cuyo plan está **activo** en ese mes (`isDebtPaymentActive`: si hay `paymentEndDate`, el mes cuenta si `paymentEndDate >= inicio de mes`).

- **Upcoming fechado vs sin fecha:** en `projectNetWorthSeries`, flujos **con** `dueDate` aplican su efecto **acumulado** hasta cada punto; los **sin** fecha usan un reparto **lineal** sobre **90 días** desde la fecha de referencia (`distributedUndatedUpcomingAdjustment`). Los ingresos positivos sin fecha también alimentan un “boost” mensual para aportaciones (`upcomingPositiveAdjustmentForContribution`) con la misma ventana.

- **Orden de drenaje por gastos próximos:** `drainExpenseFromAssets` ordena activos: primero **líquidos**, luego menor `annualChangeRate` esperado; el remanente no cubierto incrementa `cumulativeUndrainedExpense` y retrocede parte del capital aportado en la fórmula del net worth.

- **Aportaciones a activos:** primero contribuciones **fijas** (mensual o weekly→×52/12); si el total fijo supera el ahorro del mes, se **escala** proporcionalmente (`fixedScalingFactor`). El remanente (más boost de upcoming positivo) se reparte entre activos con **% del restante**, normalizando si la suma de porcentajes supera 100.

- **Principal de pasivo derivado en UI:** en `LiabilitiesTabView`, “Derive principal from recurring payment” calcula principal = `paymentAmount × número de intervalos` desde **hoy** (start of day) hasta **fin de plan**: mensual = componente `.month` del intervalo; semanal = `ceil(días/7)`. Si no se deriva, el plan de pago puede ir vacío en persistencia.

- **Filas derivadas de presupuesto:** solo se emiten si hay `paymentAmount`, `paymentEndDate` **y** `paymentEndDate > today`; representan “siguiente mes” de cuota con nota fija (`Derived from payment plan`). Pasivos con plan vencido se **eliminan** al arrancar (`purgeFinishedPaymentPlans`).

- **Horizonte de meses:** `projectionHorizonYears` acota entre **5 y 70** años; si hay varias personas en alcance “hogar”, gobierna el **máximo** de años hasta `projectionTargetAge`; sin fechas de nacimiento, fallback **30** años.

- **Runway y cobertura próxima:** runway usa solo activos con `isLiquid == true`. Ratio de cobertura upcoming = ingresos próximos / gastos próximos (evita división por cero en servicio).

- **Baseline para hitos en gráfica de proyección:** `upcomingNetForMilestoneBaseline` suma flujos con fecha solo si caen en **ventana 0–90 días**; los **sin** fecha suman el **importe completo** al baseline usado en `ProjectionTabView` — coherente que debe preservarse si se portan los mismos marcadores.

- **FIRE:** fases al incorporar pensiones; impuesto por tramos sobre pensión; ratio ganancia no realizada respecto a valor para CGT; búsqueda binaria sobre capital necesario; comparación del objetivo de la **fase activa** con la serie proyectada. Textos en Retirement aclaran **dinero de hoy** cuando inflación está activa en ajustes del hogar.

- **Ordenación Budget:** líneas ordenadas por total de categoría descendente, luego categoría, importe, nombre (`BudgetTabView.sortedEntries`) — detalle de UX pero fija expectativa del usuario.

---

## Visión de producto

**Producto objetivo (nueva línea):** aplicación de finanzas personales **self-hosted**, accesible desde **cualquier dispositivo**, multi-usuario, centrada en hogar con **titularidad por persona**, presupuesto mensual, flujos próximos, **proyección de patrimonio** y módulo **FIRE/jubilación** — misma ambición analítica que el Mac, sin depender del escritorio Apple.

El prototipo Mac fue **local-first** por naturaleza de Swift; el futuro del producto es **servidor como fuente de verdad** bajo control del usuario (Docker), con soberanía de datos y un **backup en archivo único** adaptado al entorno (misma función que el export cifrado del Mac, formato nuevo).

Preferencia de producto: **multi-usuario con login propio**, lo que exige **cuentas, hogares, membresías y permisos** y políticas de seguridad para datos financieros en red — capacidades que la nueva línea incorpora de forma nativa, no como parche del modelo monopuesto del Mac.

---

## Core features (must-have de capacidad; mismo modelo conceptual)

Lista **orientativa** alineada con `Domain.swift`. El inventario cerrado del MVP debe extraerse de **todo** el árbol `renderer` + `core` (no solo esta lista). Persistencia en Mac vía `SQLiteStore`; en servidor, **otra implementación** con el mismo comportamiento persistido y expuesto.

- **Hogar:** nombre, moneda base (EUR / USD / GBP), flags de proyección (inflación, edad objetivo de horizonte 65–105), modo “mostrar edad” en ejes o etiquetas.
- **Miembros del hogar:** nombre, primario, fecha de nacimiento (clave para horizonte de proyección y pensiones).
- **Categorías personalizables** por ámbito: activos, pasivos, ingresos, gastos; borrado con **remap** si hay ítems usando la categoría (`AppState.remapCategoryItems`). En el MVP nuevo **no** se insertan categorías “de fábrica”; el usuario las define o las trae por import.
- **Activos:** valor, categoría, titular, opcional precio de compra y rentabilidad implícita, tasa de cambio anual esperada, **aportaciones recurrentes** (fijo mensual/semanal o **% del presupuesto restante**), bandera **líquido / no líquido**, notas.
- **Pasivos:** principal, categoría, etiqueta de tipo personalizada, APR opcional, **plan de pago** (importe, frecuencia, fecha fin); los planes vencidos se **purgan al iniciar** (`purgeFinishedPaymentPlans`).
- **Presupuesto mensual:** líneas de ingreso y gasto por persona y categoría.
- **Planeación “Upcoming”:** ingresos/gastos esperados con importe, categoría, opcional fecha de vencimiento, notas.
- **Persistencia, backup monofichero e intercambio CSV (nueva línea):** ver sección MVP — ambos caminos de datos son **must-have de capacidad** donde el Mac los ofrece al usuario; formatos y UX **adecuados al servidor**, sin leer `.ffbackup` ni zip legacy del Mac.
- **Validación de datos de entrada** (`ValidationService`): rangos sensatos en tasas, montos, nombres obligatorios, coherencia de aportaciones recurrentes, etc. — **sí** en paridad con Mac.

---

## Features derivadas (cálculos y vistas que emergen del núcleo)

No son “otra fuente de verdad”: dependen de los registros anteriores y de reglas en `SummaryService` + orquestación en `AppState`. La lista siguiente es **resumen**; el comportamiento exacto está en la sección de contratos micro y en los tests (`SummaryServiceTests`, `AppStateMetricsTests`, etc.).

### Resumen (Summary)

- Totales: **patrimonio neto**, activos, pasivos, ratio deuda/activos.
- Desglose por categoría (activos) y por categoría / etiqueta de tipo (pasivos).
- **Salud financiera mensual:** ingresos, gastos, ahorro, tasa de ahorro, **runway** (activos líquidos / gastos mensuales), totales de upcoming y ratio cobertura ingreso/gasto próximo.
- **Ahorro “sin planes de deuda”:** KPIs alternativos que suman de vuelta el impacto de filas derivadas de pasivos (`monthlySavingsWithoutLiabilityPaymentPlans`).

### Presupuesto / pasivos enlazados

- **Filas de gasto derivadas** de planes de pago activos de pasivos (`derivedBudgetEntriesFromPaymentPlans`): duplicarían categoría/importe del pasivo para reflejar cash-flow real sin que el usuario las escriba a mano.

### Proyección de patrimonio neto (pestaña Projection)

Motor mensual que combina:

1. **Ahorro mensual proyectado:** ingresos − gastos base − **pagos de deuda activos** en ese mes (`projectedMonthlySavings` + `isDebtPaymentActive`).
2. **Distribución de aportaciones a activos:** primero contribuciones **fijas** (escaladas si superan el ahorro disponible), luego **porcentaje del remanente** (normalizado si suman más de 100%).
3. **Crecimiento mensual compuesto** por activo según tasa anual y nuevas aportaciones.
4. **“Upcoming” con fecha:** impacto **acumulado firmado** entre referencia y cada punto temporal.
5. **“Upcoming” sin fecha:** fracción **lineal en ~90 días** desde la fecha de referencia (`distributedUndatedUpcomingAdjustment`) tanto para el neto como para el **refuerzo positivo** que alimenta aportaciones.
6. **Gastos próximos:** deducen valor de activos con una **cola de drenaje** (primero líquidos, luego menor rentabilidad esperada); lo no cubierto acumula “undrained” y ajusta capital aportado.
7. **Patrimonio neto proyectado:** punto de partida + ajustes upcoming firmados + ahorros acumulados + **contribución del “growth”** de activos (valor − coste inicial − capital aportado acumulado por activo), menos gastos no cubiertos según el modelo.
8. **Modo inflación:** convierte la serie a “dinero de hoy” opcionalmente.
9. **Capital aportado acumulado** como serie paralela (`contributedCapital`).

Horizonte en años: derivado de **edades de nacimiento + edad objetivo** con límites 5–70 años (`projectionHorizonYears`); fallback 30 años si faltan fechas.

La UI de proyección además calcula **hitos visuales** (milestones en la gráfica) sobre la serie — lógica local en `ProjectionTabView.swift`.

### Jubilación / FIRE (pestaña Retirement)

Construido sobre la **misma serie de patrimonio neto** más un modelo de **fases**:

- **Gasto anual objetivo en jubilación:** manual, igual al gasto actual anualizado, o gasto actual ± porcentaje (`FireRetirementExpenseMode`).
- **Retiradas:** tasa fija (p.ej. 4%) y modo “auto” declarado en dominio (la unificación práctica de tasas está en `resolvedWithdrawalRatePercent`).
- **Colchón / padding de seguridad** multiplicador sobre el capital objetivo.
- **Pensiones por persona:** importe mensual, edad de inicio → stream con `startMonthIndex` relativo a la edad actual.
- **Fases:** cada vez que entra un nuevo stream de pensión, se recalcula ingreso neto de pensión (tras impuesto por tramos), necesidad anual cubierta por cartera, y **capital objetivo** vía función que **invierte** el problema “¿qué capital produce X neto tras CGT?” usando **búsqueda binaria** y modelo de retirada bruta − impuesto sobre la parte ganancia (`requiredPortfolioForNetNeed`, `annualNetWithdrawal`).
- **Ratio imponible de ganancias** a partir de coste vs valor de activos (`taxableGainRatio`).
- **Tramos impositivos** normalizados para ganancias de capital y para renta de pensión (por defecto España en `AppState`).
- **Estado del hito:** alcanzado antes/después de pensión o no alcanzado en el horizonte; fecha o edad estimada cuando la proyección cruza el objetivo de la fase activa (`firstReachMonth` + `activePhase`).

Persistencia específica FIRE: JSON por hogar en SQLite (`PersistedFireSettings` en `AppState`).

---

## Nice to have (solo tras paridad de capacidades con el Mac)

Lo que ya existe en el Mac como parte usable del producto (donuts en Summary, gráficos en Upcoming, zoom/pan e hitos en proyección, multi-login self-hosted, etc.) es **MVP**, no “nice to have”; véanse vistas en `renderer/Views`.

- **Post-paridad:** import bancario automático, más monedas o FX, Monte Carlo, amortización tipo tabla de préstamo, recordatorios push, PWA offline, etc.

**Sin** puente ni migración desde el Mac obsoleto. **Sin** modo demo, datos ficticios ni categorías por defecto — política cerrada en la sección MVP.

---

## Complejidad digna de mención (para roadmap y tests)

- **Simulación mensual:** muchas fuentes acopladas (ahorro, deuda, upcoming fechado/no fechado, drenaje de activos, crecimiento, inflación). Un cambio pequeño desplaza FIRE y la gráfica.
- **Upcoming sin fecha:** convención explícita de reparto lineal en ~90 días; debe mantenerse como spec si se reimplementa.
- **Contribuciones a activos:** interacción entre techo de ahorro, boost por upcoming positivo, y escalado cuando los fijos superan el ahorro.
- **FIRE por fases:** pensión escalonada, objetivos de cartera por fase, impuestos por tramos, búsqueda binaria sobre capital.
- **Multi-usuario (nuevo):** el Mac actual es un solo espacio de datos con filtro por persona; el servidor necesita modelo de hogares, invitaciones, roles y visibilidad.

---

## Opciones estratégicas (sin elegir stack todavía)

Objetivo declarado: **Docker + self-host + acceso desde otros dispositivos + multi-usuario**.

- **MVP Docker:** **paridad de capacidades** con el Mac según código (must-haves por defecto); exclusión solo de **artefactos de arquitectura Mac**. **Implementación libre** y optimizada para Docker/web. Incluye backup monofichero propio **y** intercambio CSV multi-dataset como **capacidad** (UX adaptada). **Cero** migración desde la app macOS obsoleta.
- **Reimplementación desde cero en el nuevo repo:** el binario macOS **no** es el runtime del futuro; se construye **front + API** (stack a elegir) que cumplan los mismos **resultados numéricos y reglas micro**, validados contra `tests/coreTests` y `tests/desktopTests` como oráculos. Envolver el `.app` no es estrategia viable para multi-dispositivo ni para la línea deprecada que se quiere apagar.
- **Datos y soberanía:** volumen Docker + motor de persistencia; **backup monofichero** para el usuario (off-site) forma parte del **MVP**; copias a nivel infra (snapshots de volumen/BD) son complementarias, no sustituto del export desde la app.
- **Seguridad multi-usuario:** HTTPS detrás de reverse proxy, sesiones, hashing de contraseñas, y límites de API son parte del producto “self-hosted financiero”, no solo infra.

**Repositorio:** el **nuevo proyecto es el repositorio principal** del producto FutureFin en adelante. El repo macOS puede archivarse o quedar congelado; no hay programa de migración de usuarios ni compatibilidad hacia atrás con sus datos.

---

## Multi-usuario (cerrado en especificación)

Decisión de producto reflejada en [`docs/spec/AUTH_MODEL.md`](../spec/AUTH_MODEL.md):

- **Un hogar por instalación** (sin selector ni creación libre de varios hogares).
- **Nombre del hogar no editable** por el usuario en MVP.
- **Invitaciones:** solo el **`owner`** aprueba el alta de nuevos usuarios al hogar.
- **Roles:** `owner`, `member`, `viewer` (este último recomendado si el esfuerzo es bajo).
- **Datos:** un único conjunto persistido; **vista individual vs conjunta** es filtro de UI, sin ocultación servidor entre miembros en MVP.
