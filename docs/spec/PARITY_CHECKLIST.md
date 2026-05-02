# Checklist de paridad MVP — referencia cliente macOS (Swift)

Fuente de comportamiento: código en `FutureFin` / `FinFuture` (`renderer/`, `src/core/`, `tests/`). El README del repo Swift **no** es fiable.

## Política global

| Área | MVP |
|------|-----|
| Paridad | **Capacidades de usuario** iguales al Mac según código; **implementación libre** (web, API, BD). |
| Oráculos numéricos | Tests Swift listados en [`ORACLE_TESTS.md`](./ORACLE_TESTS.md). |
| Datos iniciales | **Sin** hogar demo, **sin** `demo()` en memoria, **sin** categorías por defecto insertadas por la app. Estado vacío hasta creación o import. |
| Persistencia rota | Error explícito / healthcheck fallido; **no** datos ficticios. |
| Cliente Mac obsoleto | **Sin** import de `.ffbackup` ni datos legacy Mac; solo formatos **nuevos** ([BACKUP_AND_CSV_SPEC.md](./BACKUP_AND_CSV_SPEC.md)). |

## Exclusiones explícitas (no paridad)

- Migración `FinFuture` → `FutureFin` en `~/Library/Application Support`.
- SQLite local del usuario y rutas de escritorio.
- `FolderDialog` / flujo “elegir carpeta” del Finder (sustituir por ZIP/download/upload).
- Configuración de ventana macOS (autosave frame, tabbing).
- Build `.app` / `build_macos_app.sh` como artefacto del producto servidor.
- `NSAboutPanel` (sustituir por página/modal “Acerca de” web si se desea la misma información).
- Primera ejecución con `seedDefaultData`, `defaultCategories` automáticas, o `AppState.demo()`.

## Shell de aplicación

- [x] Ventana principal con navegación equivalente a **7 pestañas**: Summary, Assets, Liabilities, Budget, Upcoming, Retirement, Projection (`FinFutureApp.swift` / `RootTabView`). *(Web `apps/web`: las siete están en la barra; Jubilación y Proyección llevan vistas esqueleto con texto de paridad hasta motor FIRE/gráfico.)*
- [x] Barra de error global cuando hay `lastErrorMessage` equivalente (validación / persistencia). *(Web: región `app-global-errors` con `aria-live` y banners por dominio — sesión, instalación, categorías, activos, etc.)*
- [x] **PersonFilterBar** / modo de vista: **hogar (todo)** vs **usuario actual** (solo filas con `owner_user_id` = sesión); mismo dataset persistido; ver [`AUTH_MODEL.md`](./AUTH_MODEL.md). *(Web: barra bajo pestañas + query API `view=mine`.)*
- [x] Settings accesible como en Mac (macOS: ventana Settings; web: ruta `/settings` o modal persistente). *(Web: pestaña **Ajustes**.)*

## Summary

- [x] KPIs: Net Worth, Total Assets, Total Liabilities, Debt/Assets ratio (`SummaryMetricGrid`). *(Web + `GET /v1/summary`; purga de pasivos vencidos antes de agregar.)*
- [ ] **SummaryFireRow**: FIRE Target + FIRE ETA (modo fecha vs edad según `showAgeMode`), tiempo restante si aplica. *(Parcial: Resumen muestra bloque **solo configuración** — inflación proyección, edad horizonte, `show_age_mode` persistidos en instalación; sin ETA hasta motor FIRE.)*
- [ ] Financial health grid: ingresos, gastos, ahorro, savings rate, runway, upcoming totals, coverage ratio; KPIs opcionales “sin planes de deuda” (`monthlySavingsWithoutLiabilityPaymentPlans`).
- [ ] Gráficos donut / breakdown por categoría activos y pasivos (`HoverDonutChart`, `SummaryBreakdownCharts`).
- [ ] `InlineHelpIcon` / textos de ayuda equivalentes donde existan en SwiftUI.

## Assets

- [ ] Tabla/registro: nombre, categoría, valor, APR opcional vía retorno implícito si hay purchase price, aportaciones recurrentes (fijo / % remanente), líquido, notas.
- [ ] CRUD, ordenación y filtros por persona alineados a `AssetsTabView`.

## Liabilities

- [ ] Registro con principal, categoría, etiqueta de tipo, APR opcional, plan de pago (importe, frecuencia weekly/monthly, fecha fin).
- [ ] Toggle **derivar principal desde plan de pago** con la misma regla de intervalos que `LiabilitiesTabView` (mensual / semanal desde startOfDay).
- [ ] Purga de pasivos con plan vencido al iniciar sesión o equivalente (“startup hook”) como `purgeFinishedPaymentPlans`.

## Budget

- [ ] Ingresos y gastos recurrentes; totales y filas por categoría.
- [ ] Ordenación: por total de categoría descendente, luego categoría, importe, nombre (`BudgetTabView.sortedEntries`).
- [ ] Filas **derivadas** de planes de pago de pasivos visibles en presupuesto pero **excluidas** del motor de proyección de patrimonio (solo entradas persistidas en `budget` para la serie).

## Upcoming (Planning)

- [ ] Lista de flujos con dirección, categoría, título, importe esperado, fecha opcional, notas.
- [ ] Gráfico / resumen direccional si existe en `PlanningTabView`.

## Retirement (FIRE)

- [ ] Tarjetas KPI: objetivo de cartera, gasto anual objetivo, ETA, gap, fases. *(Web: pestaña **Jubilación** con secciones etiquetadas y lectura de horizonte instalación; KPIs numéricos pendientes de API.)*
- [ ] Modos de gasto en jubilación: manual / actual / actual ± % (`FireRetirementExpenseMode`).
- [ ] Retirada: modo fijo vs auto (`FireWithdrawalMode`), tasa, padding de seguridad.
- [ ] Pensiones por persona (importe mensual, edad inicio).
- [ ] Tramos IRPF ganancias capital + tramos renta pensión (editable como en Mac; defaults España opcional).
- [ ] Copy “dinero de hoy” cuando inflación ajustada en hogar.

## Projection

- [ ] Serie mensual de patrimonio neto + capital aportado acumulado. *(Web: pestaña **Proyección** esqueleto + uso de flags de instalación; serie y gráfico pendientes.)*
- [ ] Modo inflación acorde a hogar.
- [ ] Interacciones: zoom, pan, hitos/marcadores (`ProjectionTabView`); baseline de hitos usando lógica equivalente a `upcomingNetForMilestoneBaseline`.

## Settings — Installation & Members

- [x] Ajustes del hogar: moneda base, inflación de proyección, edad objetivo horizonte (**un hogar por instalación; nombre del hogar no editable por usuario en MVP**). *(Moneda en setup inicial; inflación / edad objetivo / `show_age_mode` editables por propietario vía `PATCH /v1/installation` y formulario **Proyección y modo de edad** en Ajustes.)*
- [ ] CRUD personas: nombre, primaria, fecha de nacimiento; eliminar con reasignación de ítems.
- [ ] `show_age_mode` u homónimo en API si está en modelo hogar.
- [ ] Invitaciones: el **owner** aprueba altas de otros **users** antes de que tengan acceso al hogar ([`AUTH_MODEL.md`](./AUTH_MODEL.md)).

## Settings — Backups (capacidad, UX web)

- [ ] Export **ZIP** conteniendo los mismos CSV que `importExportCSVFilenames` (ver spec backup).
- [ ] Import ZIP con confirmación destructiva y diagnósticos por sección.
- [ ] Export backup **monofichero** cifrado + import con contraseña (formato nuevo).

## Settings — Categories

- [x] CRUD categorías por ámbito (asset, liability, income, expense). *(Web modales + API; borrado sin remap.)*
- [ ] Borrado con flujo **remap** cuando la categoría está en uso.

## Validación

- [ ] Reglas equivalentes a `ValidationService` (rangos, obligatoriedad, coherencia aportaciones).

## Motor de negocio (invisible pero obligatorio)

- [ ] `SummaryService.snapshot`, `financialHealthMetrics`, `projectNetWorthSeries`, `fireMilestone` — contratos micro en dossier y [`ORACLE_TESTS.md`](./ORACLE_TESTS.md).

## Multi-usuario (nuevo respecto al Mac)

- [x] Login, **una membresía de hogar por instalación**, roles, **flujo de invitación con aceptación del owner** — [`AUTH_MODEL.md`](./AUTH_MODEL.md). *(Sesión cookie, `/v1/installation`, aprobación pendientes en Ajustes.)*

## Principio UX “Excel con esteroides”

- [ ] Sin botón global “Calcular”; KPIs y gráficos **reactivos** ante cada cambio de campo (debounce opcional solo por rendimiento, no por modelo mental).
