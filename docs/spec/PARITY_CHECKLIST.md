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

- [x] Ventana principal con navegación equivalente a **7 pestañas**: Summary, Assets, Liabilities, Budget, Upcoming, Retirement, Projection (`FinFutureApp.swift` / `RootTabView`). *(Web `apps/web`: las siete en la barra; **Jubilación** y **Proyección** con datos MVP — `GET /v1/fire/snapshot`, `GET /v1/projection/series` — más texto de gap vs motor Swift completo.)*
- [x] Barra de error global cuando hay `lastErrorMessage` equivalente (validación / persistencia). *(Web: región `app-global-errors` con `aria-live` y banners por dominio — sesión, instalación, categorías, activos, etc.)*
- [x] **PersonFilterBar** / modo de vista: **hogar (todo)** vs **usuario actual** (solo filas con `owner_user_id` = sesión); mismo dataset persistido; ver [`AUTH_MODEL.md`](./AUTH_MODEL.md). *(Web: selector discreto bajo pestañas + `view=mine` en API; persistencia en `localStorage`; escritorio solo con membresía activa — bootstrap vs pendiente vía `GET /v1/installation/session-context`.)*
- [x] Settings accesible como en Mac (macOS: ventana Settings; web: ruta `/settings` o modal persistente). *(Web: pestaña **Ajustes**.)*

## Summary

- [x] KPIs: Net Worth, Total Assets, Total Liabilities, Debt/Assets ratio (`SummaryMetricGrid`). *(Web + `GET /v1/summary`; purga de pasivos vencidos antes de agregar.)*
- [ ] **SummaryFireRow**: FIRE Target + FIRE ETA (modo fecha vs edad según `showAgeMode`), tiempo restante si aplica. *(Parcial: **Jubilación** + KPIs MVP en `GET /v1/fire/snapshot` — 25×, gap, meses lineales, tasa implícita — sin `fireMilestone` ni ETA oráculo Swift.)*
- [x] Financial health grid (MVP): ingresos/gastos/cuotas derivadas/neto/tasas, runway en líquidos, sumas de próximos y cobertura — objeto `financial_health` en `GET /v1/summary` alineado al mismo cómputo que `GET /v1/budget`. *(Los importes “próximos” son sumas de `expected_amount`, no oráculo golden vs Swift — ver [`ORACLE_TESTS.md`](./ORACLE_TESTS.md). KPI “sin planes de deuda” ≈ `monthly_net_excluding_derived_debt` + `savings_rate_excluding_derived_debt`.)*
- [x] Desglose por categoría (activos, pasivos) y por etiqueta `type_tag` en resumen — `GET /v1/summary` + tablas/barras en pestaña **Resumen**. *(Donuts MVP: gradiente cónico + leyenda en **Resumen**; hover/tooltip tipo Mac pendiente.)*
- [x] Ayuda en línea MVP (`InlineHint`, ícono «i» con tooltip) en paneles clave: FIRE/resumen, presupuesto derivado, próximos, proyección, pasivos (derivar principal). *(Swift `InlineHelpIcon` puede tener más puntos de anclaje.)*

## Assets

- [x] Tabla/registro MVP: nombre, categoría, valor, compra, **Δ compra** (retorno acumulado vs compra), líquido, notas + CRUD API. *(Pendientes respecto al Mac: APR/TAE implícita explícita, aportaciones recurrentes fijo / % remanente.)*
- [x] CRUD y filtro por persona (`view=mine` / hogar). *(Sin ordenación Mac explícita documentada en API; lista por `sort_index`.)*

## Liabilities

- [x] Registro con principal, categoría, etiqueta, `type_tag`, APR opcional, plan de pago (importe, frecuencia weekly/monthly, fecha fin).
- [x] Toggle **derivar principal desde plan de pago** (`derive_principal_from_plan`) con regla de intervalos alineada al dossier (mensual / semanal desde “hoy” civil de la instalación).
- [x] Purga de pasivos con plan vencido en rutas que lo requieren (`purge_expired_liabilities` antes de listados, resumen y presupuesto).

## Budget

- [x] Ingresos y gastos recurrentes persistidos; totales mensualizados y filas derivadas de pasivos con plan activo (`GET /v1/budget`).
- [x] Ordenación en web `sortBudgetEntriesMacStyle` (total por categoría ↓, nombre categoría, importe, etiqueta).
- [x] Filas **derivadas** de planes de pasivos **visibles** en presupuesto (tabla + totales `GET /v1/budget`).
- [ ] **Exclusión** de esas derivadas en **motor de proyección** (`projectNetWorthSeries`). *(Existe `GET /v1/projection/series` MVP lineal con neto mensual del presupuesto — incluye contexto de derivadas en totales de presupuesto; exclusión explícita en serie aún no aplicada.)*

## Upcoming (Planning)

- [x] Lista de flujos con dirección (vía categoría income/expense), categoría, título, importe esperado, fecha opcional, notas (`GET /v1/planning/flows`).
- [x] Resumen direccional (sumas entradas / salidas / neto) en pestaña **Próximos**. *(Gráfico MVP de barras proporcionales SVG además de tarjetas.)*

## Retirement (FIRE)

- [x] Tarjetas KPI **MVP**: patrimonio neto, gasto anual estimado (×12), objetivo 25×, gap, tasa implícita gastos/NW, meses lineales hasta 25× (si ahorro neto > 0), neto mensual presupuesto — `GET /v1/fire/snapshot` + pestaña **Jubilación**. *(Sin ETA/oráculo `fireMilestone`; sin «fases».)*
- [ ] Modos de gasto en jubilación: manual / actual / actual ± % (`FireRetirementExpenseMode`).
- [ ] Retirada: modo fijo vs auto (`FireWithdrawalMode`), tasa, padding de seguridad.
- [ ] Pensiones por persona (importe mensual, edad inicio).
- [ ] Tramos IRPF ganancias capital + tramos renta pensión (editable como en Mac; defaults España opcional).
- [x] Copy orientativa sobre **nominal vs «dinero de hoy»** cuando inflación está activa en la instalación (pestaña **Jubilación**). *(Mac puede refinar textos con motor FIRE.)*

## Projection

- [x] Serie mensual de patrimonio neto **MVP** — `GET /v1/projection/series` (lineal NW₀ + t×neto presupuesto) + gráfico SVG en **Proyección**. *(Sin capital aportado acumulado ni `projectNetWorthSeries` Swift.)*
- [ ] Modo inflación acorde a hogar.
- [ ] Interacciones: zoom, pan, hitos/marcadores (`ProjectionTabView`); baseline de hitos usando lógica equivalente a `upcomingNetForMilestoneBaseline`.

## Settings — Installation & Members

- [x] Ajustes del hogar: moneda base, inflación de proyección, edad objetivo horizonte (**un hogar por instalación; nombre del hogar no editable por usuario en MVP**). *(Moneda en setup inicial; inflación / edad objetivo / `show_age_mode` editables por propietario vía `PATCH /v1/installation` y formulario **Proyección y modo de edad** en Ajustes.)*
- [x] CRUD personas: nombre, primaria, fecha de nacimiento — tabla `persons`, `GET/POST/PATCH/DELETE /v1/persons`, UI **Ajustes**. *(Eliminar: si era titular se promueve otra persona; **sin** `person_id` en activos — reasignación de ítems Mac no aplica en MVP.)*
- [x] `show_age_mode` en modelo instalación (`InstallationSnapshot`, `PATCH /v1/installation`, lectura en Resumen/Ajustes/Jubilación). *(Personas con DOB en API/UI.)*
- [x] Invitaciones: el **owner** aprueba altas en **Ajustes**; usuarios sin membresía no ven el escritorio (solo pendiente o bootstrap) — [`AUTH_MODEL.md`](./AUTH_MODEL.md).

## Settings — Backups (capacidad, UX web)

- [x] Export **ZIP** CSV MVP — `GET /v1/backup/export.zip` (solo **owner**); siete ficheros según [BACKUP_AND_CSV_SPEC.md](./BACKUP_AND_CSV_SPEC.md) (`summary_household`, `summary_people`, `categories`, `assets`, `liabilities`, `budget`, `planning`). *(Botón en **Ajustes**; sin cifrado.)*
- [ ] Import ZIP con confirmación destructiva y diagnósticos por sección.
- [ ] Export backup **monofichero** cifrado + import con contraseña (formato nuevo).

## Settings — Categories

- [x] CRUD categorías por ámbito (asset, liability, income, expense). *(Web modales + API.)*
- [x] Borrado con **remap** cuando la categoría está en uso — query `remap_to` en `DELETE /v1/categories/{id}` + modal en **Ajustes** (mismo ámbito).

## Validación

- [x] Validación MVP en API handlers (rangos, obligatoriedad, coherencia básica por recurso). *(Port exhaustivo de `ValidationService` Swift + reglas de aportaciones — pendiente.)*

## Motor de negocio (invisible pero obligatorio)

- [x] Totales tipo **`snapshot`** (activos/pasivos/neto/ratio + **desgloses**) y **`financialHealthMetrics`** MVP en `GET /v1/summary` / presupuesto — ver dossier; oráculos golden [`ORACLE_TESTS.md`](./ORACLE_TESTS.md) pendientes.
- [ ] **`projectNetWorthSeries`** y **`fireMilestone`** completos en servidor (y tests cruzados Swift).

## Multi-usuario (nuevo respecto al Mac)

- [x] Login, **una membresía de hogar por instalación**, roles, **flujo de invitación con aceptación del owner** — [`AUTH_MODEL.md`](./AUTH_MODEL.md). *(Sesión cookie, `/v1/installation`, aprobación pendientes en Ajustes.)*

## Principio UX “Excel con esteroides”

- [x] Sin botón global **«Calcular»**; KPIs y listas se actualizan al guardar / cambiar pestaña / recargar datos (paridad mental MVP; debounce solo si hiciera falta por rendimiento).
