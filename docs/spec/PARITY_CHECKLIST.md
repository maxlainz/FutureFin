# Checklist MVP — alcance funcional y criterios de terminado

Este documento fija el **alcance mínimo** para considerar el MVP “terminado” y evitar trabajo a medias.

## Política global

| Área | MVP |
|------|-----|
| Alcance | **Capacidades de usuario** definidas en este checklist; **implementación libre** (web, API, BD). |
| Datos iniciales | **Sin** hogar demo, **sin** `demo()` en memoria, **sin** categorías por defecto insertadas por la app. Estado vacío hasta creación o import. |
| Persistencia rota | Error explícito / healthcheck fallido; **no** datos ficticios. |
| Import/export | Solo formatos del producto: ZIP CSV y backup monofichero según [BACKUP_AND_CSV_SPEC.md](./BACKUP_AND_CSV_SPEC.md). |

## Shell de aplicación

- [x] Navegación principal con **7 pestañas**: Resumen, Activos, Pasivos, Presupuesto, Próximos, Jubilación, Proyección.
- [x] Barra de error global cuando hay `lastErrorMessage` equivalente (validación / persistencia). *(Web: región `app-global-errors` con `aria-live` y banners por dominio — sesión, instalación, categorías, activos, etc.)*
- [x] Modo de vista: **hogar (todo)** vs **usuario actual** (solo filas con `owner_user_id` = sesión); mismo dataset persistido; ver [`AUTH_MODEL.md`](./AUTH_MODEL.md).
- [x] Ajustes accesibles como pestaña dedicada (**Ajustes**).

## Summary

- [x] KPIs: Net Worth, Total Assets, Total Liabilities, Debt/Assets ratio (`SummaryMetricGrid`). *(Web + `GET /v1/summary`; purga de pasivos vencidos antes de agregar.)*
- [x] Financial health grid (MVP): ingresos/gastos/cuotas derivadas/neto/tasas, runway en líquidos, sumas de próximos y cobertura — objeto `financial_health` en `GET /v1/summary` alineado al mismo cómputo que `GET /v1/budget`.
- [x] Desglose por categoría (activos, pasivos) y por etiqueta `type_tag` en resumen — `GET /v1/summary` + tablas/barras en pestaña **Resumen**. *(Hover/tooltip pendiente.)*
- [x] Ayuda en línea MVP (`InlineHint`, ícono «i» con tooltip) en paneles clave: FIRE/resumen, presupuesto derivado, próximos, proyección, pasivos (derivar principal).

## Assets

- [x] Tabla/registro MVP: nombre, categoría, valor, compra, **Δ compra** (retorno acumulado vs compra), líquido, notas + CRUD API.
- [x] CRUD y filtro por persona (`view=mine` / hogar).

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

- [x] Tarjetas KPI **MVP**: patrimonio neto, gasto anual estimado (×12), objetivo 25×, gap, tasa implícita gastos/NW, meses lineales hasta 25× (si ahorro neto > 0), neto mensual presupuesto — `GET /v1/fire/snapshot` + pestaña **Jubilación**.
- [ ] Modos de gasto en jubilación: manual / actual / actual ± % (`FireRetirementExpenseMode`).
- [ ] Retirada: modo fijo vs auto (`FireWithdrawalMode`), tasa, padding de seguridad.
- [ ] Pensiones por persona (importe mensual, edad inicio).
- [ ] Tramos IRPF ganancias capital + tramos renta pensión (editable; defaults España opcional).
- [x] Copy orientativa sobre **nominal vs «dinero de hoy»** cuando inflación está activa en la instalación (pestaña **Jubilación**).

## Projection

- [x] Serie mensual de patrimonio neto **MVP** — `GET /v1/projection/series` (lineal NW₀ + t×neto presupuesto) + gráfico SVG en **Proyección**.
- [ ] Modo inflación acorde a hogar.
- [ ] Interacciones: zoom, pan, hitos/marcadores (`ProjectionTabView`); baseline de hitos usando lógica equivalente a `upcomingNetForMilestoneBaseline`.

## Settings — Installation & Members

- [x] Ajustes del hogar: moneda base, inflación de proyección, edad objetivo horizonte (**un hogar por instalación; nombre del hogar no editable por usuario en MVP**). *(Moneda en setup inicial; inflación / edad objetivo / `show_age_mode` editables por propietario vía `PATCH /v1/installation` y formulario **Proyección y modo de edad** en Ajustes.)*
- [x] CRUD personas: nombre, primaria, fecha de nacimiento — tabla `persons`, `GET/POST/PATCH/DELETE /v1/persons`, UI **Ajustes**. *(Eliminar: si era titular se promueve otra persona.)*
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

- [x] Validación MVP en API handlers (rangos, obligatoriedad, coherencia básica por recurso).

## Motor de negocio (invisible pero obligatorio)

- [x] Totales tipo **`snapshot`** (activos/pasivos/neto/ratio + **desgloses**) y **`financialHealthMetrics`** MVP en `GET /v1/summary` / presupuesto.
- [ ] **`projectNetWorthSeries`** y **`fireMilestone`** completos en servidor.

## Multi-usuario

- [x] Login, **una membresía de hogar por instalación**, roles, **flujo de invitación con aceptación del owner** — [`AUTH_MODEL.md`](./AUTH_MODEL.md). *(Sesión cookie, `/v1/installation`, aprobación pendientes en Ajustes.)*

## Principio UX “Excel con esteroides”

- [x] Sin botón global **«Calcular»**; KPIs y listas se actualizan al guardar / cambiar pestaña / recargar datos (paridad mental MVP; debounce solo si hiciera falta por rendimiento).
