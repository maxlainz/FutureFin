# Plan de implementación de F2 — API y MCP de «Planes» (6.0.0)

Plan ejecutable de la fase F2 del [RFC](planes-6.0.md). Lo sigue cualquier sesión que continúe el
trabajo: cada PR sale de `next/6.0` en una rama `feat/6.0-f2-…`, vuelve por PR a `next/6.0` y tiene que
quedar en verde por sí solo (tests de integración con Postgres incluidos; web typecheck/tests si toca
la web). Ningún commit ni PR lleva atribución de herramienta.

**Precondición**: F1a, F1b y F1c mergeados en `next/6.0` (motor: `start_month`,
`income_expense_changes` + `build_change_vectors`, `DeficitPolicy::SinkOnly` + ventas explícitas +
`build_asset_sale_orders`, `McOutcome::uncovered_free_probability`).

## Decisiones del owner tomadas al planificar (2026-10-17)

- **D-31**: al retirar las fechas de fin del presupuesto (D-18), las partidas con fecha de fin
  futura **se borran sin importarlas** al plan. El gasto pasa a ser indefinido; se declara en el
  CHANGELOG.
- **D-32**: los «Próximos» importados (D-27) entran con `keeps_purchasing_power = false` (importe
  fijo): su caja queda idéntica. «Importe fijo» es el modo correcto para un pago aplazado sin
  intereses o cualquier cosa de valor nominal cerrado: se toma tal cual, sin indexar ni deflactar.

- **D-33 (rendimiento)**: **ninguna página espera a Monte Carlo.** En la 5.0, `/v1/summary`
  ejecuta el solve de nivel 1 dentro de la petición (4-7 s en frío; con tres peticiones frías a la vez,
  más de 10 s) y todos los KPIs esperan a que termine. En 6.0: el Resumen y la serie solo devuelven
  datos deterministas; la probabilidad de éxito del plan se calcula **en segundo plano**, con
  single-flight y clave por contenido del plan (`plans_stamp`), y se publica en su propio endpoint
  o campo de estado (`computing` | `ready`) que la SPA consulta con un indicador de carga, sin
  bloquear el resto. Warm-up al arrancar y al reanudar sesión, no solo en el login. Test de
  presupuesto de latencia: Resumen en frío < 300 ms. El owner decidió **no** sacar parche 5.0.x.

## Regla única de meses

Módulo nuevo `apps/api/src/handlers/plans/engine_map.rs`, `loop_month_of(today, date) -> u32` =
`12·Δaños + Δmeses + 1`; una fecha pasada → 1. Es la misma rejilla que el mapeo legacy de flujos
(hueco `idx` = mes del bucle `idx+1`) y que F1a (lo que empieza en `s` aparece en el índice `s−1`).
`end_date` inclusivo, mismo mapeo.

## Migraciones

1. **F2-1 `…_plans.sql`** (aditiva): `plans` (`owner_user_id` NULL = plan de hogar, solo esquema
   hasta F3; `name`; `is_active`; `assumptions` JSONB; `revision`; `sort_index`; timestamps; índices
   únicos parciales «un activo por owner» y «un activo de hogar»), `plan_blocks` (un bloque «General»
   al crear el plan) y `plan_changes` (`kind` ∈ income|expense|asset_in|asset_sale|loan, `start_date`,
   `end_date`, `amount` con signo en euros de hoy, `amount_unit` eur|pct, `cadence` monthly|one_off,
   `keeps_purchasing_power`, `asset_id` → assets ON DELETE SET NULL, `target_change_id` → el
   `asset_in` del propio plan, `params` JSONB validado por enum serde, `enabled`, CHECK de forma por
   tipo: pct solo mensual; asset_sale con exactamente uno de asset_id/target_change_id; loan siempre
   nominal). `PlanAssumptions` tipado (`deny_unknown_fields` en escritura):
   `inflation_override_pct` [−2, 50], `income_follows_inflation` (true),
   `expense_follows_inflation` (true), `horizon_age` 90 [85, 105], `market_return_adjustment_pp` 0
   [−5, 5]. El override del reparto (D-15) queda fuera de F2.
2. **F2-2 `…_drop_users_retirement_profile.sql`**: pérdida de datos aceptada (D-7); CHANGELOG.
3. **F2-5**: `…_import_planning_flows_into_plans.sql` (D-27/D-30/D-32: plan «Importado» por owner con
   un bloque «Simulación importada»; activo si algún flujo estaba encendido y el owner no tiene otro;
   one_off sin fecha → hoy (calendar_tz) + 90 días; per_month → monthly con su ventana; categoría y
   `show_in_chart` se pierden), `…_drop_planning_flows.sql`,
   `…_budget_entries_drop_retirement_flags.sql` (D-18/D-31: sin importar). Backup
   `CURRENT_SCHEMA_VERSION` 14 una sola vez, con `payload_v13_to_v14` que aplica las mismas reglas.

## Handlers y rutas (F2-1)

`apps/api/src/handlers/plans/{mod,crud,changes,validate,engine_map}.rs` con cores compartidos con MCP:
listar/leer (view `mine` por defecto, `household` solo lectura), crear/editar/borrar plan, bloques y
cambios, `activate_plan_core(plan_id: Option)` (None = solo la realidad; transacción con `FOR UPDATE`,
desactivar antes de activar; 23505 concurrente → 409). Toda escritura: `require_row_owner` (403
`not_row_owner`), `role_can_write`, sube `plans.revision`, y tras el commit
`refresh_projection_after_mutation` (invalidación completa por instalación, sin warm-up). Rutas
`/v1/plans`, `/v1/plans/{id}`, `POST /v1/plans/{id}/activate`, `POST /v1/plans/deactivate`,
`/v1/plans/{id}/blocks[/{bid}]`, `/v1/plans/{id}/changes[/{cid}]`; openapi; `error-codes.json`
(`plan_not_found`, `plan_change_invalid_shape`, `household_plans_not_available`,
`plan_not_in_scope`); `/v1/changes`. Tests `plans_crud.rs`, `plans_ownership.rs`.

## Cableado de la proyección

- **F2-3 (semántica 6.0 sin plan)**: `deficit_policy: SinkOnly`, ingresos y gastos siguen la
  inflación, horizonte 90 por defecto, fase de jubilación desactivada; colchón sin SWR (D-22, se
  retira `runway_is_indefinite`); campos nuevos `first_uncovered_month_index/date/series_position`,
  `deficit_coverage_net/tax` por punto, `deficit_policy`, `inflation_used_percent`,
  `inflation_source`. Bandas: éxito = `uncovered_free_probability`; fuera `success_threshold_pct`,
  `success_verdict`, `strategy`.
- **F2-4 (plan activo)**: `engine_map::apply_plan` → `build_change_vectors`,
  `build_asset_sale_orders`, activos y pasivos extra con `start_month` (valor nominal =
  `valor·f(s−1)` si mantiene poder adquisitivo; σ en el MISMO map que el vector de volatilidades;
  cuota francesa con una función pura nueva del motor), ajuste de mercado sobre cada rentabilidad.
  Una sola `EffectiveAssumptions` alimenta input y builders. `?plan_id=` en series y bandas (por
  defecto el activo; `none` = realidad; un borrador propio permitido para «Comparar»). Hogar = suma
  por miembro con el plan activo de cada uno (F3 lo unifica). Cache: `plans_stamp` (hash de
  `(plan_id, revision)` en scope) en las claves de series y bandas. Resumen: bloque `active_plan`
  (probabilidad de las bandas con la semilla estable, primer mes sin cubrir). Metas y
  `deflate_amount` usan el plan activo y ecoan la inflación.

## Retirada

- **F2-2 (API + web en el mismo PR)**: `retirement_profile.rs`, `retirement_solver.rs`, cachés de plan
  en `state.rs`, campos de jubilación y FIRE de `projection.rs`, equivalentes de jubilación de
  `budget.rs`, `summary_plan`; web: `RetirementView` y helpers, paso de onboarding, tarjeta «Tu plan»,
  `/jubilacion` redirige; `fire-parity.json`. Tests asociados borrados o reescritos.
- **F2-5**: `planning.rs`, `upcoming_*` del Resumen, tramo `planning` → `plan`; web: `UpcomingView`
  (redirecciones hasta F4) y la UI de los flags del presupuesto.
- **F2-7 (motor, solo cuando la API ya no lo use)**: `phases.rs`, `target.rs`, `withdrawal.rs`,
  `solve.rs`, `LegacyDrain`, solves estocásticos y sus tests. Pines: capturar `pins-6.0.json` sobre
  la batería 6.0 ANTES de borrar y exigir que no cambie después; `pins-4.15.json` y
  `pins-5.0-outputs.json` se retiran (decisión de change-control declarada en CHANGELOG); la batería
  de degeneración se porta (≥ 22 casos).

## MCP

Borrar `get/update_retirement_profile` (F2-2) y las cuatro tools de planning flows (F2-5); reescribir
en el mismo PR que cambia cada forma `get_projection`, `get_projection_bands`, `get_summary`,
`deflate_amount`, `list_goals`; las tools de budget entries rechazan los campos retirados con 400
`field_retired_use_plan_change`. F2-6 añade `list/get/create/update/delete_plan`, `activate_plan`
(sin preview, devuelve el impacto) y CRUD de bloques y cambios (`delete_plan` y `delete_plan_block`
con two-phase + `confirm_token`); todas con `require_mcp_write`. `simulate_projection` pasa a
«simular un plan sin activarlo». Instrucciones del servidor: sección PLANES. Fixtures y tests del
catálogo en cada PR; fila de hueco pendiente en la skill de paridad mientras dure F2.

## PRs y aceptación

| PR | Contenido | Debe demostrar |
|---|---|---|
| F2-1 | Tablas + CRUD | Ninguna cifra se mueve; tests de propiedad y la carrera 409 |
| F2-2 | Retirada de la jubilación (API, 2 tools MCP, web) | Pines `?months=` intactos; web typecheck y vitest |
| F2-3 | Semántica 6.0 sin plan | Pines de API recapturados con la cifra predicha en el PR antes de ejecutar |
| F2-4 | Plan activo, `?plan_id`, cache, Resumen, metas, bandas | Tests con cifras predichas (abajo) |
| F2-5 | Migración de importación, D-18, backup v14, retirada de planning | Test de migración parcial; paridad SQL ↔ v13→v14 |
| F2-6 | Tools MCP de planes, `simulate_projection`, instrucciones | Catálogo congelado; presupuesto de descripciones medido |
| F2-7 | Limpieza del motor | `pins-6.0.json` sin cambios; puerta de degeneración en verde |

## Riesgos (cifras mal en silencio) y su prueba

1. Mes desplazado: gasto puntual X en el mes civil +6, retornos e inflación 0 →
   `nw[7] − nw[6] = neto − X`; si X supera el sumidero, `first_uncovered_month_index = 6`.
2. Euros de hoy: override 2 %, +100 €/mes con poder adquisitivo desde s = 13, ingreso base 0 →
   delta de caja 102,0000 en el mes 13; sin poder adquisitivo, 100.
3. Override desalineado: `inflation_used_percent` = override y `net_worth_real[12] = nominal/1,02`.
4. Indexación del ingreso: ingreso 3.000, gasto 2.000, inflación 2,5 %, sumidero en efectivo al 0 % →
   `NW(12) ≈ 12.136,9` frente a 12.000 con ingreso plano.
5. SinkOnly: sumidero líquido de 10k, déficit 1k/mes, base sin plusvalía →
   `first_uncovered_month_index = 10`; el ilíquido intacto.
6. Volatilidades desalineadas por activos del plan: ampliar
   `the_volatility_vector_follows_the_asset_order`.
7. Cache: activar, editar un cambio, desactivar → MISS con cifras nuevas en cada paso.
8. Hogar: `household = Σ mine(miembro)` exacto con planes distintos.
9. Importación: la caja mensual nominal de los flujos con fecha antes de migrar = los vectores
   nuevos; solo los sin fecha se mueven (de repartirse en 90 días a un pago a +90 días).
